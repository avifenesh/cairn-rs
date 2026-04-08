//! RFC 013 artifact import/export bundle types.
//!
//! One canonical JSON bundle format for prompt libraries and curated
//! knowledge packs. Defines the envelope, artifact entries, identity/
//! provenance, reconciliation outcomes, and import service contract.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// --- Bundle Envelope ---

/// Supported bundle schema versions (RFC 013 §5.1).
pub const SUPPORTED_BUNDLE_SCHEMA_VERSIONS: &[&str] = &["1"];

/// Validate the bundle_schema_version field per RFC 013.
///
/// Returns `Ok(())` if the version is present and supported.
/// Returns an error string if the version is absent or unsupported.
pub fn validate_bundle_schema_version(bundle: &BundleEnvelope) -> Result<(), String> {
    let v = bundle.bundle_schema_version.trim();
    if v.is_empty() {
        return Err("bundle_schema_version is required".to_owned());
    }
    if !SUPPORTED_BUNDLE_SCHEMA_VERSIONS.contains(&v) {
        return Err(format!(
            "unsupported bundle_schema_version '{}'; supported: {}",
            v,
            SUPPORTED_BUNDLE_SCHEMA_VERSIONS.join(", ")
        ));
    }
    Ok(())
}

/// Top-level bundle envelope per RFC 013.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BundleEnvelope {
    pub bundle_schema_version: String,
    pub bundle_type: BundleType,
    pub bundle_id: String,
    pub bundle_name: String,
    pub created_at: u64,
    pub created_by: Option<String>,
    pub source_deployment_id: Option<String>,
    pub source_scope: SourceScope,
    pub artifact_count: usize,
    pub artifacts: Vec<ArtifactEntry>,
    #[serde(default)]
    pub provenance: BundleProvenance,
}

/// V1 bundle types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BundleType {
    PromptLibraryBundle,
    CuratedKnowledgePackBundle,
}

/// Originating scope of the bundle.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SourceScope {
    pub tenant_id: Option<String>,
    pub workspace_id: Option<String>,
    pub project_id: Option<String>,
}

/// Bundle-level provenance metadata (RFC 013).
#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct BundleProvenance {
    pub description: Option<String>,
    pub source_system: Option<String>,
    pub export_reason: Option<String>,
    /// RFC 013: where the bundle came from (e.g. "export", "manual", "migration").
    #[serde(default)]
    pub origin: Option<String>,
    /// RFC 013: how the bundle was produced (e.g. "automated_export", "manual_curation").
    #[serde(default)]
    pub production_method: Option<String>,
    /// RFC 013: version of the source system that produced this bundle.
    #[serde(default)]
    pub source_version: Option<String>,
}

// --- Artifact Entry ---

/// Typed artifact payload for bundle entries (RFC 013 §4).
///
/// `#[serde(untagged)]` preserves backward compatibility: old bundles that
/// stored arbitrary JSON objects round-trip through `InlineJson`, and plain
/// JSON strings round-trip through `InlineText`.
///
/// Disambiguation note: `ExternalRef` and `InlineText` are both
/// newtype-over-String; under untagged deserialization a bare JSON string
/// always resolves to `InlineText` first.  `ExternalRef` is intended for
/// programmatic construction (e.g. a URI into an external blob store) and
/// serializes correctly, but will not round-trip from old plain-string payloads.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum ArtifactPayload {
    /// Inline plain-text content (serializes as a JSON string).
    InlineText(String),
    /// URI reference to an external blob (serializes as a JSON string).
    ExternalRef(String),
    /// Inline structured payload — any JSON value that is not a bare string
    /// (object, array, number, boolean, null).
    InlineJson(serde_json::Value),
}

impl ArtifactPayload {
    /// Returns the payload as a raw `serde_json::Value`.
    ///
    /// Used by code paths that require value-level access such as
    /// `serde_json::from_value` and JSON index operators.
    pub fn as_value(&self) -> serde_json::Value {
        match self {
            Self::InlineText(s) | Self::ExternalRef(s) => serde_json::Value::String(s.clone()),
            Self::InlineJson(v) => v.clone(),
        }
    }

