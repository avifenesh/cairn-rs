//! Static provider registry — known LLM providers with API endpoints,
//! authentication, and model metadata.
//!
//! Cairn is provider-agnostic. This registry is reference data that helps
//! the platform auto-configure connections when a user supplies a
//! `"provider/model"` string or sets a known env var.
//!
//! Users can connect any OpenAI-compatible endpoint not in this registry
//! via `POST /v1/providers/connections` with a custom base URL.

use serde::{Deserialize, Serialize};

/// API wire format used by a provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiFormat {
    /// Anthropic's native Messages API (different SSE events, system prompt handling).
    Anthropic,
    /// OpenAI-compatible `/v1/chat/completions` (used by most providers).
    OpenAiCompatible,
}

/// Capability flags for a provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub streaming: bool,
    pub tool_use: bool,
    pub vision: bool,
    pub thinking: bool,
    pub system_prompt: bool,
    pub caching: bool,
}

/// A known model within a provider.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownModel {
    pub id: &'static str,
    pub context_window: u64,
    pub capabilities: ProviderCapabilities,
}

/// A known LLM provider.
#[derive(Clone, Debug)]
pub struct ProviderDef {
    /// Short identifier (e.g. `"openai"`, `"anthropic"`, `"groq"`).
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// Default API base URL.
    pub api_base: &'static str,
    /// Environment variable names for API key (checked in order).
    pub env_keys: &'static [&'static str],
    /// Wire format.
    pub api_format: ApiFormat,
    /// Default model when none specified.
    pub default_model: &'static str,
    /// Known models with context windows and capabilities.
    pub models: &'static [KnownModel],
}

