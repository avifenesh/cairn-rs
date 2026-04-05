//! Model catalog integration tests (GAP-001).
//!
//! Validates the `ModelRegistry` + `builtin_catalog()` pipeline end-to-end,
//! proving that the model registry compiles, the bundled catalog ships the
//! expected models, filtering works correctly, and `capabilities()` maps
//! each entry's boolean flags to the right `ProviderCapability` set.
//!
//! Built-in catalog snapshot (5 models as of GAP-001):
//!   claude-3-5-sonnet-20241022  anthropic  Brain   200k ctx
//!   claude-3-haiku-20240307     anthropic  Light   200k ctx
//!   gpt-4o                      openai     Brain   128k ctx
//!   gpt-4o-mini                 openai     Light   128k ctx
//!   meta-llama/llama-3.1-…:free openrouter Light   131k ctx  Free

use cairn_domain::{
    model_catalog::{builtin_catalog, ModelEntry, ModelRegistry, ModelTier},
    providers::{ProviderCapability, ProviderCostType},
};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Minimal entry factory for non-builtin tests.
fn entry(id: &str, provider: &str, tier: ModelTier) -> ModelEntry {
    ModelEntry {
        id: id.to_owned(),
        provider: provider.to_owned(),
        display_name: format!("{id} display"),
        context_len: 8_000, // below HighContextWindow threshold
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

// ── 1. builtin_catalog() ships exactly 5 models ───────────────────────────────

#[test]
fn builtin_catalog_has_exactly_five_models() {
    let cat = builtin_catalog();
    assert_eq!(cat.len(), 5, "GAP-001 requires 5 bundled models; got {}", cat.len());
}

// ── 2. ModelRegistry loads builtin_catalog correctly ─────────────────────────

#[test]
fn registry_with_builtin_catalog_has_all_five_entries() {
    let reg = ModelRegistry::with_entries(builtin_catalog());

    assert_eq!(reg.len(), 5);
    assert!(!reg.is_empty());

    // All 5 bundled IDs must be retrievable.
    for id in [
        "claude-3-5-sonnet-20241022",
        "claude-3-haiku-20240307",
        "gpt-4o",
        "gpt-4o-mini",
        "meta-llama/llama-3.1-8b-instruct:free",
    ] {
        assert!(reg.get(id).is_some(), "missing expected model: {id}");
    }
}

// ── 3. list_by_tier (by_tier) returns correct models ─────────────────────────

#[test]
fn list_by_tier_brain_returns_two_models() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let brain = reg.by_tier(ModelTier::Brain);

    assert_eq!(brain.len(), 2, "Brain tier must have exactly 2 models");

    let ids: Vec<_> = brain.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"claude-3-5-sonnet-20241022"), "Sonnet must be Brain");
    assert!(ids.contains(&"gpt-4o"),                    "GPT-4o must be Brain");
}

#[test]
fn list_by_tier_light_returns_three_models() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let light = reg.by_tier(ModelTier::Light);

    assert_eq!(light.len(), 3, "Light tier must have exactly 3 models");

    let ids: Vec<_> = light.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"claude-3-haiku-20240307"),                  "Haiku must be Light");
    assert!(ids.contains(&"gpt-4o-mini"),                              "GPT-4o Mini must be Light");
    assert!(ids.contains(&"meta-llama/llama-3.1-8b-instruct:free"),    "Llama must be Light");
}

#[test]
fn list_by_tier_mid_returns_empty_for_builtin_catalog() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let mid = reg.by_tier(ModelTier::Mid);

    assert!(mid.is_empty(), "no builtin models are in the Mid tier");
}

#[test]
fn list_by_tier_excludes_disabled_entries() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());

    // Disable one Brain model.
    let mut sonnet = reg.get("claude-3-5-sonnet-20241022").unwrap().clone();
    sonnet.enabled = false;
    reg.register(sonnet);

    let brain = reg.by_tier(ModelTier::Brain);
    assert_eq!(brain.len(), 1, "disabled model excluded from by_tier");
    assert_eq!(brain[0].id, "gpt-4o");
}

// ── 4. list_by_provider filters correctly ────────────────────────────────────

#[test]
fn list_by_provider_anthropic_returns_two_models() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let anthropic = reg.by_provider("anthropic");

    assert_eq!(anthropic.len(), 2);
    assert!(anthropic.iter().all(|e| e.provider == "anthropic"));

    let ids: Vec<_> = anthropic.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"claude-3-5-sonnet-20241022"));
    assert!(ids.contains(&"claude-3-haiku-20240307"));
}

#[test]
fn list_by_provider_openai_returns_two_models() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let openai = reg.by_provider("openai");

    assert_eq!(openai.len(), 2);
    let ids: Vec<_> = openai.iter().map(|e| e.id.as_str()).collect();
    assert!(ids.contains(&"gpt-4o"));
    assert!(ids.contains(&"gpt-4o-mini"));
}

