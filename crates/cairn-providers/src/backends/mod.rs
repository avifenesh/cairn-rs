//! Backend implementations.
//!
//! Most backends use the shared [`OpenAiCompat`] with different
//! [`ProviderConfig`] presets.  Only Bedrock has its own wire format.

pub mod bedrock;
