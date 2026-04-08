//! RFC 002 user message lifecycle integration tests.
//!
//! `UserMessageAppended` carries user input into the session/run context.
//! Messages are stored in the durable event log and retrieved via
//! read_by_entity (scoped to the Run entity) or read_stream.
//!
//! Validates:
//! - UserMessageAppended lands in the event log with content preserved.
//! - Multiple messages to the same run are retrievable in order.
//! - Messages are session-scoped and do not leak between sessions.
//! - Content round-trips through the event log without loss.

use std::sync::Arc;

use cairn_domain::events::UserMessageAppended;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId,
};
use cairn_store::{event_log::EntityRef, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_msg", "ws_msg", "proj_msg")
}

fn session(n: &str) -> SessionId {
    SessionId::new(format!("sess_msg_{n}"))
}
fn run(n: &str) -> RunId {
    RunId::new(format!("run_msg_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn msg_event(
    id: &str,
    sess: &str,
    r: &str,
    content: &str,
    sequence: u64,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        id,
        RuntimeEvent::UserMessageAppended(UserMessageAppended {
            project: project(),
            session_id: session(sess),
            run_id: run(r),
            content: content.to_owned(),
            sequence,
            appended_at_ms: ts,
        }),
    )
}

/// Seed a session + run pair.
async fn seed_session_run(store: &Arc<InMemoryStore>, n: &str) {
    store
        .append(&[
            ev(
                &format!("evt_sess_{n}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session(n),
                }),
            ),
            ev(
                &format!("evt_run_{n}"),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session(n),
                    run_id: run(n),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2) + (3): Create session; append UserMessageAppended; verify it is
/// stored in the event log with content and session context.
#[tokio::test]
async fn user_message_appended_is_stored_in_session_context() {
    let store = Arc::new(InMemoryStore::new());
    seed_session_run(&store, "a").await;

    // (2) Append a user message.
    store
        .append(&[msg_event(
            "evt_msg_1",
            "a",
            "a",
            "Hello Cairn — what tasks are running?",
            1,
            10_000,
        )])
        .await
        .unwrap();

    // (3) Message must be in the event log.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    let msg = events.iter().find(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::UserMessageAppended(m)
                if m.session_id == session("a")
                && m.run_id == run("a")
                && m.content == "Hello Cairn — what tasks are running?"
        )
    });
    assert!(
        msg.is_some(),
        "UserMessageAppended must appear in the event log"
    );

    // Message carries the correct session context.
    if let RuntimeEvent::UserMessageAppended(m) = &msg.unwrap().envelope.payload {
        assert_eq!(m.session_id, session("a"), "session_id must match");
        assert_eq!(m.run_id, run("a"), "run_id must match");
        assert_eq!(m.sequence, 1, "sequence must be preserved");
        assert_eq!(m.appended_at_ms, 10_000, "timestamp must be preserved");
    }
}

/// (4) + (5): Multiple messages to the same session/run are all stored and
/// can be retrieved in sequence order via read_by_entity.
#[tokio::test]
async fn multiple_messages_stored_and_ordered_by_sequence() {
    let store = Arc::new(InMemoryStore::new());
    seed_session_run(&store, "b").await;

    // Append 4 messages at increasing timestamps.
    store
        .append(&[
            msg_event("evt_msg_b1", "b", "b", "First message", 1, 1_000),
            msg_event("evt_msg_b2", "b", "b", "Second message", 2, 2_000),
            msg_event("evt_msg_b3", "b", "b", "Third message", 3, 3_000),
            msg_event("evt_msg_b4", "b", "b", "Fourth message", 4, 4_000),
        ])
        .await
        .unwrap();

    // Read events scoped to the run entity — returns only events for this run.
    let run_events = EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("b")), None, 100)
        .await
        .unwrap();

    // Extract UserMessageAppended events from the run's entity stream.
    let messages: Vec<&UserMessageAppended> = run_events
        .iter()
        .filter_map(|e| {
            if let RuntimeEvent::UserMessageAppended(m) = &e.envelope.payload {
                Some(m)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        messages.len(),
        4,
        "all 4 messages must be present in the run's event stream"
    );

    // Messages are returned in log-append order (monotonic positions).
    for window in messages.windows(2) {
        assert!(
            window[0].sequence < window[1].sequence,
            "messages must be in ascending sequence order: {} < {}",
            window[0].sequence,
            window[1].sequence
        );
        assert!(
            window[0].appended_at_ms < window[1].appended_at_ms,
            "message timestamps must be ascending"
        );
    }

    // Specific content is preserved per message.
    assert_eq!(messages[0].content, "First message");
    assert_eq!(messages[3].content, "Fourth message");
}

/// (6): Messages from different sessions are isolated — read_by_entity for
/// one run must not return messages from a different run/session.
#[tokio::test]
async fn messages_from_different_sessions_are_isolated() {
    let store = Arc::new(InMemoryStore::new());
    seed_session_run(&store, "x").await;
    seed_session_run(&store, "y").await;

    store
        .append(&[
            msg_event("evt_msg_x1", "x", "x", "Message for session X", 1, 1_000),
            msg_event("evt_msg_y1", "y", "y", "Message for session Y", 1, 2_000),
            msg_event("evt_msg_x2", "x", "x", "Another X message", 2, 3_000),
        ])
        .await
        .unwrap();

    // Session X's run sees only X messages.
    let x_events = EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("x")), None, 100)
        .await
        .unwrap();
    let x_msgs: Vec<_> = x_events
        .iter()
        .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(_)))
        .collect();

    assert_eq!(x_msgs.len(), 2, "run_x must have exactly 2 messages");
    assert!(x_msgs.iter().all(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(m) if m.session_id == session("x"))
    }), "all X messages must belong to session X");
    assert!(
        !x_msgs.iter().any(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(m) if m.content == "Message for session Y")
        }),
        "session X must not see session Y's messages"
    );

    // Session Y's run sees only Y messages.
    let y_events = EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("y")), None, 100)
        .await
        .unwrap();
    let y_msgs: Vec<_> = y_events
        .iter()
        .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(_)))
        .collect();

    assert_eq!(y_msgs.len(), 1, "run_y must have exactly 1 message");
    assert!(
        !y_msgs.iter().any(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(m) if m.session_id == session("x"))
        }),
        "session Y must not see session X's messages"
    );
}

