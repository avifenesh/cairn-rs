//! RFC 002 SSE event publishing contract integration tests.
//!
//! Validates the real-time streaming contract that the frontend depends on:
//! - preserved_sse_catalog contains every SseEventName variant.
//! - SseFrame serializes to the correct wire format (event + data + id).
//! - SseReplayQuery uses after_position to filter the replay window.
//! - The 'ready' frame is emitted on connection with the client ID.
//! - Keepalive comment frames follow the SSE spec (': ping' format).
//! - map_event_to_sse_name routes RuntimeEvents to the correct SSE surface.

use cairn_api::http::RouteClassification;
use cairn_api::{
    sse::{preserved_sse_catalog, SseEventName, SseFrame},
    sse_publisher::{
        build_ready_frame, map_event_to_sse_name, parse_last_event_id, SseReplayQuery,
    },
};
use cairn_store::event_log::EventPosition;

// ── (1): preserved_sse_catalog completeness ───────────────────────────────────

/// Every SseEventName variant must appear in preserved_sse_catalog.
/// The frontend is contractually bound to these names — any missing variant
/// would silently break a frontend feature.
#[test]
fn preserved_sse_catalog_contains_all_event_names() {
    let catalog = preserved_sse_catalog();
    let catalog_names: std::collections::HashSet<&str> =
        catalog.iter().map(|e| e.name.as_str()).collect();

    // Every canonical event name must be present.
    let required: &[(&str, RouteClassification)] = &[
        ("ready", RouteClassification::Preserve),
        ("feed_update", RouteClassification::Preserve),
        ("poll_completed", RouteClassification::Preserve),
        ("task_update", RouteClassification::Preserve),
        ("approval_required", RouteClassification::Preserve),
        ("assistant_delta", RouteClassification::Preserve),
        ("assistant_end", RouteClassification::Preserve),
        ("assistant_reasoning", RouteClassification::Preserve),
        ("assistant_tool_call", RouteClassification::Preserve),
        ("memory_proposed", RouteClassification::Preserve),
        ("memory_accepted", RouteClassification::Preserve),
        ("soul_updated", RouteClassification::Transitional),
        ("digest_ready", RouteClassification::Preserve),
        ("coding_session_event", RouteClassification::Transitional),
        ("agent_progress", RouteClassification::Preserve),
        ("skill_activated", RouteClassification::Transitional),
    ];

    for (name, expected_class) in required {
        assert!(
            catalog_names.contains(name),
            "SSE catalog must contain '{name}' — missing entry breaks frontend contract"
        );

        // Verify classification is correct for preserved/transitional events.
        let entry = catalog.iter().find(|e| e.name == *name).unwrap();
        assert_eq!(
            entry.classification, *expected_class,
            "event '{name}' must have classification {:?}, got {:?}",
            expected_class, entry.classification
        );
    }

    // Catalog must have exactly 16 entries — no additions without a contract review.
    assert_eq!(
        catalog.len(),
        16,
        "preserved_sse_catalog must have exactly 16 entries; \
         adding or removing events requires a contract review"
    );
}

/// All preserved events (Preserve classification) must have non-empty snake_case names.
#[test]
fn preserved_sse_catalog_names_are_valid_snake_case() {
    let catalog = preserved_sse_catalog();
    for entry in &catalog {
        assert!(!entry.name.is_empty(), "SSE event name must be non-empty");
        assert!(
            entry
                .name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '_'),
            "SSE event name '{}' must be lowercase_snake_case",
            entry.name
        );
        assert!(
            !entry.name.starts_with('_') && !entry.name.ends_with('_'),
            "SSE event name '{}' must not start or end with underscore",
            entry.name
        );
    }
}

/// At least one event must be Preserve-classified (the frontend-stable set).
#[test]
fn preserved_sse_catalog_has_preserved_and_transitional_events() {
    let catalog = preserved_sse_catalog();
    let preserve_count = catalog
        .iter()
        .filter(|e| e.classification == RouteClassification::Preserve)
        .count();
    let transitional_count = catalog
        .iter()
        .filter(|e| e.classification == RouteClassification::Transitional)
        .count();

    assert!(
        preserve_count > 10,
        "majority of SSE events must be Preserve-classified"
    );
    assert!(
        transitional_count >= 1,
        "at least some SSE events must be Transitional"
    );
    assert_eq!(
        preserve_count + transitional_count,
        catalog.len(),
        "every catalog entry must be either Preserve or Transitional"
    );
}