#[test]
fn list_by_provider_openrouter_returns_one_model() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let openrouter = reg.by_provider("openrouter");

    assert_eq!(openrouter.len(), 1);
    assert_eq!(openrouter[0].id, "meta-llama/llama-3.1-8b-instruct:free");
    assert_eq!(openrouter[0].cost_type, ProviderCostType::Free);
}

#[test]
fn list_by_provider_unknown_returns_empty() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    assert!(reg.by_provider("unknown-llm-co").is_empty());
}

// ── 5. reload() atomically replaces catalog ───────────────────────────────────

#[test]
fn reload_replaces_all_existing_entries() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());
    assert_eq!(reg.len(), 5);

    // After reload only the two new entries remain.
    reg.reload(vec![
        entry("model-new-a", "vendor-x", ModelTier::Brain),
        entry("model-new-b", "vendor-x", ModelTier::Mid),
    ]);

    assert_eq!(reg.len(), 2, "reload must discard old entries");

    // Old entries gone.
    for old_id in ["claude-3-5-sonnet-20241022", "gpt-4o"] {
        assert!(reg.get(old_id).is_none(), "{old_id} must be absent after reload");
    }

    // New entries present.
    assert!(reg.get("model-new-a").is_some());
    assert!(reg.get("model-new-b").is_some());
}

#[test]
fn reload_with_empty_iterator_clears_catalog() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());
    reg.reload(std::iter::empty());

    assert!(reg.is_empty(), "reload with empty iterator must clear all entries");
}

#[test]
fn reload_then_repopulate_gives_fresh_catalog() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());
    reg.reload(builtin_catalog()); // idempotent reload

    assert_eq!(reg.len(), 5, "reloading same catalog must restore 5 entries");
    assert!(reg.get("gpt-4o").is_some());
}

// ── 6. capabilities() maps flags to ProviderCapability correctly ──────────────

#[test]
fn claude_sonnet_capabilities() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let sonnet = reg.get("claude-3-5-sonnet-20241022").unwrap();
    let caps = sonnet.capabilities();

    // claude-3-5-sonnet: streaming=true, tools=true, json_mode=true,
    //                    reasoning=false, image input, context=200k
    assert!(caps.contains(&ProviderCapability::Streaming),         "Sonnet: Streaming");
    assert!(caps.contains(&ProviderCapability::ToolUse),           "Sonnet: ToolUse");
    assert!(caps.contains(&ProviderCapability::StructuredOutput),  "Sonnet: StructuredOutput (json_mode)");
    assert!(caps.contains(&ProviderCapability::ImageInput),        "Sonnet: ImageInput");
    assert!(caps.contains(&ProviderCapability::HighContextWindow), "Sonnet: HighContextWindow (200k)");
    assert!(!caps.contains(&ProviderCapability::ReasoningTrace),   "Sonnet: no ReasoningTrace");
}

#[test]
fn claude_haiku_capabilities() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let haiku = reg.get("claude-3-haiku-20240307").unwrap();
    let caps = haiku.capabilities();

    // haiku: streaming=true, tools=true, json_mode=false, image input, context=200k
    assert!(caps.contains(&ProviderCapability::Streaming));
    assert!(caps.contains(&ProviderCapability::ToolUse));
    assert!(caps.contains(&ProviderCapability::ImageInput));
    assert!(caps.contains(&ProviderCapability::HighContextWindow));
    assert!(!caps.contains(&ProviderCapability::StructuredOutput),
        "Haiku: no StructuredOutput (json_mode=false)");
    assert!(!caps.contains(&ProviderCapability::ReasoningTrace));
}

#[test]
fn gpt4o_capabilities() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let gpt4o = reg.get("gpt-4o").unwrap();
    let caps = gpt4o.capabilities();

    // gpt-4o: streaming, tools, json_mode, image, 128k context
    assert!(caps.contains(&ProviderCapability::Streaming));
    assert!(caps.contains(&ProviderCapability::ToolUse));
    assert!(caps.contains(&ProviderCapability::StructuredOutput));
    assert!(caps.contains(&ProviderCapability::ImageInput));
    assert!(caps.contains(&ProviderCapability::HighContextWindow));
    assert!(!caps.contains(&ProviderCapability::ReasoningTrace));
}

#[test]
fn llama_free_capabilities() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let llama = reg.get("meta-llama/llama-3.1-8b-instruct:free").unwrap();
    let caps = llama.capabilities();

    // llama: streaming=true, tools=true, json_mode=false, text-only, 131k ctx
    assert!(caps.contains(&ProviderCapability::Streaming));
    assert!(caps.contains(&ProviderCapability::ToolUse));
    assert!(caps.contains(&ProviderCapability::HighContextWindow), "131k >= 100k threshold");
    assert!(!caps.contains(&ProviderCapability::StructuredOutput), "no json_mode");
    assert!(!caps.contains(&ProviderCapability::ImageInput),       "text-only input");
    assert!(!caps.contains(&ProviderCapability::ReasoningTrace));
}

