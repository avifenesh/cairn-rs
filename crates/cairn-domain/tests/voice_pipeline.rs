//! GAP-015 voice pipeline integration tests.
//!
//! Validates the voice type contract:
//! - SpeechToTextRequest construction and field access.
//! - VoiceFormat enum variants, MIME types, extensions, and serde round-trip.
//! - TranscriptSegment carries timing and text data correctly.
//! - SpeechToTextResult confidence scoring and is_reliable().
//! - TextToSpeechRequest builder with voice_id and speed.
//! - TextToSpeechResult serialises and reports audio presence.

use cairn_domain::voice::{
    SpeechToTextRequest, SpeechToTextResult, TextToSpeechRequest, TextToSpeechResult,
    TranscriptSegment, VoiceFormat,
};

// ── (1) SpeechToTextRequest with Wav format ───────────────────────────────────

/// (1) Create SpeechToTextRequest with Wav format and verify all fields.
#[test]
fn stt_request_wav_format_fields() {
    // 16-bit mono WAV header stub (44 bytes zeros + payload).
    let fake_wav: Vec<u8> = std::iter::once(b'R')
        .chain(std::iter::once(b'I'))
        .chain(std::iter::once(b'F'))
        .chain(std::iter::once(b'F'))
        .chain(vec![0u8; 40])
        .collect();

    let req = SpeechToTextRequest::new(fake_wav.clone(), VoiceFormat::Wav);

    assert_eq!(req.format, VoiceFormat::Wav, "format must be Wav");
    assert_eq!(
        req.audio_bytes.len(),
        fake_wav.len(),
        "audio_bytes must be preserved"
    );
    assert_eq!(
        req.audio_bytes[0..4],
        [b'R', b'I', b'F', b'F'],
        "audio bytes content preserved"
    );
    assert!(
        req.language.is_none(),
        "language must default to None (auto-detect)"
    );
}

/// SpeechToTextRequest with explicit language hint.
#[test]
fn stt_request_with_language_hint() {
    let req = SpeechToTextRequest::new(vec![1, 2, 3, 4], VoiceFormat::Wav).with_language("en-US");

    assert_eq!(
        req.language.as_deref(),
        Some("en-US"),
        "language must be set"
    );
    assert_eq!(req.format, VoiceFormat::Wav);
}

/// SpeechToTextRequest supports all four VoiceFormat variants.
#[test]
fn stt_request_accepts_any_format() {
    for fmt in [
        VoiceFormat::Wav,
        VoiceFormat::Mp3,
        VoiceFormat::Ogg,
        VoiceFormat::Webm,
    ] {
        let req = SpeechToTextRequest::new(vec![0u8; 8], fmt);
        assert_eq!(req.format, fmt, "format must match for {:?}", fmt);
    }
}

// ── (2) Fields serialize correctly ───────────────────────────────────────────
// SpeechToTextRequest itself doesn't derive Serialize/Deserialize.
// The serialisation contract is tested through SpeechToTextResult and
// TranscriptSegment, which do derive Serde.

/// SpeechToTextResult serialises and deserialises without data loss.
#[test]
fn stt_result_serializes_all_fields() {
    let result = SpeechToTextResult {
        transcript: "hello world from Wav audio".to_owned(),
        confidence: 0.93,
        duration_ms: 2_500,
        segments: vec![
            TranscriptSegment {
                start_ms: 0,
                end_ms: 1_200,
                text: "hello world".to_owned(),
            },
            TranscriptSegment {
                start_ms: 1_200,
                end_ms: 2_500,
                text: "from Wav audio".to_owned(),
            },
        ],
    };

    let json = serde_json::to_value(&result).unwrap();

    assert_eq!(json["transcript"], "hello world from Wav audio");
    assert!((json["confidence"].as_f64().unwrap() - 0.93).abs() < 0.001);
    assert_eq!(json["duration_ms"], 2_500u64);
    assert_eq!(json["segments"].as_array().unwrap().len(), 2);
    assert_eq!(json["segments"][0]["text"], "hello world");
    assert_eq!(json["segments"][0]["start_ms"], 0u64);
    assert_eq!(json["segments"][1]["end_ms"], 2_500u64);
}

// ── (3) TextToSpeechRequest with voice_id and speed ──────────────────────────

/// (3) Create TextToSpeechRequest with voice_id and speed; verify all fields.
#[test]
fn tts_request_voice_id_and_speed() {
    let req = TextToSpeechRequest::new(
        "Welcome to Cairn. How can I help you today?",
        "en-US-AriaNeural",
    )
    .with_speed(1.25)
    .with_format(VoiceFormat::Wav);

    assert_eq!(req.text, "Welcome to Cairn. How can I help you today?");
    assert_eq!(req.voice_id, "en-US-AriaNeural");
    assert!((req.speed - 1.25).abs() < 1e-6, "speed must be 1.25");
    assert_eq!(req.format, VoiceFormat::Wav, "format must be Wav");
}

