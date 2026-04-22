//! Bootstrap binary for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces
//!
mod bin_admin;
mod bin_events;
mod bin_export;
mod bin_frontend;
mod bin_handlers;
mod bin_health;
mod bin_providers;
mod bin_router;
mod bin_seed;
mod bin_state;
mod bin_types;
mod bin_websocket;
#[allow(dead_code)]
mod bundles;
#[allow(dead_code)]
mod entitlements;
mod openapi_spec;
#[allow(dead_code)]
mod sse_hooks;
#[allow(dead_code)]
mod templates;
#[allow(dead_code)]
mod validate;

#[allow(unused_imports)]
use bin_admin::*;
#[allow(unused_imports)]
use bin_events::*;
#[allow(unused_imports)]
use bin_export::*;
#[allow(unused_imports)]
use bin_frontend::*;
#[allow(unused_imports)]
use bin_handlers::*;
#[allow(unused_imports)]
use bin_health::*;
#[allow(unused_imports)]
use bin_providers::*;
#[allow(unused_imports)]
use bin_router::*;
#[allow(unused_imports)]
use bin_seed::*;
#[allow(unused_imports)]
use bin_state::*;
#[allow(unused_imports)]
use bin_types::*;
#[allow(unused_imports)]
use bin_websocket::*;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Re-exported for #[cfg(test)] modules that use `super::*`
#[allow(unused_imports)]
use axum::http::StatusCode;
#[allow(unused_imports)]
use axum::response::{IntoResponse, Response};
#[allow(unused_imports)]
use axum::Json;
#[allow(unused_imports)]
use std::time::Instant;

#[allow(unused_imports)]
use cairn_api::auth::{
    AuthPrincipal, Authenticator, ServiceTokenAuthenticator, ServiceTokenRegistry,
};
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, EncryptionKeySource, StorageBackend};
use cairn_runtime::provider_health::ProviderHealthService;
#[allow(unused_imports)]
use cairn_runtime::sessions::SessionService;
use cairn_runtime::{CredentialService, DefaultsService};
#[allow(unused_imports)]
use cairn_runtime::{InMemoryServices, OllamaEmbeddingProvider, OllamaModel, OllamaProvider};
use cairn_store::pg::PgMigrationRunner;
use cairn_store::pg::{PgAdapter, PgEventLog};
use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog};
use cairn_store::DbAdapter;
use cairn_store::{EventLog, EventPosition};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;

// PgBackend, SqliteBackend, RateBucket, AppState, NotificationBuffer,
// AppMetrics, RequestLogBuffer → bin_state.rs
// RequestId, ApiError, pagination_headers, PaginationQuery, ProjectQuery → bin_types.rs

// ── Metrics middleware ────────────────────────────────────────────────────────

// Version+changelog, webhook test, rate-limit → bin_handlers.rs
// Detailed health handler → bin_health.rs
// WebSocket handler → bin_websocket.rs
// Ollama, provider discovery, generate, embed, stream, model mgmt → bin_providers.rs
// ── Arg parsing ───────────────────────────────────────────────────────────────

fn parse_args_from(args: &[String]) -> BootstrapConfig {
    let mut config = BootstrapConfig::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    config.mode = match args[i].as_str() {
                        "team" | "self-hosted" => DeploymentMode::SelfHostedTeam,
                        _ => DeploymentMode::Local,
                    };
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    if let Ok(port) = args[i].parse::<u16>() {
                        config.listen_port = port;
                    }
                }
            }
            "--addr" => {
                i += 1;
                if i < args.len() {
                    config.listen_addr = args[i].clone();
                }
            }
            "--db" => {
                i += 1;
                if i < args.len() {
                    let val = &args[i];
                    if val == "memory" {
                        config.storage = StorageBackend::InMemory;
                    } else if val.starts_with("postgres://") || val.starts_with("postgresql://") {
                        config.storage = StorageBackend::Postgres {
                            connection_url: val.clone(),
                        };
                    } else {
                        config.storage = StorageBackend::Sqlite { path: val.clone() };
                    }
                }
            }
            "--role" => {
                i += 1;
                if i < args.len() {
                    config.process_role =
                        cairn_api::bootstrap::ProcessRole::from_str_loose(&args[i]);
                }
            }
            "--encryption-key-env" => {
                i += 1;
                if i < args.len() {
                    config.encryption_key = EncryptionKeySource::EnvVar {
                        var_name: args[i].clone(),
                    };
                }
            }
            _ => {}
        }
        i += 1;
    }

    if config.mode == DeploymentMode::SelfHostedTeam {
        if config.listen_addr == "127.0.0.1" {
            config.listen_addr = "0.0.0.0".to_owned();
        }
        if matches!(config.encryption_key, EncryptionKeySource::LocalAuto) {
            config.encryption_key = EncryptionKeySource::None;
        }
        // RFC 020 team-mode storage invariant is enforced in `parse_args`
        // AFTER `resolve_storage_from_env` runs, so the `DATABASE_URL` path
        // is covered as well as the `--db` CLI path. Not enforced here so
        // the invariant can't be silently bypassed by choosing env vars.
    }

    config
}