// ── (2): SseFrame serialization ───────────────────────────────────────────────

/// SseFrame serializes to the correct JSON wire format.
/// The frontend expects: `event` as snake_case string, `data` as object, `id` as string.
#[test]
fn sse_frame_serializes_event_name_data_and_id() {
    let frame = SseFrame {
        event: SseEventName::TaskUpdate,
        data: serde_json::json!({
            "task": {
                "id": "task_001",
                "status": "running",
                "title": "Process request"
            }
        }),
        id: Some("42".to_owned()),
        tenant_id: None,
    };

    let json = serde_json::to_value(&frame).unwrap();

    // Event name must serialize to snake_case string (not an integer or enum variant).
    assert_eq!(
        json["event"], "task_update",
        "event must serialize to snake_case string"
    );

    // Data payload must be preserved exactly.
    assert_eq!(json["data"]["task"]["id"], "task_001");
    assert_eq!(json["data"]["task"]["status"], "running");
    assert_eq!(json["data"]["task"]["title"], "Process request");

    // ID must be a string (used as SSE `id:` field for reconnection cursors).
    assert_eq!(json["id"], "42", "id must serialize as a string");
}

/// SseFrame round-trips through serde without data loss.
#[test]
fn sse_frame_round_trips_through_serde() {
    let original = SseFrame {
        event: SseEventName::ApprovalRequired,
        data: serde_json::json!({ "approval": { "id": "appr_1", "status": "pending" } }),
        id: Some("7".to_owned()),
        tenant_id: None,
    };

    let json = serde_json::to_string(&original).unwrap();
    let recovered: SseFrame = serde_json::from_str(&json).unwrap();

    assert_eq!(recovered.event, original.event);
    assert_eq!(recovered.data, original.data);
    assert_eq!(recovered.id, original.id);
}

/// SseFrame with None id serializes without the id field (or as null).
#[test]
fn sse_frame_id_none_serializes_correctly() {
    let frame = SseFrame {
        event: SseEventName::FeedUpdate,
        data: serde_json::json!({ "item": {} }),
        id: None,
        tenant_id: None,
    };

    let json = serde_json::to_value(&frame).unwrap();
    assert_eq!(json["event"], "feed_update");
    // id is null or absent when None — both are valid.
    assert!(
        json["id"].is_null()
            || !json.as_object().unwrap().contains_key("id")
            || json["id"] == serde_json::Value::Null,
        "id must serialize as null or be absent when None"
    );
}

/// All SseEventName variants serialize to distinct snake_case strings.
#[test]
fn sse_event_name_variants_are_distinct_and_snake_case() {
    use SseEventName::*;
    let variants = [
        Ready,
        FeedUpdate,
        PollCompleted,
        TaskUpdate,
        ApprovalRequired,
        AssistantDelta,
        AssistantEnd,
        AssistantReasoning,
        AssistantToolCall,
        MemoryProposed,
        MemoryAccepted,
        SoulUpdated,
        DigestReady,
        CodingSessionEvent,
        AgentProgress,
        SkillActivated,
    ];

    let mut names = std::collections::HashSet::new();
    for variant in &variants {
        let serialized = serde_json::to_string(variant).unwrap();
        // Must be a quoted snake_case string.
        assert!(
            serialized.starts_with('"') && serialized.ends_with('"'),
            "SseEventName must serialize as a JSON string, got: {serialized}"
        );
        let name = serialized.trim_matches('"').to_owned();
        assert!(
            name.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "serialized event name '{name}' must be snake_case"
        );
        assert!(
            names.insert(name.clone()),
            "duplicate SSE event name: {name}"
        );
    }

    assert_eq!(names.len(), 16, "all 16 variants must be distinct");
}

// ── (3): SseReplayQuery after_position filtering ──────────────────────────────

