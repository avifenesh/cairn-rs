//! Model registry — TOML-backed, in-memory model catalog (GAP-001).
//!
//! The bundled `cairn-domain/assets/models.toml` is embedded at compile time.
//! An optional user-override file (default `~/.cairn/models.toml`) may add or
//! replace entries at runtime; user entries win on id conflict.
//!
//! `ModelRegistry` is `Send + Sync`; interior mutability uses `RwLock`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, RwLock};

use cairn_domain::model_catalog::{ModelEntry, ModelTier};
use cairn_domain::providers::ProviderCostType;
use serde::Deserialize;

/// Embedded bundled catalog — loaded from `cairn-domain/assets/models.toml`.
const BUNDLED_TOML: &str = include_str!("../../cairn-domain/assets/models.toml");

/// TOML-facing representation of a model entry.
///
/// Field names match the `models.toml` schema (e.g. `cost_in` / `cost_out`).
/// Converted to `ModelEntry` after parsing.
#[derive(Deserialize)]
struct TomlEntry {
    id: String,
    provider: String,
    display_name: String,
    context_len: u32,
    #[serde(default)]
    tier: ModelTier,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    cost_type: ProviderCostType,
    /// Cost per 1M input tokens (TOML key: `cost_in`).
    #[serde(default, rename = "cost_in")]
    cost_per_1m_input: f64,
    /// Cost per 1M output tokens (TOML key: `cost_out`).
    #[serde(default, rename = "cost_out")]
    cost_per_1m_output: f64,
    #[serde(default)]
    cache_read_per_1m: f64,
    #[serde(default)]
    cache_write_per_1m: f64,
    #[serde(default = "default_max_tokens")]
    max_tokens: u32,
    #[serde(default = "default_min_cacheable_tokens")]
    min_cacheable_tokens: u32,
    #[serde(default = "default_cache_type")]
    cache_type: String,
    #[serde(default)]
    reasoning: bool,
    #[serde(default = "default_true")]
    supports_tools: bool,
    #[serde(default = "default_true")]
    supports_streaming: bool,
    #[serde(default)]
    supports_json_mode: bool,
    #[serde(default = "default_text_modality")]
    input_modalities: Vec<String>,
    #[serde(default = "default_text_modality")]
    output_modalities: Vec<String>,
}

fn default_true() -> bool {
    true
}
fn default_max_tokens() -> u32 {
    4096
}
fn default_min_cacheable_tokens() -> u32 {
    1024
}
fn default_cache_type() -> String {
    "automatic".to_owned()
}
fn default_text_modality() -> Vec<String> {
    vec!["text".to_owned()]
}

impl From<TomlEntry> for ModelEntry {
    fn from(t: TomlEntry) -> Self {
        ModelEntry {
            id: t.id,
            provider: t.provider,
            display_name: t.display_name,
            context_len: t.context_len,
            tier: t.tier,
            tags: t.tags,
            enabled: t.enabled,
            cost_type: t.cost_type,
            cost_per_1m_input: t.cost_per_1m_input,
            cost_per_1m_output: t.cost_per_1m_output,
            cache_read_per_1m: t.cache_read_per_1m,
            cache_write_per_1m: t.cache_write_per_1m,
            max_tokens: t.max_tokens,
            min_cacheable_tokens: t.min_cacheable_tokens,
            cache_type: t.cache_type,
            reasoning: t.reasoning,
            supports_tools: t.supports_tools,
            supports_streaming: t.supports_streaming,
            supports_json_mode: t.supports_json_mode,
            input_modalities: t.input_modalities,
            output_modalities: t.output_modalities,
        }
    }
}

#[derive(Deserialize)]
struct TomlFile {
    models: Vec<TomlEntry>,
}

fn parse_toml(src: &str) -> Result<Vec<ModelEntry>, String> {
    let tf: TomlFile = toml::from_str(src).map_err(|e| e.to_string())?;
    Ok(tf.models.into_iter().map(ModelEntry::from).collect())
}

/// In-memory model catalog with optional TOML load and manual registration.
///
/// Use [`ModelRegistry::with_bundled()`] to load the shipped defaults,
/// or [`ModelRegistry::empty()`] for test scenarios.
#[derive(Clone)]
pub struct ModelRegistry {
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    models: HashMap<String, ModelEntry>,
}

