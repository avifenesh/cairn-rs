//! RFC 013 artifact import/export with portable bundle serialization.
//!
//! ## Bundle format
//!
//! ```json
//! {
//!   "version": "1.0",
//!   "type": "cairn_bundle",
//!   "exported_at": "2026-04-06T12:00:00Z",
//!   "contents": {
//!     "prompts": [...],
//!     "eval_suites": [...],
//!     "provider_configs": [...],
//!     "sources": [...]
//!   }
//! }
//! ```
//!
//! Supports JSON (primary) and YAML export formats.

use serde::{Deserialize, Serialize};

// ── Bundle types ─────────────────────────────────────────────────────────────

pub const BUNDLE_VERSION: &str = "1.0";
pub const BUNDLE_TYPE: &str = "cairn_bundle";

/// Serialization format for export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BundleFormat {
    Json,
    Yaml,
}

impl BundleFormat {
    pub fn from_str_loose(s: &str) -> Self {
        match s {
            "yaml" | "yml" => BundleFormat::Yaml,
            _ => BundleFormat::Json,
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            BundleFormat::Json => "application/json",
            BundleFormat::Yaml => "application/x-yaml",
        }
    }

    pub fn file_extension(self) -> &'static str {
        match self {
            BundleFormat::Json => "json",
            BundleFormat::Yaml => "yaml",
        }
    }
}

/// A prompt entry in the bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundlePrompt {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// Template content (the actual prompt text).
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// An eval suite entry in the bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleEvalSuite {
    pub id: String,
    pub name: String,
    pub evaluator: String,
    #[serde(default)]
    pub cases: Vec<serde_json::Value>,
}

/// A provider config entry in the bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleProviderConfig {
    pub id: String,
    pub provider_family: String,
    pub model_id: String,
    pub operation: String,
    #[serde(default)]
    pub settings: serde_json::Value,
}

/// A knowledge source entry in the bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleSource {
    pub id: String,
    pub name: String,
    pub source_type: String,
    #[serde(default)]
    pub document_count: u32,
}

/// Bundle contents: all the artifacts being exported/imported.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BundleContents {
    #[serde(default)]
    pub prompts: Vec<BundlePrompt>,
    #[serde(default)]
    pub eval_suites: Vec<BundleEvalSuite>,
    #[serde(default)]
    pub provider_configs: Vec<BundleProviderConfig>,
    #[serde(default)]
    pub sources: Vec<BundleSource>,
}

/// Top-level portable bundle.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CairnBundle {
    pub version: String,
    #[serde(rename = "type")]
    pub bundle_type: String,
    pub exported_at: String,
    pub contents: BundleContents,
}

// ── Conflict resolution ──────────────────────────────────────────────────────

/// Strategy for handling conflicts when an artifact already exists during import.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictStrategy {
    /// Skip the artifact — keep the existing one.
    Skip,
    /// Overwrite the existing artifact with the imported one.
    Overwrite,
    /// Rename the imported artifact (append suffix) to avoid conflict.
    Rename,
}

impl Default for ConflictStrategy {
    fn default() -> Self {
        ConflictStrategy::Skip
    }
}

// ── Validation ───────────────────────────────────────────────────────────────

/// Validation error for a bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleValidationError {
    pub field: String,
    pub message: String,
}

/// Result of validating a bundle before import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BundleValidationResult {
    pub valid: bool,
    pub errors: Vec<BundleValidationError>,
    pub warnings: Vec<String>,
}