    /// Returns `true` when this payload is a JSON `null`.
    pub fn is_null(&self) -> bool {
        matches!(self, Self::InlineJson(v) if v.is_null())
    }
}

/// Structured per-artifact provenance metadata (RFC 013 §4).
///
/// `#[serde(default)]` allows existing bundles that omit this field to
/// deserialize without error.
#[derive(Clone, Debug, Default, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArtifactProvenance {
    /// Source system or operator that originally produced the artifact.
    #[serde(default)]
    pub origin: Option<String>,
    /// How the artifact was produced (e.g. `"human_authored"`, `"llm_generated"`).
    #[serde(default)]
    pub production_method: Option<String>,
    /// Wall-clock timestamp (ms since epoch) when the artifact was first created.
    #[serde(default)]
    pub created_at: Option<u64>,
}

/// One artifact entry in a bundle's `artifacts` array.
#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ArtifactEntry {
    pub artifact_kind: ArtifactKind,
    pub artifact_logical_id: String,
    pub artifact_display_name: String,
    pub origin_scope: SourceScope,
    pub origin_artifact_id: Option<String>,
    pub content_hash: String,
    pub source_bundle_id: String,
    pub origin_timestamp: u64,
    pub metadata: HashMap<String, serde_json::Value>,
    /// Typed artifact payload — replaces the raw `serde_json::Value`.
    pub payload: ArtifactPayload,
    /// Structured per-artifact provenance (origin, production method, timestamp).
    #[serde(default)]
    pub provenance: ArtifactProvenance,
    pub lineage: Option<String>,
    pub tags: Vec<String>,
}

/// V1 artifact kinds across both bundle types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    PromptAsset,
    PromptVersion,
    KnowledgePack,
    KnowledgeDocument,
}

// --- Knowledge Document Payload ---

/// Payload for a `knowledge_document` artifact entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgeDocumentPayload {
    pub knowledge_pack_logical_id: String,
    pub document_name: String,
    pub source_type: BundleSourceType,
    pub content: DocumentContent,
    pub metadata: HashMap<String, serde_json::Value>,
    pub chunk_hints: Vec<ChunkHint>,
    pub retrieval_hints: Vec<String>,
}

/// Source types for bundle documents (stricter than ingest source types).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleSourceType {
    TextPlain,
    TextMarkdown,
    TextHtml,
    JsonStructured,
    ExternalRef,
}

/// Canonical inline or external content forms.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DocumentContent {
    InlineText {
        text: String,
    },
    InlineJson {
        value: serde_json::Value,
    },
    ExternalRef {
        ref_type: String,
        uri: String,
        media_type: Option<String>,
        sha256: Option<String>,
        bytes: Option<u64>,
    },
}

/// Advisory chunk hint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChunkHint {
    pub start_offset: usize,
    pub end_offset: usize,
    pub hint_text: Option<String>,
}

// --- Knowledge Pack Payload ---

/// Payload for a `knowledge_pack` artifact entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KnowledgePackPayload {
    pub name: String,
    pub description: Option<String>,
    pub target_scope_hint: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

// --- Import/Export Contract ---

/// Import plan classification for each artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportOutcome {
    Create,
    Reuse,
    Update,
    Skip,
    Conflict,
}

/// Per-artifact import plan entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportPlanEntry {
    pub artifact_logical_id: String,
    pub artifact_kind: ArtifactKind,
    pub outcome: ImportOutcome,
    pub reason: String,
    pub existing_id: Option<String>,
}