/// Resolve the storage backend from environment when no `--db` flag was given.
///
/// Priority: `DATABASE_URL` env var → InMemory fallback.
/// This runs after CLI parsing so `--db` always wins.
fn resolve_storage_from_env(config: &mut BootstrapConfig) {
    if !matches!(config.storage, StorageBackend::InMemory) {
        return; // --db flag was given, don't override
    }
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let url = url.trim().to_owned();
        if !url.is_empty() {
            if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                config.storage = StorageBackend::Postgres {
                    connection_url: url,
                };
            } else if url.starts_with("sqlite:") || url.ends_with(".db") {
                config.storage = StorageBackend::Sqlite { path: url };
            }
        }
    }
}

fn parse_args() -> BootstrapConfig {
    let args: Vec<String> = std::env::args().collect();
    let mut config = parse_args_from(&args);
    resolve_storage_from_env(&mut config);
    // Enforce the RFC 020 team-mode storage invariant AFTER env-var
    // resolution so a `DATABASE_URL=sqlite:/path/prod.db` footgun is
    // caught the same as a `--db /path/prod.db` one. This was gemini-
    // code-assist high-priority finding on PR #77 and is the correct
    // refusal point.
    cairn_app::bootstrap::enforce_team_mode_storage_invariant(&config);
    config
}

// ── Entry point ───────────────────────────────────────────────────────────────

// ── Graceful shutdown ─────────────────────────────────────────────────────────

/// Returns a future that resolves when SIGINT (Ctrl-C) or SIGTERM is received.
///
/// On non-Unix platforms only Ctrl-C is supported.
async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => { eprintln!("shutdown: SIGINT received");  },
        _ = terminate => { eprintln!("shutdown: SIGTERM received"); },
    }
}

/// Snapshot the in-memory event log and notification buffer to
/// `/tmp/cairn-shutdown-buffer.json` so they survive a server restart.
///
/// This is best-effort — failures are logged but do not block exit.
async fn flush_state_to_disk(state: &AppState) {
    const FLUSH_PATH: &str = "/tmp/cairn-shutdown-buffer.json";
    const MAX_EVENTS: usize = 5_000;

    // ── Events ────────────────────────────────────────────────────────────────
    let events = match state.runtime.store.read_stream(None, MAX_EVENTS).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shutdown: could not read event buffer: {e}");
            vec![]
        }
    };
    let event_snapshots: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "position":   e.position.0,
                "stored_at":  e.stored_at,
                "event_type": event_type_name(&e.envelope.payload),
            })
        })
        .collect();

    // ── Notifications ─────────────────────────────────────────────────────────
    // Serialise while holding the lock, then release before writing to disk.
    let (notif_count, notif_json) = match state.notifications.read() {
        Ok(buf) => {
            let list = buf.list(200);
            let json: Vec<serde_json::Value> = list
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "id":         n.id,
                        "type":       n.notif_type,
                        "message":    n.message,
                        "entity_id":  n.entity_id,
                        "href":       n.href,
                        "read":       n.read,
                        "created_at": n.created_at,
                    })
                })
                .collect();
            (json.len(), json)
        }
        Err(_) => (0, vec![]),
    };

    // ── Uptime ────────────────────────────────────────────────────────────────
    let uptime_secs = state.started_at.elapsed().as_secs();

    let payload = serde_json::json!({
        "flushed_at":        now_iso8601(),
        "uptime_seconds":    uptime_secs,
        "event_count":       event_snapshots.len(),
        "events":            event_snapshots,
        "notification_count": notif_count,
        "notifications":     notif_json,
    });

    match serde_json::to_string_pretty(&payload) {
        Ok(text) => match std::fs::write(FLUSH_PATH, text) {
            Ok(()) => eprintln!(
                "shutdown: flushed {} events + {} notifications → {FLUSH_PATH}",
                events.len(),
                notif_count,
            ),
            Err(e) => eprintln!("shutdown: write failed ({FLUSH_PATH}): {e}"),
        },
        Err(e) => eprintln!("shutdown: serialisation failed: {e}"),
    }
}