#[test]
fn high_context_window_threshold_is_100k() {
    // Below threshold: no flag.
    let mut below = entry("small", "x", ModelTier::Mid);
    below.context_len = 99_999;
    assert!(!below.capabilities().contains(&ProviderCapability::HighContextWindow));

    // At threshold: flag present.
    let mut at = entry("at-threshold", "x", ModelTier::Mid);
    at.context_len = 100_000;
    assert!(at.capabilities().contains(&ProviderCapability::HighContextWindow));

    // Above threshold: flag present.
    let mut above = entry("large", "x", ModelTier::Mid);
    above.context_len = 200_000;
    assert!(above.capabilities().contains(&ProviderCapability::HighContextWindow));
}

#[test]
fn reasoning_trace_capability_from_reasoning_flag() {
    let mut e = entry("thinker", "vendor", ModelTier::Brain);
    e.reasoning = true;

    let caps = e.capabilities();
    assert!(caps.contains(&ProviderCapability::ReasoningTrace));
    // Other flags off by default in helper (except Streaming and ToolUse).
    assert!(caps.contains(&ProviderCapability::Streaming));
    assert!(caps.contains(&ProviderCapability::ToolUse));
}

#[test]
fn image_input_capability_requires_image_modality() {
    let mut e = entry("vision", "vendor", ModelTier::Brain);
    e.input_modalities = vec!["text".to_owned(), "image".to_owned()];

    assert!(e.capabilities().contains(&ProviderCapability::ImageInput));

    // Text-only: no ImageInput.
    let text_only = entry("text-only", "vendor", ModelTier::Brain);
    assert!(!text_only.capabilities().contains(&ProviderCapability::ImageInput));
}

// ── 7. all() returns entries sorted by ID ────────────────────────────────────

#[test]
fn all_returns_entries_sorted_by_id() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let all = reg.all();

    assert_eq!(all.len(), 5);
    for window in all.windows(2) {
        assert!(window[0].id <= window[1].id, "all() must be sorted by id");
    }
}

// ── 8. user entry overrides builtin on same ID ────────────────────────────────

#[test]
fn user_entry_overrides_builtin_on_same_id() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());
    let original_tier = reg.get("gpt-4o").unwrap().tier;
    assert_eq!(original_tier, ModelTier::Brain);

    // Operator demotes gpt-4o to Mid for their deployment.
    let mut custom = reg.get("gpt-4o").unwrap().clone();
    custom.tier = ModelTier::Mid;
    custom.display_name = "GPT-4o (custom)".to_owned();
    reg.register(custom);

    assert_eq!(reg.len(), 5, "override must not add a duplicate");
    assert_eq!(reg.get("gpt-4o").unwrap().tier, ModelTier::Mid);
    assert_eq!(reg.get("gpt-4o").unwrap().display_name, "GPT-4o (custom)");

    // Brain tier should now only have Sonnet.
    assert_eq!(reg.by_tier(ModelTier::Brain).len(), 1);
}

// ── 9. estimate_cost_micros on builtin models ─────────────────────────────────

#[test]
fn estimate_cost_metered_on_claude_sonnet() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let sonnet = reg.get("claude-3-5-sonnet-20241022").unwrap();

    // input: 1M tokens at $3/M = $3.00
    // output: 1M tokens at $15/M = $15.00
    // total: $18.00 = 18_000_000 µUSD
    let cost = sonnet.estimate_cost_micros(1_000_000, 1_000_000);
    assert_eq!(cost, 18_000_000);
}

#[test]
fn estimate_cost_zero_for_free_model() {
    let reg = ModelRegistry::with_entries(builtin_catalog());
    let llama = reg.get("meta-llama/llama-3.1-8b-instruct:free").unwrap();

    assert_eq!(llama.cost_type, ProviderCostType::Free);
    assert_eq!(llama.estimate_cost_micros(1_000_000, 1_000_000), 0,
        "Free model must return 0 regardless of token counts");
}

// ── 10. enabled() excludes disabled entries ───────────────────────────────────

#[test]
fn enabled_excludes_disabled_models() {
    let mut reg = ModelRegistry::with_entries(builtin_catalog());

    // Disable two models.
    for id in ["gpt-4o", "gpt-4o-mini"] {
        let mut e = reg.get(id).unwrap().clone();
        e.enabled = false;
        reg.register(e);
    }

    let enabled = reg.enabled();
    assert_eq!(enabled.len(), 3, "3 models remain enabled");
    assert!(enabled.iter().all(|e| e.enabled));
    assert!(!enabled.iter().any(|e| e.id == "gpt-4o" || e.id == "gpt-4o-mini"));
}
