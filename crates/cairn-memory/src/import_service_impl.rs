use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};

use crate::bundles::{
    ArtifactEntry, ArtifactKind, BundleEnvelope, ConflictResolutionStrategy, ImportOutcome,
    ImportPlan, ImportPlanEntry, ImportReport, ImportReportEntry, ImportService,
    KnowledgeDocumentPayload, SourceScope, ValidationReport,
};
use crate::in_memory::InMemoryDocumentStore;
use crate::ingest::{IngestRequest, IngestService};
use crate::pipeline::{
    bundle_source_type_to_ingest, compute_content_hash, extract_document_content_text,
    DocumentStore, IngestPipeline, ParagraphChunker,
};

pub struct InMemoryImportService {
    store: Arc<InMemoryDocumentStore>,
    ingest: IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>,
}

impl InMemoryImportService {
    pub fn new(store: Arc<InMemoryDocumentStore>) -> Self {
        let ingest = IngestPipeline::new(store.clone(), ParagraphChunker::default());
        Self { store, ingest }
    }

    fn target_project(target_scope: &SourceScope) -> Result<ProjectKey, String> {
        let tenant_id = target_scope
            .tenant_id
            .as_deref()
            .ok_or_else(|| "target scope missing tenant_id".to_owned())?;
        let workspace_id = target_scope
            .workspace_id
            .as_deref()
            .ok_or_else(|| "target scope missing workspace_id".to_owned())?;
        let project_id = target_scope
            .project_id
            .as_deref()
            .ok_or_else(|| "target scope missing project_id".to_owned())?;
        Ok(ProjectKey::new(tenant_id, workspace_id, project_id))
    }

    fn validation_errors_for_artifact(artifact: &ArtifactEntry) -> Vec<String> {
        let mut errors = Vec::new();

        if artifact.payload.is_null() {
            errors.push(format!(
                "artifact {} is missing payload",
                artifact.artifact_logical_id
            ));
            return errors;
        }

        if artifact.artifact_kind == ArtifactKind::KnowledgeDocument {
            let payload_value = artifact.payload.as_value();
            if serde_json::from_value::<KnowledgeDocumentPayload>(payload_value.clone()).is_err() {
                errors.push(format!(
                    "artifact {} has invalid knowledge_document payload",
                    artifact.artifact_logical_id
                ));
            } else if extract_document_content_text(&payload_value)
                .map(|text| text.trim().is_empty())
                .unwrap_or(true)
            {
                errors.push(format!(
                    "artifact {} has empty knowledge document content",
                    artifact.artifact_logical_id
                ));
            }
        }

        errors
    }

    async fn plan_entry(
        &self,
        artifact: &ArtifactEntry,
        project: Option<&ProjectKey>,
        existing_hashes: Option<&std::collections::HashSet<String>>,
    ) -> ImportPlanEntry {
        if artifact.payload.is_null() {
            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Conflict,
                reason: "artifact payload is missing".to_owned(),
                existing_id: None,
            };
        }