/// TextToSpeechRequest defaults: speed=1.0, format=Mp3.
#[test]
fn tts_request_default_speed_and_format() {
    let req = TextToSpeechRequest::new("Hello.", "voice_default");
    assert!((req.speed - 1.0).abs() < 1e-6, "default speed must be 1.0");
    assert_eq!(req.format, VoiceFormat::Mp3, "default format must be Mp3");
}

/// Speed can be set to sub-normal (slow-read) and super-normal (fast-read).
#[test]
fn tts_request_speed_variants() {
    let slow = TextToSpeechRequest::new("Slow text.", "voice_a").with_speed(0.75);
    let fast = TextToSpeechRequest::new("Fast text.", "voice_b").with_speed(2.0);

    assert!((slow.speed - 0.75).abs() < 1e-6);
    assert!((fast.speed - 2.0).abs() < 1e-6);
}

// ── (4) VoiceFormat serde round-trip ─────────────────────────────────────────

/// (4) VoiceFormat enum serialises to snake_case strings and round-trips.
#[test]
fn voice_format_serde_round_trip_all_variants() {
    let cases: &[(VoiceFormat, &str)] = &[
        (VoiceFormat::Wav, "\"wav\""),
        (VoiceFormat::Mp3, "\"mp3\""),
        (VoiceFormat::Ogg, "\"ogg\""),
        (VoiceFormat::Webm, "\"webm\""),
    ];

    for (fmt, expected_json) in cases {
        let serialized = serde_json::to_string(fmt).unwrap();
        assert_eq!(
            serialized, *expected_json,
            "{fmt:?} must serialise to {expected_json}"
        );

        let deserialized: VoiceFormat = serde_json::from_str(*expected_json).unwrap();
        assert_eq!(
            deserialized, *fmt,
            "{expected_json} must deserialise back to {fmt:?}"
        );
    }
}

/// VoiceFormat survives a full JSON document round-trip embedded in a struct.
#[test]
fn voice_format_survives_nested_json_round_trip() {
    let result = TextToSpeechResult {
        audio_bytes: vec![1, 2, 3],
        duration_ms: 1_000,
        format: VoiceFormat::Ogg,
    };

    let json = serde_json::to_string(&result).unwrap();
    let recovered: TextToSpeechResult = serde_json::from_str(&json).unwrap();

    assert_eq!(
        recovered.format,
        VoiceFormat::Ogg,
        "format must survive round-trip"
    );
    assert_eq!(recovered.audio_bytes, vec![1, 2, 3]);
    assert_eq!(recovered.duration_ms, 1_000);
}

// ── (5) TranscriptSegment timing data ────────────────────────────────────────