impl ProviderDef {
    /// Read API key from environment using this provider's env key list.
    pub fn api_key_from_env(&self) -> Option<String> {
        for key in self.env_keys {
            if let Ok(val) = std::env::var(key) {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
        None
    }

    /// Whether this provider requires an API key (local providers like Ollama do not).
    pub fn requires_key(&self) -> bool {
        !self.env_keys.is_empty()
    }

    /// Context window for a model, falling back to 128K default.
    pub fn context_window(&self, model: &str) -> u64 {
        self.models
            .iter()
            .find(|m| m.id == model)
            .map(|m| m.context_window)
            .unwrap_or(128_000)
    }

    /// Whether this provider is currently available (key configured or not required).
    pub fn is_available(&self) -> bool {
        !self.requires_key() || self.api_key_from_env().is_some()
    }
}

// ── Capability shorthands ────────────────────────────────────────────────────

const FULL: ProviderCapabilities = ProviderCapabilities {
    streaming: true,
    tool_use: true,
    vision: true,
    thinking: false,
    system_prompt: true,
    caching: false,
};

const FULL_THINKING: ProviderCapabilities = ProviderCapabilities {
    streaming: true,
    tool_use: true,
    vision: true,
    thinking: true,
    system_prompt: true,
    caching: true,
};

const BASIC: ProviderCapabilities = ProviderCapabilities {
    streaming: true,
    tool_use: true,
    vision: false,
    thinking: false,
    system_prompt: true,
    caching: false,
};

// ── Registry ─────────────────────────────────────────────────────────────────

pub static PROVIDERS: &[ProviderDef] = &[
    ProviderDef {
        id: "anthropic",
        name: "Anthropic",
        api_base: "https://api.anthropic.com",
        env_keys: &["ANTHROPIC_API_KEY"],
        api_format: ApiFormat::Anthropic,
        default_model: "claude-sonnet-4-6",
        models: &[
            KnownModel {
                id: "claude-opus-4-6",
                context_window: 200_000,
                capabilities: FULL_THINKING,
            },
            KnownModel {
                id: "claude-sonnet-4-6",
                context_window: 200_000,
                capabilities: FULL_THINKING,
            },
            KnownModel {
                id: "claude-haiku-4-5",
                context_window: 200_000,
                capabilities: FULL,
            },
        ],
    },
    ProviderDef {
        id: "openai",
        name: "OpenAI",
        api_base: "https://api.openai.com/v1",
        env_keys: &["OPENAI_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "gpt-4o",
        models: &[
            KnownModel {
                id: "gpt-4o",
                context_window: 128_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "gpt-4-turbo",
                context_window: 128_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "o1",
                context_window: 200_000,
                capabilities: FULL_THINKING,
            },
            KnownModel {
                id: "o3",
                context_window: 200_000,
                capabilities: FULL_THINKING,
            },
        ],
    },
    ProviderDef {
        id: "google",
        name: "Google",
        api_base: "https://generativelanguage.googleapis.com/v1beta/openai",
        env_keys: &["GOOGLE_API_KEY", "GEMINI_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "gemini-2.0-flash",
        models: &[
            KnownModel {
                id: "gemini-2.0-flash",
                context_window: 1_000_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "gemini-2.0-pro",
                context_window: 1_000_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "gemini-1.5-pro",
                context_window: 2_000_000,
                capabilities: FULL,
            },
        ],
    },
    ProviderDef {
        id: "mistral",
        name: "Mistral",
        api_base: "https://api.mistral.ai/v1",
        env_keys: &["MISTRAL_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "mistral-large-latest",
        models: &[
            KnownModel {
                id: "mistral-large-latest",
                context_window: 128_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "codestral-latest",
                context_window: 256_000,
                capabilities: BASIC,
            },
        ],
    },
    ProviderDef {
        id: "groq",
        name: "Groq",
        api_base: "https://api.groq.com/openai/v1",
        env_keys: &["GROQ_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "llama-3.3-70b-versatile",
        models: &[
            KnownModel {
                id: "llama-3.3-70b-versatile",
                context_window: 128_000,
                capabilities: BASIC,
            },
            KnownModel {
                id: "llama-3.1-8b-instant",
                context_window: 128_000,
                capabilities: BASIC,
            },
        ],
    },
    ProviderDef {
        id: "deepseek",
        name: "DeepSeek",
        api_base: "https://api.deepseek.com/v1",
        env_keys: &["DEEPSEEK_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "deepseek-chat",
        models: &[
            KnownModel {
                id: "deepseek-chat",
                context_window: 64_000,
                capabilities: FULL,
            },
            KnownModel {
                id: "deepseek-coder",
                context_window: 64_000,
                capabilities: BASIC,
            },
        ],
    },
    ProviderDef {
        id: "xai",
        name: "xAI",
        api_base: "https://api.x.ai/v1",
        env_keys: &["XAI_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "grok-2",
        models: &[KnownModel {
            id: "grok-2",
            context_window: 128_000,
            capabilities: FULL,
        }],
    },
    ProviderDef {
        id: "together",
        name: "Together",
        api_base: "https://api.together.xyz/v1",
        env_keys: &["TOGETHER_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
        models: &[KnownModel {
            id: "meta-llama/Meta-Llama-3.1-70B-Instruct-Turbo",
            context_window: 128_000,
            capabilities: BASIC,
        }],
    },
    ProviderDef {
        id: "fireworks",
        name: "Fireworks",
        api_base: "https://api.fireworks.ai/inference/v1",
        env_keys: &["FIREWORKS_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "accounts/fireworks/models/llama-v3p1-70b-instruct",
        models: &[KnownModel {
            id: "accounts/fireworks/models/llama-v3p1-70b-instruct",
            context_window: 128_000,
            capabilities: BASIC,
        }],
    },
    ProviderDef {
        id: "perplexity",
        name: "Perplexity",
        api_base: "https://api.perplexity.ai",
        env_keys: &["PERPLEXITY_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "llama-3.1-sonar-large-128k-online",
        models: &[KnownModel {
            id: "llama-3.1-sonar-large-128k-online",
            context_window: 128_000,
            capabilities: BASIC,
        }],
    },
    ProviderDef {
        id: "cerebras",
        name: "Cerebras",
        api_base: "https://api.cerebras.ai/v1",
        env_keys: &["CEREBRAS_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "llama3.1-70b",
        models: &[KnownModel {
            id: "llama3.1-70b",
            context_window: 128_000,
            capabilities: BASIC,
        }],
    },
    ProviderDef {
        id: "ollama",
        name: "Ollama",
        api_base: "http://localhost:11434/v1",
        env_keys: &[],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "llama3.1",
        models: &[],
    },
    ProviderDef {
        id: "openrouter",
        name: "OpenRouter",
        api_base: "https://openrouter.ai/api/v1",
        env_keys: &["OPENROUTER_API_KEY"],
        api_format: ApiFormat::OpenAiCompatible,
        default_model: "openrouter/auto",
        models: &[
            KnownModel { id: "openrouter/free", context_window: 200_000, capabilities: FULL },
            KnownModel { id: "openrouter/auto", context_window: 128_000, capabilities: FULL },
            KnownModel { id: "google/gemma-3-4b-it:free", context_window: 128_000, capabilities: BASIC },
            KnownModel { id: "meta-llama/llama-4-scout:free", context_window: 512_000, capabilities: FULL },
            KnownModel { id: "deepseek/deepseek-chat-v3-0324:free", context_window: 64_000, capabilities: FULL },
            KnownModel { id: "qwen/qwen3-8b:free", context_window: 128_000, capabilities: FULL_THINKING },
        ],
    },
    ProviderDef {
        id: "bedrock",
        name: "AWS Bedrock",
        api_base: "https://bedrock-runtime.us-east-2.amazonaws.com",
        env_keys: &["BEDROCK_API_KEY", "AWS_BEARER_TOKEN_BEDROCK"],
        api_format: ApiFormat::OpenAiCompatible, // uses Converse API internally, but listed for registry
        default_model: "minimax.minimax-m2.5",
        models: &[
            KnownModel { id: "minimax.minimax-m2.5", context_window: 128_000, capabilities: FULL },
        ],
    },
];

/// Look up a provider by ID.
pub fn lookup(provider_id: &str) -> Option<&'static ProviderDef> {
    PROVIDERS.iter().find(|e| e.id == provider_id)
}

/// All registered providers.
pub fn all() -> &'static [ProviderDef] {
    PROVIDERS
}

/// Providers that have valid auth configured in the environment.
pub fn available() -> Vec<&'static ProviderDef> {
    PROVIDERS.iter().filter(|e| e.is_available()).collect()
}

/// Resolve a `"provider/model"` string into (provider_def, model_name).
///
/// Accepts:
/// - `"openai/gpt-4o"` — explicit provider routing
/// - `"gpt-4o"` — auto-detect from known model prefixes
/// - `"claude-sonnet-4-6"` — auto-detect Anthropic
///
/// Returns `None` if the provider is unknown and auto-detection fails.
pub fn resolve_model_string(model: &str) -> Option<(&'static ProviderDef, String)> {
    // Explicit: "provider/model"
    if let Some((provider_id, model_name)) = model.split_once('/') {
        if let Some(def) = lookup(provider_id) {
            return Some((def, model_name.to_owned()));
        }
        // Could be a model with slashes (e.g. "meta-llama/llama-3.1-70b")
        // Fall through to auto-detect
    }

    // Auto-detect from known model IDs
    for provider in PROVIDERS {
        for known in provider.models {
            if known.id == model {
                return Some((provider, model.to_owned()));
            }
        }
    }

    // Auto-detect from name prefixes
    let lower = model.to_lowercase();
    if lower.starts_with("claude") || lower.starts_with("anthropic") {
        return lookup("anthropic").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("gpt-") || lower.starts_with("o1") || lower.starts_with("o3") {
        return lookup("openai").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("gemini") {
        return lookup("google").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("mistral") || lower.starts_with("codestral") {
        return lookup("mistral").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("grok") {
        return lookup("xai").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("deepseek") {
        return lookup("deepseek").map(|d| (d, model.to_owned()));
    }
    if lower.starts_with("llama") {
        // Could be Groq, Together, or Ollama — prefer Groq if available
        return lookup("groq").map(|d| (d, model.to_owned()));
    }

    None
}

/// Look up context window for any model string, checking the registry first,
/// then falling back to a default.
pub fn context_window_for(model: &str) -> u64 {
    // Check explicit provider routing
    if let Some((def, model_name)) = resolve_model_string(model) {
        let ctx = def.context_window(&model_name);
        if ctx != 128_000 {
            return ctx;
        }
        // Also check with the full model string (for slash-containing IDs)
        let ctx2 = def.context_window(model);
        if ctx2 != 128_000 {
            return ctx2;
        }
    }

    // Check all providers for a matching model ID
    for provider in PROVIDERS {
        for known in provider.models {
            if known.id == model {
                return known.context_window;
            }
        }
    }

    128_000 // safe default
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_has_providers() {
        assert!(PROVIDERS.len() >= 10);
    }

    #[test]
    fn lookup_known_provider() {
        let openai = lookup("openai").unwrap();
        assert_eq!(openai.name, "OpenAI");
        assert_eq!(openai.api_format, ApiFormat::OpenAiCompatible);
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("nonexistent").is_none());
    }

    #[test]
    fn ollama_requires_no_key() {
        let ollama = lookup("ollama").unwrap();
        assert!(!ollama.requires_key());
        assert!(ollama.is_available());
    }

    #[test]
    fn resolve_explicit_provider_model() {
        let (def, model) = resolve_model_string("openai/gpt-4o").unwrap();
        assert_eq!(def.id, "openai");
        assert_eq!(model, "gpt-4o");
    }

    #[test]
    fn resolve_auto_detect_claude() {
        let (def, _model) = resolve_model_string("claude-sonnet-4-6").unwrap();
        assert_eq!(def.id, "anthropic");
    }

    #[test]
    fn resolve_auto_detect_gpt() {
        let (def, _model) = resolve_model_string("gpt-4o").unwrap();
        assert_eq!(def.id, "openai");
    }

    #[test]
    fn resolve_auto_detect_gemini() {
        let (def, _model) = resolve_model_string("gemini-2.0-flash").unwrap();
        assert_eq!(def.id, "google");
    }

    #[test]
    fn context_window_known_model() {
        assert_eq!(context_window_for("openai/gpt-4o"), 128_000);
        assert_eq!(context_window_for("gemini-2.0-flash"), 1_000_000);
    }

    #[test]
    fn context_window_unknown_model_returns_default() {
        assert_eq!(context_window_for("some-unknown-model"), 128_000);
    }

    #[test]
    fn all_providers_have_required_fields() {
        for p in PROVIDERS {
            assert!(!p.id.is_empty(), "empty id");
            assert!(!p.name.is_empty(), "empty name for {}", p.id);
            assert!(!p.api_base.is_empty(), "empty api_base for {}", p.id);
            assert!(
                !p.default_model.is_empty(),
                "empty default_model for {}",
                p.id
            );
        }
    }
}