// Demo data seeding → bin_seed.rs
#[tokio::main]
async fn main() {
    // Load .env file if present (dev convenience — not required in production).
    // Silently ignored when the file doesn't exist.
    let _ = dotenvy::dotenv();

    // Initialise structured request tracing.  Operators can tune verbosity via
    // the RUST_LOG env var (e.g. RUST_LOG=cairn_app=info,tower_http=debug).
    //
    // When CAIRN_LOG_DIR is set, logs are also written to daily-rotating files.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Ok(log_dir) = std::env::var("CAIRN_LOG_DIR") {
        let log_dir = log_dir.trim().to_owned();
        if !log_dir.is_empty() {
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;

            let file_appender = tracing_appender::rolling::daily(&log_dir, "cairn.log");
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_target(false)
                .compact()
                .with_ansi(false);
            let stdout_layer = tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact();
            tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();
            eprintln!("logs: rotating daily to {log_dir}/cairn.*.log");
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(false)
                .compact()
                .init();
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .compact()
            .init();
    }

    let config = parse_args();

    // ── Token registry ────────────────────────────────────────────────────────
    // Priority: CAIRN_ADMIN_TOKEN_FILE > CAIRN_ADMIN_TOKEN > default dev token.
    // CAIRN_ADMIN_TOKEN_FILE reads from a file path (Docker secrets pattern).
    let admin_token = if let Ok(file_path) = std::env::var("CAIRN_ADMIN_TOKEN_FILE") {
        let file_path = file_path.trim().to_owned();
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                let token = contents.trim().to_owned();
                if token.is_empty() {
                    eprintln!("error: CAIRN_ADMIN_TOKEN_FILE at {file_path} is empty");
                    std::process::exit(1);
                }
                eprintln!("auth: admin token loaded from file {file_path}");
                token
            }
            Err(e) => {
                eprintln!("error: cannot read CAIRN_ADMIN_TOKEN_FILE at {file_path}: {e}");
                std::process::exit(1);
            }
        }
    } else {
        std::env::var("CAIRN_ADMIN_TOKEN").unwrap_or_else(|_| {
            if config.mode == DeploymentMode::SelfHostedTeam {
                eprintln!(
                    "error: CAIRN_ADMIN_TOKEN env var is required in team mode. \
                     Set it to a strong random token before starting."
                );
                std::process::exit(1);
            }
            "dev-admin-token".to_owned()
        })
    };
    if admin_token == "dev-admin-token" {
        eprintln!(
            "⚠ auth: using default dev-admin-token — override with CAIRN_ADMIN_TOKEN in production"
        );
    } else {
        eprintln!("auth: admin token configured");
    }

    // ── Durable backends (Postgres / SQLite) ────────────────────────────────
    let pg;
    let sqlite;
    match &config.storage {
        StorageBackend::Postgres { connection_url } => {
            let url = connection_url.clone();
            // T6b-C3: log only the redacted URL so credentials don't end
            // up in journald / CloudWatch.
            eprintln!(
                "store: connecting to Postgres at {}",
                cairn_app::redact_dsn(&url)
            );
            match PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
            {
                Ok(pool) => {
                    eprintln!("store: Postgres connection established");
                    let migrator = PgMigrationRunner::new(pool.clone());
                    match migrator.run_pending().await {
                        Ok(applied) if applied.is_empty() => {
                            eprintln!("store: Postgres schema is up to date");
                        }
                        Ok(applied) => {
                            eprintln!("store: applied {} migration(s):", applied.len());
                            for m in &applied {
                                eprintln!("  V{:03}__{}", m.version, m.name);
                            }
                        }
                        Err(e) => {
                            eprintln!("error: Postgres migration failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    let pg_event_log = Arc::new(PgEventLog::new(pool.clone()));
                    let backend = Arc::new(PgBackend {
                        event_log: pg_event_log.clone(),
                        adapter: Arc::new(PgAdapter::new(pool)),
                    });
                    eprintln!("store: Postgres backend active (all service events dual-written)");
                    pg = Some(backend);
                    sqlite = None;
                }
                Err(e) => {
                    eprintln!("error: failed to connect to Postgres: {e}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::Sqlite { path } => {
            // Normalise the URL: accept bare paths like "cairn.db" or "sqlite:cairn.db".
            let url = if path.starts_with("sqlite:") {
                path.clone()
            } else {
                format!("sqlite:{path}")
            };
            let sqlite_path = path
                .strip_prefix("sqlite:")
                .unwrap_or(path.as_str())
                .to_owned();
            eprintln!(
                "store: connecting to SQLite at {}",
                cairn_app::redact_dsn(&url)
            );
            match SqlitePoolOptions::new()
                .max_connections(1) // SQLite is not safe with multiple writers
                .connect(&url)
                .await
            {
                Ok(pool) => {
                    eprintln!("store: SQLite connection established");
                    let adapter = SqliteAdapter::new(pool.clone());
                    match adapter.migrate().await {
                        Ok(()) => eprintln!("store: SQLite schema applied"),
                        Err(e) => {
                            eprintln!("error: SQLite migration failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    let sqlite_event_log = Arc::new(SqliteEventLog::new(pool));
                    let backend = Arc::new(SqliteBackend {
                        event_log: sqlite_event_log.clone(),
                        adapter: Arc::new(adapter),
                        path: PathBuf::from(sqlite_path),
                    });
                    eprintln!("store: SQLite backend active (all service events dual-written)");
                    pg = None;
                    sqlite = Some(backend);
                }
                Err(e) => {
                    eprintln!("error: failed to connect to SQLite: {e}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::InMemory => {
            eprintln!(
                "⚠ store: using in-memory backend — ALL DATA WILL BE LOST on restart. \
                 Set DATABASE_URL or use --db to configure a durable store."
            );
            pg = None;
            sqlite = None;
        }
    }

    // ── Lib.rs AppState (catalog-driven router, shared runtime) ─────────────
    let mut lib_state = Arc::new(
        cairn_app::AppState::new(config.clone())
            .await
            .expect("failed to initialise lib AppState"),
    );
    // Register the admin token in the SHARED token registry so both routers
    // authenticate identically.
    lib_state.service_tokens.register(
        admin_token.clone(),
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("default")),
        },
    );

    // ── Startup replay from durable event log ────────────────────────────────
    // When a Postgres or SQLite backend is available, replay its event log into
    // the InMemoryStore so that projections (sessions, runs, tasks, approvals,
    // etc.) are warm on restart rather than empty.
    //
    // Replay runs in batches of 10 000 events to bound peak memory.  All events
    // are fed through InMemoryStore::append, which applies the same
    // apply_projection logic used during normal writes — guaranteeing that the
    // in-memory state is identical to what would have accumulated from scratch.
    {
        const REPLAY_BATCH: usize = 10_000;
        let durable_log: Option<&dyn EventLog> = if let Some(ref backend) = pg {
            Some(backend.event_log.as_ref())
        } else if let Some(ref backend) = sqlite {
            Some(backend.event_log.as_ref())
        } else {
            None
        };

        if let Some(log) = durable_log {
            eprintln!("store: replaying event log into InMemory projections…");
            let mut after: Option<EventPosition> = None;
            let mut total = 0usize;
            loop {
                let batch = match log.read_stream(after, REPLAY_BATCH).await {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("store: replay error reading batch after {after:?}: {e}");
                        std::process::exit(1);
                    }
                };
                if batch.is_empty() {
                    break;
                }
                after = batch.last().map(|e| e.position);
                let batch_len = batch.len();
                total += batch_len;
                let envelopes: Vec<_> = batch.into_iter().map(|e| e.envelope).collect();
                if let Err(e) = lib_state.runtime.store.append(&envelopes).await {
                    eprintln!("store: replay error applying batch: {e}");
                    std::process::exit(1);
                }
                if batch_len < REPLAY_BATCH {
                    // Last batch — no need to fetch again.
                    break;
                }
            }
            if total > 0 {
                eprintln!("store: replayed {total} event(s) — projections warm");
            } else {
                eprintln!("store: event log empty — starting with clean projections");
            }
        }
    }

    // ── Seed the service-layer event ID counter above existing events ─────────
    // The make_envelope() counter starts at 0 on each process startup and
    // generates IDs like "evt_<timestamp>_<n>".  Seeding with the current
    // InMemory head position ensures IDs are unique across restarts even if
    // two events happen to share the same millisecond timestamp.
    {
        let head = lib_state
            .runtime
            .store
            .head_position()
            .await
            .unwrap_or(None);
        let floor = head.map(|p| p.0).unwrap_or(0);
        cairn_runtime::seed_event_counter(floor);
    }

    // ── Ollama local LLM provider (optional) ─────────────────────────────────
    let ollama: Option<Arc<OllamaProvider>> = if let Some(provider) = OllamaProvider::from_env() {
        eprintln!("ollama: connecting to {}", provider.host());
        match provider.health_check().await {
            Ok(tags) => {
                if tags.models.is_empty() {
                    eprintln!("ollama: reachable but no models loaded");
                } else {
                    let names: Vec<&str> = tags.models.iter().map(|m| m.name.as_str()).collect();
                    eprintln!(
                        "ollama: {} model(s) available: {}",
                        names.len(),
                        names.join(", ")
                    );
                }
                Some(Arc::new(provider))
            }
            Err(e) => {
                eprintln!("ollama: health check failed ({e}) — provider disabled");
                None
            }
        }
    } else {
        None
    };

    // ── Provider construction via cairn-providers ──────────────────────────────
    // All providers are constructed through ProviderBuilder using runtime config.
    // cairn-providers implements cairn-domain's GenerationProvider trait via the
    // bridge module, so everything plugs into the existing orchestrate/generate paths.
    use cairn_providers::backends::bedrock::Bedrock as CairnBedrock;
    use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};
    use cairn_runtime::RuntimeConfig;

    let normalize_model = |model: String| {
        let trimmed = model.trim();
        if trimmed.is_empty() || trimmed == "default" {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };
    let configured_generate_model = normalize_model(
        lib_state
            .runtime
            .runtime_config
            .default_generate_model()
            .await,
    );
    let configured_brain_model =
        normalize_model(lib_state.runtime.runtime_config.default_brain_model().await)
            .or_else(|| configured_generate_model.clone());

    let openai_compat_brain: Option<Arc<OpenAiCompat>> = {
        let brain_url = std::env::var("CAIRN_BRAIN_URL")
            .or_else(|_| std::env::var("OPENAI_COMPAT_BASE_URL"))
            .ok()
            .filter(|u| !u.is_empty());
        let brain_key = std::env::var("CAIRN_BRAIN_KEY")
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .unwrap_or_default();
        brain_url.and_then(|url| {
            eprintln!(
                "openai-compat (brain): configured at {url} model={}",
                configured_brain_model.as_deref().unwrap_or("<unset>")
            );
            match OpenAiCompat::new(
                ProviderConfig::default(),
                brain_key,
                Some(url),
                configured_brain_model.clone(),
                None,
                None,
                None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (brain): invalid config: {err}");
                    None
                }
            }
        })
    };
    let openai_compat_worker: Option<Arc<OpenAiCompat>> = {
        let worker_url = std::env::var("CAIRN_WORKER_URL")
            .or_else(|_| std::env::var("OPENAI_COMPAT_BASE_URL"))
            .ok()
            .filter(|u| !u.is_empty());
        let worker_key = std::env::var("CAIRN_WORKER_KEY")
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .unwrap_or_default();
        worker_url.and_then(|url| {
            eprintln!(
                "openai-compat (worker): configured at {url} model={}",
                configured_generate_model.as_deref().unwrap_or("<unset>")
            );
            match OpenAiCompat::new(
                ProviderConfig::default(),
                worker_key,
                Some(url),
                configured_generate_model.clone(),
                None,
                None,
                None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (worker): invalid config: {err}");
                    None
                }
            }
        })
    };
    let openai_compat_openrouter: Option<Arc<OpenAiCompat>> = {
        RuntimeConfig::openrouter_api_key().and_then(|key| {
            eprintln!("openai-compat (openrouter): configured — brain=openrouter/free worker=google/gemma-3-4b-it:free");
            match OpenAiCompat::new(
                ProviderConfig::OPENROUTER,
                key,
                None, None, None, None, None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (openrouter): invalid config: {err}");
                    None
                }
            }
        })
    };

    // Legacy alias: expose the first configured provider as `openai_compat`.
    let openai_compat: Option<Arc<OpenAiCompat>> = openai_compat_brain
        .clone()
        .or_else(|| openai_compat_worker.clone())
        .or_else(|| openai_compat_openrouter.clone());

    // Bedrock provider via cairn-providers.
    let bedrock: Option<Arc<CairnBedrock>> = CairnBedrock::from_env().map(|p| {
        eprintln!(
            "bedrock: configured — model={} region={}",
            p.model_id(),
            p.region()
        );
        Arc::new(p)
    });

    {
        use cairn_domain::providers::{EmbeddingProvider, GenerationProvider};
        use cairn_providers::chat::ChatProvider;

        lib_state.runtime.provider_registry.set_startup_fallbacks(
            cairn_runtime::StartupFallbackProviders {
                ollama: ollama.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        Arc::new(OllamaEmbeddingProvider::new(provider.host()))
                            as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("ollama", None)
                }),
                brain: openai_compat_brain.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                worker: openai_compat_worker.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                openrouter: openai_compat_openrouter.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openrouter", Some(provider.model.clone()))
                }),
                bedrock: bedrock.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                    )
                    .with_metadata("bedrock", Some(provider.model_id().to_owned()))
                }),
            },
        );
    }

    // Wire brain provider into lib_state for the orchestrate endpoint.
    // Priority: brain → worker → OpenRouter → Bedrock → Ollama.
    {
        use cairn_domain::providers::GenerationProvider;
        let brain: Option<Arc<dyn GenerationProvider>> = openai_compat_brain
            .as_ref()
            .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            .or_else(|| {
                openai_compat_worker
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                openai_compat_openrouter
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                bedrock
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                ollama
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            });
        let lib_mut = match Arc::get_mut(&mut lib_state) {
            Some(m) => m,
            None => {
                // T6b-C4: fail loud with a useful error rather than
                // a bare `.expect()` panic. The Arc must not have
                // been cloned before this point — any clone here is
                // a programming error introduced by a refactor, and
                // a stack trace from panic is less helpful than a
                // named diagnostic.
                eprintln!(
                    "fatal: lib_state was cloned before brain_provider was wired — \
                     check AppState::new for stray clones"
                );
                std::process::exit(1);
            }
        };
        if let Some(b) = brain {
            lib_mut.brain_provider = Some(b);
            eprintln!("brain provider: wired to lib_state");
        }
        if let Some(ref br) = bedrock {
            lib_mut.bedrock_provider = Some(br.clone() as Arc<dyn GenerationProvider>);
            eprintln!("bedrock provider: wired to lib_state");
        }
    }

    // ── Wire GitHub App integration into lib_state ────────────────────────────
    // The integration registry (`IntegrationRegistry`) is the canonical home for
    // all integrations. We register a `GitHubPlugin` there.
    //
    // TODO(integration-migration): The legacy `state.github` (`GitHubIntegration`)
    // is ALSO set here because the webhook/queue/scan handlers in lib.rs still
    // access its concrete fields (credentials, installations, issue_queue, etc.)
    // directly.  Once `Integration` trait exposes those fields (or we add
    // `as_any()` for downcasting), migrate the handlers and remove `state.github`.
    {
        let github_app_id = std::env::var("GITHUB_APP_ID").ok();
        let github_key_file = std::env::var("GITHUB_PRIVATE_KEY_FILE").ok();
        let github_webhook_secret = std::env::var("GITHUB_WEBHOOK_SECRET").ok();

        if let (Some(app_id_str), Some(key_file), Some(webhook_secret)) =
            (github_app_id, github_key_file, github_webhook_secret)
        {
            match app_id_str.parse::<u64>() {
                Ok(app_id) => match std::fs::read(&key_file) {
                    Ok(pem_bytes) => match cairn_github::AppCredentials::new(app_id, &pem_bytes) {
                        Ok(credentials) => {
                            // Legacy shim — kept until handlers are migrated to the registry.
                            // See TODO(integration-migration) above.
                            let github = cairn_app::GitHubIntegration {
                                credentials: credentials.clone(),
                                webhook_secret: webhook_secret.clone(),
                                installations: tokio::sync::RwLock::new(
                                    std::collections::HashMap::new(),
                                ),
                                event_actions: tokio::sync::RwLock::new(vec![]),
                                issue_queue: tokio::sync::RwLock::new(
                                    std::collections::VecDeque::new(),
                                ),
                                queue_paused: std::sync::atomic::AtomicBool::new(false),
                                queue_running: std::sync::atomic::AtomicBool::new(false),
                                max_concurrent: std::sync::atomic::AtomicU32::new(3),
                                run_semaphore: std::sync::Arc::new(tokio::sync::Semaphore::new(3)),
                                http: reqwest::Client::new(),
                            };
                            // Canonical registration — the integration registry is the
                            // single source of truth for all integrations.
                            let github_plugin = cairn_integrations::github::GitHubPlugin::new(
                                credentials,
                                webhook_secret,
                                3,
                            );
                            // T6b-C4: same fail-loud pattern as above.
                            let lib_mut = match Arc::get_mut(&mut lib_state) {
                                Some(m) => m,
                                None => {
                                    eprintln!(
                                        "fatal: lib_state was cloned before github was wired"
                                    );
                                    std::process::exit(1);
                                }
                            };
                            lib_mut.github = Some(Arc::new(github));
                            let registry = match Arc::get_mut(&mut lib_mut.integrations) {
                                Some(r) => r,
                                None => {
                                    eprintln!(
                                        "fatal: integrations registry was cloned before github was registered"
                                    );
                                    std::process::exit(1);
                                }
                            };
                            registry.register_sync(Arc::new(github_plugin));
                            eprintln!("GitHub App: wired (app_id={app_id})");
                        }
                        Err(e) => {
                            eprintln!(
                                    "WARNING: GitHub App key invalid: {e} — GitHub integration disabled"
                                );
                        }
                    },
                    Err(e) => {
                        eprintln!(
                            "WARNING: Cannot read {key_file}: {e} — GitHub integration disabled"
                        );
                    }
                },
                Err(_) => {
                    eprintln!(
                        "WARNING: GITHUB_APP_ID is not a valid number — GitHub integration disabled"
                    );
                }
            }
        }
    }

    // ── Wire built-in tool registry into lib_state ───────────────────────────
    // Build with the real RetrievalService + IngestPipeline so the orchestrator
    // can actually search and store memory during execution.
    {
        use cairn_memory::{retrieval::RetrievalService, IngestService};
        let retrieval = lib_state.retrieval.clone() as Arc<dyn RetrievalService>;
        let ingest = lib_state.ingest.clone() as Arc<dyn IngestService>;
        let registry = cairn_app::tool_impls::build_tool_registry(
            retrieval,
            ingest,
            lib_state.project_repo_access.clone(),
            lib_state.repo_clone_cache.clone(),
        );
        // T6b-C4: fail loud.
        let lib_mut = match Arc::get_mut(&mut lib_state) {
            Some(m) => m,
            None => {
                eprintln!("fatal: lib_state was cloned before tool_registry was wired");
                std::process::exit(1);
            }
        };
        lib_mut.tool_registry = Some(Arc::new(registry));
        eprintln!("tool registry: memory tools + cairn.registerRepo wired");
    }

    // ── Binary-specific state (shares runtime + tokens with lib.rs) ────────
    let state = AppState {
        runtime: lib_state.runtime.clone(),
        started_at: Arc::new(lib_state.started_at),
        tokens: lib_state.service_tokens.clone(),
        pg,
        sqlite,
        mode: config.mode,
        document_store: lib_state.document_store.clone(),
        retrieval: lib_state.retrieval.clone(),
        ingest: lib_state.ingest.clone(),
        ollama,
        openai_compat_brain,
        openai_compat_worker,
        openai_compat_openrouter,
        openai_compat,
        metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
        rate_limits: Arc::new(Mutex::new(HashMap::new())),
        request_log: lib_state.request_log.clone(),
        notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
        templates: Arc::new(templates::TemplateRegistry::with_builtins()),
        entitlements: Arc::new(entitlements::EntitlementService::new()),
        bedrock: bedrock.clone(),
        process_role: config.process_role,
    };

    // ── Wire secondary event log (covers all service-layer appends) ─────────
    // All 109 store.append() call sites in 42 service files are covered by
    // setting the secondary log here once. Any event written by RunService,
    // TaskService, ApprovalService etc. is automatically dual-written.
    if let Some(ref pg_backend) = state.pg {
        state
            .runtime
            .store
            .set_secondary_log(pg_backend.event_log.clone());
        eprintln!("store: service-layer events will dual-write to Postgres");
    } else if let Some(ref sq_backend) = state.sqlite {
        state
            .runtime
            .store
            .set_secondary_log(sq_backend.event_log.clone());
        eprintln!("store: service-layer events will dual-write to SQLite");
    }

    // ── Demo seed data (local mode only, only when event log is empty) ─────────
    // Skip seeding when a durable backend (Postgres/SQLite) already has events
    // from a previous run.  After startup replay the in-memory store's head
    // position tells us whether there is pre-existing data to preserve.
    let event_log_empty = state
        .runtime
        .store
        .head_position()
        .await
        .unwrap_or(None)
        .is_none();
    // ── Always ensure the canonical "default" tenant exists ─────────────────
    // This is idempotent — if the tenant already exists, create() returns Err
    // which we ignore. Needed so provider connections, route policies, etc.
    // work out-of-the-box on first boot.
    {
        use cairn_domain::{tenancy::ProjectKey, TenantId};
        use cairn_runtime::{
            projects::ProjectService, tenants::TenantService, workspaces::WorkspaceService,
        };
        let _ = state
            .runtime
            .tenants
            .create(TenantId::new("default"), "Default".into())
            .await;
        let _ = state
            .runtime
            .workspaces
            .create(
                TenantId::new("default"),
                cairn_domain::WorkspaceId::new("default"),
                "Default".into(),
            )
            .await;
        let _ = state
            .runtime
            .projects
            .create(
                ProjectKey::new("default", "default", "default"),
                "Default".into(),
            )
            .await;
    }

    if state.mode == DeploymentMode::Local && event_log_empty {
        seed_demo_data(&state).await;
    }

    // ── RFC 020 readiness: branch flips ──────────────────────────────────────
    // The `ReadinessState` on `lib_state` starts with every branch `Pending`.
    // We flip each branch to `Complete` as the corresponding startup work
    // finishes. Branches whose real per-branch recovery hasn't landed yet
    // (run recovery, tool cache warmup, decision cache warmup, etc.) flip
    // with `count = 0` — "nothing to recover yet, so complete" — and will
    // be re-wired by later RFC 020 tracks (#149, #151, #152). The final
    // `mark_ready` flip happens after the HTTP listener binds so that the
    // readiness gate + `/health/ready` are observable during recovery.
    use cairn_runtime::startup::BranchStatus;

    // Step 2-ish: event log is reachable since we completed replays' prep
    // and the store has been opened. For now "event_log" = the store-side
    // event stream we just prepared above.
    lib_state.readiness.update_branch("2", |b| {
        b.event_log = BranchStatus::complete(0);
    });

    // Step 4a: sandbox reconciliation.
    match lib_state.sandbox_service.recover_all().await {
        Ok(summary) => {
            if summary.reconnected > 0 || summary.preserved > 0 || summary.failed > 0 {
                eprintln!(
                    "sandbox recovery: reconnected={} preserved={} failed={}",
                    summary.reconnected, summary.preserved, summary.failed
                );
            }
            let count = (summary.reconnected as u64).saturating_add(summary.preserved as u64);
            lib_state.readiness.update_branch("4a", |b| {
                b.sandboxes = BranchStatus::complete(count);
            });
        }
        Err(error) => {
            eprintln!("sandbox recovery failed: {error}");
            lib_state.readiness.update_branch("4a", |b| {
                b.sandboxes = BranchStatus::failed(error.to_string());
            });
        }
    }

    // ── RFC 020 Track 1: run-level recovery ──────────────────────────────
    //
    // Operational recovery (lease expiry, attempt timeouts, dependency
    // reconciliation, …) is owned unconditionally by FF's 14 background
    // scanners. This sweep covers the other half: non-terminal runs that
    // existed in the cairn event log when the previous boot died. The
    // service emits advisory `RecoveryAttempted`/`RecoveryCompleted`
    // events keyed by `boot_id`, advances `WaitingApproval` runs whose
    // approval resolved during the crash window, and fails out wedged
    // `Running` runs with no checkpoint and no recent progress. Actual
    // re-execution happens on the orchestrator's next tick — recovery
    // only prepares state.
    //
    // On error we halt startup. A cairn-app that can't reach the store
    // long enough to read run state has no business serving traffic.
    // Multi-instance correctness is deferred to a future RFC (RFC 020
    // delta Gap 2 resolution — v1 is single-instance).
    {
        let boot_id = cairn_domain::BootId::new(uuid::Uuid::now_v7().to_string());
        eprintln!("cairn-app boot_id={boot_id}");
        let recovery_service =
            cairn_runtime::RecoveryServiceImpl::new(lib_state.runtime.store.clone());
        match cairn_runtime::RecoveryService::recover_all(&recovery_service, &boot_id).await {
            Ok(summary) => {
                if summary.scanned_runs > 0 {
                    eprintln!(
                        "run recovery: scanned={} recovered={} advanced={} failed={} boot_id={}",
                        summary.scanned_runs,
                        summary.recovered_runs,
                        summary.advanced_runs,
                        summary.failed_runs,
                        boot_id,
                    );
                }
            }
            Err(error) => {
                // Halt startup: cairn-app does not serve traffic without
                // reconciled run state (RFC 020 Open Q #7 / Gap 6
                // resolution). systemd / Kubernetes re-runs us; an
                // operator gets paged after N failures.
                eprintln!("run recovery failed: {error}");
                std::process::exit(1);
            }
        }
    }

    // ── Startup replays ────────────────────────────────────────────────────────
    // Replay all store events into in-memory projections so pre-existing data
    // (seeded above or loaded from a snapshot) is immediately visible without
    // requiring an SSE connection first.
    lib_state.replay_graph().await;
    lib_state.replay_evals().await;
    lib_state.replay_triggers().await;
    lib_state.runtime.store.reset_usage_counters();

    // Flip the remaining RFC 020 readiness branches. Each represents a
    // startup concern whose real per-branch recovery is handled by a
    // later track. Marking them `complete` with `count = 0` means
    // "nothing to recover here yet" — the branch exists in the progress
    // JSON so clients see the complete contract.
    lib_state.readiness.update_branch("5", |b| {
        b.graph = BranchStatus::complete(0);
        b.memory = BranchStatus::complete(0);
        b.evals = BranchStatus::complete(0);
        b.repo_store = BranchStatus::complete(0);
        b.plugin_host = BranchStatus::complete(0);
        b.providers = BranchStatus::complete(0);
        b.tool_result_cache = BranchStatus::complete(0);
        b.decision_cache = BranchStatus::complete(0);
        b.webhook_dedup = BranchStatus::complete(0);
        b.triggers = BranchStatus::complete(0);
        b.runs = BranchStatus::complete(0);
    });

    eprintln!("cairn-app starting with role: {}", config.process_role);

    // ── RFC 011: Role-based startup ──────────────────────────────────────────
    if config.process_role.serves_http() {
        // ── Router ───────────────────────────────────────────────────────────
        let state_for_flush = state.clone();
        let app = build_router(lib_state.clone(), state);

        let addr = format!("{}:{}", config.listen_addr, config.listen_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));

        // Use the bound address rather than the requested one — when
        // `listen_port == 0`, the kernel picks a free port and integration
        // tests scrape this line to discover it.
        let bound = listener
            .local_addr()
            .unwrap_or_else(|e| panic!("failed to read bound addr: {e}"));
        eprintln!("cairn-app listening on http://{bound}");

        // RFC 020 §"Startup order" step 6: flip readiness to ready in a
        // background task so the HTTP listener is already accepting
        // connections (liveness + `/health/ready` responding 503 with the
        // progress JSON) by the time the final flip happens. In normal
        // production this races the first client request and wins; under
        // `CAIRN_TEST_STARTUP_DELAY_MS` (dev/test builds only) we sleep
        // first so integration tests can observe the 503-with-progress
        // response the RFC 020 contract promises.
        let readiness_for_flip = lib_state.readiness.clone();
        tokio::spawn(async move {
            #[cfg(debug_assertions)]
            if let Ok(ms) = std::env::var("CAIRN_TEST_STARTUP_DELAY_MS") {
                if let Ok(delay) = ms.parse::<u64>() {
                    tracing::warn!(
                        delay_ms = delay,
                        "CAIRN_TEST_STARTUP_DELAY_MS set — delaying readiness flip \
                         (debug build only; release strips this hook)"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
            readiness_for_flip.mark_ready();
            tracing::info!("cairn-app readiness: /health/ready now returns 200");
        });

        // ── Graceful shutdown wiring ─────────────────────────────────────────
        let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);

        let watchdog_state = state_for_flush.clone();
        let watchdog = tokio::spawn(async move {
            let mut rx = signal_rx;
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                if *rx.borrow() {
                    break;
                }
            }
            eprintln!("shutdown: draining in-flight requests (max 30s)…");
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            flush_state_to_disk(&watchdog_state).await;
            eprintln!("shutdown: 30s drain timeout — forcing exit");
            std::process::exit(0);
        });

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                wait_for_shutdown_signal().await;
                let _ = signal_tx.send(true);
            })
            .await
            .unwrap_or_else(|e| eprintln!("server error: {e}"));

        watchdog.abort();
        eprintln!("shutdown: all connections drained");
        flush_state_to_disk(&state_for_flush).await;
        eprintln!("shutdown: complete");
    } else {
        // ── WorkerOnly mode: no HTTP server, run task processing loop ────────
        eprintln!("cairn-app running in worker-only mode (no HTTP server)");
        eprintln!("connected to same store — processing tasks until shutdown signal");

        // Run a simple claim/execute loop until a shutdown signal arrives.
        // Both roles share the same store, so workers see events from the API.
        let shutdown = wait_for_shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    eprintln!("shutdown: worker received signal, exiting");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    // Worker tick: run due health checks, recovery sweeps, etc.
                    // These are non-blocking and use the shared store.
                    let _ = state.runtime.provider_health
                        .run_due_health_checks()
                        .await;
                }
            }
        }

        eprintln!("shutdown: worker complete");
    }
}

// LLM trace handlers → bin_handlers.rs
// OpenAPI spec, Swagger UI, embedded frontend → bin_frontend.rs
// build_router → bin_router.rs