/// (5) TranscriptSegment carries start_ms, end_ms, and text correctly.
#[test]
fn transcript_segment_timing_data() {
    let segments = vec![
        TranscriptSegment {
            start_ms: 0,
            end_ms: 850,
            text: "Hello,".to_owned(),
        },
        TranscriptSegment {
            start_ms: 850,
            end_ms: 1_600,
            text: "how are".to_owned(),
        },
        TranscriptSegment {
            start_ms: 1_600,
            end_ms: 2_400,
            text: "you today?".to_owned(),
        },
    ];

    // Segments are contiguous (no gaps).
    for window in segments.windows(2) {
        assert_eq!(
            window[0].end_ms, window[1].start_ms,
            "segments must be contiguous"
        );
    }

    // Duration of each segment is positive.
    for seg in &segments {
        assert!(seg.end_ms > seg.start_ms, "end_ms must be after start_ms");
    }

    // Text content is preserved.
    assert_eq!(segments[0].text, "Hello,");
    assert_eq!(segments[1].text, "how are");
    assert_eq!(segments[2].text, "you today?");

    // Full-transcript reconstruction from segments.
    let full: String = segments
        .iter()
        .map(|s| s.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(full, "Hello, how are you today?");
}

/// TranscriptSegment serialises timing fields as integers.
#[test]
fn transcript_segment_serializes_timing_as_integers() {
    let seg = TranscriptSegment {
        start_ms: 1_234,
        end_ms: 5_678,
        text: "Test segment.".to_owned(),
    };
    let json = serde_json::to_value(&seg).unwrap();
    assert_eq!(json["start_ms"], 1_234u64);
    assert_eq!(json["end_ms"], 5_678u64);
    assert_eq!(json["text"], "Test segment.");
}

// ── (6) SpeechToTextResult confidence scoring ────────────────────────────────

/// (6) is_reliable() requires non-empty transcript AND confidence ≥ threshold.
#[test]
fn stt_result_confidence_scoring() {
    let high_conf = SpeechToTextResult {
        transcript: "Clear audio transcription.".to_owned(),
        confidence: 0.96,
        duration_ms: 1_500,
        segments: vec![],
    };

    let low_conf = SpeechToTextResult {
        transcript: "Noisy background unclear.".to_owned(),
        confidence: 0.45,
        duration_ms: 1_500,
        segments: vec![],
    };

    let empty_transcript = SpeechToTextResult {
        transcript: String::new(),
        confidence: 1.0, // perfect confidence but no text
        duration_ms: 0,
        segments: vec![],
    };

    // High confidence above various thresholds.
    assert!(
        high_conf.is_reliable(0.80),
        "0.96 confidence must be reliable at 0.80 threshold"
    );
    assert!(
        high_conf.is_reliable(0.95),
        "0.96 confidence must be reliable at 0.95 threshold"
    );
    assert!(
        !high_conf.is_reliable(0.97),
        "0.96 confidence must fail 0.97 threshold"
    );

    // Low confidence below 0.80 threshold.
    assert!(
        !low_conf.is_reliable(0.80),
        "0.45 confidence must not be reliable at 0.80 threshold"
    );
    assert!(
        low_conf.is_reliable(0.40),
        "0.45 confidence is reliable at 0.40 threshold"
    );

    // Empty transcript is never reliable regardless of confidence.
    assert!(
        !empty_transcript.is_reliable(0.0),
        "empty transcript must never be reliable"
    );
}

/// Confidence is preserved through serde round-trip.
#[test]
fn stt_result_confidence_survives_round_trip() {
    let original = SpeechToTextResult {
        transcript: "serde test".to_owned(),
        confidence: 0.876_543,
        duration_ms: 800,
        segments: vec![],
    };

    let json = serde_json::to_string(&original).unwrap();
    let recovered: SpeechToTextResult = serde_json::from_str(&json).unwrap();

    assert!(
        (recovered.confidence - original.confidence).abs() < 0.001,
        "confidence must survive serde round-trip: expected {} got {}",
        original.confidence,
        recovered.confidence
    );
}

// ── (7) VoiceFormat variants ──────────────────────────────────────────────────

/// (7) All four VoiceFormat variants exist and have correct MIME types and extensions.
#[test]
fn voice_format_all_variants_wav_mp3_ogg_webm() {
    // WAV
    assert_eq!(VoiceFormat::Wav.mime_type(), "audio/wav");
    assert_eq!(VoiceFormat::Wav.extension(), "wav");

    // MP3
    assert_eq!(VoiceFormat::Mp3.mime_type(), "audio/mpeg");
    assert_eq!(VoiceFormat::Mp3.extension(), "mp3");

    // OGG
    assert_eq!(VoiceFormat::Ogg.mime_type(), "audio/ogg");
    assert_eq!(VoiceFormat::Ogg.extension(), "ogg");

    // WEBM
    assert_eq!(VoiceFormat::Webm.mime_type(), "audio/webm");
    assert_eq!(VoiceFormat::Webm.extension(), "webm");
}

/// All VoiceFormat MIME types start with "audio/".
#[test]
fn voice_format_mime_types_all_start_with_audio() {
    for fmt in [
        VoiceFormat::Wav,
        VoiceFormat::Mp3,
        VoiceFormat::Ogg,
        VoiceFormat::Webm,
    ] {
        assert!(
            fmt.mime_type().starts_with("audio/"),
            "{fmt:?} MIME type must start with 'audio/', got '{}'",
            fmt.mime_type()
        );
    }
}

/// All VoiceFormat extensions are non-empty and contain no dots.
#[test]
fn voice_format_extensions_are_clean() {
    for fmt in [
        VoiceFormat::Wav,
        VoiceFormat::Mp3,
        VoiceFormat::Ogg,
        VoiceFormat::Webm,
    ] {
        let ext = fmt.extension();
        assert!(!ext.is_empty(), "{fmt:?} extension must be non-empty");
        assert!(
            !ext.contains('.'),
            "{fmt:?} extension must not contain a dot, got '{ext}'"
        );
        assert!(
            ext.chars().all(|c| c.is_ascii_alphanumeric()),
            "extension must be alphanumeric"
        );
    }
}

/// All four VoiceFormat variants are distinct (no duplicate MIME types or extensions).
#[test]
fn voice_format_variants_are_distinct() {
    let variants = [
        VoiceFormat::Wav,
        VoiceFormat::Mp3,
        VoiceFormat::Ogg,
        VoiceFormat::Webm,
    ];

    let mime_types: Vec<_> = variants.iter().map(|f| f.mime_type()).collect();
    let extensions: Vec<_> = variants.iter().map(|f| f.extension()).collect();

    // Unique MIME types.
    let unique_mimes: std::collections::HashSet<_> = mime_types.iter().collect();
    assert_eq!(unique_mimes.len(), 4, "all MIME types must be distinct");

    // Unique extensions.
    let unique_exts: std::collections::HashSet<_> = extensions.iter().collect();
    assert_eq!(unique_exts.len(), 4, "all extensions must be distinct");
}