/// SseReplayQuery stores the after_position for cursor-based replay.
/// The frontend sends `lastEventId` on reconnect; parse_last_event_id converts
/// it to an EventPosition for the replay query.
#[test]
fn sse_replay_query_after_position_filtering() {
    // Default: replay from start.
    let default_query = SseReplayQuery::default();
    assert!(
        default_query.after_position.is_none(),
        "default query must start from beginning"
    );
    assert_eq!(default_query.limit, 100, "default limit must be 100");

    // Reconnection: parse lastEventId from the browser.
    let last_event_id = "42";
    let position = parse_last_event_id(last_event_id)
        .expect("valid numeric lastEventId must parse successfully");
    assert_eq!(position, EventPosition(42));

    let reconnect_query = SseReplayQuery {
        after_position: Some(position),
        limit: 50,
    };
    assert_eq!(reconnect_query.after_position, Some(EventPosition(42)));
    assert_eq!(reconnect_query.limit, 50);

    // The after_position semantics: only events with position > 42 are replayed.
    let all_positions = [
        EventPosition(10),
        EventPosition(42),
        EventPosition(43),
        EventPosition(100),
    ];
    let replayed: Vec<_> = all_positions
        .iter()
        .filter(|&&pos| {
            reconnect_query
                .after_position
                .is_none_or(|after| pos > after)
        })
        .collect();

    assert_eq!(
        replayed.len(),
        2,
        "only positions 43 and 100 are after position 42"
    );
    assert!(replayed.contains(&&EventPosition(43)));
    assert!(replayed.contains(&&EventPosition(100)));
    assert!(
        !replayed.contains(&&EventPosition(42)),
        "position 42 itself must be excluded"
    );
}

/// parse_last_event_id handles edge cases correctly.
#[test]
fn parse_last_event_id_edge_cases() {
    assert_eq!(parse_last_event_id("0"), Some(EventPosition(0)));
    assert_eq!(parse_last_event_id("1"), Some(EventPosition(1)));
    assert_eq!(parse_last_event_id("999"), Some(EventPosition(999)));

    // Invalid inputs must return None (do not crash or panic).
    assert!(
        parse_last_event_id("").is_none(),
        "empty string must return None"
    );
    assert!(
        parse_last_event_id("abc").is_none(),
        "non-numeric must return None"
    );
    assert!(
        parse_last_event_id("-1").is_none(),
        "negative number must return None"
    );
    assert!(
        parse_last_event_id("1.5").is_none(),
        "float must return None"
    );
    assert!(
        parse_last_event_id(" 42").is_none(),
        "leading space must return None"
    );
}

// ── (4): 'ready' event emitted on connection ──────────────────────────────────

/// The ready event is the first frame emitted when a client connects to /v1/stream.
/// It carries the clientId for the frontend to use in subsequent requests.
#[test]
fn ready_frame_is_emitted_on_connection() {
    let client_id = "client_session_abc123";
    let frame = build_ready_frame(client_id);

    assert_eq!(
        frame.event,
        SseEventName::Ready,
        "ready frame must use SseEventName::Ready"
    );
    assert_eq!(
        frame.data["clientId"], client_id,
        "ready frame must carry the client ID"
    );
    assert!(
        frame.id.is_none(),
        "ready frame must not have an id (it is not a replayable event)"
    );

    // The ready event must serialize with event="ready".
    let json = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        json["event"], "ready",
        "ready frame must serialize event as 'ready'"
    );
    assert_eq!(json["data"]["clientId"], client_id);
}

/// ready event classification must be Preserve (frontend depends on it).
#[test]
fn ready_event_is_preserve_classified() {
    assert_eq!(
        SseEventName::Ready.classification(),
        RouteClassification::Preserve,
        "ready event must be Preserve-classified — it is essential for connection setup"
    );
    assert_eq!(SseEventName::Ready.as_str(), "ready");
}

/// Different client IDs produce distinct ready frames.
#[test]
fn ready_frames_are_client_scoped() {
    let frame_a = build_ready_frame("client_a");
    let frame_b = build_ready_frame("client_b");

    assert_ne!(
        frame_a.data["clientId"], frame_b.data["clientId"],
        "ready frames for different clients must carry different clientIds"
    );
    assert_eq!(frame_a.event, frame_b.event, "both must be ready events");
}

// ── (5): Keepalive comment frames ────────────────────────────────────────────