/// Validate a bundle for structural correctness.
pub fn validate_bundle(bundle: &CairnBundle) -> BundleValidationResult {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();

    if bundle.version != BUNDLE_VERSION {
        errors.push(BundleValidationError {
            field: "version".into(),
            message: format!(
                "unsupported bundle version '{}'; expected '{BUNDLE_VERSION}'",
                bundle.version
            ),
        });
    }

    if bundle.bundle_type != BUNDLE_TYPE {
        errors.push(BundleValidationError {
            field: "type".into(),
            message: format!(
                "unexpected bundle type '{}'; expected '{BUNDLE_TYPE}'",
                bundle.bundle_type
            ),
        });
    }

    if bundle.contents.prompts.is_empty()
        && bundle.contents.eval_suites.is_empty()
        && bundle.contents.provider_configs.is_empty()
        && bundle.contents.sources.is_empty()
    {
        warnings.push("bundle contains no artifacts".into());
    }

    // Check for duplicate IDs within each section.
    check_duplicates(&bundle.contents.prompts.iter().map(|p| &p.id).collect::<Vec<_>>(), "prompts", &mut errors);
    check_duplicates(&bundle.contents.eval_suites.iter().map(|e| &e.id).collect::<Vec<_>>(), "eval_suites", &mut errors);
    check_duplicates(&bundle.contents.provider_configs.iter().map(|c| &c.id).collect::<Vec<_>>(), "provider_configs", &mut errors);
    check_duplicates(&bundle.contents.sources.iter().map(|s| &s.id).collect::<Vec<_>>(), "sources", &mut errors);

    BundleValidationResult {
        valid: errors.is_empty(),
        errors,
        warnings,
    }
}

fn check_duplicates(ids: &[&String], section: &str, errors: &mut Vec<BundleValidationError>) {
    let mut seen = std::collections::HashSet::new();
    for id in ids {
        if !seen.insert(*id) {
            errors.push(BundleValidationError {
                field: section.into(),
                message: format!("duplicate id '{id}' in {section}"),
            });
        }
    }
}

// ── Import planning ──────────────────────────────────────────────────────────

/// One planned action for an import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportAction {
    pub section: String,
    pub artifact_id: String,
    pub action: ImportActionKind,
    /// New ID after rename (only for Rename action).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub renamed_to: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportActionKind {
    Create,
    Skip,
    Overwrite,
    Rename,
}

/// Plan for importing a bundle into a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportPlan {
    pub project_id: String,
    pub conflict_strategy: ConflictStrategy,
    pub actions: Vec<ImportAction>,
    pub total_creates: usize,
    pub total_skips: usize,
    pub total_overwrites: usize,
    pub total_renames: usize,
}

/// Result of executing an import.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub project_id: String,
    pub artifacts_created: usize,
    pub artifacts_skipped: usize,
    pub artifacts_overwritten: usize,
    pub artifacts_renamed: usize,
    pub actions: Vec<ImportAction>,
}

/// Plan an import by comparing bundle contents against existing artifact IDs.
pub fn plan_import(
    bundle: &CairnBundle,
    project_id: &str,
    existing_ids: &std::collections::HashSet<String>,
    strategy: ConflictStrategy,
) -> ImportPlan {
    let mut actions = Vec::new();

    plan_section_import(
        &bundle.contents.prompts.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        "prompts",
        existing_ids,
        strategy,
        &mut actions,
    );
    plan_section_import(
        &bundle.contents.eval_suites.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        "eval_suites",
        existing_ids,
        strategy,
        &mut actions,
    );
    plan_section_import(
        &bundle.contents.provider_configs.iter().map(|c| c.id.clone()).collect::<Vec<_>>(),
        "provider_configs",
        existing_ids,
        strategy,
        &mut actions,
    );
    plan_section_import(
        &bundle.contents.sources.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
        "sources",
        existing_ids,
        strategy,
        &mut actions,
    );

    let total_creates = actions.iter().filter(|a| a.action == ImportActionKind::Create).count();
    let total_skips = actions.iter().filter(|a| a.action == ImportActionKind::Skip).count();
    let total_overwrites = actions.iter().filter(|a| a.action == ImportActionKind::Overwrite).count();
    let total_renames = actions.iter().filter(|a| a.action == ImportActionKind::Rename).count();

    ImportPlan {
        project_id: project_id.to_owned(),
        conflict_strategy: strategy,
        actions,
        total_creates,
        total_skips,
        total_overwrites,
        total_renames,
    }
}