        if artifact.artifact_kind != ArtifactKind::KnowledgeDocument {
            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Conflict,
                reason: "artifact kind is not supported by curated knowledge-pack import"
                    .to_owned(),
                existing_id: None,
            };
        }

        let Some(_project) = project else {
            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Conflict,
                reason: "target scope must include tenant_id, workspace_id, and project_id"
                    .to_owned(),
                existing_id: None,
            };
        };

        let Some(existing_hashes) = existing_hashes else {
            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Conflict,
                reason: "could not inspect existing content hashes".to_owned(),
                existing_id: None,
            };
        };

        let payload_value = artifact.payload.as_value();
        let payload =
            match serde_json::from_value::<KnowledgeDocumentPayload>(payload_value.clone()) {
                Ok(payload) => payload,
                Err(_) => {
                    return ImportPlanEntry {
                        artifact_logical_id: artifact.artifact_logical_id.clone(),
                        artifact_kind: artifact.artifact_kind,
                        outcome: ImportOutcome::Conflict,
                        reason: "artifact payload is not a valid knowledge document".to_owned(),
                        existing_id: None,
                    }
                }
            };

        let Some(content) = extract_document_content_text(&payload_value) else {
            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Conflict,
                reason: "knowledge document payload does not contain importable content".to_owned(),
                existing_id: None,
            };
        };

        let normalized_hash = compute_content_hash(&content);
        if existing_hashes.contains(&normalized_hash) {
            let existing_id = self
                .store
                .get_status(&KnowledgeDocumentId::new(&artifact.artifact_logical_id))
                .await
                .ok()
                .flatten()
                .map(|_| artifact.artifact_logical_id.clone());

            return ImportPlanEntry {
                artifact_logical_id: artifact.artifact_logical_id.clone(),
                artifact_kind: artifact.artifact_kind,
                outcome: ImportOutcome::Skip,
                reason: "matching content hash already exists in the target project".to_owned(),
                existing_id,
            };
        }

        let source_type = bundle_source_type_to_ingest(Some(payload.source_type));
        let create_reason = format!(
            "new {} document content for target project",
            match source_type {
                crate::ingest::SourceType::PlainText => "plain_text",
                crate::ingest::SourceType::Markdown => "markdown",
                crate::ingest::SourceType::Html => "html",
                crate::ingest::SourceType::StructuredJson => "structured_json",
                crate::ingest::SourceType::KnowledgePack => "knowledge_pack",
                crate::ingest::SourceType::JsonStructured => "json_structured",
            },
        );

        ImportPlanEntry {
            artifact_logical_id: artifact.artifact_logical_id.clone(),
            artifact_kind: artifact.artifact_kind,
            outcome: ImportOutcome::Create,
            reason: create_reason,
            existing_id: None,
        }
    }

    async fn next_versioned_document_id(
        &self,
        artifact_logical_id: &str,
    ) -> Result<KnowledgeDocumentId, String> {
        let mut next_version = 2u32;
        loop {
            let candidate =
                KnowledgeDocumentId::new(format!("{artifact_logical_id}_v{next_version}"));
            if self
                .store
                .get_status(&candidate)
                .await
                .map_err(|e| e.to_string())?
                .is_none()
            {
                return Ok(candidate);
            }
            next_version += 1;
        }
    }

    async fn apply_duplicate_strategy(
        &self,
        strategy: ConflictResolutionStrategy,
        entry: &ImportPlanEntry,
        artifact: &ArtifactEntry,
        project: &ProjectKey,
        bundle_id: &str,
        source_type: crate::ingest::SourceType,
        content: String,
        existing_ids: &[KnowledgeDocumentId],
    ) -> Result<(ImportReportEntry, Option<&'static str>), String> {
        match strategy {
            ConflictResolutionStrategy::Skip => Ok((
                ImportReportEntry {
                    artifact_logical_id: entry.artifact_logical_id.clone(),
                    artifact_kind: entry.artifact_kind,
                    outcome: ImportOutcome::Skip,
                    reason: "matching content already exists; skipped".to_owned(),
                    created_object_id: existing_ids.first().map(ToString::to_string),
                },
                Some("skipped"),
            )),
            ConflictResolutionStrategy::Overwrite => {
                let target_id = if self
                    .store
                    .get_status(&KnowledgeDocumentId::new(&artifact.artifact_logical_id))
                    .await
                    .map_err(|e| e.to_string())?
                    .is_some()
                {
                    KnowledgeDocumentId::new(&artifact.artifact_logical_id)
                } else {
                    existing_ids[0].clone()
                };
                self.store.remove_document(&target_id);
                self.ingest
                    .submit(IngestRequest {
                        document_id: target_id.clone(),
                        source_id: SourceId::new(bundle_id),
                        source_type,
                        project: project.clone(),
                        content,
                        tags: vec![],
                        corpus_id: None,
                        bundle_source_id: None,
                        import_id: None,
                    })
                    .await
                    .map_err(|error| format!("overwrite ingest failed: {error}"))?;

                Ok((
                    ImportReportEntry {
                        artifact_logical_id: entry.artifact_logical_id.clone(),
                        artifact_kind: entry.artifact_kind,
                        outcome: ImportOutcome::Update,
                        reason: "matching content already existed; overwritten".to_owned(),
                        created_object_id: Some(target_id.to_string()),
                    },
                    Some("overwritten"),
                ))
            }
            ConflictResolutionStrategy::Rename => {
                let versioned_id = self
                    .next_versioned_document_id(&artifact.artifact_logical_id)
                    .await?;
                self.ingest
                    .submit(IngestRequest {
                        document_id: versioned_id.clone(),
                        source_id: SourceId::new(bundle_id),
                        source_type,
                        project: project.clone(),
                        content,
                        tags: vec![],
                        corpus_id: None,
                        bundle_source_id: None,
                        import_id: None,
                    })
                    .await
                    .map_err(|error| format!("versioned ingest failed: {error}"))?;

                Ok((
                    ImportReportEntry {
                        artifact_logical_id: entry.artifact_logical_id.clone(),
                        artifact_kind: entry.artifact_kind,
                        outcome: ImportOutcome::Create,
                        reason: "matching content already existed; imported with renamed ID"
                            .to_owned(),
                        created_object_id: Some(versioned_id.to_string()),
                    },
                    Some("versioned"),
                ))
            }
        }
    }
}

