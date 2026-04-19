pub(crate) mod admin;
pub(crate) mod approvals;
pub(crate) mod auth_tokens;
pub(crate) mod bundles_handlers;
pub(crate) mod decisions;
pub(crate) mod evals;
pub(crate) mod feed;
pub(crate) mod github;
pub(crate) mod graph;
pub(crate) mod health;
pub(crate) mod integrations;
pub(crate) mod memory;
pub(crate) mod plugins;
pub(crate) mod prompts;
pub(crate) mod providers;
pub(crate) mod runs;
pub(crate) mod sessions;
pub(crate) mod signals;
pub(crate) mod sqeq;
// T6c-C1: `sse` is `pub` so the binary-side WS handler in
// `bin_websocket.rs` can re-use `ws_event_tenant_id` for tenant filtering.
pub mod sse;
pub(crate) mod tasks;
pub(crate) mod tools;
pub(crate) mod workers;
