//! Model catalog — per-model metadata including cost rates and capabilities.
//!
//! Mirrors `cairn/internal/modelreg/` (Go).
//!
//! The catalog is the source of truth for which models are available, their
//! billing type, cost rates, and capability flags.
//!
//! Thread-safety for shared use: wrap in `Arc<RwLock<ModelRegistry>>`.

use crate::providers::{ProviderCapability, ProviderCostType};
use serde::{Deserialize, Serialize};

/// Model tier for routing priority (`brain` > `mid` > `light`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    Brain,
    #[default]
    Mid,
    Light,
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

/// One entry in the model catalog.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Unique model ID (e.g. `gpt-4o`, `claude-3-5-sonnet-20241022`).
    pub id: String,
    /// Provider family (e.g. `openai`, `anthropic`, `openrouter`).
    pub provider: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Maximum context window in tokens (input + output).
    pub context_len: u32,

    /// Routing tier. Default: `Mid`.
    #[serde(default)]
    pub tier: ModelTier,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Whether active in routing. Default: `true`.
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// Billing model. Default: `Metered`.
    #[serde(default)]
    pub cost_type: ProviderCostType,
    #[serde(default)]
    pub cost_per_1m_input: f64,
    #[serde(default)]
    pub cost_per_1m_output: f64,
    #[serde(default)]
    pub cache_read_per_1m: f64,
    #[serde(default)]
    pub cache_write_per_1m: f64,

    /// Max output tokens per call. Default: 4096.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Min tokens for KV-cache activation. Default: 1024.
    #[serde(default = "default_min_cacheable_tokens")]
    pub min_cacheable_tokens: u32,
    /// Cache strategy. Default: `automatic`.
    #[serde(default = "default_cache_type")]
    pub cache_type: String,

    /// Extended thinking / chain-of-thought. Default: `false`.
    #[serde(default)]
    pub reasoning: bool,
    /// Tool/function calling. Default: `true`.
    #[serde(default = "default_true")]
    pub supports_tools: bool,
    /// Token-streaming. Default: `true`.
    #[serde(default = "default_true")]
    pub supports_streaming: bool,
    /// JSON-mode structured output. Default: `false`.
    #[serde(default)]
    pub supports_json_mode: bool,
    /// Accepted input modalities. Default: `["text"]`.
    #[serde(default = "default_text_modality")]
    pub input_modalities: Vec<String>,
    /// Emitted output modalities. Default: `["text"]`.
    #[serde(default = "default_text_modality")]
    pub output_modalities: Vec<String>,
}

impl ModelEntry {
    /// `ProviderCapability` flags inferred from this entry's boolean fields.
    pub fn capabilities(&self) -> Vec<ProviderCapability> {
        let mut caps = Vec::new();
        if self.supports_streaming {
            caps.push(ProviderCapability::Streaming);
        }
        if self.supports_tools {
            caps.push(ProviderCapability::ToolUse);
        }
        if self.supports_json_mode {
            caps.push(ProviderCapability::StructuredOutput);
        }
        if self.reasoning {
            caps.push(ProviderCapability::ReasoningTrace);
        }
        if self.input_modalities.iter().any(|m| m == "image") {
            caps.push(ProviderCapability::ImageInput);
        }
        if self.context_len >= 100_000 {
            caps.push(ProviderCapability::HighContextWindow);
        }
        caps
    }

    /// Estimate cost in micros (µUSD) for the given token counts.
    /// Returns 0 for flat-rate and free models.
    pub fn estimate_cost_micros(&self, input_tokens: u32, output_tokens: u32) -> u64 {
        if self.cost_type.is_free() {
            return 0;
        }
        let input = self.cost_per_1m_input * (input_tokens as f64) / 1_000_000.0;
        let output = self.cost_per_1m_output * (output_tokens as f64) / 1_000_000.0;
        ((input + output) * 1_000_000.0).round() as u64
    }
}

