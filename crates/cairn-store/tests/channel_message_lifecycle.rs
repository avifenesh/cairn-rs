//! RFC 002 — Channel message lifecycle tests.
//!
//! Validates the full inter-agent messaging pipeline through the event log
//! and synchronous projection:
//!
//! - `ChannelCreated` registers a channel with `name` and `capacity`.
//! - `ChannelMessageSent` appends messages with `sender_id`, `body`, and
//!   `sent_at_ms`; unconsumed messages have `consumed_by = None`.
//! - `ChannelMessageConsumed` marks a specific message consumed: sets
//!   `consumed_by` and `consumed_at_ms` in the read model.
//! - Unconsumed messages are still returned by `list_messages`.
//! - Capacity is stored on the channel record and respected by consumers
//!   (enforcement is in the service layer; the store records the truth).
//! - `list_channels` scopes to the project; channels from other projects
//!   are not visible.

use cairn_domain::{
    events::{ChannelCreated, ChannelMessageConsumed, ChannelMessageSent},
    tenancy::OwnershipKey,
    ChannelId, EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, TenantId,
    WorkspaceId,
};
use cairn_store::{projections::ChannelReadModel, EventLog, InMemoryStore};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn project(suffix: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(format!("tenant_{suffix}")),
        workspace_id: WorkspaceId::new(format!("ws_{suffix}")),
        project_id: ProjectId::new(format!("proj_{suffix}")),
    }
}

fn ownership(proj: &ProjectKey) -> OwnershipKey {
    OwnershipKey::Project(proj.clone())
}

// ── Event helpers ─────────────────────────────────────────────────────────────

async fn create_channel(
    store: &InMemoryStore,
    event_id: &str,
    channel_id: &str,
    proj: &ProjectKey,
    name: &str,
    capacity: u32,
    at: u64,
) {
    store
        .append(&[EventEnvelope::new(
            EventId::new(event_id),
            EventSource::Runtime,
            ownership(proj),
            RuntimeEvent::ChannelCreated(ChannelCreated {
                channel_id: ChannelId::new(channel_id),
                project: proj.clone(),
                name: name.to_owned(),
                capacity,
                created_at_ms: at,
            }),
        )])
        .await
        .unwrap();
}

async fn send_message(
    store: &InMemoryStore,
    event_id: &str,
    channel_id: &str,
    proj: &ProjectKey,
    message_id: &str,
    sender_id: &str,
    body: &str,
    at: u64,
) {
    store
        .append(&[EventEnvelope::new(
            EventId::new(event_id),
            EventSource::Runtime,
            ownership(proj),
            RuntimeEvent::ChannelMessageSent(ChannelMessageSent {
                channel_id: ChannelId::new(channel_id),
                project: proj.clone(),
                message_id: message_id.to_owned(),
                sender_id: sender_id.to_owned(),
                body: body.to_owned(),
                sent_at_ms: at,
            }),
        )])
        .await
        .unwrap();
}

async fn consume_message(
    store: &InMemoryStore,
    event_id: &str,
    channel_id: &str,
    proj: &ProjectKey,
    message_id: &str,
    consumed_by: &str,
    at: u64,
) {
    store
        .append(&[EventEnvelope::new(
            EventId::new(event_id),
            EventSource::Runtime,
            ownership(proj),
            RuntimeEvent::ChannelMessageConsumed(ChannelMessageConsumed {
                channel_id: ChannelId::new(channel_id),
                project: proj.clone(),
                message_id: message_id.to_owned(),
                consumed_by: consumed_by.to_owned(),
                consumed_at_ms: at,
            }),
        )])
        .await
        .unwrap();
}

// ── 1. ChannelCreated stores the channel record ───────────────────────────────

#[tokio::test]
async fn channel_created_appears_in_read_model() {
    let store = InMemoryStore::new();
    let proj = project("a");

    create_channel(&store, "e1", "chan_1", &proj, "Agent Chat", 100, 1_000).await;

    let record = ChannelReadModel::get_channel(&store, &ChannelId::new("chan_1"))
        .await
        .unwrap()
        .expect("channel must exist after ChannelCreated");

    assert_eq!(record.channel_id.as_str(), "chan_1");
    assert_eq!(record.name, "Agent Chat");
    assert_eq!(record.capacity, 100);
    assert_eq!(record.project, proj);
    assert_eq!(record.created_at, 1_000);
}

