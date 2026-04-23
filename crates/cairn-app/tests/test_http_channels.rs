//! End-to-end HTTP coverage for `/v1/channels` runtime-channel CRUD.
//!
//! Closes issue #139 (UI rewired to the real `/v1/channels` API). This test
//! pins the contract the new ChannelsPage depends on:
//!   POST /v1/channels                  → create (201)
//!   GET  /v1/channels                  → list with ListResponse<Channel>
//!   POST /v1/channels/:id/send         → publish
//!   GET  /v1/channels/:id/messages     → messages include the one we sent

mod support;

use serde_json::{json, Value};
use support::live_fabric::LiveHarness;

#[tokio::test]
async fn channel_create_send_list_roundtrip() {
    let h = LiveHarness::setup().await;

    // 1. Create a channel.
    let create_res = h
        .client()
        .post(format!("{}/v1/channels", h.base_url))
        .bearer_auth(&h.admin_token)
        .json(&json!({
            "tenant_id":    h.tenant,
            "workspace_id": h.workspace,
            "project_id":   h.project,
            "name":         "alerts",
            "capacity":     32u32,
        }))
        .send()
        .await
        .expect("POST /v1/channels reaches server");
    assert_eq!(
        create_res.status().as_u16(),
        201,
        "channel create: {}",
        create_res.text().await.unwrap_or_default(),
    );
    let created: Value = create_res.json().await.expect("channel json");
    let channel_id = created
        .get("channel_id")
        .and_then(Value::as_str)
        .expect("channel_id in create response")
        .to_owned();
    assert_eq!(created.get("name").and_then(Value::as_str), Some("alerts"));
    assert_eq!(created.get("capacity").and_then(Value::as_u64), Some(32));

    // 2. List channels — scoped to this project, should include the one we created.
    let list_res = h
        .client()
        .get(format!(
            "{}/v1/channels?tenant_id={}&workspace_id={}&project_id={}",
            h.base_url, h.tenant, h.workspace, h.project,
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/channels reaches server");
    assert_eq!(
        list_res.status().as_u16(),
        200,
        "channel list: {}",
        list_res.text().await.unwrap_or_default(),
    );
    let list: Value = list_res.json().await.expect("list json");
    let items = list
        .get("items")
        .and_then(Value::as_array)
        .expect("ListResponse.items array");
    assert!(
        items
            .iter()
            .any(|ch| ch.get("channel_id").and_then(Value::as_str) == Some(channel_id.as_str())),
        "created channel missing from list: {list}",
    );

    // 3. Send a message.
    let send_res = h
        .client()
        .post(format!("{}/v1/channels/{}/send", h.base_url, channel_id))
        .bearer_auth(&h.admin_token)
        .json(&json!({ "sender_id": "operator", "body": "hello world" }))
        .send()
        .await
        .expect("POST /v1/channels/:id/send reaches server");
    assert_eq!(
        send_res.status().as_u16(),
        200,
        "channel send: {}",
        send_res.text().await.unwrap_or_default(),
    );
    let sent: Value = send_res.json().await.expect("send response json");
    assert!(
        sent.get("message_id").and_then(Value::as_str).is_some(),
        "send response missing message_id: {sent}",
    );

    // 4. List messages on the channel — the message we just sent should be there.
    let msgs_res = h
        .client()
        .get(format!(
            "{}/v1/channels/{}/messages",
            h.base_url, channel_id
        ))
        .bearer_auth(&h.admin_token)
        .send()
        .await
        .expect("GET /v1/channels/:id/messages reaches server");
    assert_eq!(
        msgs_res.status().as_u16(),
        200,
        "channel messages: {}",
        msgs_res.text().await.unwrap_or_default(),
    );
    let messages: Value = msgs_res.json().await.expect("messages json");
    let arr = messages.as_array().expect("messages is an array");
    assert!(
        arr.iter().any(|m| {
            m.get("sender_id").and_then(Value::as_str) == Some("operator")
                && m.get("body").and_then(Value::as_str) == Some("hello world")
        }),
        "sent message not found in list: {messages}",
    );
}