impl ModelRegistry {
    /// Create an empty registry (useful in tests).
    pub fn empty() -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner {
                models: HashMap::new(),
            })),
        }
    }

    /// Create a registry pre-populated with the bundled `models.toml`.
    pub fn with_bundled() -> Result<Self, String> {
        let entries = parse_toml(BUNDLED_TOML)?;
        let mut models = HashMap::with_capacity(entries.len());
        for e in entries {
            models.insert(e.id.clone(), e);
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner { models })),
        })
    }

    /// Load and overlay a TOML file on top of the existing entries.
    /// User entries win on `id` conflict.
    /// Missing files are silently skipped; parse errors are returned.
    pub fn load_from_toml(&self, path: &Path) -> Result<(), String> {
        let src = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        let entries = parse_toml(&src)?;
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        for e in entries {
            inner.models.insert(e.id.clone(), e);
        }
        Ok(())
    }

    /// Register (or replace) a single entry.
    pub fn register(&self, entry: ModelEntry) {
        let mut inner = self.inner.write().unwrap_or_else(|e| e.into_inner());
        inner.models.insert(entry.id.clone(), entry);
    }

    /// Look up a model by its ID. Returns `None` if not found.
    pub fn get(&self, model_id: &str) -> Option<ModelEntry> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.models.get(model_id).cloned()
    }

    /// Return all models matching the given tier, regardless of `enabled`.
    pub fn list_by_tier(&self, tier: ModelTier) -> Vec<ModelEntry> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .models
            .values()
            .filter(|e| e.tier == tier)
            .cloned()
            .collect()
    }

    /// Return all models for a given provider id.
    pub fn list_by_provider(&self, provider_id: &str) -> Vec<ModelEntry> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner
            .models
            .values()
            .filter(|e| e.provider == provider_id)
            .cloned()
            .collect()
    }

    /// Return all entries (snapshot).
    pub fn all(&self) -> Vec<ModelEntry> {
        let inner = self.inner.read().unwrap_or_else(|e| e.into_inner());
        inner.models.values().cloned().collect()
    }

    /// Number of entries in the registry.
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .models
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_registry_empty_starts_empty() {
        let reg = ModelRegistry::empty();
        assert!(reg.is_empty());
        assert!(reg.get("gpt-4o").is_none());
    }

    #[test]
    fn model_registry_with_bundled_loads_all_models() {
        let reg = ModelRegistry::with_bundled().expect("bundled TOML must parse");
        assert!(reg.len() > 0, "bundled catalog must not be empty");
    }

    #[test]
    fn model_registry_bundled_contains_gpt4o() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let e = reg
            .get("gpt-4o")
            .expect("gpt-4o must be in bundled catalog");
        assert_eq!(e.provider, "openai");
        assert_eq!(e.tier, ModelTier::Brain);
        assert_eq!(e.cost_type, ProviderCostType::Metered);
        assert!(e.cost_per_1m_input > 0.0);
    }

    #[test]
    fn model_registry_bundled_contains_claude_sonnet() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let e = reg
            .get("claude-sonnet-4-6")
            .expect("claude-sonnet-4-6 must be in catalog");
        assert_eq!(e.provider, "anthropic");
        assert_eq!(e.tier, ModelTier::Mid);
        assert!(e.input_modalities.contains(&"image".to_owned()));
    }

    #[test]
    fn model_registry_bundled_contains_haiku() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let e = reg
            .get("claude-haiku-4-5")
            .expect("claude-haiku-4-5 must be in catalog");
        assert_eq!(e.tier, ModelTier::Light);
    }

    #[test]
    fn model_registry_register_overrides_entry() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let original = reg.get("gpt-4o").unwrap();
        let mut updated = original.clone();
        updated.enabled = false;
        reg.register(updated);
        assert!(!reg.get("gpt-4o").unwrap().enabled);
    }

    #[test]
    fn model_registry_list_by_tier_brain() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let brain = reg.list_by_tier(ModelTier::Brain);
        assert!(!brain.is_empty(), "must have at least one Brain-tier model");
        for e in &brain {
            assert_eq!(
                e.tier,
                ModelTier::Brain,
                "all returned entries must be Brain tier"
            );
        }
    }

    #[test]
    fn model_registry_list_by_tier_light() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let light = reg.list_by_tier(ModelTier::Light);
        assert!(!light.is_empty());
        for e in &light {
            assert_eq!(e.tier, ModelTier::Light);
        }
    }

    #[test]
    fn model_registry_list_by_provider_openai() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let openai = reg.list_by_provider("openai");
        assert!(!openai.is_empty());
        for e in &openai {
            assert_eq!(e.provider, "openai");
        }
    }

    #[test]
    fn model_registry_load_from_toml_file() {
        use std::io::Write;
        let dir = std::env::temp_dir();
        let path = dir.join("cairn_test_models.toml");
        let toml_str = r#"
[[models]]
id = "my-custom-model"
provider = "custom"
display_name = "Custom Model"
tier = "mid"
context_len = 8000
cost_in = 1.0
cost_out = 2.0
enabled = true
"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(toml_str.as_bytes()).unwrap();
        }

        let reg = ModelRegistry::empty();
        reg.load_from_toml(&path).expect("load must succeed");
        let e = reg
            .get("my-custom-model")
            .expect("custom model must be loaded");
        assert_eq!(e.provider, "custom");
        assert_eq!(e.context_len, 8000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn model_registry_load_from_toml_missing_file_is_ok() {
        let reg = ModelRegistry::empty();
        let result = reg.load_from_toml(Path::new("/nonexistent/path/models.toml"));
        assert!(result.is_ok(), "missing file must be silently skipped");
    }

    #[test]
    fn model_registry_free_models_have_zero_cost() {
        let reg = ModelRegistry::with_bundled().unwrap();
        for e in reg.all() {
            if e.cost_type == ProviderCostType::Free {
                assert_eq!(
                    e.cost_per_1m_input, 0.0,
                    "free model {} must have zero cost_in",
                    e.id
                );
                assert_eq!(
                    e.cost_per_1m_output, 0.0,
                    "free model {} must have zero cost_out",
                    e.id
                );
            }
        }
    }

    #[test]
    fn model_registry_user_override_wins_on_conflict() {
        let reg = ModelRegistry::with_bundled().unwrap();
        let original_cost = reg.get("gpt-4o").unwrap().cost_per_1m_input;

        use std::io::Write;
        let path = std::env::temp_dir().join("cairn_override_test.toml");
        let override_toml = r#"
[[models]]
id = "gpt-4o"
provider = "openai"
display_name = "GPT-4o (Override)"
tier = "brain"
context_len = 128000
cost_in = 999.0
cost_out = 999.0
enabled = true
"#;
        {
            let mut f = std::fs::File::create(&path).unwrap();
            f.write_all(override_toml.as_bytes()).unwrap();
        }

        reg.load_from_toml(&path).unwrap();
        let updated = reg.get("gpt-4o").unwrap();
        assert_eq!(
            updated.cost_per_1m_input, 999.0,
            "user override must win over bundled entry"
        );
        assert_ne!(updated.cost_per_1m_input, original_cost);

        let _ = std::fs::remove_file(&path);
    }
}