#[tokio::test]
async fn get_channel_returns_none_for_unknown_id() {
    let store = InMemoryStore::new();
    let result = ChannelReadModel::get_channel(&store, &ChannelId::new("ghost_chan"))
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── 2. ChannelMessageSent appends messages ───────────────────────────────────

#[tokio::test]
async fn three_messages_sent_are_all_stored() {
    let store = InMemoryStore::new();
    let proj = project("b");
    let ch = "chan_3msg";

    create_channel(&store, "e1", ch, &proj, "Test Channel", 10, 1_000).await;

    for i in 1..=3u32 {
        send_message(
            &store,
            &format!("e_msg_{i}"),
            ch,
            &proj,
            &format!("msg_{i}"),
            "agent_a",
            &format!("Hello {i}"),
            1_000 + i as u64,
        )
        .await;
    }

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 100)
        .await
        .unwrap();

    assert_eq!(messages.len(), 3, "all 3 messages must be in the channel");
}

#[tokio::test]
async fn sent_message_fields_are_preserved() {
    let store = InMemoryStore::new();
    let proj = project("fields");
    let ch = "chan_fields";

    create_channel(&store, "e1", ch, &proj, "Fields Chan", 50, 1_000).await;
    send_message(
        &store,
        "e2",
        ch,
        &proj,
        "msg_f1",
        "agent_x",
        "Important payload",
        5_000,
    )
    .await;

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 10)
        .await
        .unwrap();

    assert_eq!(messages.len(), 1);
    let m = &messages[0];
    assert_eq!(m.message_id, "msg_f1");
    assert_eq!(m.sender_id, "agent_x");
    assert_eq!(m.body, "Important payload");
    assert_eq!(m.sent_at_ms, 5_000);
    assert!(
        m.consumed_by.is_none(),
        "newly sent message must not be consumed"
    );
    assert!(m.consumed_at_ms.is_none());
}

// ── 3. ChannelMessageConsumed marks the message ───────────────────────────────

#[tokio::test]
async fn consumed_message_has_consumed_by_and_consumed_at_set() {
    let store = InMemoryStore::new();
    let proj = project("c");
    let ch = "chan_consume";

    create_channel(&store, "e1", ch, &proj, "Consume Chan", 10, 1_000).await;
    send_message(
        &store,
        "e2",
        ch,
        &proj,
        "msg_consume",
        "sender_a",
        "Consume me",
        2_000,
    )
    .await;
    consume_message(&store, "e3", ch, &proj, "msg_consume", "consumer_b", 3_000).await;

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 10)
        .await
        .unwrap();

    assert_eq!(messages.len(), 1);
    let m = &messages[0];

    // (4) Verify consumed_by and consumed_at_ms are set.
    assert_eq!(
        m.consumed_by.as_deref(),
        Some("consumer_b"),
        "consumed_by must be set after ChannelMessageConsumed"
    );
    assert_eq!(
        m.consumed_at_ms,
        Some(3_000),
        "consumed_at_ms must be set after ChannelMessageConsumed"
    );
}

#[tokio::test]
async fn consume_unknown_message_is_a_no_op() {
    let store = InMemoryStore::new();
    let proj = project("noop");
    let ch = "chan_noop";

    create_channel(&store, "e1", ch, &proj, "No-op Chan", 5, 1_000).await;
    // Consume a message_id that was never sent.
    consume_message(&store, "e2", ch, &proj, "ghost_msg", "consumer", 2_000).await;

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 10)
        .await
        .unwrap();
    assert!(
        messages.is_empty(),
        "no messages should exist for ghost consume"
    );
}

// ── 5. Unconsumed messages are still queryable ────────────────────────────────

#[tokio::test]
async fn unconsumed_messages_remain_after_first_is_consumed() {
    let store = InMemoryStore::new();
    let proj = project("d");
    let ch = "chan_unconsume";

    create_channel(&store, "e1", ch, &proj, "Mixed Chan", 20, 1_000).await;

    for i in 1..=3u32 {
        send_message(
            &store,
            &format!("e_s_{i}"),
            ch,
            &proj,
            &format!("msg_{i}"),
            "sender",
            &format!("body {i}"),
            1_000 + i as u64,
        )
        .await;
    }

    // Consume only the first message.
    consume_message(&store, "e_c1", ch, &proj, "msg_1", "consumer_x", 5_000).await;

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 100)
        .await
        .unwrap();

    assert_eq!(
        messages.len(),
        3,
        "all 3 messages (consumed + unconsumed) must be returned"
    );

    let consumed: Vec<_> = messages
        .iter()
        .filter(|m| m.consumed_by.is_some())
        .collect();
    let unconsumed: Vec<_> = messages
        .iter()
        .filter(|m| m.consumed_by.is_none())
        .collect();

    assert_eq!(consumed.len(), 1, "exactly 1 message must be consumed");
    assert_eq!(unconsumed.len(), 2, "exactly 2 messages must be unconsumed");

    assert_eq!(consumed[0].message_id, "msg_1");
    let unconsumed_ids: Vec<&str> = unconsumed.iter().map(|m| m.message_id.as_str()).collect();
    assert!(unconsumed_ids.contains(&"msg_2") && unconsumed_ids.contains(&"msg_3"));
}