#[async_trait]
impl ImportService for InMemoryImportService {
    type Error = String;

    async fn validate(&self, bundle: &BundleEnvelope) -> Result<ValidationReport, Self::Error> {
        let mut errors = Vec::new();

        // RFC 013: "Every structured bundle must have one canonical envelope."
        // The bundle_schema_version MUST be present and match a supported version.
        const SUPPORTED_SCHEMA_VERSIONS: &[&str] = &["1"];
        if bundle.bundle_schema_version.trim().is_empty() {
            errors.push("bundle_schema_version is required".to_owned());
        } else if !SUPPORTED_SCHEMA_VERSIONS.contains(&bundle.bundle_schema_version.as_str()) {
            errors.push(format!(
                "unsupported bundle_schema_version '{}'; supported versions: {}",
                bundle.bundle_schema_version,
                SUPPORTED_SCHEMA_VERSIONS.join(", ")
            ));
        }

        if bundle.created_by.as_deref().map(str::trim).unwrap_or("").is_empty() {
            errors.push("bundle created_by is required".to_owned());
        }

        if bundle.artifact_count != bundle.artifacts.len() {
            errors.push(format!(
                "bundle artifact_count {} does not match artifacts.len() {}",
                bundle.artifact_count,
                bundle.artifacts.len()
            ));
        }

        for artifact in &bundle.artifacts {
            errors.extend(Self::validation_errors_for_artifact(artifact));
        }

        let valid = errors.is_empty();
        Ok(ValidationReport {
            errors,
            warnings: vec![],
            valid,
        })
    }

    async fn plan(
        &self,
        bundle: &BundleEnvelope,
        target_scope: &SourceScope,
    ) -> Result<ImportPlan, Self::Error> {
        let target_project = Self::target_project(target_scope).ok();
        let existing_hashes = match target_project.as_ref() {
            Some(project) => Some(
                self.store
                    .chunk_hashes_for_project(project)
                    .await
                    .map_err(|e| e.to_string())?,
            ),
            None => None,
        };

        let mut entries = Vec::with_capacity(bundle.artifacts.len());
        for artifact in &bundle.artifacts {
            entries.push(
                self.plan_entry(artifact, target_project.as_ref(), existing_hashes.as_ref())
                    .await,
            );
        }

        let (create_count, reuse_count, update_count, skip_count, conflict_count) =
            ImportPlan::summarize_counts(&entries);

        Ok(ImportPlan {
            bundle_id: bundle.bundle_id.clone(),
            target_scope: target_scope.clone(),
            conflict_resolution: ConflictResolutionStrategy::Skip,
            entries,
            create_count,
            reuse_count,
            update_count,
            skip_count,
            conflict_count,
        })
    }