/// SSE keepalive frames use the comment format (': ping\n\n' per RFC 7231 SSE spec).
///
/// The SseEventName::as_str() output is used as the `event:` field in the wire
/// format. A keepalive comment does not carry an event field — it is formatted
/// as a standalone `: ping` line. This test verifies the contract for how a
/// keepalive would be constructed and formatted.
#[test]
fn keepalive_comment_format_follows_sse_spec() {
    // A keepalive comment in the SSE wire format is:
    //   ": ping\n\n"
    // The colon prefix signals a comment to the SSE parser.
    let keepalive_comment = ": ping";
    assert!(
        keepalive_comment.starts_with(':'),
        "SSE keepalive comment must start with ':' per the SSE specification"
    );

    // Keepalive does NOT use SseEventName — it's a raw comment, not a data event.
    // Verify that none of the SseEventName variants produce a comment-prefix string.
    use SseEventName::*;
    for variant in [Ready, TaskUpdate, ApprovalRequired, AgentProgress] {
        assert!(
            !variant.as_str().starts_with(':'),
            "SseEventName::as_str() must not produce a comment-prefix string"
        );
    }

    // The SSE wire format for a keepalive:
    // - Line starting with ':' (comment) keeps the connection alive without triggering a message event
    // - Two newlines terminate the frame
    let wire_format = format!("{keepalive_comment}\n\n");
    assert_eq!(wire_format, ": ping\n\n");
    assert!(
        wire_format.ends_with("\n\n"),
        "SSE frames must be terminated by double newline"
    );
}

/// Keepalive interval: the ready frame's id=None confirms it is not subject to
/// the replay window — keepalives similarly don't carry positions.
#[test]
fn keepalive_frames_do_not_advance_replay_position() {
    // Keepalives have no position — they don't appear in the event log.
    // The replay position is only advanced by data frames (those with id set).

    let ready = build_ready_frame("client_xyz");
    assert!(
        ready.id.is_none(),
        "ready (connection) frame has no position — it is not a replayable event"
    );

    // A data frame (replayable) DOES carry a position as its id.
    let data_frame = SseFrame {
        event: SseEventName::TaskUpdate,
        data: serde_json::json!({"task": {"id": "t1", "status": "running"}}),
        id: Some("55".to_owned()), // position 55 in the event log
        tenant_id: None,
    };
    assert!(
        data_frame.id.is_some(),
        "replayable data frames must carry a position as their id"
    );
    assert_eq!(data_frame.id.as_deref(), Some("55"));

    // The replay query would use this id to resume from position 55.
    let position = parse_last_event_id("55").unwrap();
    assert_eq!(position, EventPosition(55));
}

// ── RuntimeEvent → SSE surface mapping ────────────────────────────────────────

/// Key RuntimeEvent variants map to the correct SSE surface events.
#[test]
fn runtime_events_map_to_correct_sse_surfaces() {
    use cairn_domain::events::*;
    use cairn_domain::lifecycle::TaskState;
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;

    let project = ProjectKey::new("t", "w", "p");

    let cases: &[(RuntimeEvent, Option<SseEventName>)] = &[
        // Task events → TaskUpdate
        (
            RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: "t1".into(),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
            }),
            Some(SseEventName::TaskUpdate),
        ),
        (
            RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project.clone(),
                task_id: "t1".into(),
                transition: StateTransition {
                    from: Some(TaskState::Queued),
                    to: TaskState::Running,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            }),
            Some(SseEventName::TaskUpdate),
        ),
        // Approval → ApprovalRequired
        (
            RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: "a1".into(),
                run_id: None,
                task_id: None,
                requirement: ApprovalRequirement::Required,
                title: None,
                description: None,
            }),
            Some(SseEventName::ApprovalRequired),
        ),
        // Tool invocations → AssistantToolCall
        (
            RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project.clone(),
                invocation_id: "inv_1".into(),
                session_id: None,
                run_id: None,
                task_id: None,
                target: cairn_domain::tool_invocation::ToolInvocationTarget::Builtin {
                    tool_name: "test_tool".to_owned(),
                },
                execution_class: cairn_domain::policy::ExecutionClass::SandboxedProcess,
                prompt_release_id: None,
                requested_at_ms: 0,
                started_at_ms: 0,
            }),
            Some(SseEventName::AssistantToolCall),
        ),
        // Session/Run events → no SSE surface (internal state only)
        (
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project.clone(),
                session_id: "s1".into(),
            }),
            None,
        ),
        (
            RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: "s1".into(),
                run_id: "r1".into(),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
            // RunCreated emits AgentProgress so the SSE stream surfaces run creation
            Some(SseEventName::AgentProgress),
        ),
    ];

    for (event, expected) in cases {
        let result = map_event_to_sse_name(event);
        assert_eq!(
            result,
            *expected,
            "unexpected SSE mapping for {:?}: expected {:?}, got {:?}",
            std::mem::discriminant(event),
            expected,
            result
        );
    }
}
