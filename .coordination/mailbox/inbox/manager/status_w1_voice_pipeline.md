# STATUS: voice_pipeline

**Task:** GAP-015 voice pipeline integration test  
**Tests passed:** 17/17  
**File:** `crates/cairn-domain/tests/voice_pipeline.rs`

Tests:
- `stt_request_wav_format_fields`
- `stt_request_with_language_hint`
- `stt_request_accepts_any_format`
- `stt_result_serializes_all_fields`
- `tts_request_voice_id_and_speed`
- `tts_request_default_speed_and_format`
- `tts_request_speed_variants`
- `voice_format_serde_round_trip_all_variants`
- `voice_format_survives_nested_json_round_trip`
- `transcript_segment_timing_data`
- `transcript_segment_serializes_timing_as_integers`
- `stt_result_confidence_scoring`
- `stt_result_confidence_survives_round_trip`
- `voice_format_all_variants_wav_mp3_ogg_webm`
- `voice_format_mime_types_all_start_with_audio`
- `voice_format_extensions_are_clean`
- `voice_format_variants_are_distinct`