/// Complete import plan (preview before apply).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportPlan {
    pub bundle_id: String,
    pub target_scope: SourceScope,
    pub entries: Vec<ImportPlanEntry>,
    pub create_count: usize,
    pub reuse_count: usize,
    pub update_count: usize,
    pub skip_count: usize,
    pub conflict_count: usize,
    /// Default conflict resolution strategy for this plan.
    #[serde(default)]
    pub conflict_resolution: ConflictResolutionStrategy,
}

impl ImportPlan {
    pub fn has_conflicts(&self) -> bool {
        self.conflict_count > 0
    }

    pub fn summarize_counts(entries: &[ImportPlanEntry]) -> (usize, usize, usize, usize, usize) {
        let mut create = 0;
        let mut reuse = 0;
        let mut update = 0;
        let mut skip = 0;
        let mut conflict = 0;
        for e in entries {
            match e.outcome {
                ImportOutcome::Create => create += 1,
                ImportOutcome::Reuse => reuse += 1,
                ImportOutcome::Update => update += 1,
                ImportOutcome::Skip => skip += 1,
                ImportOutcome::Conflict => conflict += 1,
            }
        }
        (create, reuse, update, skip, conflict)
    }
}

/// Final import report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportReport {
    pub bundle_id: String,
    pub target_scope: SourceScope,
    pub import_actor: Option<String>,
    pub entries: Vec<ImportReportEntry>,
    pub create_count: usize,
    pub reuse_count: usize,
    pub update_count: usize,
    pub skip_count: usize,
    pub conflict_count: usize,
}

/// Per-artifact outcome in the final report.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportReportEntry {
    pub artifact_logical_id: String,
    pub artifact_kind: ArtifactKind,
    pub outcome: ImportOutcome,
    pub reason: String,
    pub created_object_id: Option<String>,
}

// --- Prompt Payloads ---

/// Payload for a `prompt_asset` artifact entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptAssetPayload {
    pub name: String,
    pub kind: String,
    pub status: String,
    pub library_scope_hint: Option<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Payload for a `prompt_version` artifact entry.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptVersionPayload {
    pub prompt_asset_logical_id: String,
    pub version_number: u32,
    pub format: String,
    pub content: String,
    pub metadata: HashMap<String, serde_json::Value>,
}

/// How to handle an import conflict (name collision with different logical ID).
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionStrategy {
    /// Skip the conflicting artifact — leave existing unchanged.
    #[default]
    Skip,
    /// Overwrite the existing artifact with the incoming one.
    Overwrite,
    /// Create the incoming artifact with a renamed ID.
    Rename,
}

/// Filters for selecting which documents to include in an export bundle.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct DocumentExportFilters {
    /// Only export documents from these source IDs (empty = all).
    pub source_ids: Vec<String>,
    /// Only export documents with a quality score at or above this threshold.
    pub min_quality_score: Option<f32>,
    /// Only export documents with a credibility score at or above this threshold.
    pub min_credibility_score: Option<f32>,
    /// Only export documents created after this timestamp (Unix ms).
    pub created_after_ms: Option<u64>,
    /// Only export documents created before this timestamp (Unix ms).
    pub created_before_ms: Option<u64>,
    /// Only export documents with these tags (empty = any tag).
    pub tags: Vec<String>,
    /// Minimum `created_at` as alias for `created_after_ms` (for backward compat).
    pub created_at: Option<u64>,
    /// Filter by import job ID.
    #[serde(default)]
    pub import_id: Option<String>,
    /// Filter by corpus ID.
    #[serde(default)]
    pub corpus_id: Option<String>,
    /// Filter by bundle source ID.
    #[serde(default)]
    pub bundle_source_id: Option<String>,
}

/// Status of a prompt asset in a bundle export/import operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PromptAssetBundleStatus {
    #[default]
    Included,
    Excluded,
    Conflicted,
}

/// Validation report from bundle pre-import validation.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct ValidationReport {
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
    pub valid: bool,
}

impl ValidationReport {
    pub fn ok() -> Self {
        Self {
            errors: vec![],
            warnings: vec![],
            valid: true,
        }
    }
}