    async fn apply(
        &self,
        plan: &ImportPlan,
        bundle: &BundleEnvelope,
    ) -> Result<ImportReport, Self::Error> {
        let project = Self::target_project(&plan.target_scope)?;
        let artifacts: HashMap<&str, &ArtifactEntry> = bundle
            .artifacts
            .iter()
            .map(|artifact| (artifact.artifact_logical_id.as_str(), artifact))
            .collect();

        let mut report_entries = Vec::with_capacity(plan.entries.len());

        for entry in &plan.entries {
            let artifact = artifacts.get(entry.artifact_logical_id.as_str()).copied();

            match entry.outcome {
                ImportOutcome::Create => {
                    let Some(artifact) = artifact else {
                        report_entries.push(ImportReportEntry {
                            artifact_logical_id: entry.artifact_logical_id.clone(),
                            artifact_kind: entry.artifact_kind,
                            outcome: ImportOutcome::Conflict,
                            reason: "artifact was present in plan but missing from bundle"
                                .to_owned(),
                            created_object_id: None,
                        });
                        continue;
                    };

                    let apply_payload_value = artifact.payload.as_value();
                    let payload = match serde_json::from_value::<KnowledgeDocumentPayload>(
                        apply_payload_value.clone(),
                    ) {
                        Ok(payload) => payload,
                        Err(_) => {
                            report_entries.push(ImportReportEntry {
                                artifact_logical_id: entry.artifact_logical_id.clone(),
                                artifact_kind: entry.artifact_kind,
                                outcome: ImportOutcome::Conflict,
                                reason: "artifact payload is not a valid knowledge document"
                                    .to_owned(),
                                created_object_id: None,
                            });
                            continue;
                        }
                    };

                    let Some(content) = extract_document_content_text(&apply_payload_value) else {
                        report_entries.push(ImportReportEntry {
                            artifact_logical_id: entry.artifact_logical_id.clone(),
                            artifact_kind: entry.artifact_kind,
                            outcome: ImportOutcome::Conflict,
                            reason:
                                "knowledge document payload does not contain importable content"
                                    .to_owned(),
                            created_object_id: None,
                        });
                        continue;
                    };

                    let source_type = bundle_source_type_to_ingest(Some(payload.source_type));
                    let normalized_hash = compute_content_hash(&content);
                    let existing_ids = self.store.document_ids_by_hash(&normalized_hash);
                    if !existing_ids.is_empty() {
                        let (report_entry, counter) = self
                            .apply_duplicate_strategy(
                                plan.conflict_resolution,
                                entry,
                                artifact,
                                &project,
                                &bundle.bundle_id,
                                source_type,
                                content.to_string(),
                                &existing_ids,
                            )
                            .await?;
                        let _ = counter;
                        report_entries.push(report_entry);
                        continue;
                    }

                    match self
                        .ingest
                        .submit(IngestRequest {
                            document_id: KnowledgeDocumentId::new(&artifact.artifact_logical_id),
                            source_id: SourceId::new(&bundle.bundle_id),
                            source_type,
                            project: project.clone(),
                            content,
                            tags: vec![],
                            corpus_id: None,
                            bundle_source_id: None,
                            import_id: None,
                        })
                        .await
                    {
                        Ok(()) => {
                            report_entries.push(ImportReportEntry {
                                artifact_logical_id: entry.artifact_logical_id.clone(),
                                artifact_kind: entry.artifact_kind,
                                outcome: ImportOutcome::Create,
                                reason: entry.reason.clone(),
                                created_object_id: Some(entry.artifact_logical_id.clone()),
                            })
                        }
                        Err(error) => report_entries.push(ImportReportEntry {
                            artifact_logical_id: entry.artifact_logical_id.clone(),
                            artifact_kind: entry.artifact_kind,
                            outcome: ImportOutcome::Conflict,
                            reason: format!("ingest failed: {error}"),
                            created_object_id: None,
                        }),
                    }
                }
                ImportOutcome::Skip => {
                    if plan.conflict_resolution != ConflictResolutionStrategy::Skip {
                        if let Some(artifact) = artifact {
                            let skip_pv = artifact.payload.as_value();
                            if let Ok(payload) = serde_json::from_value::<KnowledgeDocumentPayload>(
                                skip_pv.clone(),
                            ) {
                                if let Some(content) =
                                    extract_document_content_text(&skip_pv)
                                {
                                    let source_type =
                                        bundle_source_type_to_ingest(Some(payload.source_type));
                                    let existing_ids = self.store.document_ids_by_hash(
                                        &compute_content_hash(&content),
                                    );
                                    if !existing_ids.is_empty() {
                                        let (report_entry, counter) = self
                                            .apply_duplicate_strategy(
                                                plan.conflict_resolution,
                                                entry,
                                                artifact,
                                                &project,
                                                &bundle.bundle_id,
                                                source_type,
                                                content,
                                                &existing_ids,
                                            )
                                            .await?;
                                        let _ = counter;
                                        report_entries.push(report_entry);
                                        continue;
                                    }
                                }
                            }
                        }
                    }
                    report_entries.push(ImportReportEntry {
                        artifact_logical_id: entry.artifact_logical_id.clone(),
                        artifact_kind: entry.artifact_kind,
                        outcome: ImportOutcome::Skip,
                        reason: entry.reason.clone(),
                        created_object_id: entry.existing_id.clone(),
                    })
                }
                ImportOutcome::Conflict => report_entries.push(ImportReportEntry {
                    artifact_logical_id: entry.artifact_logical_id.clone(),
                    artifact_kind: entry.artifact_kind,
                    outcome: ImportOutcome::Conflict,
                    reason: entry.reason.clone(),
                    created_object_id: None,
                }),
                ImportOutcome::Reuse | ImportOutcome::Update => {
                    report_entries.push(ImportReportEntry {
                        artifact_logical_id: entry.artifact_logical_id.clone(),
                        artifact_kind: entry.artifact_kind,
                        outcome: entry.outcome,
                        reason: entry.reason.clone(),
                        created_object_id: entry.existing_id.clone(),
                    })
                }
            }
        }

        let mut create_count = 0;
        let mut reuse_count = 0;
        let mut update_count = 0;
        let mut skip_count = 0;
        let mut conflict_count = 0;
        for entry in &report_entries {
            match entry.outcome {
                ImportOutcome::Create => create_count += 1,
                ImportOutcome::Reuse => reuse_count += 1,
                ImportOutcome::Update => update_count += 1,
                ImportOutcome::Skip => skip_count += 1,
                ImportOutcome::Conflict => conflict_count += 1,
            }
        }

        Ok(ImportReport {
            bundle_id: bundle.bundle_id.clone(),
            target_scope: plan.target_scope.clone(),
            import_actor: bundle.created_by.clone(),
            entries: report_entries,
            create_count,
            reuse_count,
            update_count,
            skip_count,
            conflict_count,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use crate::bundles::{BundleEnvelope, BundleProvenance, BundleType, ImportService};

    fn minimal_bundle(schema_version: &str) -> BundleEnvelope {
        BundleEnvelope {
            bundle_schema_version: schema_version.to_owned(),
            bundle_type: BundleType::CuratedKnowledgePackBundle,
            bundle_id: "test_bundle".to_owned(),
            bundle_name: "Test Bundle".to_owned(),
            created_at: 0,
            created_by: Some("test_operator".to_owned()),
            source_deployment_id: None,
            source_scope: crate::bundles::SourceScope {
                tenant_id: Some("t1".to_owned()),
                workspace_id: Some("w1".to_owned()),
                project_id: Some("p1".to_owned()),
            },
            artifact_count: 0,
            artifacts: vec![],
            provenance: BundleProvenance {
                description: None,
                source_system: None,
                export_reason: None,
                origin: None,
                production_method: None,
                source_version: None,
            },
        }
    }

    /// RFC 013 §5.1: bundle_schema_version MUST be present.
    #[tokio::test]
    async fn validate_rejects_empty_schema_version() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let svc = InMemoryImportService::new(store);
        let bundle = minimal_bundle("");
        let report = svc.validate(&bundle).await.unwrap();
        assert!(
            !report.errors.is_empty(),
            "empty bundle_schema_version must produce errors"
        );
        assert!(
            report.errors.iter().any(|e| e.contains("bundle_schema_version")),
            "error must mention bundle_schema_version, got: {:?}", report.errors
        );
    }

    /// RFC 013 §5.1: unsupported bundle_schema_version MUST be rejected.
    #[tokio::test]
    async fn validate_rejects_unsupported_schema_version() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let svc = InMemoryImportService::new(store);
        let bundle = minimal_bundle("99");
        let report = svc.validate(&bundle).await.unwrap();
        assert!(
            !report.errors.is_empty(),
            "unsupported bundle_schema_version must produce errors"
        );
        assert!(
            report.errors.iter().any(|e| e.contains("unsupported") || e.contains("bundle_schema_version")),
            "error must mention unsupported schema, got: {:?}", report.errors
        );
    }

    /// RFC 013 §5.1: version "1" is the supported v1 schema version.
    #[tokio::test]
    async fn validate_accepts_schema_version_1() {
        let store = Arc::new(InMemoryDocumentStore::new());
        let svc = InMemoryImportService::new(store);
        let bundle = minimal_bundle("1");
        let report = svc.validate(&bundle).await.unwrap();
        assert!(
            report.errors.is_empty(),
            "version-1 bundle must pass schema validation, errors: {:?}", report.errors
        );
    }
}
