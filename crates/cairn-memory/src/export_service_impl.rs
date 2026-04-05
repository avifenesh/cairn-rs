use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::{ProjectKey, PromptAssetId};
use cairn_store::projections::{PromptAssetReadModel, PromptVersionReadModel};
use cairn_store::InMemoryStore;

use crate::bundles::{
    ArtifactEntry, ArtifactKind, BundleEnvelope, BundleProvenance, BundleSourceType, BundleType,
    DocumentContent, DocumentExportFilters, ExportService, KnowledgeDocumentPayload,
    PromptAssetPayload, PromptVersionPayload, SourceScope,
};
use crate::in_memory::{ExportableDocument, InMemoryDocumentStore};
use crate::pipeline::compute_content_hash;

pub struct InMemoryExportService {
    document_store: Arc<InMemoryDocumentStore>,
    prompt_store: Arc<InMemoryStore>,
    created_by: String,
}

impl InMemoryExportService {
    pub fn new(
        document_store: Arc<InMemoryDocumentStore>,
        prompt_store: Arc<InMemoryStore>,
        created_by: impl Into<String>,
    ) -> Self {
        Self {
            document_store,
            prompt_store,
            created_by: created_by.into(),
        }
    }

    fn now_millis() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn build_bundle_id(bundle_type: BundleType, bundle_name: &str, created_at: u64) -> String {
        let prefix = match bundle_type {
            BundleType::CuratedKnowledgePackBundle => "knowledge_pack",
            BundleType::PromptLibraryBundle => "prompt_library",
        };
        let slug = bundle_name
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() {
                    ch.to_ascii_lowercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        format!("{prefix}_{slug}_{created_at}")
    }

    fn project_scope(project: &ProjectKey) -> SourceScope {
        SourceScope {
            tenant_id: Some(project.tenant_id.as_str().to_owned()),
            workspace_id: Some(project.workspace_id.as_str().to_owned()),
            project_id: Some(project.project_id.as_str().to_owned()),
        }
    }

    fn workspace_scope(workspace_id: &str) -> SourceScope {
        SourceScope {
            tenant_id: None,
            workspace_id: Some(workspace_id.to_owned()),
            project_id: None,
        }
    }

    fn tenant_scope(tenant_id: &str) -> SourceScope {
        SourceScope {
            tenant_id: Some(tenant_id.to_owned()),
            workspace_id: None,
            project_id: None,
        }
    }

    fn bundle_source_type(source_type: crate::ingest::SourceType) -> BundleSourceType {
        match source_type {
            crate::ingest::SourceType::PlainText => BundleSourceType::TextPlain,
            crate::ingest::SourceType::Markdown => BundleSourceType::TextMarkdown,
            crate::ingest::SourceType::Html => BundleSourceType::TextHtml,
            crate::ingest::SourceType::StructuredJson => BundleSourceType::JsonStructured,
            crate::ingest::SourceType::KnowledgePack => BundleSourceType::ExternalRef,
            crate::ingest::SourceType::JsonStructured => BundleSourceType::JsonStructured,
        }
    }

    fn metadata_map(value: &Option<serde_json::Value>) -> HashMap<String, serde_json::Value> {
        value
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|object| {
                object
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn prompt_kind_string(raw: &str) -> String {
        raw.trim().to_owned()
    }

    fn prompt_status_string(raw: &str) -> String {
        raw.trim().to_owned()
    }

    fn document_matches_filters(
        document: &ExportableDocument,
        filters: &DocumentExportFilters,
    ) -> bool {
        if !filters.source_ids.is_empty()
            && !filters
                .source_ids
                .iter()
                .any(|id| document.source_id.as_str() == id.as_str())
        {
            return false;
        }
        if !filters.tags.is_empty()
            && !filters
                .tags
                .iter()
                .all(|filter_tag| document.tags.iter().any(|tag| tag == filter_tag))
        {
            return false;
        }
        if let Some(after) = filters.created_after_ms {
            if document.created_at < after {
                return false;
            }
        }
        if let Some(before) = filters.created_before_ms {
            if document.created_at > before {
                return false;
            }
        }
        if let Some(min_cred) = filters.min_credibility_score {
            if document.credibility_score.unwrap_or(0.0) < min_cred {
                return false;
            }
        }

        true
    }
}

impl InMemoryExportService {
    pub async fn export_documents(
        &self,
        bundle_name: &str,
        project: &ProjectKey,
        filters: &DocumentExportFilters,
    ) -> Result<BundleEnvelope, String> {
        let created_at = Self::now_millis();
        let bundle_id = Self::build_bundle_id(
            BundleType::CuratedKnowledgePackBundle,
            bundle_name,
            created_at,
        );
        let source_scope = Self::project_scope(project);

        let mut artifacts = Vec::new();
        for document in self
            .document_store
            .exportable_documents()
            .into_iter()
            .filter(|document| document.project == *project)
            .filter(|document| Self::document_matches_filters(document, filters))
        {
            let payload = KnowledgeDocumentPayload {
                knowledge_pack_logical_id: bundle_id.clone(),
                document_name: document
                    .title
                    .clone()
                    .unwrap_or_else(|| document.document_id.as_str().to_owned()),
                source_type: Self::bundle_source_type(document.source_type),
                content: DocumentContent::InlineText {
                    text: document.text.clone(),
                },
                metadata: Self::metadata_map(&document.provenance_metadata),
                chunk_hints: vec![],
                retrieval_hints: document.tags.clone(),
            };

            let content_hash = compute_content_hash(&document.text);
            let source_bundle_id = document
                .provenance
                .as_ref()
                .and_then(|p| p.get("source_bundle_id").and_then(|v| v.as_str()).map(str::to_owned))
                .unwrap_or_else(|| bundle_id.clone());
            let lineage = document
                .provenance
                .as_ref()
                .and_then(|p| p.get("lineage").and_then(|v| v.as_str()).map(str::to_owned));

            artifacts.push(ArtifactEntry {
                artifact_kind: ArtifactKind::KnowledgeDocument,
                artifact_logical_id: document.document_id.as_str().to_owned(),
                artifact_display_name: document
                    .title
                    .clone()
                    .unwrap_or_else(|| document.document_id.as_str().to_owned()),
                origin_scope: Self::project_scope(&document.project),
                origin_artifact_id: Some(document.document_id.as_str().to_owned()),
                content_hash,
                source_bundle_id,
                origin_timestamp: document.created_at,
                metadata: {
                    let mut m = Self::metadata_map(&document.provenance_metadata);
                    if let Some(prov) = &document.provenance {
                        m.insert("import_provenance".to_owned(), prov.clone());
                    }
                    m
                },
                payload: crate::bundles::ArtifactPayload::InlineJson(
                    serde_json::to_value(payload).map_err(|err| err.to_string())?,
                ),
                provenance: crate::bundles::ArtifactProvenance::default(),
                lineage,
                tags: document.tags,
            });
        }

        Ok(BundleEnvelope {
            bundle_schema_version: "1".to_owned(),
            bundle_type: BundleType::CuratedKnowledgePackBundle,
            bundle_id,
            bundle_name: bundle_name.to_owned(),
            created_at,
            created_by: Some(self.created_by.clone()),
            source_deployment_id: None,
            source_scope,
            artifact_count: artifacts.len(),
            artifacts,
            provenance: BundleProvenance {
                description: Some(format!("Exported {} knowledge documents", bundle_name)),
                source_system: Some("cairn-memory".to_owned()),
                export_reason: Some("operator_export".to_owned()),
                origin: Some("export".to_owned()),
                production_method: Some("automated_export".to_owned()),
                source_version: None,
            },
        })
    }

    pub async fn export_prompts(
        &self,
        bundle_name: &str,
        tenant_id: &str,
        asset_ids: &[PromptAssetId],
    ) -> Result<BundleEnvelope, String> {
        let created_at = Self::now_millis();
        let bundle_id =
            Self::build_bundle_id(BundleType::PromptLibraryBundle, bundle_name, created_at);
        let mut artifacts = Vec::new();

        for asset_id in asset_ids {
            let asset = PromptAssetReadModel::get(&*self.prompt_store, asset_id)
                .await
                .map_err(|err| err.to_string())?
                .ok_or_else(|| format!("prompt asset not found: {}", asset_id.as_str()))?;

            let asset_payload = PromptAssetPayload {
                name: asset.name.clone(),
                kind: Self::prompt_kind_string(&asset.kind),
                status: Self::prompt_status_string(&asset.status),
                library_scope_hint: if asset.scope.is_empty() { None } else { Some(asset.scope.clone()) },
                metadata: HashMap::from([
                    (
                        "updated_at".to_owned(),
                        serde_json::Value::from(asset.updated_at),
                    ),
                    (
                        "workspace_id".to_owned(),
                        serde_json::Value::from(asset.workspace.as_str()),
                    ),
                    (
                        "tenant_id".to_owned(),
                        serde_json::Value::from(tenant_id),
                    ),
                ]),
            };
            let asset_payload_json =
                serde_json::to_value(&asset_payload).map_err(|err| err.to_string())?;
            artifacts.push(ArtifactEntry {
                artifact_kind: ArtifactKind::PromptAsset,
                artifact_logical_id: asset.prompt_asset_id.as_str().to_owned(),
                artifact_display_name: asset.name.clone(),
                origin_scope: Self::workspace_scope(&asset.workspace),
                origin_artifact_id: Some(asset.prompt_asset_id.as_str().to_owned()),
                content_hash: compute_content_hash(&asset_payload_json.to_string()),
                source_bundle_id: bundle_id.clone(),
                origin_timestamp: asset.created_at,
                metadata: HashMap::from([
                    (
                        "status".to_owned(),
                        serde_json::Value::from(asset.status.as_str()),
                    ),
                    (
                        "scope".to_owned(),
                        serde_json::Value::from(asset.scope.as_str()),
                    ),
                ]),
                payload: crate::bundles::ArtifactPayload::InlineJson(asset_payload_json),
                provenance: crate::bundles::ArtifactProvenance::default(),
                lineage: None,
                tags: vec!["prompt_asset".to_owned()],
            });

            for version in PromptVersionReadModel::list_by_asset(
                &*self.prompt_store,
                &asset.prompt_asset_id,
                1000,
                0,
            )
            .await
            .map_err(|err| err.to_string())?
            {
                let version_payload = PromptVersionPayload {
                    prompt_asset_logical_id: asset.prompt_asset_id.as_str().to_owned(),
                    version_number: version.version_number,
                    format: "text".to_owned(),
                    content: String::new(),
                    metadata: HashMap::from([(
                        "content_hash".to_owned(),
                        serde_json::Value::from(version.content_hash.as_str()),
                    )]),
                };

                artifacts.push(ArtifactEntry {
                    artifact_kind: ArtifactKind::PromptVersion,
                    artifact_logical_id: version.prompt_version_id.as_str().to_owned(),
                    artifact_display_name: format!(
                        "{} v{}",
                        asset.name,
                        version.version_number,
                    ),
                    origin_scope: Self::workspace_scope(&version.workspace),
                    origin_artifact_id: Some(version.prompt_version_id.as_str().to_owned()),
                    content_hash: version.content_hash.clone(),
                    source_bundle_id: bundle_id.clone(),
                    origin_timestamp: version.created_at,
                    metadata: HashMap::new(),
                    payload: crate::bundles::ArtifactPayload::InlineJson(
                        serde_json::to_value(version_payload)
                            .map_err(|err| err.to_string())?,
                    ),
                    provenance: crate::bundles::ArtifactProvenance::default(),
                    lineage: Some(asset.prompt_asset_id.as_str().to_owned()),
                    tags: vec!["prompt_version".to_owned()],
                });
            }
        }

        Ok(BundleEnvelope {
            bundle_schema_version: "1".to_owned(),
            bundle_type: BundleType::PromptLibraryBundle,
            bundle_id,
            bundle_name: bundle_name.to_owned(),
            created_at,
            created_by: Some(self.created_by.clone()),
            source_deployment_id: None,
            source_scope: Self::tenant_scope(tenant_id),
            artifact_count: artifacts.len(),
            artifacts,
            provenance: BundleProvenance {
                description: Some(format!("Exported prompt library {}", bundle_name)),
                source_system: Some("cairn-memory".to_owned()),
                export_reason: Some("operator_export".to_owned()),
                origin: Some("export".to_owned()),
                production_method: Some("automated_export".to_owned()),
                source_version: None,
            },
        })
    }

}

#[async_trait]
impl ExportService for InMemoryExportService {
    type Error = String;

    async fn export(
        &self,
        bundle_name: &str,
        bundle_type: crate::bundles::BundleType,
        source_scope: &crate::bundles::SourceScope,
    ) -> Result<crate::bundles::BundleEnvelope, Self::Error> {
        use crate::bundles::BundleType;
        match bundle_type {
            BundleType::CuratedKnowledgePackBundle => {
                let project = cairn_domain::ProjectKey::new(
                    source_scope.tenant_id.as_deref().unwrap_or("_"),
                    source_scope.workspace_id.as_deref().unwrap_or("_"),
                    source_scope.project_id.as_deref().unwrap_or("_"),
                );
                let filters = crate::bundles::DocumentExportFilters::default();
                self.export_documents(bundle_name, &project, &filters).await
            }
            BundleType::PromptLibraryBundle => {
                let tenant_id = source_scope.tenant_id.as_deref().unwrap_or("_");
                self.export_prompts(bundle_name, tenant_id, &[]).await
            }
        }
    }
}