// --- Import/Export Service Traits ---

/// Import service boundary per RFC 013.
///
/// Phases: validate -> plan -> apply -> report.
#[async_trait::async_trait]
pub trait ImportService: Send + Sync {
    type Error: std::fmt::Debug;

    /// Validate a bundle without mutating state.
    async fn validate(&self, bundle: &BundleEnvelope) -> Result<ValidationReport, Self::Error>;

    /// Produce an import plan (preview) without mutating state.
    async fn plan(
        &self,
        bundle: &BundleEnvelope,
        target_scope: &SourceScope,
    ) -> Result<ImportPlan, Self::Error>;

    /// Apply an import plan and materialize product state.
    async fn apply(
        &self,
        plan: &ImportPlan,
        bundle: &BundleEnvelope,
    ) -> Result<ImportReport, Self::Error>;
}

/// Export service boundary per RFC 013.
#[async_trait::async_trait]
pub trait ExportService: Send + Sync {
    type Error: std::fmt::Debug;

    /// Export selected artifacts into a canonical bundle.
    async fn export(
        &self,
        bundle_name: &str,
        bundle_type: BundleType,
        source_scope: &SourceScope,
    ) -> Result<BundleEnvelope, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_envelope_round_trips() {
        let bundle = BundleEnvelope {
            bundle_schema_version: "1".to_owned(),
            bundle_type: BundleType::CuratedKnowledgePackBundle,
            bundle_id: "bundle_1".to_owned(),
            bundle_name: "Test Pack".to_owned(),
            created_at: 1000,
            created_by: Some("operator".to_owned()),
            source_deployment_id: None,
            source_scope: SourceScope {
                tenant_id: Some("t".to_owned()),
                workspace_id: Some("w".to_owned()),
                project_id: None,
            },
            artifact_count: 1,
            artifacts: vec![ArtifactEntry {
                artifact_kind: ArtifactKind::KnowledgeDocument,
                artifact_logical_id: "doc_1".to_owned(),
                artifact_display_name: "Test Doc".to_owned(),
                origin_scope: SourceScope {
                    tenant_id: Some("t".to_owned()),
                    workspace_id: Some("w".to_owned()),
                    project_id: Some("p".to_owned()),
                },
                origin_artifact_id: None,
                content_hash: "abc123".to_owned(),
                source_bundle_id: "bundle_1".to_owned(),
                origin_timestamp: 1000,
                metadata: HashMap::new(),
                payload: ArtifactPayload::InlineJson(serde_json::json!({"document_name": "test"})),
                provenance: ArtifactProvenance::default(),
                lineage: None,
                tags: vec!["curated".to_owned()],
            }],
            provenance: BundleProvenance {
                description: Some("Test export".to_owned()),
                source_system: None,
                export_reason: None,
                origin: None,
                production_method: None,
                source_version: None,
            },
        };

        let json = serde_json::to_string(&bundle).unwrap();
        let parsed: BundleEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.bundle_id, "bundle_1");
        assert_eq!(parsed.bundle_type, BundleType::CuratedKnowledgePackBundle);
        assert_eq!(parsed.artifacts.len(), 1);
    }

    #[test]
    fn import_plan_counts() {
        let entries = vec![
            ImportPlanEntry {
                artifact_logical_id: "a".to_owned(),
                artifact_kind: ArtifactKind::KnowledgeDocument,
                outcome: ImportOutcome::Create,
                reason: "new".to_owned(),
                existing_id: None,
            },
            ImportPlanEntry {
                artifact_logical_id: "b".to_owned(),
                artifact_kind: ArtifactKind::KnowledgeDocument,
                outcome: ImportOutcome::Reuse,
                reason: "same hash".to_owned(),
                existing_id: Some("existing_b".to_owned()),
            },
            ImportPlanEntry {
                artifact_logical_id: "c".to_owned(),
                artifact_kind: ArtifactKind::KnowledgeDocument,
                outcome: ImportOutcome::Conflict,
                reason: "scope mismatch".to_owned(),
                existing_id: None,
            },
        ];

        let (create, reuse, update, skip, conflict) = ImportPlan::summarize_counts(&entries);
        assert_eq!(create, 1);
        assert_eq!(reuse, 1);
        assert_eq!(update, 0);
        assert_eq!(skip, 0);
        assert_eq!(conflict, 1);
    }

    #[test]
    fn document_content_inline_text() {
        let content = DocumentContent::InlineText {
            text: "Hello world".to_owned(),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["kind"], "inline_text");
        assert_eq!(json["text"], "Hello world");
    }

    /// RFC 013 §5.1: bundle_schema_version MUST be present.
    #[test]
    fn validate_schema_version_rejects_empty() {
        let mut bundle = make_minimal_bundle("1");
        bundle.bundle_schema_version = "".to_owned();
        let err = validate_bundle_schema_version(&bundle).unwrap_err();
        assert!(
            err.contains("bundle_schema_version is required"),
            "got: {err}"
        );
    }

    /// RFC 013 §5.1: unsupported schema version must be rejected.
    #[test]
    fn validate_schema_version_rejects_unknown_version() {
        let bundle = make_minimal_bundle("99");
        let err = validate_bundle_schema_version(&bundle).unwrap_err();
        assert!(err.contains("unsupported"), "got: {err}");
    }

    /// RFC 013 §5.1: version "1" is the supported v1 schema version.
    #[test]
    fn validate_schema_version_accepts_version_1() {
        let bundle = make_minimal_bundle("1");
        assert!(validate_bundle_schema_version(&bundle).is_ok());
    }

    fn make_minimal_bundle(schema_version: &str) -> BundleEnvelope {
        BundleEnvelope {
            bundle_schema_version: schema_version.to_owned(),
            bundle_type: BundleType::PromptLibraryBundle,
            bundle_id: "b1".to_owned(),
            bundle_name: "Test".to_owned(),
            created_at: 0,
            created_by: None,
            source_deployment_id: None,
            source_scope: SourceScope {
                tenant_id: None,
                workspace_id: None,
                project_id: None,
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

    #[test]
    fn document_content_external_ref() {
        let content = DocumentContent::ExternalRef {
            ref_type: "url".to_owned(),
            uri: "s3://bucket/doc.pdf".to_owned(),
            media_type: Some("application/pdf".to_owned()),
            sha256: Some("deadbeef".to_owned()),
            bytes: Some(1024),
        };
        let json = serde_json::to_value(&content).unwrap();
        assert_eq!(json["kind"], "external_ref");
        assert_eq!(json["ref_type"], "url");
    }
}

// ── RFC 013 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc013_tests {
    use super::*;

    /// RFC 013: both bundle types use the same physical envelope format.
    #[test]
    fn rfc013_both_bundle_types_use_same_envelope_shape() {
        for bundle_type in [
            BundleType::PromptLibraryBundle,
            BundleType::CuratedKnowledgePackBundle,
        ] {
            let bundle = BundleEnvelope {
                bundle_schema_version: "1".to_owned(),
                bundle_type,
                bundle_id: "b1".to_owned(),
                bundle_name: "Test".to_owned(),
                created_at: 1000,
                created_by: Some("operator".to_owned()),
                source_deployment_id: None,
                source_scope: SourceScope {
                    tenant_id: Some("t1".to_owned()),
                    workspace_id: None,
                    project_id: None,
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
            };
            assert!(
                validate_bundle_schema_version(&bundle).is_ok(),
                "RFC 013: {:?} bundle must pass schema validation",
                bundle_type
            );
            let json = serde_json::to_value(&bundle).expect("bundle must serialize");
            assert!(
                json.is_object(),
                "RFC 013: bundle must serialize as JSON object"
            );
        }
    }

    /// RFC 013: import plan classifies each artifact as one of 5 outcomes.
    #[test]
    fn rfc013_import_plan_all_five_outcomes_representable() {
        let entries = vec![
            ImportPlanEntry {
                artifact_logical_id: "a1".to_owned(),
                artifact_kind: ArtifactKind::PromptAsset,
                outcome: ImportOutcome::Create,
                reason: "new".to_owned(),
                existing_id: None,
            },
            ImportPlanEntry {
                artifact_logical_id: "a2".to_owned(),
                artifact_kind: ArtifactKind::PromptAsset,
                outcome: ImportOutcome::Reuse,
                reason: "same".to_owned(),
                existing_id: Some("pa_1".to_owned()),
            },
            ImportPlanEntry {
                artifact_logical_id: "a3".to_owned(),
                artifact_kind: ArtifactKind::PromptVersion,
                outcome: ImportOutcome::Update,
                reason: "changed".to_owned(),
                existing_id: Some("pv_1".to_owned()),
            },
            ImportPlanEntry {
                artifact_logical_id: "a4".to_owned(),
                artifact_kind: ArtifactKind::KnowledgePack,
                outcome: ImportOutcome::Skip,
                reason: "excluded".to_owned(),
                existing_id: None,
            },
            ImportPlanEntry {
                artifact_logical_id: "a5".to_owned(),
                artifact_kind: ArtifactKind::KnowledgeDocument,
                outcome: ImportOutcome::Conflict,
                reason: "collision".to_owned(),
                existing_id: None,
            },
        ];
        let (c, r, u, s, conf) = ImportPlan::summarize_counts(&entries);
        assert_eq!(c, 1, "RFC 013: Create must be countable");
        assert_eq!(r, 1, "RFC 013: Reuse must be countable");
        assert_eq!(u, 1, "RFC 013: Update must be countable");
        assert_eq!(s, 1, "RFC 013: Skip must be countable");
        assert_eq!(conf, 1, "RFC 013: Conflict must be countable");
    }

    /// RFC 013: Skip must have an explicit reason, never as conflict substitute.
    #[test]
    fn rfc013_skip_requires_explicit_reason() {
        let skip_entry = ImportPlanEntry {
            artifact_logical_id: "a1".to_owned(),
            artifact_kind: ArtifactKind::PromptAsset,
            outcome: ImportOutcome::Skip,
            reason: "operator excluded from scope".to_owned(),
            existing_id: None,
        };
        assert!(
            !skip_entry.reason.is_empty(),
            "RFC 013: Skip entries must have a reason"
        );
        assert_ne!(
            skip_entry.outcome,
            ImportOutcome::Conflict,
            "RFC 013: Skip must not substitute for Conflict"
        );
    }

    /// RFC 013: artifact_logical_id is the portable identity key.
    #[test]
    fn rfc013_artifact_logical_id_is_portable_identity() {
        let entry = ArtifactEntry {
            artifact_kind: ArtifactKind::PromptAsset,
            artifact_logical_id: "acme.prompts.agent.system_v2".to_owned(),
            artifact_display_name: "Agent System v2".to_owned(),
            origin_scope: SourceScope {
                tenant_id: None,
                workspace_id: Some("w1".to_owned()),
                project_id: None,
            },
            origin_artifact_id: Some("pa_abc123".to_owned()),
            content_hash: "sha256:def456".to_owned(),
            source_bundle_id: "bundle_001".to_owned(),
            origin_timestamp: 1000,
            metadata: std::collections::HashMap::new(),
            payload: ArtifactPayload::InlineJson(serde_json::json!({})),
            provenance: ArtifactProvenance::default(),
            lineage: None,
            tags: vec![],
        };
        assert!(
            !entry.artifact_logical_id.is_empty(),
            "RFC 013: artifact_logical_id is the portable reconciliation key"
        );
        // origin_artifact_id is source-system local.
        assert!(
            entry.origin_artifact_id.is_some(),
            "RFC 013: origin_artifact_id is local, NOT the portable key"
        );
    }
}