#[tokio::test]
async fn consuming_all_messages_marks_all_consumed() {
    let store = InMemoryStore::new();
    let proj = project("all_consumed");
    let ch = "chan_all";

    create_channel(&store, "e1", ch, &proj, "All Chan", 5, 1_000).await;

    for i in 1..=3u32 {
        send_message(
            &store,
            &format!("s{i}"),
            ch,
            &proj,
            &format!("m{i}"),
            "s",
            &format!("b{i}"),
            i as u64,
        )
        .await;
        consume_message(
            &store,
            &format!("c{i}"),
            ch,
            &proj,
            &format!("m{i}"),
            "consumer",
            100 + i as u64,
        )
        .await;
    }

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 100)
        .await
        .unwrap();

    assert!(
        messages.iter().all(|m| m.consumed_by.is_some()),
        "all messages must be consumed"
    );
}

// ── 6. Channel capacity is stored on the record ───────────────────────────────

#[tokio::test]
async fn channel_capacity_is_stored_on_record() {
    let store = InMemoryStore::new();
    let proj = project("cap");

    create_channel(&store, "e1", "chan_cap", &proj, "Capped", 5, 1_000).await;

    let rec = ChannelReadModel::get_channel(&store, &ChannelId::new("chan_cap"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        rec.capacity, 5,
        "capacity must match the ChannelCreated event value"
    );
}

#[tokio::test]
async fn channels_with_different_capacities_are_stored_independently() {
    let store = InMemoryStore::new();
    let proj = project("caps");

    create_channel(&store, "e1", "chan_small", &proj, "Small", 3, 1_000).await;
    create_channel(&store, "e2", "chan_large", &proj, "Large", 100, 1_000).await;
    create_channel(
        &store,
        "e3",
        "chan_unbounded",
        &proj,
        "Infinite",
        u32::MAX,
        1_000,
    )
    .await;

    let small = ChannelReadModel::get_channel(&store, &ChannelId::new("chan_small"))
        .await
        .unwrap()
        .unwrap();
    let large = ChannelReadModel::get_channel(&store, &ChannelId::new("chan_large"))
        .await
        .unwrap()
        .unwrap();
    let unbounded = ChannelReadModel::get_channel(&store, &ChannelId::new("chan_unbounded"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(small.capacity, 3);
    assert_eq!(large.capacity, 100);
    assert_eq!(unbounded.capacity, u32::MAX);
}

#[tokio::test]
async fn list_messages_respects_limit_parameter() {
    let store = InMemoryStore::new();
    let proj = project("limit");
    let ch = "chan_limit";

    create_channel(&store, "e1", ch, &proj, "Limit Chan", 20, 1_000).await;

    for i in 0..10u32 {
        send_message(
            &store,
            &format!("s{i}"),
            ch,
            &proj,
            &format!("m{i}"),
            "s",
            "body",
            i as u64,
        )
        .await;
    }

    let first_five = ChannelReadModel::list_messages(&store, &ChannelId::new(ch), 5)
        .await
        .unwrap();
    assert_eq!(first_five.len(), 5, "limit must restrict returned messages");
}

// ── 7. Cross-channel isolation ────────────────────────────────────────────────

#[tokio::test]
async fn messages_are_scoped_to_their_channel() {
    let store = InMemoryStore::new();
    let proj = project("iso");

    create_channel(&store, "e1", "chan_iso_a", &proj, "Channel A", 10, 1_000).await;
    create_channel(&store, "e2", "chan_iso_b", &proj, "Channel B", 10, 1_000).await;

    send_message(
        &store,
        "e3",
        "chan_iso_a",
        &proj,
        "msg_a1",
        "sender",
        "For A",
        2_000,
    )
    .await;
    send_message(
        &store,
        "e4",
        "chan_iso_a",
        &proj,
        "msg_a2",
        "sender",
        "Also A",
        3_000,
    )
    .await;
    send_message(
        &store,
        "e5",
        "chan_iso_b",
        &proj,
        "msg_b1",
        "sender",
        "For B",
        2_000,
    )
    .await;

    let a_messages = ChannelReadModel::list_messages(&store, &ChannelId::new("chan_iso_a"), 100)
        .await
        .unwrap();
    let b_messages = ChannelReadModel::list_messages(&store, &ChannelId::new("chan_iso_b"), 100)
        .await
        .unwrap();

    assert_eq!(a_messages.len(), 2, "channel A must have 2 messages");
    assert_eq!(b_messages.len(), 1, "channel B must have 1 message");

    let a_ids: Vec<&str> = a_messages.iter().map(|m| m.message_id.as_str()).collect();
    assert!(a_ids.contains(&"msg_a1") && a_ids.contains(&"msg_a2"));
    assert_eq!(b_messages[0].message_id, "msg_b1");
}

#[tokio::test]
async fn consuming_message_in_one_channel_does_not_affect_other() {
    let store = InMemoryStore::new();
    let proj = project("xiso");

    create_channel(&store, "e1", "chan_x1", &proj, "X1", 10, 1_000).await;
    create_channel(&store, "e2", "chan_x2", &proj, "X2", 10, 1_000).await;

    send_message(
        &store,
        "e3",
        "chan_x1",
        &proj,
        "shared_msg_id",
        "s",
        "body",
        2_000,
    )
    .await;
    send_message(
        &store,
        "e4",
        "chan_x2",
        &proj,
        "shared_msg_id",
        "s",
        "body",
        2_000,
    )
    .await;

    // Consume in chan_x1 only.
    consume_message(
        &store,
        "e5",
        "chan_x1",
        &proj,
        "shared_msg_id",
        "consumer",
        3_000,
    )
    .await;

    let x1_msgs = ChannelReadModel::list_messages(&store, &ChannelId::new("chan_x1"), 10)
        .await
        .unwrap();
    let x2_msgs = ChannelReadModel::list_messages(&store, &ChannelId::new("chan_x2"), 10)
        .await
        .unwrap();

    assert!(
        x1_msgs[0].consumed_by.is_some(),
        "chan_x1 message must be consumed"
    );
    assert!(
        x2_msgs[0].consumed_by.is_none(),
        "chan_x2 message must remain unconsumed"
    );
}

#[tokio::test]
async fn list_channels_scoped_to_project() {
    let store = InMemoryStore::new();
    let proj_a = project("pa");
    let proj_b = project("pb");

    create_channel(&store, "e1", "chan_pa1", &proj_a, "PA Chan 1", 5, 1_000).await;
    create_channel(&store, "e2", "chan_pa2", &proj_a, "PA Chan 2", 5, 1_000).await;
    create_channel(&store, "e3", "chan_pb1", &proj_b, "PB Chan 1", 5, 1_000).await;

    let pa_channels = ChannelReadModel::list_channels(&store, &proj_a, 100, 0)
        .await
        .unwrap();
    let pb_channels = ChannelReadModel::list_channels(&store, &proj_b, 100, 0)
        .await
        .unwrap();

    assert_eq!(pa_channels.len(), 2, "project A must have 2 channels");
    assert_eq!(pb_channels.len(), 1, "project B must have 1 channel");

    let pa_ids: Vec<&str> = pa_channels.iter().map(|c| c.channel_id.as_str()).collect();
    assert!(pa_ids.contains(&"chan_pa1") && pa_ids.contains(&"chan_pa2"));
    assert_eq!(pb_channels[0].channel_id.as_str(), "chan_pb1");
}

// ── 8. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn all_lifecycle_events_appear_in_log() {
    let store = InMemoryStore::new();
    let proj = project("log");
    let ch = "chan_log";

    create_channel(&store, "e1", ch, &proj, "Log Chan", 10, 1_000).await;
    send_message(&store, "e2", ch, &proj, "msg_log", "s", "hello", 2_000).await;
    consume_message(&store, "e3", ch, &proj, "msg_log", "c", 3_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 3);

    assert!(
        matches!(&all[0].envelope.payload, RuntimeEvent::ChannelCreated(e)
        if e.channel_id.as_str() == ch)
    );
    assert!(
        matches!(&all[1].envelope.payload, RuntimeEvent::ChannelMessageSent(e)
        if e.message_id == "msg_log")
    );
    assert!(
        matches!(&all[2].envelope.payload, RuntimeEvent::ChannelMessageConsumed(e)
        if e.consumed_by == "c" && e.consumed_at_ms == 3_000)
    );
}

#[tokio::test]
async fn empty_channel_has_no_messages() {
    let store = InMemoryStore::new();
    let proj = project("empty");
    create_channel(&store, "e1", "chan_empty", &proj, "Empty Chan", 10, 1_000).await;

    let messages = ChannelReadModel::list_messages(&store, &ChannelId::new("chan_empty"), 100)
        .await
        .unwrap();
    assert!(messages.is_empty());
}
