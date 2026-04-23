//! Bootstrap binary support for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces

pub mod bootstrap;
pub mod errors;
pub mod extractors;
pub mod fabric_adapter;
pub mod handlers;
pub mod helpers;
pub mod marketplace_routes;
pub mod metrics;
#[cfg(feature = "metrics-otel")]
pub mod metrics_otel;
#[cfg(any(feature = "metrics-core", feature = "metrics-providers"))]
pub mod metrics_tap;
pub mod middleware;
pub mod repo_routes;
pub mod router;
pub mod sandbox;
pub mod sse_hooks;
pub mod state;
pub mod telemetry_routes;
pub mod tokens;
pub mod tool_impls;
pub mod trigger_routes;
pub mod triggers;
pub mod validate;

// Re-exports for backward compatibility
pub use bootstrap::{parse_args, parse_args_from, run_bootstrap};
pub use errors::AppApiError;

// T6c-C1/C2: re-export the tenant-scope helper used by the binary-side
// WebSocket handler (`bin_websocket.rs`) so cross-tenant event fan-out
// is gated there just like it is on the SSE path. The SSE tenant
// helper is reached via `cairn_app::handlers::sse::ws_event_tenant_id`.
pub use extractors::is_admin_principal;

/// T6b-C3: redact the password component of a database connection URL
/// so startup logs don't leak credentials to journald / CloudWatch.
///
/// Returns the input unchanged on parse error (falling back to logging
/// the opaque URL is no worse than the pre-fix baseline, and the parse
/// error itself is not actionable for the operator).
pub fn redact_dsn(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(mut parsed) if parsed.password().is_some() => {
            // Ignore the result — Url::set_password returns Err only
            // when the scheme forbids credentials, which can't happen
            // for postgres:// / sqlite://.
            let _ = parsed.set_password(Some("***"));
            parsed.to_string()
        }
        _ => url.to_owned(),
    }
}
#[allow(unused_imports)]
pub(crate) use errors::*;
#[allow(unused_imports)]
pub(crate) use extractors::*;
pub use helpers::event_type_name;
#[allow(unused_imports)]
pub(crate) use helpers::*;
#[allow(unused_imports)]
pub(crate) use metrics::*;
#[allow(unused_imports)]
pub(crate) use middleware::*;
pub use router::AppBootstrap;
#[allow(unused_imports)]
pub(crate) use router::*;
#[allow(unused_imports)]
pub(crate) use sandbox::*;
#[allow(unused_imports)]
pub(crate) use state::*;
pub use state::{
    AppState, GitHubEventAction, GitHubIntegration, IssueQueueEntry, IssueQueueStatus,
    WebhookAction,
};
#[allow(unused_imports)]
pub(crate) use tokens::*;
#[allow(unused_imports)]
pub(crate) use triggers::*;

pub(crate) use cairn_runtime::RuntimeError;
pub(crate) use cairn_tools::cancel_plugin_invocation;

// ── Handler re-exports ─────────────────────────────────────────────────────────
#[allow(unused_imports)]
pub(crate) use handlers::admin::*;
#[allow(unused_imports)]
pub(crate) use handlers::approvals::*;
#[allow(unused_imports)]
pub(crate) use handlers::auth_tokens::*;
#[allow(unused_imports)]
pub(crate) use handlers::bundles_handlers::*;
#[allow(unused_imports)]
pub(crate) use handlers::decisions::*;
#[allow(unused_imports)]
pub(crate) use handlers::evals::*;
#[allow(unused_imports)]
pub(crate) use handlers::feed::*;
#[allow(unused_imports)]
pub(crate) use handlers::github::*;
#[allow(unused_imports)]
pub(crate) use handlers::graph::*;
#[allow(unused_imports)]
pub(crate) use handlers::health::*;
#[allow(unused_imports)]
pub(crate) use handlers::integrations::*;
#[allow(unused_imports)]
pub(crate) use handlers::memory::*;
#[allow(unused_imports)]
pub(crate) use handlers::plugins::*;
#[allow(unused_imports)]
pub(crate) use handlers::prompts::*;
#[allow(unused_imports)]
pub(crate) use handlers::providers::*;
#[allow(unused_imports)]
pub(crate) use handlers::runs::*;
#[allow(unused_imports)]
pub(crate) use handlers::sessions::*;
#[allow(unused_imports)]
pub(crate) use handlers::signals::*;
#[allow(unused_imports)]
pub(crate) use handlers::skills::*;
#[allow(unused_imports)]
pub(crate) use handlers::sqeq::*;
#[allow(unused_imports)]
pub(crate) use handlers::sse::*;
#[allow(unused_imports)]
pub(crate) use handlers::tasks::*;
#[allow(unused_imports)]
pub(crate) use handlers::tools::*;
#[allow(unused_imports)]
pub(crate) use handlers::workers::*;
