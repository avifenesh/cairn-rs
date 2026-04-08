//! Voice service boundaries (GAP-015).
//!
//! Async traits for speech-to-text and text-to-speech. Concrete
//! implementations wire to whisper.cpp (STT) or edge-tts (TTS).
//! `InMemoryVoiceService` in `services::voice_impl` provides a
//! stub for tests and deployments without a voice back-end.

use async_trait::async_trait;
use cairn_domain::voice::{
    SpeechToTextRequest, SpeechToTextResult, TextToSpeechRequest, TextToSpeechResult,
};

use crate::error::RuntimeError;

/// Speech-to-text transcription service boundary.
#[async_trait]
pub trait SpeechToTextService: Send + Sync {
    /// Transcribe the audio in `req` and return the transcript.
    async fn transcribe(
        &self,
        req: SpeechToTextRequest,
    ) -> Result<SpeechToTextResult, RuntimeError>;
}

/// Text-to-speech synthesis service boundary.
#[async_trait]
pub trait TextToSpeechService: Send + Sync {
    /// Synthesize speech from the text in `req` and return audio bytes.
    async fn synthesize(
        &self,
        req: TextToSpeechRequest,
    ) -> Result<TextToSpeechResult, RuntimeError>;
}
