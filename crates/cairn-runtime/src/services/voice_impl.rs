//! In-memory (stub) voice service implementation (GAP-015).
//!
//! Returns canned results for testing and for deployments that have not
//! configured a real STT/TTS back-end. No audio processing is performed.

use async_trait::async_trait;
use cairn_domain::voice::{
    SpeechToTextRequest, SpeechToTextResult, TextToSpeechRequest, TextToSpeechResult,
    TranscriptSegment,
};

use crate::error::RuntimeError;
use crate::voice::{SpeechToTextService, TextToSpeechService};

// ── InMemoryVoiceService ───────────────────────────────────────────────────

/// Stub voice service for testing.
///
/// `transcribe` returns a fixed transcript;
/// `synthesize` returns an empty-bytes result with the requested format.
pub struct InMemoryVoiceService {
    /// Canned transcript returned by `transcribe`.
    pub canned_transcript: String,
    /// Confidence score returned by `transcribe` (default 0.95).
    pub canned_confidence: f32,
}

impl InMemoryVoiceService {
    /// Stub that returns `"stub transcript"` with confidence 0.95.
    pub fn new() -> Self {
        Self {
            canned_transcript: "stub transcript".to_owned(),
            canned_confidence: 0.95,
        }
    }

    /// Override the canned transcript returned by this stub.
    pub fn with_transcript(mut self, transcript: impl Into<String>) -> Self {
        self.canned_transcript = transcript.into();
        self
    }
}

impl Default for InMemoryVoiceService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SpeechToTextService for InMemoryVoiceService {
    async fn transcribe(
        &self,
        req: SpeechToTextRequest,
    ) -> Result<SpeechToTextResult, RuntimeError> {
        // Estimate duration from byte count: assume 16 kHz mono 16-bit PCM ≈ 32 bytes/ms.
        let duration_ms = (req.audio_bytes.len() as u64).saturating_div(32).max(1);
        let segment = TranscriptSegment {
            start_ms: 0,
            end_ms: duration_ms,
            text: self.canned_transcript.clone(),
        };
        Ok(SpeechToTextResult {
            transcript: self.canned_transcript.clone(),
            confidence: self.canned_confidence,
            duration_ms,
            segments: vec![segment],
        })
    }
}

#[async_trait]
impl TextToSpeechService for InMemoryVoiceService {
    async fn synthesize(
        &self,
        req: TextToSpeechRequest,
    ) -> Result<TextToSpeechResult, RuntimeError> {
        // Estimate duration: ~150 words per minute, each word ~400 ms.
        let word_count = req.text.split_whitespace().count() as u64;
        let duration_ms = word_count * 400;
        Ok(TextToSpeechResult {
            audio_bytes: vec![], // no real synthesis
            duration_ms,
            format: req.format,
        })
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::voice::VoiceFormat;

    // 1. transcribe returns canned transcript
    #[tokio::test]
    async fn transcribe_returns_canned_transcript() {
        let svc = InMemoryVoiceService::new();
        let req = SpeechToTextRequest::new(vec![0u8; 320], VoiceFormat::Wav);
        let result = svc.transcribe(req).await.unwrap();
        assert_eq!(result.transcript, "stub transcript");
    }

    // 2. transcribe confidence is set
    #[tokio::test]
    async fn transcribe_confidence_is_set() {
        let svc = InMemoryVoiceService::new();
        let req = SpeechToTextRequest::new(vec![0u8; 64], VoiceFormat::Wav);
        let result = svc.transcribe(req).await.unwrap();
        assert!((result.confidence - 0.95).abs() < 1e-6);
    }

    // 3. transcribe with custom canned transcript
    #[tokio::test]
    async fn transcribe_custom_canned_transcript() {
        let svc = InMemoryVoiceService::new().with_transcript("hello world");
        let req = SpeechToTextRequest::new(vec![0u8; 32], VoiceFormat::Webm);
        let result = svc.transcribe(req).await.unwrap();
        assert_eq!(result.transcript, "hello world");
    }

    // 4. transcribe result has one segment
    #[tokio::test]
    async fn transcribe_result_has_segment() {
        let svc = InMemoryVoiceService::new();
        let req = SpeechToTextRequest::new(vec![0u8; 320], VoiceFormat::Wav);
        let result = svc.transcribe(req).await.unwrap();
        assert_eq!(result.segments.len(), 1);
        assert_eq!(result.segments[0].text, result.transcript);
    }

    // 5. transcribe duration estimated from byte count
    #[tokio::test]
    async fn transcribe_duration_estimated_from_bytes() {
        let svc = InMemoryVoiceService::new();
        // 3200 bytes / 32 bytes per ms = 100 ms
        let req = SpeechToTextRequest::new(vec![0u8; 3200], VoiceFormat::Wav);
        let result = svc.transcribe(req).await.unwrap();
        assert_eq!(result.duration_ms, 100);
    }

    // 6. synthesize returns requested format
    #[tokio::test]
    async fn synthesize_returns_requested_format() {
        let svc = InMemoryVoiceService::new();
        let req = TextToSpeechRequest::new("hello there", "en-US-AriaNeural")
            .with_format(VoiceFormat::Ogg);
        let result = svc.synthesize(req).await.unwrap();
        assert_eq!(result.format, VoiceFormat::Ogg);
    }

    // 7. synthesize stub returns empty audio bytes
    #[tokio::test]
    async fn synthesize_stub_returns_empty_bytes() {
        let svc = InMemoryVoiceService::new();
        let req = TextToSpeechRequest::new("test", "voice-1");
        let result = svc.synthesize(req).await.unwrap();
        assert!(!result.has_audio(), "stub must return empty audio bytes");
    }

    // 8. synthesize duration estimated from word count
    #[tokio::test]
    async fn synthesize_duration_from_word_count() {
        let svc = InMemoryVoiceService::new();
        // "one two three" = 3 words × 400 ms = 1200 ms
        let req = TextToSpeechRequest::new("one two three", "v");
        let result = svc.synthesize(req).await.unwrap();
        assert_eq!(result.duration_ms, 1200);
    }
}
