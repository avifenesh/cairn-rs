//! Voice STT/TTS domain types (GAP-015).
//!
//! Mirrors `cairn/internal/voice/` — whisper.cpp speech-to-text and
//! edge-tts text-to-speech. These are pure value types; the service
//! traits live in `cairn-runtime::voice`.

use serde::{Deserialize, Serialize};

// ── Audio format ───────────────────────────────────────────────────────────

/// Audio container format for voice I/O.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VoiceFormat {
    /// PCM audio in a RIFF/WAV container.
    Wav,
    /// MPEG Layer III compressed audio.
    Mp3,
    /// Ogg Vorbis compressed audio.
    Ogg,
    /// WebM container (typically with Opus codec).
    Webm,
}

impl VoiceFormat {
    /// Canonical MIME type for this format.
    pub fn mime_type(self) -> &'static str {
        match self {
            VoiceFormat::Wav => "audio/wav",
            VoiceFormat::Mp3 => "audio/mpeg",
            VoiceFormat::Ogg => "audio/ogg",
            VoiceFormat::Webm => "audio/webm",
        }
    }

    /// Conventional file extension (without leading dot).
    pub fn extension(self) -> &'static str {
        match self {
            VoiceFormat::Wav => "wav",
            VoiceFormat::Mp3 => "mp3",
            VoiceFormat::Ogg => "ogg",
            VoiceFormat::Webm => "webm",
        }
    }
}

// ── Speech-to-Text ─────────────────────────────────────────────────────────

/// Request to transcribe audio to text.
#[derive(Clone, Debug)]
pub struct SpeechToTextRequest {
    /// Raw audio bytes in the specified format.
    pub audio_bytes: Vec<u8>,
    /// Container format of `audio_bytes`.
    pub format: VoiceFormat,
    /// BCP-47 language hint (e.g. `"en"`, `"fr"`). `None` = auto-detect.
    pub language: Option<String>,
}

impl SpeechToTextRequest {
    pub fn new(audio_bytes: Vec<u8>, format: VoiceFormat) -> Self {
        Self {
            audio_bytes,
            format,
            language: None,
        }
    }

    pub fn with_language(mut self, lang: impl Into<String>) -> Self {
        self.language = Some(lang.into());
        self
    }
}

/// A time-aligned segment within a transcript.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TranscriptSegment {
    /// Segment start time in milliseconds from audio start.
    pub start_ms: u64,
    /// Segment end time in milliseconds from audio start.
    pub end_ms: u64,
    /// Transcribed text for this segment.
    pub text: String,
}

/// Result of a speech-to-text transcription.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SpeechToTextResult {
    /// Full transcript (concatenation of all segments).
    pub transcript: String,
    /// Model confidence score in [0.0, 1.0].
    pub confidence: f32,
    /// Total audio duration in milliseconds.
    pub duration_ms: u64,
    /// Time-aligned segments (may be empty for non-segmented backends).
    pub segments: Vec<TranscriptSegment>,
}

impl SpeechToTextResult {
    /// Whether the transcript is non-empty and confidence is above `threshold`.
    pub fn is_reliable(&self, threshold: f32) -> bool {
        !self.transcript.is_empty() && self.confidence >= threshold
    }
}

// ── Text-to-Speech ─────────────────────────────────────────────────────────

/// Request to synthesize speech from text.
#[derive(Clone, Debug)]
pub struct TextToSpeechRequest {
    /// Text to synthesize (plain text; SSML not required but may be supported).
    pub text: String,
    /// Voice identifier (backend-specific, e.g. `"en-US-AriaNeural"` for edge-tts).
    pub voice_id: String,
    /// Playback speed multiplier. `1.0` = normal, `0.5` = half-speed, `2.0` = double.
    pub speed: f32,
    /// Desired output format.
    pub format: VoiceFormat,
}

impl TextToSpeechRequest {
    pub fn new(text: impl Into<String>, voice_id: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            voice_id: voice_id.into(),
            speed: 1.0,
            format: VoiceFormat::Mp3,
        }
    }

    pub fn with_format(mut self, format: VoiceFormat) -> Self {
        self.format = format;
        self
    }

    pub fn with_speed(mut self, speed: f32) -> Self {
        self.speed = speed;
        self
    }
}