/// (7): Message content round-trips through the event log without loss.
///
/// Tests Unicode, special characters, long content, and empty content.
#[tokio::test]
async fn message_content_round_trips_without_loss() {
    let store = Arc::new(InMemoryStore::new());
    seed_session_run(&store, "rt").await;

    let long_content = "A".repeat(4096);
    let test_messages: &[(&str, &str)] = &[
        ("ascii", "Hello, world!"),
        ("unicode", "こんにちは 🤖 Bonjour — привет"),
        ("special", "Line 1\nLine 2\tTabbed\r\nWindows line ending"),
        (
            "json_like",
            "{ \"key\": \"value\", \"nested\": { \"a\": 1 } }",
        ),
        ("long", &long_content),
        ("empty", ""),
    ];

    let mut events_to_append = Vec::new();
    for (i, (id_suffix, content)) in test_messages.iter().enumerate() {
        events_to_append.push(msg_event(
            &format!("evt_rt_{id_suffix}"),
            "rt",
            "rt",
            content,
            (i + 1) as u64,
            (i as u64 + 1) * 1_000,
        ));
    }
    store.append(&events_to_append).await.unwrap();

    // Read back all messages via the event log.
    let run_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("rt")), None, 100)
            .await
            .unwrap();

    let recovered: Vec<&UserMessageAppended> = run_events
        .iter()
        .filter_map(|e| {
            if let RuntimeEvent::UserMessageAppended(m) = &e.envelope.payload {
                Some(m)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        recovered.len(),
        test_messages.len(),
        "all {} messages must be recoverable",
        test_messages.len()
    );

    // Content of each message must be exactly preserved.
    for (i, (_, expected_content)) in test_messages.iter().enumerate() {
        assert_eq!(
            recovered[i].content,
            *expected_content,
            "content must round-trip without loss for message {i}: \
             expected {:?} (len {}), got {:?} (len {})",
            &expected_content[..expected_content.len().min(50)],
            expected_content.len(),
            &recovered[i].content[..recovered[i].content.len().min(50)],
            recovered[i].content.len()
        );
    }
}

/// Cursor-based pagination through user messages in a run.
#[tokio::test]
async fn user_messages_support_cursor_based_pagination() {
    let store = Arc::new(InMemoryStore::new());
    seed_session_run(&store, "pg").await;

    // Append 6 messages.
    for i in 1..=6u64 {
        store
            .append(&[msg_event(
                &format!("evt_msg_pg_{i}"),
                "pg",
                "pg",
                &format!("Message {i}"),
                i,
                i * 1_000,
            )])
            .await
            .unwrap();
    }

    // Page 1: first 3 messages from this run.
    let page1 = EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("pg")), None, 3)
        .await
        .unwrap();
    assert_eq!(page1.len(), 3, "page 1 must return 3 events");

    // Continue from cursor.
    let cursor = page1.last().unwrap().position;
    let page2 =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Run(run("pg")), Some(cursor), 10)
            .await
            .unwrap();

    // Page 1 includes RunCreated + first 2 messages (limit=3).
    // Page 2 has the remaining 4 message events after the cursor.
    let page2_msgs: Vec<_> = page2
        .iter()
        .filter(|e| matches!(&e.envelope.payload, RuntimeEvent::UserMessageAppended(_)))
        .collect();
    assert_eq!(
        page2_msgs.len(),
        4,
        "page 2 must return the 4 remaining messages"
    );

    // No overlap between pages.
    let p1_positions: std::collections::HashSet<_> = page1.iter().map(|e| e.position).collect();
    let p2_positions: std::collections::HashSet<_> = page2.iter().map(|e| e.position).collect();
    assert!(
        p1_positions.is_disjoint(&p2_positions),
        "pages must not overlap"
    );
}
