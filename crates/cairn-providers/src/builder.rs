//! Runtime provider construction.
//!
//! [`ProviderBuilder`] constructs any backend from a [`Backend`] enum and
//! runtime configuration.  Operators add providers through the API with
//! their own endpoint URL, API key, and model.

use crate::chat::ChatProvider;
use crate::error::ProviderError;
use crate::wire::openai_compat::{OpenAiCompat, ProviderConfig};

/// Supported LLM backend families.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Backend {
    OpenAI,
    Anthropic,
    Ollama,
    DeepSeek,
    Xai,
    Google,
    Groq,
    AzureOpenAI,
    OpenRouter,
    MiniMax,
    /// AWS Bedrock Converse API — full-featured (guardrails, documents, tool_config).
    Bedrock,
    /// AWS Bedrock OpenAI-compatible gateway — simpler, standard wire format.
    BedrockCompat,
    /// Any OpenAI-compatible endpoint — operator supplies URL + key.
    OpenAiCompatible,
}

impl std::str::FromStr for Backend {
    type Err = ProviderError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(Self::OpenAI),
            "anthropic" => Ok(Self::Anthropic),
            "ollama" => Ok(Self::Ollama),
            "deepseek" => Ok(Self::DeepSeek),
            "xai" => Ok(Self::Xai),
            "google" | "gemini" => Ok(Self::Google),
            "groq" => Ok(Self::Groq),
            "azure-openai" | "azure_openai" | "azureopenai" => Ok(Self::AzureOpenAI),
            "openrouter" => Ok(Self::OpenRouter),
            "minimax" => Ok(Self::MiniMax),
            "bedrock" | "aws-bedrock" | "bedrock-converse" => Ok(Self::Bedrock),
            "bedrock-compat" | "bedrock-openai" | "bedrock_compat" => Ok(Self::BedrockCompat),
            "openai-compatible" | "openai_compatible" | "generic" | "custom" => {
                Ok(Self::OpenAiCompatible)
            }
            _ => Err(ProviderError::InvalidRequest(format!(
                "unknown backend: {s}"
            ))),
        }
    }
}

impl std::fmt::Display for Backend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::OpenAI => "openai",
            Self::Anthropic => "anthropic",
            Self::Ollama => "ollama",
            Self::DeepSeek => "deepseek",
            Self::Xai => "xai",
            Self::Google => "google",
            Self::Groq => "groq",
            Self::AzureOpenAI => "azure-openai",
            Self::OpenRouter => "openrouter",
            Self::MiniMax => "minimax",
            Self::Bedrock => "bedrock",
            Self::BedrockCompat => "bedrock-compat",
            Self::OpenAiCompatible => "openai-compatible",
        })
    }
}

impl Backend {
    /// Get the preset [`ProviderConfig`] for this backend.
    pub fn config(&self) -> ProviderConfig {
        match self {
            Self::OpenAI => ProviderConfig::OPENAI,
            Self::Anthropic => ProviderConfig::ANTHROPIC,
            Self::Ollama => ProviderConfig::OLLAMA,
            Self::DeepSeek => ProviderConfig::DEEPSEEK,
            Self::Xai => ProviderConfig::XAI,
            Self::Google => ProviderConfig::GOOGLE,
            Self::Groq => ProviderConfig::GROQ,
            Self::AzureOpenAI => ProviderConfig::AZURE_OPENAI,
            Self::OpenRouter => ProviderConfig::OPENROUTER,
            Self::MiniMax => ProviderConfig::MINIMAX,
            Self::BedrockCompat => ProviderConfig::BEDROCK_COMPAT,
            Self::Bedrock | Self::OpenAiCompatible => ProviderConfig::default(),
        }
    }
}

/// Builder for constructing any LLM provider at runtime.
pub struct ProviderBuilder {
    backend: Backend,
    api_key: Option<String>,
    base_url: Option<String>,
    model: Option<String>,
    max_tokens: Option<u32>,
    temperature: Option<f32>,
    timeout_secs: Option<u64>,
    region: Option<String>,
}

impl ProviderBuilder {
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            api_key: None,
            base_url: None,
            model: None,
            max_tokens: None,
            temperature: None,
            timeout_secs: None,
            region: None,
        }
    }

    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }
    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }
    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }
    pub fn timeout_secs(mut self, s: u64) -> Self {
        self.timeout_secs = Some(s);
        self
    }
    pub fn region(mut self, r: impl Into<String>) -> Self {
        self.region = Some(r.into());
        self
    }

    /// Build a boxed [`ChatProvider`].
    pub fn build_chat(self) -> Result<Box<dyn ChatProvider>, ProviderError> {
        if self.backend == Backend::OpenAiCompatible && self.base_url.is_none() {
            return Err(ProviderError::InvalidRequest(
                "endpoint URL required for generic backend".to_owned(),
            ));
        }
        let key = self.api_key.unwrap_or_default();

        // Bedrock has its own wire format.
        if self.backend == Backend::Bedrock {
            let region = self.region.unwrap_or_else(|| "us-west-2".to_owned());
            let model = self
                .model
                .unwrap_or_else(|| "minimax.minimax-m2.5".to_owned());
            return Ok(Box::new(crate::backends::bedrock::Bedrock::new(
                model, region, key,
            )));
        }

        // Everything else is OpenAI-compatible — one struct, different config.
        let config = self.backend.config();
        Ok(Box::new(OpenAiCompat::new(
            config,
            key,
            self.base_url,
            self.model,
            self.max_tokens,
            self.temperature,
            self.timeout_secs,
        )?))
    }
}