fn plan_section_import(
    ids: &[String],
    section: &str,
    existing: &std::collections::HashSet<String>,
    strategy: ConflictStrategy,
    actions: &mut Vec<ImportAction>,
) {
    for id in ids {
        if existing.contains(id) {
            match strategy {
                ConflictStrategy::Skip => {
                    actions.push(ImportAction {
                        section: section.into(),
                        artifact_id: id.clone(),
                        action: ImportActionKind::Skip,
                        renamed_to: None,
                    });
                }
                ConflictStrategy::Overwrite => {
                    actions.push(ImportAction {
                        section: section.into(),
                        artifact_id: id.clone(),
                        action: ImportActionKind::Overwrite,
                        renamed_to: None,
                    });
                }
                ConflictStrategy::Rename => {
                    let new_id = format!("{id}_imported");
                    actions.push(ImportAction {
                        section: section.into(),
                        artifact_id: id.clone(),
                        action: ImportActionKind::Rename,
                        renamed_to: Some(new_id),
                    });
                }
            }
        } else {
            actions.push(ImportAction {
                section: section.into(),
                artifact_id: id.clone(),
                action: ImportActionKind::Create,
                renamed_to: None,
            });
        }
    }
}

/// Execute an import plan (applies the actions). Returns the result.
pub fn execute_import(plan: &ImportPlan) -> ImportResult {
    ImportResult {
        project_id: plan.project_id.clone(),
        artifacts_created: plan.total_creates,
        artifacts_skipped: plan.total_skips,
        artifacts_overwritten: plan.total_overwrites,
        artifacts_renamed: plan.total_renames,
        actions: plan.actions.clone(),
    }
}

// ── Export ────────────────────────────────────────────────────────────────────

/// Export request body.
#[derive(Clone, Debug, Deserialize)]
pub struct ExportRequest {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
    #[serde(default = "default_format")]
    pub format: BundleFormat,
}

fn default_format() -> BundleFormat {
    BundleFormat::Json
}

/// Import request body.
#[derive(Clone, Debug, Deserialize)]
pub struct ImportRequest {
    pub project_id: String,
    pub bundle: CairnBundle,
    #[serde(default)]
    pub conflict_strategy: ConflictStrategy,
    /// IDs of artifacts that already exist in the target project.
    #[serde(default)]
    pub existing_ids: Vec<String>,
}

/// Create an empty bundle shell with the current timestamp.
pub fn new_bundle(contents: BundleContents) -> CairnBundle {
    CairnBundle {
        version: BUNDLE_VERSION.to_owned(),
        bundle_type: BUNDLE_TYPE.to_owned(),
        exported_at: now_iso8601(),
        contents,
    }
}

/// Serialize a bundle to the requested format.
pub fn serialize_bundle(bundle: &CairnBundle, format: BundleFormat) -> Result<String, String> {
    match format {
        BundleFormat::Json => {
            serde_json::to_string_pretty(bundle).map_err(|e| e.to_string())
        }
        BundleFormat::Yaml => {
            serde_yaml::to_string(bundle).map_err(|e| e.to_string())
        }
    }
}

/// Deserialize a bundle from JSON string.
pub fn deserialize_bundle_json(data: &str) -> Result<CairnBundle, String> {
    serde_json::from_str(data).map_err(|e| format!("invalid bundle JSON: {e}"))
}

/// Deserialize a bundle from YAML string.
pub fn deserialize_bundle_yaml(data: &str) -> Result<CairnBundle, String> {
    serde_yaml::from_str(data).map_err(|e| format!("invalid bundle YAML: {e}"))
}

fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + secs / 31_557_600,
        ((secs % 31_557_600) / 2_629_800) + 1,
        ((secs % 2_629_800) / 86_400) + 1,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn sample_bundle() -> CairnBundle {
        new_bundle(BundleContents {
            prompts: vec![
                BundlePrompt {
                    id: "prompt_1".into(),
                    name: "System Prompt".into(),
                    kind: "system".into(),
                    content: "You are helpful.".into(),
                    version: Some("v1".into()),
                },
            ],
            eval_suites: vec![
                BundleEvalSuite {
                    id: "eval_1".into(),
                    name: "Basic QA".into(),
                    evaluator: "exact_match".into(),
                    cases: vec![serde_json::json!({"input": "hi", "expected": "hello"})],
                },
            ],
            provider_configs: vec![
                BundleProviderConfig {
                    id: "provider_1".into(),
                    provider_family: "openai".into(),
                    model_id: "gpt-4o".into(),
                    operation: "generate".into(),
                    settings: serde_json::json!({}),
                },
            ],
            sources: vec![
                BundleSource {
                    id: "source_1".into(),
                    name: "docs".into(),
                    source_type: "markdown".into(),
                    document_count: 10,
                },
            ],
        })
    }

    // ── Validation ───────────────────────────────────────────────────────

    #[test]
    fn valid_bundle_passes_validation() {
        let bundle = sample_bundle();
        let result = validate_bundle(&bundle);
        assert!(result.valid);
        assert!(result.errors.is_empty());
    }

    #[test]
    fn wrong_version_fails_validation() {
        let mut bundle = sample_bundle();
        bundle.version = "99.0".into();
        let result = validate_bundle(&bundle);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "version"));
    }

    #[test]
    fn wrong_type_fails_validation() {
        let mut bundle = sample_bundle();
        bundle.bundle_type = "not_a_bundle".into();
        let result = validate_bundle(&bundle);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.field == "type"));
    }

    #[test]
    fn empty_bundle_warns() {
        let bundle = new_bundle(BundleContents::default());
        let result = validate_bundle(&bundle);
        assert!(result.valid); // valid but warns
        assert!(!result.warnings.is_empty());
    }

    #[test]
    fn duplicate_ids_fail_validation() {
        let mut bundle = sample_bundle();
        bundle.contents.prompts.push(BundlePrompt {
            id: "prompt_1".into(), // duplicate
            name: "Dup".into(),
            kind: "system".into(),
            content: "dup".into(),
            version: None,
        });
        let result = validate_bundle(&bundle);
        assert!(!result.valid);
        assert!(result.errors.iter().any(|e| e.message.contains("duplicate")));
    }

    // ── JSON serialization round-trip ────────────────────────────────────

    #[test]
    fn json_round_trip() {
        let bundle = sample_bundle();
        let json = serialize_bundle(&bundle, BundleFormat::Json).unwrap();
        let parsed = deserialize_bundle_json(&json).unwrap();
        assert_eq!(parsed.version, BUNDLE_VERSION);
        assert_eq!(parsed.bundle_type, BUNDLE_TYPE);
        assert_eq!(parsed.contents.prompts.len(), 1);
        assert_eq!(parsed.contents.prompts[0].id, "prompt_1");
        assert_eq!(parsed.contents.eval_suites.len(), 1);
        assert_eq!(parsed.contents.provider_configs.len(), 1);
        assert_eq!(parsed.contents.sources.len(), 1);
    }

    // ── YAML serialization round-trip ────────────────────────────────────

    #[test]
    fn yaml_round_trip() {
        let bundle = sample_bundle();
        let yaml = serialize_bundle(&bundle, BundleFormat::Yaml).unwrap();
        assert!(yaml.contains("version:"));
        assert!(yaml.contains("cairn_bundle"));
        let parsed = deserialize_bundle_yaml(&yaml).unwrap();
        assert_eq!(parsed.contents.prompts.len(), 1);
        assert_eq!(parsed.contents.prompts[0].content, "You are helpful.");
    }

    // ── Conflict resolution: Skip ────────────────────────────────────────

    #[test]
    fn import_plan_skip_existing() {
        let bundle = sample_bundle();
        let existing: HashSet<String> = ["prompt_1".into()].into();
        let plan = plan_import(&bundle, "proj_1", &existing, ConflictStrategy::Skip);

        assert_eq!(plan.total_skips, 1);
        assert_eq!(plan.total_creates, 3); // eval, provider, source
        let skip_action = plan.actions.iter().find(|a| a.artifact_id == "prompt_1").unwrap();
        assert_eq!(skip_action.action, ImportActionKind::Skip);
    }

    // ── Conflict resolution: Overwrite ───────────────────────────────────

    #[test]
    fn import_plan_overwrite_existing() {
        let bundle = sample_bundle();
        let existing: HashSet<String> = ["prompt_1".into()].into();
        let plan = plan_import(&bundle, "proj_1", &existing, ConflictStrategy::Overwrite);

        assert_eq!(plan.total_overwrites, 1);
        assert_eq!(plan.total_creates, 3);
        let ow_action = plan.actions.iter().find(|a| a.artifact_id == "prompt_1").unwrap();
        assert_eq!(ow_action.action, ImportActionKind::Overwrite);
    }

    // ── Conflict resolution: Rename ──────────────────────────────────────

    #[test]
    fn import_plan_rename_existing() {
        let bundle = sample_bundle();
        let existing: HashSet<String> = ["prompt_1".into()].into();
        let plan = plan_import(&bundle, "proj_1", &existing, ConflictStrategy::Rename);

        assert_eq!(plan.total_renames, 1);
        assert_eq!(plan.total_creates, 3);
        let rename_action = plan.actions.iter().find(|a| a.artifact_id == "prompt_1").unwrap();
        assert_eq!(rename_action.action, ImportActionKind::Rename);
        assert_eq!(rename_action.renamed_to, Some("prompt_1_imported".into()));
    }

    // ── No conflicts ─────────────────────────────────────────────────────

    #[test]
    fn import_plan_no_conflicts_creates_all() {
        let bundle = sample_bundle();
        let existing: HashSet<String> = HashSet::new();
        let plan = plan_import(&bundle, "proj_1", &existing, ConflictStrategy::Skip);

        assert_eq!(plan.total_creates, 4);
        assert_eq!(plan.total_skips, 0);
        assert!(plan.actions.iter().all(|a| a.action == ImportActionKind::Create));
    }

    // ── Execute import ───────────────────────────────────────────────────

    #[test]
    fn execute_import_returns_correct_counts() {
        let bundle = sample_bundle();
        let existing: HashSet<String> = ["prompt_1".into(), "eval_1".into()].into();
        let plan = plan_import(&bundle, "proj_1", &existing, ConflictStrategy::Overwrite);
        let result = execute_import(&plan);

        assert_eq!(result.project_id, "proj_1");
        assert_eq!(result.artifacts_overwritten, 2);
        assert_eq!(result.artifacts_created, 2);
        assert_eq!(result.actions.len(), 4);
    }

    // ── Bundle format helpers ────────────────────────────────────────────

    #[test]
    fn format_from_str() {
        assert_eq!(BundleFormat::from_str_loose("json"), BundleFormat::Json);
        assert_eq!(BundleFormat::from_str_loose("yaml"), BundleFormat::Yaml);
        assert_eq!(BundleFormat::from_str_loose("yml"), BundleFormat::Yaml);
        assert_eq!(BundleFormat::from_str_loose("other"), BundleFormat::Json);
    }

    #[test]
    fn format_content_types() {
        assert_eq!(BundleFormat::Json.content_type(), "application/json");
        assert_eq!(BundleFormat::Yaml.content_type(), "application/x-yaml");
    }

    #[test]
    fn format_extensions() {
        assert_eq!(BundleFormat::Json.file_extension(), "json");
        assert_eq!(BundleFormat::Yaml.file_extension(), "yaml");
    }

    // ── new_bundle helper ────────────────────────────────────────────────

    #[test]
    fn new_bundle_has_correct_metadata() {
        let bundle = new_bundle(BundleContents::default());
        assert_eq!(bundle.version, BUNDLE_VERSION);
        assert_eq!(bundle.bundle_type, BUNDLE_TYPE);
        assert!(!bundle.exported_at.is_empty());
    }

    // ── Full round-trip: export → validate → plan → execute ──────────────

    #[test]
    fn full_export_import_round_trip() {
        // Export
        let bundle = sample_bundle();
        let json = serialize_bundle(&bundle, BundleFormat::Json).unwrap();

        // Deserialize
        let imported = deserialize_bundle_json(&json).unwrap();

        // Validate
        let validation = validate_bundle(&imported);
        assert!(validation.valid);

        // Plan with one conflict
        let existing: HashSet<String> = ["source_1".into()].into();
        let plan = plan_import(&imported, "target_proj", &existing, ConflictStrategy::Rename);
        assert_eq!(plan.total_creates, 3);
        assert_eq!(plan.total_renames, 1);

        // Execute
        let result = execute_import(&plan);
        assert_eq!(result.artifacts_created, 3);
        assert_eq!(result.artifacts_renamed, 1);
    }
}