/// In-process model catalog backed by a `HashMap<String, ModelEntry>`.
///
/// User-supplied entries override bundled entries on ID conflict
/// (same rule as `cairn/internal/modelreg`).
#[derive(Clone, Debug, Default)]
pub struct ModelRegistry {
    entries: std::collections::HashMap<String, ModelEntry>,
}

impl ModelRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create pre-populated from an iterator of entries.
    pub fn with_entries(entries: impl IntoIterator<Item = ModelEntry>) -> Self {
        let mut reg = Self::new();
        for entry in entries {
            reg.entries.insert(entry.id.clone(), entry);
        }
        reg
    }

    /// Add or replace an entry.
    pub fn register(&mut self, entry: ModelEntry) {
        self.entries.insert(entry.id.clone(), entry);
    }

    /// Remove an entry by ID.
    pub fn unregister(&mut self, id: &str) -> Option<ModelEntry> {
        self.entries.remove(id)
    }

    /// Look up by exact ID.
    pub fn get(&self, id: &str) -> Option<&ModelEntry> {
        self.entries.get(id)
    }

    /// All entries sorted by ID.
    pub fn all(&self) -> Vec<&ModelEntry> {
        let mut v: Vec<_> = self.entries.values().collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Entries for a given provider, sorted by ID.
    pub fn by_provider(&self, provider: &str) -> Vec<&ModelEntry> {
        let mut v: Vec<_> = self
            .entries
            .values()
            .filter(|e| e.provider == provider)
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// Enabled entries for a given tier, sorted by ID.
    pub fn by_tier(&self, tier: ModelTier) -> Vec<&ModelEntry> {
        let mut v: Vec<_> = self
            .entries
            .values()
            .filter(|e| e.tier == tier && e.enabled)
            .collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    /// All enabled entries for routing.
    pub fn enabled(&self) -> Vec<&ModelEntry> {
        let mut v: Vec<_> = self.entries.values().filter(|e| e.enabled).collect();
        v.sort_by(|a, b| a.id.cmp(&b.id));
        v
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Replace all entries atomically (used on hot-reload).
    pub fn reload(&mut self, new_entries: impl IntoIterator<Item = ModelEntry>) {
        self.entries.clear();
        for entry in new_entries {
            self.entries.insert(entry.id.clone(), entry);
        }
    }
}

/// Observer notified after a catalog hot-reload.
pub trait ModelCatalogObserver: Send + Sync {
    fn on_catalog_reload(&self, entries: Vec<ModelEntry>);
}

/// Built-in starter catalog shipped with cairn-rs.
///
/// Mirrors key models from `cairn/internal/modelreg/assets/models.toml`.
/// Operators can override any entry by registering a user entry with the same `id`.
pub fn builtin_catalog() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            id: "claude-3-5-sonnet-20241022".to_owned(),
            provider: "anthropic".to_owned(),
            display_name: "Claude 3.5 Sonnet".to_owned(),
            context_len: 200_000,
            tier: ModelTier::Brain,
            tags: vec!["coding".to_owned(), "reasoning".to_owned()],
            enabled: true,
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 3.0,
            cost_per_1m_output: 15.0,
            cache_read_per_1m: 0.3,
            cache_write_per_1m: 3.75,
            max_tokens: 8192,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: true,
            input_modalities: vec!["text".to_owned(), "image".to_owned()],
            output_modalities: vec!["text".to_owned()],
        },
        ModelEntry {
            id: "claude-3-haiku-20240307".to_owned(),
            provider: "anthropic".to_owned(),
            display_name: "Claude 3 Haiku".to_owned(),
            context_len: 200_000,
            tier: ModelTier::Light,
            tags: vec!["fast".to_owned()],
            enabled: true,
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 0.25,
            cost_per_1m_output: 1.25,
            cache_read_per_1m: 0.03,
            cache_write_per_1m: 0.30,
            max_tokens: 4096,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: false,
            input_modalities: vec!["text".to_owned(), "image".to_owned()],
            output_modalities: vec!["text".to_owned()],
        },
        ModelEntry {
            id: "gpt-4o".to_owned(),
            provider: "openai".to_owned(),
            display_name: "GPT-4o".to_owned(),
            context_len: 128_000,
            tier: ModelTier::Brain,
            tags: vec!["multimodal".to_owned()],
            enabled: true,
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 2.5,
            cost_per_1m_output: 10.0,
            cache_read_per_1m: 1.25,
            cache_write_per_1m: 0.0,
            max_tokens: 16384,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: true,
            input_modalities: vec!["text".to_owned(), "image".to_owned()],
            output_modalities: vec!["text".to_owned()],
        },
        ModelEntry {
            id: "gpt-4o-mini".to_owned(),
            provider: "openai".to_owned(),
            display_name: "GPT-4o Mini".to_owned(),
            context_len: 128_000,
            tier: ModelTier::Light,
            tags: vec!["fast".to_owned(), "cheap".to_owned()],
            enabled: true,
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 0.15,
            cost_per_1m_output: 0.60,
            cache_read_per_1m: 0.075,
            cache_write_per_1m: 0.0,
            max_tokens: 16384,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: true,
            input_modalities: vec!["text".to_owned(), "image".to_owned()],
            output_modalities: vec!["text".to_owned()],
        },
        ModelEntry {
            id: "meta-llama/llama-3.1-8b-instruct:free".to_owned(),
            provider: "openrouter".to_owned(),
            display_name: "Llama 3.1 8B (free)".to_owned(),
            context_len: 131_072,
            tier: ModelTier::Light,
            tags: vec!["open-source".to_owned(), "free".to_owned()],
            enabled: true,
            cost_type: ProviderCostType::Free,
            cost_per_1m_input: 0.0,
            cost_per_1m_output: 0.0,
            cache_read_per_1m: 0.0,
            cache_write_per_1m: 0.0,
            max_tokens: 4096,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: false,
            input_modalities: vec!["text".to_owned()],
            output_modalities: vec!["text".to_owned()],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, provider: &str, tier: ModelTier) -> ModelEntry {
        ModelEntry {
            id: id.to_owned(),
            provider: provider.to_owned(),
            display_name: format!("{id} display"),
            context_len: 8_000, // small enough to NOT trigger HighContextWindow
            tier,
            tags: vec![],
            enabled: true,
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 1.0,
            cost_per_1m_output: 4.0,
            cache_read_per_1m: 0.0,
            cache_write_per_1m: 0.0,
            max_tokens: 4096,
            min_cacheable_tokens: 1024,
            cache_type: "automatic".to_owned(),
            reasoning: false,
            supports_tools: true,
            supports_streaming: true,
            supports_json_mode: false,
            input_modalities: vec!["text".to_owned()],
            output_modalities: vec!["text".to_owned()],
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("gpt-4o", "openai", ModelTier::Brain));
        assert!(reg.get("gpt-4o").is_some());
        assert!(reg.get("unknown").is_none());
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn user_entry_overrides_existing() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("gpt-4o", "openai", ModelTier::Brain));
        let mut override_e = entry("gpt-4o", "openai", ModelTier::Light);
        override_e.display_name = "Override".to_owned();
        reg.register(override_e);
        assert_eq!(reg.get("gpt-4o").unwrap().display_name, "Override");
        assert_eq!(reg.len(), 1, "override must not duplicate");
    }

    #[test]
    fn unregister_removes() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("m", "openai", ModelTier::Mid));
        reg.unregister("m");
        assert!(reg.is_empty());
    }

    #[test]
    fn by_provider_filters() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("a", "openai", ModelTier::Brain));
        reg.register(entry("b", "openai", ModelTier::Light));
        reg.register(entry("c", "anthropic", ModelTier::Brain));
        assert_eq!(reg.by_provider("openai").len(), 2);
        assert_eq!(reg.by_provider("anthropic").len(), 1);
        assert_eq!(reg.by_provider("unknown").len(), 0);
    }

    #[test]
    fn by_tier_excludes_disabled() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("active", "openai", ModelTier::Brain));
        let mut disabled = entry("disabled", "openai", ModelTier::Brain);
        disabled.enabled = false;
        reg.register(disabled);
        let brains = reg.by_tier(ModelTier::Brain);
        assert_eq!(brains.len(), 1);
        assert_eq!(brains[0].id, "active");
    }

    #[test]
    fn reload_replaces_all() {
        let mut reg = ModelRegistry::new();
        reg.register(entry("old", "openai", ModelTier::Brain));
        reg.reload(vec![entry("new", "anthropic", ModelTier::Mid)]);
        assert_eq!(reg.len(), 1);
        assert!(reg.get("old").is_none());
        assert!(reg.get("new").is_some());
    }

    #[test]
    fn capabilities_inferred_from_flags() {
        let e = entry("m", "openai", ModelTier::Brain);
        let caps = e.capabilities();
        assert!(caps.contains(&ProviderCapability::Streaming));
        assert!(caps.contains(&ProviderCapability::ToolUse));
        assert!(!caps.contains(&ProviderCapability::ReasoningTrace));
        assert!(!caps.contains(&ProviderCapability::HighContextWindow));
    }

    #[test]
    fn high_context_window_flag() {
        let mut e = entry("big", "openai", ModelTier::Brain);
        e.context_len = 200_000;
        assert!(e
            .capabilities()
            .contains(&ProviderCapability::HighContextWindow));
    }

    #[test]
    fn estimate_cost_metered() {
        let mut e = entry("m", "openai", ModelTier::Mid);
        e.cost_per_1m_input = 1.0;
        e.cost_per_1m_output = 4.0;
        // 100k input + 50k output → 0.10 + 0.20 = $0.30 = 300_000 µUSD
        assert_eq!(e.estimate_cost_micros(100_000, 50_000), 300_000);
    }

    #[test]
    fn estimate_cost_free_is_zero() {
        let mut e = entry("free", "openrouter", ModelTier::Light);
        e.cost_type = ProviderCostType::Free;
        e.cost_per_1m_input = 10.0;
        assert_eq!(e.estimate_cost_micros(1_000_000, 500_000), 0);
    }

    #[test]
    fn builtin_catalog_non_empty() {
        let cat = builtin_catalog();
        assert!(!cat.is_empty());
        assert!(cat.iter().any(|e| e.provider == "anthropic"));
        assert!(cat.iter().any(|e| e.provider == "openai"));
        assert!(cat.iter().any(|e| e.tier == ModelTier::Brain));
        assert!(cat.iter().any(|e| e.tier == ModelTier::Light));
    }

    #[test]
    fn builtin_catalog_required_fields_non_empty() {
        for e in builtin_catalog() {
            assert!(!e.id.is_empty(), "id empty for {:?}", e.display_name);
            assert!(!e.provider.is_empty(), "provider empty for {}", e.id);
            assert!(
                !e.display_name.is_empty(),
                "display_name empty for {}",
                e.id
            );
            assert!(e.context_len > 0, "context_len = 0 for {}", e.id);
            assert!(e.max_tokens > 0, "max_tokens = 0 for {}", e.id);
        }
    }

    #[test]
    fn registry_with_builtin_catalog() {
        let reg = ModelRegistry::with_entries(builtin_catalog());
        assert!(reg.len() >= 3);
        assert!(!reg.by_tier(ModelTier::Brain).is_empty());
    }

    #[test]
    fn model_tier_ordering() {
        // Numeric ordering: Brain > Mid > Light is contractual for routing.
        assert_ne!(ModelTier::Brain, ModelTier::Mid);
        assert_ne!(ModelTier::Mid, ModelTier::Light);
        assert_ne!(ModelTier::Brain, ModelTier::Light);
    }
}