/// Result of a text-to-speech synthesis.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextToSpeechResult {
    /// Synthesized audio bytes in `format`.
    pub audio_bytes: Vec<u8>,
    /// Estimated audio duration in milliseconds.
    pub duration_ms: u64,
    /// Format of `audio_bytes`.
    pub format: VoiceFormat,
}

impl TextToSpeechResult {
    /// Whether synthesis produced any audio.
    pub fn has_audio(&self) -> bool {
        !self.audio_bytes.is_empty()
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_format_mime_types() {
        assert_eq!(VoiceFormat::Wav.mime_type(), "audio/wav");
        assert_eq!(VoiceFormat::Mp3.mime_type(), "audio/mpeg");
        assert_eq!(VoiceFormat::Ogg.mime_type(), "audio/ogg");
        assert_eq!(VoiceFormat::Webm.mime_type(), "audio/webm");
    }

    #[test]
    fn voice_format_extensions() {
        assert_eq!(VoiceFormat::Wav.extension(), "wav");
        assert_eq!(VoiceFormat::Mp3.extension(), "mp3");
        assert_eq!(VoiceFormat::Ogg.extension(), "ogg");
        assert_eq!(VoiceFormat::Webm.extension(), "webm");
    }

    #[test]
    fn voice_format_serde_roundtrip() {
        for fmt in [
            VoiceFormat::Wav,
            VoiceFormat::Mp3,
            VoiceFormat::Ogg,
            VoiceFormat::Webm,
        ] {
            let json = serde_json::to_string(&fmt).unwrap();
            let back: VoiceFormat = serde_json::from_str(&json).unwrap();
            assert_eq!(back, fmt);
        }
    }

    #[test]
    fn stt_request_builder() {
        let req = SpeechToTextRequest::new(vec![1, 2, 3], VoiceFormat::Wav).with_language("en");
        assert_eq!(req.format, VoiceFormat::Wav);
        assert_eq!(req.language.as_deref(), Some("en"));
        assert_eq!(req.audio_bytes.len(), 3);
    }

    #[test]
    fn stt_result_is_reliable() {
        let result = SpeechToTextResult {
            transcript: "hello world".to_owned(),
            confidence: 0.92,
            duration_ms: 1200,
            segments: vec![],
        };
        assert!(result.is_reliable(0.8));
        assert!(!result.is_reliable(0.95));
    }

    #[test]
    fn stt_result_empty_transcript_not_reliable() {
        let result = SpeechToTextResult {
            transcript: String::new(),
            confidence: 1.0,
            duration_ms: 0,
            segments: vec![],
        };
        assert!(!result.is_reliable(0.0));
    }

    #[test]
    fn tts_request_builder() {
        let req = TextToSpeechRequest::new("hello", "en-US-AriaNeural")
            .with_format(VoiceFormat::Ogg)
            .with_speed(1.5);
        assert_eq!(req.format, VoiceFormat::Ogg);
        assert!((req.speed - 1.5).abs() < 1e-6);
        assert_eq!(req.voice_id, "en-US-AriaNeural");
    }

    #[test]
    fn tts_result_has_audio() {
        let empty = TextToSpeechResult {
            audio_bytes: vec![],
            duration_ms: 0,
            format: VoiceFormat::Mp3,
        };
        assert!(!empty.has_audio());
        let filled = TextToSpeechResult {
            audio_bytes: vec![0u8; 64],
            duration_ms: 500,
            format: VoiceFormat::Mp3,
        };
        assert!(filled.has_audio());
    }

    #[test]
    fn transcript_segment_fields() {
        let seg = TranscriptSegment {
            start_ms: 0,
            end_ms: 1500,
            text: "hi there".to_owned(),
        };
        assert_eq!(seg.end_ms - seg.start_ms, 1500);
        assert_eq!(seg.text, "hi there");
    }
}
