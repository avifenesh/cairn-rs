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
use cairn_runtime::{CredentialService, DefaultsService, RecoveryService};
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
            eprintln!("store: connecting to Postgres at {url}");
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
            eprintln!("store: connecting to SQLite at {url}");
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
        let lib_mut = Arc::get_mut(&mut lib_state)
            .expect("lib_state must not be cloned before brain_provider is wired");
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
                            let lib_mut = Arc::get_mut(&mut lib_state)
                                .expect("lib_state must not be cloned before github is wired");
                            lib_mut.github = Some(Arc::new(github));
                            let registry = Arc::get_mut(&mut lib_mut.integrations)
                                .expect("integrations registry must not be cloned yet");
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
        let lib_mut = Arc::get_mut(&mut lib_state)
            .expect("lib_state must not be cloned before tool_registry is wired");
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
        request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
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

    match lib_state.sandbox_service.recover_all().await {
        Ok(summary) => {
            if summary.reconnected > 0 || summary.preserved > 0 || summary.failed > 0 {
                eprintln!(
                    "sandbox recovery: reconnected={} preserved={} failed={}",
                    summary.reconnected, summary.preserved, summary.failed
                );
            }
        }
        Err(error) => eprintln!("sandbox recovery failed: {error}"),
    }

    let recovery_now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match lib_state
        .runtime
        .recovery
        .recover_expired_leases(recovery_now_ms, 1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "lease recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("lease recovery failed: {error}"),
    }
    match lib_state
        .runtime
        .recovery
        .recover_interrupted_runs(1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "run recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("run recovery failed: {error}"),
    }
    match lib_state
        .runtime
        .recovery
        .resolve_stale_dependencies(1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "dependency recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("dependency recovery failed: {error}"),
    }

    // ── Startup replays ────────────────────────────────────────────────────────
    // Replay all store events into in-memory projections so pre-existing data
    // (seeded above or loaded from a snapshot) is immediately visible without
    // requiring an SSE connection first.
    lib_state.replay_graph().await;
    lib_state.replay_evals().await;
    lib_state.replay_triggers().await;
    lib_state.runtime.store.reset_usage_counters();

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

        eprintln!("cairn-app listening on http://{addr}");

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
// ── Test helpers (visible to all test modules via `super::`) ─────────────────

#[cfg(test)]
fn test_make_app(mut state: AppState) -> axum::Router {
    // Construct lib_state on a dedicated thread to avoid tokio runtime nesting.
    let lib_state = std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        std::sync::Arc::new(
            rt.block_on(cairn_app::AppState::new(
                cairn_api::bootstrap::BootstrapConfig::default(),
            ))
            .expect("test lib state"),
        )
    })
    .join()
    .expect("lib_state thread panicked");
    // Copy all test tokens into the lib state's token registry so the catalog
    // router's auth middleware recognises them.
    for (token, principal) in state.tokens.all_entries() {
        lib_state.service_tokens.register(token, principal);
    }
    // Share the lib_state's runtime and stores so both routers see the same data.
    state.runtime = lib_state.runtime.clone();
    state.document_store = lib_state.document_store.clone();
    state.retrieval = lib_state.retrieval.clone();
    state.ingest = lib_state.ingest.clone();
    {
        use cairn_domain::providers::{EmbeddingProvider, GenerationProvider};
        use cairn_providers::chat::ChatProvider;

        state.runtime.provider_registry.set_startup_fallbacks(
            cairn_runtime::StartupFallbackProviders {
                ollama: state.ollama.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        Arc::new(OllamaEmbeddingProvider::new(provider.host()))
                            as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("ollama", None)
                }),
                brain: state.openai_compat_brain.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                worker: state.openai_compat_worker.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                openrouter: state.openai_compat_openrouter.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openrouter", Some(provider.model.clone()))
                }),
                bedrock: state.bedrock.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                    )
                    .with_metadata("bedrock", Some(provider.model_id().to_owned()))
                }),
            },
        );
    }
    build_router(lib_state, state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use axum::Router;
    use cairn_api::bootstrap::{ServerBootstrap, StorageBackend};
    use cairn_domain::{ProjectKey, SessionId};
    use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};
    use cairn_runtime::sessions::SessionService;
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    struct RecordingBootstrap {
        seen: Mutex<Option<BootstrapConfig>>,
    }

    impl RecordingBootstrap {
        fn new() -> Self {
            Self {
                seen: Mutex::new(None),
            }
        }
        fn seen(&self) -> Option<BootstrapConfig> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl ServerBootstrap for RecordingBootstrap {
        type Error = String;
        fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error> {
            *self.seen.lock().unwrap() = Some(config.clone());
            Ok(())
        }
    }

    fn run_bootstrap<B: ServerBootstrap>(b: &B, c: &BootstrapConfig) -> Result<(), B::Error> {
        b.start(c)
    }

    /// The test token registered by default in `make_state()`.
    const TEST_TOKEN: &str = "test-admin-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TEST_TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-admin".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "test-tenant",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
        }
    }

    #[tokio::test]
    async fn admin_backup_returns_404_when_sqlite_backend_is_disabled() {
        let app = make_app(make_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/v1/admin/backup")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["code"], "not_found");
        assert_eq!(
            payload["message"],
            "SQLite backup is only available when the SQLite backend is active"
        );
    }

    fn make_app(state: AppState) -> Router {
        super::test_make_app(state)
    }

    async fn authed_json(
        app: Router,
        method: axum::http::Method,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn authed_sse_post(app: Router, uri: &str, body: serde_json::Value) -> String {
        let resp = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri(uri)
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn spawn_openai_compat_mock(text: &'static str) -> String {
        let handler = move || async move {
            Json(serde_json::json!({
                "id": format!("mock-{text}"),
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": text,
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 2,
                    "total_tokens": 5
                }
            }))
        };
        let app = Router::new()
            .route("/chat/completions", post(handler))
            .route(
                "/v1/chat/completions",
                post(move || async move {
                    Json(serde_json::json!({
                        "id": format!("mock-{text}"),
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": text,
                            },
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 3,
                            "completion_tokens": 2,
                            "total_tokens": 5
                        }
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
    }

    async fn spawn_openai_compat_embedding_mock(
        model: &'static str,
        embedding: Vec<f32>,
        token_count: u32,
    ) -> String {
        let embedding_payload =
            serde_json::Value::Array(embedding.into_iter().map(serde_json::Value::from).collect());
        let body = serde_json::json!({
            "object": "list",
            "data": [{
                "object": "embedding",
                "index": 0,
                "embedding": embedding_payload,
            }],
            "model": model,
            "usage": {
                "prompt_tokens": token_count,
                "total_tokens": token_count,
            }
        });
        let app = Router::new()
            .route(
                "/embeddings",
                post({
                    let body = body.clone();
                    move || {
                        let body = body.clone();
                        async move { Json(body) }
                    }
                }),
            )
            .route(
                "/v1/embeddings",
                post(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
    }

    async fn spawn_openai_compat_stream_mock(chunks: Vec<&'static str>) -> String {
        let mut payload = String::new();
        for chunk in chunks {
            payload.push_str("data: ");
            payload.push_str(
                &serde_json::json!({
                    "choices": [{
                        "delta": {
                            "content": chunk,
                        }
                    }]
                })
                .to_string(),
            );
            payload.push_str("\n\n");
        }
        payload.push_str("data: [DONE]\n\n");

        let app = Router::new()
            .route(
                "/chat/completions",
                post({
                    let payload = payload.clone();
                    move || {
                        let payload = payload.clone();
                        async move {
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/event-stream")
                                .body(Body::from(payload))
                                .unwrap()
                        }
                    }
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let payload = payload.clone();
                    async move {
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "text/event-stream")
                            .body(Body::from(payload))
                            .unwrap()
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
    }

    /// Issue a GET request with the test bearer token.
    async fn authed_get(app: Router, uri: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Issue a GET request with NO auth header.
    async fn unauthed_get(app: Router, uri: &str) -> axum::response::Response {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    /// Build a GET request that includes the test auth token.
    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// Build a POST request with JSON body and the test auth token.
    fn authed_post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }

    fn assert_embedding_matches(actual: &serde_json::Value, expected: &[f64]) {
        let actual = actual.as_array().expect("embedding array");
        assert_eq!(actual.len(), expected.len(), "embedding length mismatch");
        for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            let actual = actual.as_f64().expect("embedding value");
            assert!(
                (actual - expected).abs() < 1e-5,
                "embedding[{index}] expected {expected}, got {actual}"
            );
        }
    }

    // ── Arg parsing ──

    #[test]
    fn parse_args_defaults_to_local_mode() {
        let config = parse_args_from(&["cairn-app".to_owned()]);
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn parse_args_promotes_team_mode_to_public_bind() {
        let config = parse_args_from(&[
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
        ]);
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn run_bootstrap_delegates_to_server_bootstrap() {
        let b = RecordingBootstrap::new();
        let c = BootstrapConfig::team("postgres://localhost/cairn");
        run_bootstrap(&b, &c).unwrap();
        assert_eq!(b.seen(), Some(c));
    }

    #[test]
    fn parse_args_db_flag_sets_postgres() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn parse_args_db_flag_sets_sqlite() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my_data.db".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn parse_args_db_memory_sets_in_memory() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "memory".to_owned(),
        ]);
        assert!(
            matches!(c.storage, StorageBackend::InMemory),
            "--db memory must select in-memory store"
        );
    }

    #[test]
    fn resolve_storage_picks_up_database_url() {
        let mut c = parse_args_from(&["cairn-app".to_owned()]);
        assert!(matches!(c.storage, StorageBackend::InMemory));
        // Simulate DATABASE_URL being set
        std::env::set_var("DATABASE_URL", "postgres://cairn:pass@localhost/cairn");
        resolve_storage_from_env(&mut c);
        assert!(
            matches!(c.storage, StorageBackend::Postgres { .. }),
            "DATABASE_URL must be picked up when no --db flag"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn resolve_storage_db_flag_wins_over_database_url() {
        std::env::set_var("DATABASE_URL", "postgres://ignored@localhost/db");
        let mut c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my.db".to_owned(),
        ]);
        resolve_storage_from_env(&mut c);
        assert!(
            matches!(c.storage, StorageBackend::Sqlite { .. }),
            "--db flag must take precedence over DATABASE_URL"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
        ]);
        assert!(!c.credentials_available());
    }

    #[test]
    fn parse_args_port_flag_overrides_default() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--port".to_owned(),
            "8080".to_owned(),
        ]);
        assert_eq!(c.listen_port, 8080);
    }

    // ── SSE stream tests ──────────────────────────────────────────────────────

    /// Drive the SSE stream from an HTTP request using tower's oneshot and
    /// collect the first N bytes of the SSE body.
    async fn collect_sse_bytes(
        app: axum::Router,
        uri: &str,
        extra_headers: Vec<(&str, &str)>,
    ) -> Vec<u8> {
        let mut builder = Request::builder().uri(uri);
        for (k, v) in extra_headers {
            builder = builder.header(k, v);
        }
        let resp = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Read the first 4 KB to capture the initial events.
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        bytes.to_vec()
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_sends_connected_event_on_connect() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.contains("event: connected"),
            "missing connected event; got: {text}"
        );
        assert!(
            text.contains("head_position"),
            "connected payload missing head_position; got: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_replays_events_after_last_event_id() {
        let state = make_state();
        let project = ProjectKey::new("ts", "ws", "ps");

        // Create 3 sessions — generates positions 1, 2, 3.
        for i in 0u32..3 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_sse_{i}")))
                .await
                .unwrap();
        }

        // Reconnect with Last-Event-ID: 1 → should replay positions 2 and 3.
        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![("last-event-id", "1")]).await;
        let text = String::from_utf8_lossy(&raw);

        // Should contain event type and session_created payloads.
        let session_created_count = text.matches("event: session_created").count();
        assert!(
            session_created_count >= 2,
            "expected ≥2 replayed session_created events; got {session_created_count} in: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_event_includes_id_field() {
        let state = make_state();
        let project = ProjectKey::new("ti", "wi", "pi");
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_id_test"))
            .await
            .unwrap();

        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);

        // The session_created event should have an `id:` line.
        assert!(
            text.contains("\nid: ") || text.starts_with("id: "),
            "SSE id: field missing; got: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_last_event_id_zero_replays_all_events() {
        let state = make_state();
        let project = ProjectKey::new("tz", "wz", "pz");

        // Two sessions → positions 1 and 2.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_z_1"))
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_z_2"))
            .await
            .unwrap();

        // Last-Event-ID: 0 is before all positions (positions start at 1) →
        // should replay both events.
        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![("last-event-id", "0")]).await;
        let text = String::from_utf8_lossy(&raw);

        let count = text.matches("event: session_created").count();
        assert_eq!(
            count, 2,
            "expected 2 replayed events; got {count} in: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_empty_store_sends_only_connected() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);

        // Only the connected event, no session_created events.
        assert!(text.contains("event: connected"));
        assert!(
            !text.contains("event: session_created"),
            "unexpected events: {text}"
        );
    }

    // ── Integration-style route tests ──

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn get_runs_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/runs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(runs.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_run_not_found_returns_404() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/nonexistent_run_id")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Task endpoint tests ──────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_tasks_returns_empty_for_run_with_no_tasks() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent};

        let state = make_state();
        let project = ProjectKey::new("t_task", "w_task", "p_task");
        let session_id = SessionId::new("sess_task_empty");
        let run_id = cairn_domain::RunId::new("run_notasks");

        // Create session + run but add no tasks.
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_run_notasks"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_notasks/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "run with no tasks returns 200"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tasks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            tasks.as_array().unwrap().is_empty(),
            "no tasks = empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_tasks_returns_tasks_for_run() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, TaskCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("t_tasks", "w_tasks", "p_tasks");
        let session_id = SessionId::new("sess_tasks");
        let run_id = cairn_domain::RunId::new("run_withtasks");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();

        // Create run + two tasks.
        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_wt"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_task_1"),
                    EventSource::Runtime,
                    RuntimeEvent::TaskCreated(TaskCreated {
                        project: project.clone(),
                        task_id: cairn_domain::TaskId::new("task_alpha"),
                        parent_run_id: Some(run_id.clone()),
                        parent_task_id: None,
                        prompt_release_id: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_task_2"),
                    EventSource::Runtime,
                    RuntimeEvent::TaskCreated(TaskCreated {
                        project: project.clone(),
                        task_id: cairn_domain::TaskId::new("task_beta"),
                        parent_run_id: Some(run_id.clone()),
                        parent_task_id: None,
                        prompt_release_id: None,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_withtasks/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tasks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = tasks.as_array().unwrap();
        assert_eq!(arr.len(), 2, "two tasks must be returned");

        let task_ids: Vec<_> = arr.iter().map(|t| t["task_id"].as_str().unwrap()).collect();
        assert!(
            task_ids.contains(&"task_alpha"),
            "task_alpha must be in response"
        );
        assert!(
            task_ids.contains(&"task_beta"),
            "task_beta must be in response"
        );
        // Each task must link back to the run.
        for t in arr {
            assert_eq!(
                t["parent_run_id"], "run_withtasks",
                "every task must reference its parent run"
            );
        }
    }

    #[tokio::test]
    async fn list_run_tasks_returns_404_for_unknown_run() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/ghost_run/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "unknown run must return 404"
        );
    }

    // ── Approval endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_empty_for_run_with_no_approvals() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent};

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        let session_id = SessionId::new("sess_appr_empty");
        let run_id_str = "run_appr_empty";
        let run_id = cairn_domain::RunId::new(run_id_str);

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_run_ae"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            approvals.as_array().unwrap().is_empty(),
            "run with no approvals must return empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_shows_pending_approval() {
        use cairn_domain::policy::ApprovalRequirement;
        use cairn_domain::{
            ApprovalId, ApprovalRequested, EventEnvelope, EventId, EventSource, RunCreated,
            RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tb", "wb", "pb");
        let session_id = SessionId::new("sess_appr_pend");
        let run_id_str = "run_appr_pend";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_pend");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_ap"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_appr_pend"),
                    EventSource::Runtime,
                    RuntimeEvent::ApprovalRequested(ApprovalRequested {
                        project: project.clone(),
                        approval_id: approval_id.clone(),
                        run_id: Some(run_id.clone()),
                        task_id: None,
                        requirement: ApprovalRequirement::Required,
                        title: None,
                        description: None,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one pending approval");
        assert_eq!(arr[0]["approval_id"], "appr_pend");
        assert!(
            arr[0]["decision"].is_null(),
            "pending approval has no decision"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_shows_resolved_decision() {
        use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};
        use cairn_domain::{
            ApprovalId, ApprovalRequested, ApprovalResolved, EventEnvelope, EventId, EventSource,
            RunCreated, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_appr_res");
        let run_id_str = "run_appr_res";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_res");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_ar"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: run_id.clone(),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_appr_req"),
                    EventSource::Runtime,
                    RuntimeEvent::ApprovalRequested(ApprovalRequested {
                        project: project.clone(),
                        approval_id: approval_id.clone(),
                        run_id: Some(run_id.clone()),
                        task_id: None,
                        requirement: ApprovalRequirement::Required,
                        title: None,
                        description: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_appr_res"),
                    EventSource::Runtime,
                    RuntimeEvent::ApprovalResolved(ApprovalResolved {
                        project: project.clone(),
                        approval_id: approval_id.clone(),
                        decision: ApprovalDecision::Approved,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one resolved approval");
        assert_eq!(arr[0]["approval_id"], "appr_res");
        // Decision must be populated after resolution.
        assert_eq!(
            arr[0]["decision"], "approved",
            "resolved approval must carry the decision"
        );
    }

    // ── Session runs endpoint tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_empty_for_session_with_no_runs() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, SessionCreated};

        let state = make_state();
        let project = ProjectKey::new("tr1", "wr1", "pr1");
        let session_id = SessionId::new("sess_noruns");

        // Create session via event but add no runs.
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_sess_nr"),
                EventSource::Runtime,
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                }),
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_noruns/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            runs.as_array().unwrap().is_empty(),
            "session with no runs must return empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_returns_two_runs() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, SessionCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("tr2", "wr2", "pr2");
        let session_id = SessionId::new("sess_tworuns");

        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_sess_tr"),
                    EventSource::Runtime,
                    RuntimeEvent::SessionCreated(SessionCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_tr1"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: cairn_domain::RunId::new("run_tr_1"),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_tr2"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: cairn_domain::RunId::new("run_tr_2"),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_tworuns/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 2, "session must have 2 runs");

        let run_ids: Vec<_> = arr.iter().map(|r| r["run_id"].as_str().unwrap()).collect();
        assert!(run_ids.contains(&"run_tr_1"));
        assert!(run_ids.contains(&"run_tr_2"));
        // All runs belong to the session.
        for r in arr {
            assert_eq!(r["session_id"], "sess_tworuns");
        }
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_shows_parent_run_id_for_subagent() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, SessionCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("tr3", "wr3", "pr3");
        let session_id = SessionId::new("sess_subagent");

        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_sess_sa"),
                    EventSource::Runtime,
                    RuntimeEvent::SessionCreated(SessionCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                    }),
                ),
                // Root orchestrator run.
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_root"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: cairn_domain::RunId::new("run_root"),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: Some("orchestrator".to_owned()),
                    }),
                ),
                // Subagent spawned by root.
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_run_sub"),
                    EventSource::Runtime,
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project.clone(),
                        session_id: session_id.clone(),
                        run_id: cairn_domain::RunId::new("run_subagent"),
                        parent_run_id: Some(cairn_domain::RunId::new("run_root")),
                        prompt_release_id: None,
                        agent_role_id: Some("researcher".to_owned()),
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_subagent/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 2, "root + subagent = 2 runs");

        let root = arr.iter().find(|r| r["run_id"] == "run_root").unwrap();
        assert!(root["parent_run_id"].is_null(), "root run has no parent");
        assert_eq!(root["agent_role_id"], "orchestrator");

        let sub = arr.iter().find(|r| r["run_id"] == "run_subagent").unwrap();
        assert_eq!(
            sub["parent_run_id"], "run_root",
            "subagent must reference root run as parent"
        );
        assert_eq!(sub["agent_role_id"], "researcher");
    }

    #[tokio::test]
    async fn list_session_runs_returns_404_for_unknown_session() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/ghost_session/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn get_sessions_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(sessions.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_pending_approvals_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/approvals/pending")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(approvals.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolve_nonexistent_approval_returns_404() {
        let app = make_app(make_state());
        let body = serde_json::json!({"decision": "approved"});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/approvals/no_such_approval/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn resolve_bad_decision_returns_400() {
        let app = make_app(make_state());
        let body = serde_json::json!({"decision": "maybe"});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/approvals/any_id/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn runs_list_reflects_created_run() {
        let state = make_state();
        let project = ProjectKey::new("t1", "w1", "p1");
        let session_id = cairn_domain::SessionId::new("sess_1");
        let run_id = cairn_domain::RunId::new("run_1");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id, None)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/runs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(runs[0]["run_id"], "run_1");
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn get_run_by_id_returns_record() {
        let state = make_state();
        let project = ProjectKey::new("t2", "w2", "p2");
        let session_id = SessionId::new("sess_2");
        let run_id_str = "run_2";
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &session_id,
                cairn_domain::RunId::new(run_id_str),
                None,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let run: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(run["run_id"], run_id_str);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn sessions_list_reflects_created_session() {
        let state = make_state();
        let project = ProjectKey::new("t3", "w3", "p3");
        let session_id = SessionId::new("sess_3");
        state
            .runtime
            .sessions
            .create(&project, session_id)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/sessions")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(sessions.as_array().unwrap().len(), 1);
        assert_eq!(sessions[0]["session_id"], "sess_3");
    }

    // ── Prompt asset / release tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn prompt_assets_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/prompts/assets")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn prompt_assets_reflects_created_asset() {
        use cairn_domain::PromptAssetId;
        use cairn_runtime::prompt_assets::PromptAssetService as _;

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        state
            .runtime
            .prompt_assets
            .create(
                &project,
                PromptAssetId::new("asset_a"),
                "My Prompt".to_owned(),
                "system".to_owned(),
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/prompts/assets")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["prompt_asset_id"], "asset_a");
        assert_eq!(items[0]["name"], "My Prompt");
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn prompt_releases_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/prompts/releases"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    // ── Cost tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn costs_empty_store_returns_zeros() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/costs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_provider_calls"], 0);
        assert_eq!(cost["total_cost_micros"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn costs_reflects_run_cost_events() {
        use cairn_domain::{
            events::RunCostUpdated, EventEnvelope, EventId, EventSource, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_c");
        let run_id = cairn_domain::RunId::new("run_c");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_cost_c"),
                EventSource::Runtime,
                RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    session_id: Some(session_id.clone()),
                    tenant_id: Some(cairn_domain::TenantId::new("tc")),
                    delta_cost_micros: 500,
                    delta_tokens_in: 100,
                    delta_tokens_out: 50,
                    provider_call_id: "call_c".to_owned(),
                    updated_at_ms: 1000,
                }),
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/costs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 500);
        assert_eq!(cost["total_tokens_in"], 100);
        assert_eq!(cost["total_tokens_out"], 50);
    }

    // ── Provider tests ────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn providers_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/providers")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn providers_reflects_created_binding() {
        use cairn_domain::{
            events::ProviderBindingCreated,
            providers::{OperationKind, ProviderBindingSettings},
            EventEnvelope, EventId, EventSource, ProviderBindingId, ProviderConnectionId,
            ProviderModelId, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tp", "wp", "pp");

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_bind_p"),
                EventSource::Runtime,
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: project.clone(),
                    provider_binding_id: ProviderBindingId::new("bind_p"),
                    provider_connection_id: ProviderConnectionId::new("conn_p"),
                    provider_model_id: ProviderModelId::new("model_p"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 1000,
                    estimated_cost_micros: None,
                }),
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/providers")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["provider_binding_id"], "bind_p");
    }

    // ── Event replay tests ────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn events_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn events_returns_all_events_from_log() {
        let state = make_state();
        let project = ProjectKey::new("te", "we", "pe");
        let session_id = SessionId::new("sess_e");
        // Creating a session appends a SessionCreated event.
        state
            .runtime
            .sessions
            .create(&project, session_id)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert!(!arr.is_empty(), "expected at least one event");
        assert_eq!(arr[0]["event_type"], "session_created");
        assert!(arr[0]["position"].as_u64().is_some());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn events_after_cursor_paginates() {
        let state = make_state();
        let project = ProjectKey::new("tf", "wf", "pf");
        // Create 3 sessions → 3 events at positions 0, 1, 2.
        for i in 0u32..3 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_f_{i}")))
                .await
                .unwrap();
        }

        let app = make_app(state);
        // Positions start at 1. after=1 means "after position 1" → should return
        // positions 2 and 3 (the second and third SessionCreated events).
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/events?after=1&limit=10")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert_eq!(arr.len(), 2, "expected events after position 1");
        assert!(arr.iter().all(|e| e["position"].as_u64().unwrap() > 1));
    }

    #[tokio::test]
    async fn events_limit_is_respected() {
        let state = make_state();
        let project = ProjectKey::new("tg", "wg", "pg");
        for i in 0u32..5 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_g_{i}")))
                .await
                .unwrap();
        }

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/events?limit=3")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(events.as_array().unwrap().len(), 3);
    }

    // ── Event append tests (RFC 002) ──────────────────────────────────────────

    /// Build a minimal valid EventEnvelope JSON for a SessionCreated event.
    ///
    /// Serde shapes used here:
    /// - `EventSource`:  internally tagged with `"source_type"`, snake_case variants
    ///   → `Runtime` → `{"source_type":"runtime"}`
    /// - `OwnershipKey`: internally tagged with `"scope"`, snake_case variants
    ///   → `Project(…)` → `{"scope":"project","tenant_id":…,…}`
    /// - `RuntimeEvent`: internally tagged with `"event"`, snake_case variants,
    ///   content flattened → `{"event":"session_created","project":{…},"session_id":"…"}`
    /// - `SessionCreated` has no `created_at` field.
    fn session_created_envelope(event_id: &str, session_id: &str) -> serde_json::Value {
        serde_json::json!({
            "event_id": event_id,
            "source": { "source_type": "runtime" },
            "ownership": {
                "scope": "project",
                "tenant_id": "t_append",
                "workspace_id": "w_append",
                "project_id": "p_append"
            },
            "causation_id": null,
            "correlation_id": null,
            "payload": {
                "event": "session_created",
                "project": {
                    "tenant_id": "t_append",
                    "workspace_id": "w_append",
                    "project_id": "p_append"
                },
                "session_id": session_id
            }
        })
    }

    /// Same as above but with a causation_id for idempotency testing.
    fn session_created_with_causation(
        event_id: &str,
        session_id: &str,
        causation_id: &str,
    ) -> serde_json::Value {
        let mut e = session_created_envelope(event_id, session_id);
        e["causation_id"] = serde_json::json!(causation_id);
        e
    }

    async fn post_append(
        app: axum::Router,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/events/append")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn append_single_event_returns_201_with_position() {
        let app = make_app(make_state());
        let envelope = session_created_envelope("evt_a1", "sess_a1");
        let (status, results) = post_append(app, serde_json::json!([envelope])).await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["event_id"], "evt_a1");
        assert!(
            arr[0]["position"].as_u64().unwrap() > 0,
            "position must be ≥ 1"
        );
        assert_eq!(arr[0]["appended"], true);
    }

    #[tokio::test]
    async fn append_empty_batch_returns_200_empty_array() {
        let app = make_app(make_state());
        let (status, results) = post_append(app, serde_json::json!([])).await;
        assert_eq!(status, StatusCode::OK);
        assert!(results.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn append_assigns_sequential_positions() {
        let app = make_app(make_state());
        let envelopes = serde_json::json!([
            session_created_envelope("evt_seq1", "sess_seq1"),
            session_created_envelope("evt_seq2", "sess_seq2"),
            session_created_envelope("evt_seq3", "sess_seq3"),
        ]);
        let (status, results) = post_append(app, envelopes).await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        let positions: Vec<u64> = arr
            .iter()
            .map(|r| r["position"].as_u64().unwrap())
            .collect();
        // All positions must be distinct and strictly increasing.
        assert!(positions[0] < positions[1]);
        assert!(positions[1] < positions[2]);
        assert!(arr.iter().all(|r| r["appended"] == true));
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_idempotent_with_causation_id_returns_existing_position() {
        let state = make_state();
        let causation = "cmd_idem_1";

        // First append — creates the event.
        let env = session_created_with_causation("evt_idem1", "sess_idem1", causation);
        let (status1, res1) =
            post_append(make_app(state.clone()), serde_json::json!([env.clone()])).await;
        assert_eq!(status1, StatusCode::CREATED);
        let first_pos = res1[0]["position"].as_u64().unwrap();
        assert_eq!(res1[0]["appended"], true);

        // Second append — same causation_id, different event_id.
        let env2 = session_created_with_causation("evt_idem2", "sess_idem2", causation);
        let (status2, res2) = post_append(make_app(state.clone()), serde_json::json!([env2])).await;
        assert_eq!(status2, StatusCode::CREATED);
        let second_pos = res2[0]["position"].as_u64().unwrap();
        assert_eq!(
            res2[0]["appended"], false,
            "second append should be idempotent"
        );
        assert_eq!(
            second_pos, first_pos,
            "idempotent append must return the original position"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_mixed_batch_new_and_idempotent() {
        let state = make_state();
        let causation = "cmd_mixed_1";

        // Pre-append the first event.
        let env_pre = session_created_with_causation("evt_m0", "sess_m0", causation);
        post_append(make_app(state.clone()), serde_json::json!([env_pre])).await;

        // Batch: first is a duplicate (causation_id present), second is new.
        let env_dup = session_created_with_causation("evt_m1", "sess_m1", causation);
        let env_new = session_created_envelope("evt_m2", "sess_m2");
        let (status, results) = post_append(
            make_app(state.clone()),
            serde_json::json!([env_dup, env_new]),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(
            arr[0]["appended"], false,
            "first should be idempotent duplicate"
        );
        assert_eq!(arr[1]["appended"], true, "second should be newly appended");
        assert!(
            arr[1]["position"].as_u64().unwrap() > arr[0]["position"].as_u64().unwrap(),
            "new event position must be greater than duplicate's original position"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_event_appears_in_event_log_immediately() {
        let state = make_state();
        let app1 = make_app(state.clone());
        let app2 = make_app(state.clone());

        // Append one event.
        let env = session_created_envelope("evt_vis1", "sess_vis1");
        let (_, results) = post_append(app1, serde_json::json!([env])).await;
        let appended_pos = results[0]["position"].as_u64().unwrap();

        // The event should now appear in GET /v1/events.
        let resp = app2.oneshot(authed_req("/v1/events")).await.unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let positions: Vec<u64> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["position"].as_u64().unwrap())
            .collect();
        assert!(
            positions.contains(&appended_pos),
            "appended event at position {appended_pos} not found in event log; got: {positions:?}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_broadcasts_to_sse_subscribers() {
        let state = make_state();
        // Subscribe to the broadcast channel BEFORE appending.
        let mut receiver = state.runtime.store.subscribe();

        // Append one event via the handler.
        let env = session_created_envelope("evt_bc1", "sess_bc1");
        let app = make_app(state.clone());
        let (status, _) = post_append(app, serde_json::json!([env])).await;
        assert_eq!(status, StatusCode::CREATED);

        // The receiver should have gotten the event immediately.
        let received = tokio::time::timeout(std::time::Duration::from_millis(200), async {
            receiver.recv().await
        })
        .await
        .expect("broadcast delivery timed out")
        .expect("broadcast channel closed");

        assert_eq!(
            event_type_name(&received.envelope.payload),
            "session_created",
            "wrong event type in broadcast"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_no_causation_id_always_appends() {
        let state = make_state();

        // Two envelopes with the same event_id but no causation_id →
        // both get appended (no idempotency guard).
        let env1 = session_created_envelope("evt_nc1", "sess_nc1");
        let env2 = session_created_envelope("evt_nc2", "sess_nc2");

        let (_, r1) = post_append(make_app(state.clone()), serde_json::json!([env1])).await;
        let (_, r2) = post_append(make_app(state.clone()), serde_json::json!([env2])).await;

        assert_eq!(r1[0]["appended"], true);
        assert_eq!(r2[0]["appended"], true);
        // Positions are distinct.
        assert_ne!(r1[0]["position"], r2[0]["position"]);
    }

    // ── Auth middleware tests (RFC 008) ───────────────────────────────────────

    #[tokio::test]
    async fn valid_token_passes_protected_route() {
        let resp = authed_get(make_app(make_state()), "/v1/status").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_token_returns_401_on_protected_route() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(err["code"], "unauthorized");
    }

    #[tokio::test]
    async fn missing_token_returns_401_on_protected_route() {
        let resp = unauthed_get(make_app(make_state()), "/v1/runs").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(err["code"], "unauthorized");
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn health_endpoint_requires_no_token() {
        // /health is public — no Authorization header needed.
        let resp = unauthed_get(make_app(make_state()), "/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let h: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(h["ok"], true);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_endpoint_requires_no_token() {
        // /v1/stream is exempt — SSE clients use EventSource which can't set headers.
        let resp = unauthed_get(make_app(make_state()), "/v1/stream").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn multiple_tokens_can_be_registered() {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            "token-a".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "svc-a".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("t1")),
            },
        );
        tokens.register(
            "token-b".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "svc-b".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("t2")),
            },
        );
        let doc_store = std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
        let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
            doc_store.clone(),
        ));
        let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
            doc_store.clone(),
            cairn_memory::pipeline::ParagraphChunker {
                max_chunk_size: 512,
            },
        ));
        let state = AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(Instant::now()),
            tokens,
            pg: None,
            sqlite: None,
            mode: DeploymentMode::Local,
            document_store: doc_store,
            retrieval,
            ingest,
            ollama: None,
            openai_compat_brain: None,
            openai_compat_worker: None,
            openai_compat_openrouter: None,
            openai_compat: None,
            metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
            notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
            templates: Arc::new(templates::TemplateRegistry::with_builtins()),
            entitlements: Arc::new(entitlements::EntitlementService::new()),
            bedrock: None,
            process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
        };
        let app = make_app(state);

        // Both tokens are valid.
        for tok in &["token-a", "token-b"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/status")
                        .header("authorization", format!("Bearer {tok}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "token {tok} should be valid");
        }
    }

    #[tokio::test]
    async fn auth_error_body_contains_code_and_message() {
        let resp = unauthed_get(make_app(make_state()), "/v1/dashboard").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Must contain both `code` and `message` per ApiError contract.
        assert!(err.get("code").is_some(), "missing code field");
        assert!(err.get("message").is_some(), "missing message field");
    }

    // ── GET /v1/db/status tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn db_status_in_memory_backend_returns_correct_fields() {
        let resp = authed_get(make_app(make_state()), "/v1/db/status").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(status["backend"], "in_memory");
        assert_eq!(status["connected"], true);
        // In-memory mode has no migration tracking.
        assert!(status["migration_count"].is_null());
        assert!(status["schema_current"].is_null());
    }

    #[tokio::test]
    async fn db_status_requires_auth() {
        let resp = unauthed_get(make_app(make_state()), "/v1/db/status").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn db_status_shape_matches_contract() {
        let resp = authed_get(make_app(make_state()), "/v1/db/status").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // All four contract fields must be present.
        assert!(status.get("backend").is_some(), "missing backend");
        assert!(status.get("connected").is_some(), "missing connected");
        assert!(
            status.get("migration_count").is_some(),
            "missing migration_count"
        );
        assert!(
            status.get("schema_current").is_some(),
            "missing schema_current"
        );
    }

    // ── End-to-end write → project → read cycle tests ────────────────────────
    //
    // These five tests prove the full pipeline:
    //   POST /v1/events/append → InMemory synchronous projection → GET read endpoint
    // Each test uses only the HTTP surface so they exercise exactly what a real
    // client would do.

    /// (1) POST SessionCreated via /v1/events/append → GET /v1/sessions shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_session_then_list_sessions_shows_it() {
        let state = make_state();
        let envelope = session_created_envelope("evt_e2e_s1", "sess_e2e_1");
        let (status, results) =
            post_append(make_app(state.clone()), serde_json::json!([envelope])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            results[0]["appended"], true,
            "event must be freshly appended"
        );

        let resp = make_app(state)
            .oneshot(authed_req("/v1/sessions"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = sessions.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one session must appear after append");
        assert_eq!(
            arr[0]["session_id"], "sess_e2e_1",
            "session_id must match what GET /v1/sessions returns"
        );
    }

    /// (2) POST RunCreated via /v1/events/append → GET /v1/runs shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_run_then_list_runs_shows_it() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_e2e","workspace_id":"w_e2e","project_id":"p_e2e"});
        let sess_env = session_created_envelope("evt_e2e_sess", "sess_e2e_run");
        post_append(make_app(state.clone()), serde_json::json!([sess_env])).await;

        let run_env = serde_json::json!({
            "event_id": "evt_e2e_run1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_e2e", "workspace_id": "w_e2e", "project_id": "p_e2e"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "run_created", "project": proj,
                "session_id": "sess_e2e_run", "run_id": "run_e2e_1",
                "parent_run_id": null, "prompt_release_id": null, "agent_role_id": null
            }
        });
        let (status, results) =
            post_append(make_app(state.clone()), serde_json::json!([run_env])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(results[0]["appended"], true);

        let resp = make_app(state)
            .oneshot(authed_req("/v1/runs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one run must appear after append");
        assert_eq!(
            arr[0]["run_id"], "run_e2e_1",
            "run_id must match what GET /v1/runs returns"
        );
    }

    /// (3) POST ApprovalRequested → GET /v1/approvals/pending shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_approval_then_list_pending_shows_it() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_ap","workspace_id":"w_ap","project_id":"p_ap"});
        let approval_env = serde_json::json!({
            "event_id": "evt_e2e_ap1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_ap", "workspace_id": "w_ap", "project_id": "p_ap"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "approval_requested", "project": proj,
                "approval_id": "appr_e2e_1",
                "run_id": null, "task_id": null, "requirement": "required"
            }
        });
        let (status, _) =
            post_append(make_app(state.clone()), serde_json::json!([approval_env])).await;
        assert_eq!(status, StatusCode::CREATED);

        let resp = make_app(state)
            .oneshot(authed_req(
                "/v1/approvals/pending?tenant_id=t_ap&workspace_id=w_ap&project_id=p_ap",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "one pending approval must appear after append"
        );
        assert_eq!(arr[0]["approval_id"], "appr_e2e_1");
        assert!(
            arr[0]["decision"].is_null(),
            "pending approval must have null decision"
        );
    }

    /// (4) POST ApprovalRequested then POST /v1/approvals/:id/resolve(Approved)
    /// → GET /v1/approvals/pending is empty.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_resolve_approval_removes_from_pending() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_res","workspace_id":"w_res","project_id":"p_res"});
        let approval_env = serde_json::json!({
            "event_id": "evt_e2e_res1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_res", "workspace_id": "w_res", "project_id": "p_res"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "approval_requested", "project": proj,
                "approval_id": "appr_e2e_res",
                "run_id": null, "task_id": null, "requirement": "required"
            }
        });
        post_append(make_app(state.clone()), serde_json::json!([approval_env])).await;

        let resolve_resp = make_app(state.clone())
            .oneshot(authed_post(
                "/v1/approvals/appr_e2e_res/resolve",
                serde_json::json!({"decision": "approved"}),
            ))
            .await
            .unwrap();
        assert_eq!(
            resolve_resp.status(),
            StatusCode::OK,
            "resolve must return 200"
        );
        let rbody = to_bytes(resolve_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resolved: serde_json::Value = serde_json::from_slice(&rbody).unwrap();
        assert_eq!(
            resolved["decision"], "approved",
            "resolved approval must carry decision=approved"
        );

        let resp = make_app(state)
            .oneshot(authed_req(
                "/v1/approvals/pending?tenant_id=t_res&workspace_id=w_res&project_id=p_res",
            ))
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let pending: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            pending.as_array().unwrap().is_empty(),
            "pending list must be empty after approval resolved"
        );
    }

    /// (5) Append session + run, then GET /v1/dashboard shows active_runs=1.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_dashboard_active_runs_reflects_appended_run() {
        let state = make_state();

        let resp0 = make_app(state.clone())
            .oneshot(authed_req("/v1/dashboard"))
            .await
            .unwrap();
        let body0 = to_bytes(resp0.into_body(), usize::MAX).await.unwrap();
        let dash0: serde_json::Value = serde_json::from_slice(&body0).unwrap();
        assert_eq!(
            dash0["active_runs"], 0,
            "dashboard must start with 0 active runs"
        );

        let proj =
            serde_json::json!({"tenant_id":"t_dash","workspace_id":"w_dash","project_id":"p_dash"});
        let sess_env = serde_json::json!({
            "event_id": "evt_dash_sess",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_dash", "workspace_id": "w_dash", "project_id": "p_dash"},
            "causation_id": null, "correlation_id": null,
            "payload": {"event": "session_created", "project": proj, "session_id": "sess_dash_1"}
        });
        post_append(make_app(state.clone()), serde_json::json!([sess_env])).await;

        let run_env = serde_json::json!({
            "event_id": "evt_dash_run",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_dash", "workspace_id": "w_dash", "project_id": "p_dash"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "run_created", "project": proj,
                "session_id": "sess_dash_1", "run_id": "run_dash_1",
                "parent_run_id": null, "prompt_release_id": null, "agent_role_id": null
            }
        });
        post_append(make_app(state.clone()), serde_json::json!([run_env])).await;

        let resp1 = make_app(state)
            .oneshot(authed_req("/v1/dashboard"))
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let dash1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        assert_eq!(
            dash1["active_runs"], 1,
            "dashboard must show active_runs=1 after appending one RunCreated"
        );
        assert!(
            dash1["system_healthy"].as_bool().unwrap_or(false),
            "system must be healthy"
        );
    }

    // ── CORS tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn options_preflight_returns_cors_headers() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/v1/events/append")
                    .header("origin", "http://localhost:5173")
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "content-type,authorization",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "OPTIONS preflight must succeed; got {}",
            resp.status()
        );
        let h = resp.headers();
        assert!(
            h.contains_key("access-control-allow-origin"),
            "missing ACAO header"
        );
        assert!(
            h.contains_key("access-control-allow-methods"),
            "missing ACAM header"
        );
        assert!(
            h.contains_key("access-control-allow-headers"),
            "missing ACAH header"
        );
    }

    #[tokio::test]
    async fn cors_allow_origin_is_wildcard() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/health")
                    .header("origin", "https://example.com")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let acao = resp
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(acao, "*", "allow_origin must be wildcard (*)");
    }

    #[tokio::test]
    async fn regular_get_includes_cors_header() {
        let resp = authed_get(make_app(make_state()), "/v1/status").await;
        let acao = resp.headers().get("access-control-allow-origin");
        assert!(
            acao.is_some(),
            "GET response must include Access-Control-Allow-Origin"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn cors_allow_methods_includes_required_verbs() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/v1/events/append")
                    .header("origin", "http://localhost:3000")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let methods = resp
            .headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_uppercase();
        for verb in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            assert!(
                methods.contains(verb),
                "Access-Control-Allow-Methods must include {verb}; got: {methods}"
            );
        }
    }

    // ── GET /v1/sessions/:id/events tests ────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_empty_for_unknown_session() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/sessions/no_such_session/events").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_returns_events_for_session() {
        let state = make_state();
        let project = ProjectKey::new("t_sev", "w_sev", "p_sev");
        let session_id = SessionId::new("sess_sev");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/sessions/sess_sev/events").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert!(!arr.is_empty(), "session must have at least one event");
        assert_eq!(arr[0]["event_type"], "session_created");
        assert!(arr[0]["position"].as_u64().is_some());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_scoped_to_session_only() {
        let state = make_state();
        let project = ProjectKey::new("t_scope", "w_scope", "p_scope");
        // Create two sessions — each gets a SessionCreated event.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_scope_a"))
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_scope_b"))
            .await
            .unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/sessions/sess_scope_a/events").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        // Only sess_scope_a events must appear — not sess_scope_b.
        assert_eq!(
            arr.len(),
            1,
            "only one SessionCreated event for sess_scope_a"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_after_cursor_paginates() {
        use cairn_domain::{
            events::SessionStateChanged, events::StateTransition as ST, tenancy::OwnershipKey,
            EventEnvelope, EventId, EventSource,
        };

        let state = make_state();
        let project = ProjectKey::new("t_cur", "w_cur", "p_cur");
        let session_id = SessionId::new("sess_cur");
        // SessionCreated → event 1 (session-scoped).
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        // Append a SessionStateChanged directly → event 2 (also session-scoped).
        state
            .runtime
            .store
            .append(&[EventEnvelope::new(
                EventId::new("evt_ssc_cur"),
                EventSource::Runtime,
                OwnershipKey::Project(project.clone()),
                cairn_domain::RuntimeEvent::SessionStateChanged(SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: ST {
                        from: Some(cairn_domain::SessionState::Open),
                        to: cairn_domain::SessionState::Completed,
                    },
                }),
            )])
            .await
            .unwrap();

        let app_all = make_app(state.clone());
        let app_page = make_app(state.clone());

        let resp_all = authed_get(app_all, "/v1/sessions/sess_cur/events").await;
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(
            all_arr.len() >= 2,
            "expect session_created + session_state_changed"
        );

        // Use the first event position as cursor.
        let first_pos = all_arr[0]["position"].as_u64().unwrap();
        let uri = format!("/v1/sessions/sess_cur/events?after={first_pos}");
        let resp_page = authed_get(app_page, &uri).await;
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();
        assert_eq!(
            page_arr.len(),
            all_arr.len() - 1,
            "after=first_pos must return one fewer event"
        );
        assert!(page_arr
            .iter()
            .all(|e| e["position"].as_u64().unwrap() > first_pos));
    }

    // ── GET /v1/runs/:id/cost tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_returns_zeros_when_no_provider_calls() {
        let state = make_state();
        let project = ProjectKey::new("t_cost", "w_cost", "p_cost");
        let session_id = SessionId::new("sess_cost");
        let run_id = cairn_domain::RunId::new("run_cost_zero");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id, None)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_cost_zero/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"], "run_cost_zero");
        assert_eq!(cost["total_cost_micros"], 0);
        assert_eq!(cost["total_tokens_in"], 0);
        assert_eq!(cost["total_tokens_out"], 0);
        assert_eq!(cost["provider_calls"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_returns_zeros_for_unknown_run() {
        // Unknown run → no cost record → zero response (not 404).
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/nonexistent_run_cost/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 0);
        assert_eq!(cost["provider_calls"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_reflects_run_cost_updated_events() {
        use cairn_domain::{
            events::RunCostUpdated, tenancy::OwnershipKey, EventEnvelope, EventId, EventSource,
            TenantId,
        };

        let state = make_state();
        let project = ProjectKey::new("t_rcu", "w_rcu", "p_rcu");
        let session_id = SessionId::new("sess_rcu");
        let run_id = cairn_domain::RunId::new("run_rcu");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        // Two provider calls: 300 + 200 micros, 50+30 tokens in, 20+10 tokens out.
        for (i, (cost, t_in, t_out)) in [(300u64, 50u64, 20u64), (200, 30, 10)].iter().enumerate() {
            state
                .runtime
                .store
                .append(&[EventEnvelope::new(
                    EventId::new(format!("evt_rcu_{i}")),
                    EventSource::Runtime,
                    OwnershipKey::Project(project.clone()),
                    cairn_domain::RuntimeEvent::RunCostUpdated(RunCostUpdated {
                        project: project.clone(),
                        run_id: run_id.clone(),
                        session_id: Some(session_id.clone()),
                        tenant_id: Some(TenantId::new("t_rcu")),
                        delta_cost_micros: *cost,
                        delta_tokens_in: *t_in,
                        delta_tokens_out: *t_out,
                        provider_call_id: format!("call_{i}"),
                        updated_at_ms: 1_000,
                    }),
                )])
                .await
                .unwrap();
        }

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_rcu/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"], "run_rcu");
        assert_eq!(cost["total_cost_micros"], 500, "300+200 micros");
        assert_eq!(cost["total_tokens_in"], 80, "50+30 tokens in");
        assert_eq!(cost["total_tokens_out"], 30, "20+10 tokens out");
        assert_eq!(cost["provider_calls"], 2, "2 provider calls");
    }

    #[tokio::test]
    async fn provider_connection_generate_roundtrip_invalidates_to_static_fallback() {
        let static_url = spawn_openai_compat_mock("static").await;
        let dynamic_url = spawn_openai_compat_mock("dynamic").await;

        let mut state = make_state();
        state.openai_compat_worker = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::default(),
                "static-key",
                Some(static_url),
                Some("gpt-4o-mini".to_owned()),
                None,
                None,
                None,
            )
            .expect("static fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_worker.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openai",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_dynamic",
                "provider_family": "openai",
                "adapter_type": "openai_compat",
                "supported_models": ["gpt-4o-mini"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/ollama/generate",
            serde_json::json!({
                "model": "gpt-4o-mini",
                "prompt": "hello from dynamic",
            }),
        )
        .await;
        let dynamic_status = dynamic_resp.status();
        let dynamic_body = to_bytes(dynamic_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            dynamic_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&dynamic_body)
        );
        let dynamic_json: serde_json::Value = serde_json::from_slice(&dynamic_body).unwrap();
        assert_eq!(dynamic_json["text"], "dynamic");

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_resp = authed_json(
            app,
            axum::http::Method::POST,
            "/v1/providers/ollama/generate",
            serde_json::json!({
                "model": "gpt-4o-mini",
                "prompt": "hello from fallback",
            }),
        )
        .await;
        let fallback_status = fallback_resp.status();
        let fallback_body = to_bytes(fallback_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            fallback_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&fallback_body)
        );
        let fallback_json: serde_json::Value = serde_json::from_slice(&fallback_body).unwrap();
        assert_eq!(fallback_json["text"], "static");
    }

    #[tokio::test]
    async fn provider_connection_embed_roundtrip_invalidates_to_static_fallback() {
        let static_url =
            spawn_openai_compat_embedding_mock("embed-dynamic", vec![0.9, 0.8], 11).await;
        let dynamic_url =
            spawn_openai_compat_embedding_mock("embed-dynamic", vec![0.1, 0.2], 7).await;

        let mut state = make_state();
        state.openai_compat_worker = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::default(),
                "static-key",
                Some(static_url),
                Some("embed-dynamic".to_owned()),
                None,
                None,
                None,
            )
            .expect("static embedding fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_worker.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openai",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_embed_dynamic",
                "provider_family": "openai",
                "adapter_type": "openai_compat",
                "supported_models": ["embed-dynamic"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/memory/embed",
            serde_json::json!({
                "model": "embed-dynamic",
                "texts": ["hello registry"],
            }),
        )
        .await;
        let dynamic_status = dynamic_resp.status();
        let dynamic_body = to_bytes(dynamic_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            dynamic_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&dynamic_body)
        );
        let dynamic_json: serde_json::Value = serde_json::from_slice(&dynamic_body).unwrap();
        assert_eq!(dynamic_json["model"], "embed-dynamic");
        assert_eq!(dynamic_json["token_count"], 7);
        assert_embedding_matches(&dynamic_json["embeddings"][0], &[0.1, 0.2]);

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_embed_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_resp = authed_json(
            app,
            axum::http::Method::POST,
            "/v1/memory/embed",
            serde_json::json!({
                "model": "embed-dynamic",
                "texts": ["hello fallback"],
            }),
        )
        .await;
        let fallback_status = fallback_resp.status();
        let fallback_body = to_bytes(fallback_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            fallback_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&fallback_body)
        );
        let fallback_json: serde_json::Value = serde_json::from_slice(&fallback_body).unwrap();
        assert_eq!(fallback_json["model"], "embed-dynamic");
        assert_eq!(fallback_json["token_count"], 11);
        assert_embedding_matches(&fallback_json["embeddings"][0], &[0.9, 0.8]);
    }

    #[tokio::test]
    async fn provider_connection_stream_roundtrip_invalidates_to_static_fallback() {
        let static_url = spawn_openai_compat_stream_mock(vec!["static stream"]).await;
        let dynamic_url = spawn_openai_compat_stream_mock(vec!["dynamic stream"]).await;

        let mut state = make_state();
        state.openai_compat_openrouter = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::OPENROUTER,
                "static-key",
                Some(static_url),
                Some("openrouter/free".to_owned()),
                None,
                None,
                None,
            )
            .expect("static stream fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_openrouter.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openrouter",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_stream_dynamic",
                "provider_family": "openrouter",
                "adapter_type": "openrouter",
                "supported_models": ["openrouter/free"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_sse = authed_sse_post(
            app.clone(),
            "/v1/chat/stream",
            serde_json::json!({
                "model": "openrouter/free",
                "prompt": "hello stream",
            }),
        )
        .await;
        assert!(dynamic_sse.contains("event: token"));
        assert!(dynamic_sse.contains("data:"));
        assert!(dynamic_sse.contains("dynamic stream"));
        assert!(dynamic_sse.contains("event: done"));

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_stream_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_sse = authed_sse_post(
            app,
            "/v1/chat/stream",
            serde_json::json!({
                "model": "openrouter/free",
                "prompt": "hello fallback stream",
            }),
        )
        .await;
        assert!(fallback_sse.contains("event: token"));
        assert!(fallback_sse.contains("data:"));
        assert!(fallback_sse.contains("static stream"));
        assert!(fallback_sse.contains("event: done"));
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_response_has_correct_shape() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/any_run/cost").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // All four contract fields must be present.
        for field in [
            "run_id",
            "total_cost_micros",
            "total_tokens_in",
            "total_tokens_out",
            "provider_calls",
        ] {
            assert!(cost.get(field).is_some(), "missing field: {field}");
        }
    }
}

#[cfg(test)]
mod run_events_tests {
    use super::test_make_app as make_app;
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const TOKEN: &str = "test-run-events-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-run-events".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "tenant_re",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/runs/:id/events returns 404 for an unknown run.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_unknown_run_returns_empty() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/runs/no_such_run/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            events.as_array().unwrap().is_empty(),
            "unknown run must return empty event list"
        );
    }

    /// GET /v1/runs/:id/events returns all events for the run after they are appended.
    ///
    /// Proves the write → project → read cycle for the run event stream:
    /// - POST /v1/events/append with RunCreated
    /// - GET /v1/runs/:id/events returns that event
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_returns_events_for_the_run() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_re");

        // Create a session and run directly in the store.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_re_1"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_re_1"),
                RunId::new("run_re_1"),
                None,
            )
            .await
            .unwrap();

        // GET /v1/runs/run_re_1/events must return at least the RunCreated event.
        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_re_1/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(
            !arr.is_empty(),
            "run events must not be empty after run is created"
        );

        // Every returned event must carry a position and event_type.
        for event in arr {
            assert!(
                event["position"].as_u64().is_some(),
                "event must have a position"
            );
            assert!(
                !event["event_type"].as_str().unwrap_or("").is_empty(),
                "event must have an event_type"
            );
        }

        // The RunCreated event must appear.
        let has_run_created = arr.iter().any(|e| e["event_type"] == "run_created");
        assert!(
            has_run_created,
            "run_created event must appear in the run event stream"
        );
    }

    /// Cursor-based pagination: after=<position> skips earlier events.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_cursor_pagination_works() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_pg");

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_pg"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_pg"),
                RunId::new("run_pg"),
                None,
            )
            .await
            .unwrap();

        let app1 = make_app(state.clone());
        let resp_all = app1
            .oneshot(authed_req("/v1/runs/run_pg/events"))
            .await
            .unwrap();
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(!all_arr.is_empty(), "must have events");

        let first_pos = all_arr[0]["position"].as_u64().unwrap();

        // After the first event's position, all remaining events are returned.
        let uri = format!("/v1/runs/run_pg/events?after={first_pos}");
        let app2 = make_app(state);
        let resp_page = app2.oneshot(authed_req(&uri)).await.unwrap();
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();

        assert_eq!(
            page_arr.len(),
            all_arr.len() - 1,
            "after=first_pos must skip the first event"
        );
        assert!(
            page_arr
                .iter()
                .all(|e| e["position"].as_u64().unwrap() > first_pos),
            "all paginated events must be after the cursor position"
        );
    }

    /// The run event stream is scoped to its run — events from other runs are excluded.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_are_run_scoped() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_sc");

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_sc"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_sc"),
                RunId::new("run_sc_a"),
                None,
            )
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_sc"),
                RunId::new("run_sc_b"),
                None,
            )
            .await
            .unwrap();

        // Events for run_sc_a must not include run_sc_b events.
        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_sc_a/events"))
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(!arr.is_empty(), "run_sc_a must have events");
        // All returned event_type values should be run-lifecycle types, not b's events.
        // Since event_type is derived from payload, just verify run_created is present once.
        let run_created_count = arr
            .iter()
            .filter(|e| e["event_type"] == "run_created")
            .count();
        assert_eq!(
            run_created_count, 1,
            "exactly one run_created must appear (for run_sc_a, not run_sc_b)"
        );
    }
}

#[cfg(test)]
mod tool_invocations_tests {
    use super::test_make_app as make_app;
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use cairn_domain::{
        policy::ExecutionClass, tool_invocation::ToolInvocationTarget, ProjectKey, RunId,
        SessionId, ToolInvocationId,
    };
    use cairn_runtime::ToolInvocationService as _;
    use tower::ServiceExt as _;

    const TOKEN: &str = "test-tool-inv-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-tool-inv".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "tenant_ti",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/runs/:id/tool-invocations returns empty for a run with no calls.
    #[tokio::test]
    async fn tool_invocations_empty_for_run_with_no_calls() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_ti_empty"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_ti_empty"),
                RunId::new("run_ti_empty"),
                None,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_ti_empty/tool-invocations"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            records.as_array().unwrap().is_empty(),
            "run with no tool calls must return empty list"
        );
    }

    /// GET /v1/runs/:id/tool-invocations returns both calls after they are recorded.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn tool_invocations_returns_two_calls_for_run() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_two");
        let sess = SessionId::new("sess_ti_two");

        state
            .runtime
            .sessions
            .create(&project, sess.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &sess, run.clone(), None)
            .await
            .unwrap();

        // Record two tool calls on the run.
        let target = ToolInvocationTarget::Builtin {
            tool_name: "read_file".to_owned(),
        };
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_1"),
                None,
                Some(run.clone()),
                None,
                target.clone(),
                ExecutionClass::SandboxedProcess,
            )
            .await
            .unwrap();
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_2"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin {
                    tool_name: "write_file".to_owned(),
                },
                ExecutionClass::SupervisedProcess,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_ti_two/tool-invocations"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = records.as_array().unwrap();

        assert_eq!(
            arr.len(),
            2,
            "run must have exactly 2 tool invocation records"
        );

        // Both invocation IDs must be present.
        let inv_ids: Vec<&str> = arr
            .iter()
            .map(|r| r["invocation_id"].as_str().unwrap_or(""))
            .collect();
        assert!(
            inv_ids.contains(&"inv_ti_1"),
            "inv_ti_1 must be in the response"
        );
        assert!(
            inv_ids.contains(&"inv_ti_2"),
            "inv_ti_2 must be in the response"
        );

        // Both are scoped to the run.
        for record in arr {
            assert_eq!(
                record["run_id"].as_str().unwrap_or(""),
                "run_ti_two",
                "all records must be for run_ti_two"
            );
        }
    }

    /// Outcome field reflects the terminal outcome after a call completes.
    ///
    /// Records start with state=requested/started and outcome=null;
    /// after ToolInvocationCompleted the state transitions and outcome is set.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn tool_invocation_outcome_field_reflects_completion() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_outcome");
        let sess = SessionId::new("sess_ti_outcome");

        state
            .runtime
            .sessions
            .create(&project, sess.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &sess, run.clone(), None)
            .await
            .unwrap();

        // Start a tool call.
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin {
                    tool_name: "bash".to_owned(),
                },
                ExecutionClass::SupervisedProcess,
            )
            .await
            .unwrap();

        // Before completion: outcome must be null, state is not terminal.
        let app1 = make_app(state.clone());
        let resp1 = app1
            .oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations"))
            .await
            .unwrap();
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let before: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        let before_rec = &before.as_array().unwrap()[0];
        assert!(
            before_rec["outcome"].is_null(),
            "outcome must be null before completion"
        );
        let before_state = before_rec["state"].as_str().unwrap_or("");
        assert!(!before_state.is_empty(), "state field must be present");

        // Complete the call with Success.
        state
            .runtime
            .tool_invocations
            .record_completed(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                "bash".to_owned(),
            )
            .await
            .unwrap();

        // After completion: outcome must be "success", state must be "completed".
        let app2 = make_app(state);
        let resp2 = app2
            .oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations"))
            .await
            .unwrap();
        let body2 = to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        let after: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        let after_rec = &after.as_array().unwrap()[0];

        let outcome = after_rec["outcome"].as_str().unwrap_or("<null>");
        assert_eq!(
            outcome, "success",
            "outcome must be 'success' after ToolInvocationCompleted"
        );
        assert_eq!(
            after_rec["state"].as_str().unwrap_or(""),
            "completed",
            "state must be 'completed' after successful completion"
        );
    }

    /// Tool invocations endpoint requires auth.
    #[tokio::test]
    async fn tool_invocations_requires_auth() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/any_run/tool-invocations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

#[cfg(test)]
mod provider_health_tests {
    use super::test_make_app as make_app;
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use cairn_domain::{
        events::{ProviderConnectionRegistered, ProviderHealthChecked},
        providers::{
            OperationKind, ProviderBindingSettings, ProviderConnectionStatus, ProviderHealthStatus,
        },
        tenancy::TenantKey,
        EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderConnectionId,
        ProviderModelId, RuntimeEvent, TenantId,
    };
    use tower::ServiceExt as _;

    const TOKEN: &str = "test-ph-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-ph".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new("t_ph")),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/providers/health returns empty when no providers are registered.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_empty_with_no_providers() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            health.as_array().unwrap().is_empty(),
            "no providers registered — health list must be empty"
        );
    }

    /// After a healthy check, the health entry shows healthy=true and correct fields.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_shows_healthy_after_health_check() {
        use cairn_domain::events::ProviderBindingCreated;

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph");

        // Register connection + binding (needed to derive tenant for health query).
        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph_conn"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
                        tenant: TenantKey::new("t_ph"),
                        provider_connection_id: ProviderConnectionId::new("conn_ph_1"),
                        provider_family: "openai".to_owned(),
                        adapter_type: "responses".to_owned(),
                        supported_models: vec![],
                        status: ProviderConnectionStatus::Active,
                        registered_at: 1_000,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph_bind"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                        project: project.clone(),
                        provider_binding_id: ProviderBindingId::new("conn_ph_1"),
                        provider_connection_id: ProviderConnectionId::new("conn_ph_1"),
                        provider_model_id: ProviderModelId::new(
                            state.runtime.runtime_config.default_generate_model().await,
                        ),
                        operation_kind: OperationKind::Generate,
                        settings: ProviderBindingSettings::default(),
                        policy_id: None,
                        active: true,
                        created_at: 1_000,
                        estimated_cost_micros: None,
                    }),
                ),
                // Healthy check.
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph_check"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderHealthChecked(ProviderHealthChecked {
                        tenant_id: TenantId::new("t_ph"),
                        connection_id: ProviderConnectionId::new("conn_ph_1"),
                        status: ProviderHealthStatus::Healthy,
                        latency_ms: Some(95),
                        checked_at_ms: 5_000,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry must appear");
        assert_eq!(arr[0]["connection_id"], "conn_ph_1");
        assert_eq!(
            arr[0]["healthy"], true,
            "must be healthy after health check"
        );
        assert_eq!(arr[0]["consecutive_failures"], 0);
        assert_eq!(arr[0]["last_checked_at"], 5_000);
        // Status serializes to lowercase.
        assert!(
            !arr[0]["status"].as_str().unwrap_or("").is_empty(),
            "status must be set"
        );
    }

    /// After ProviderMarkedDegraded, the health entry reflects degraded status.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_shows_degraded_after_mark_degraded() {
        use cairn_domain::events::{ProviderBindingCreated, ProviderMarkedDegraded};

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph2");

        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph2_bind"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                        project: project.clone(),
                        provider_binding_id: ProviderBindingId::new("conn_ph_deg"),
                        provider_connection_id: ProviderConnectionId::new("conn_ph_deg"),
                        provider_model_id: ProviderModelId::new(
                            state.runtime.runtime_config.default_generate_model().await,
                        ),
                        operation_kind: OperationKind::Generate,
                        settings: ProviderBindingSettings::default(),
                        policy_id: None,
                        active: true,
                        created_at: 1_000,
                        estimated_cost_micros: None,
                    }),
                ),
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph2_degrade"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderMarkedDegraded(ProviderMarkedDegraded {
                        tenant_id: TenantId::new("t_ph"),
                        connection_id: ProviderConnectionId::new("conn_ph_deg"),
                        reason: "upstream latency exceeded threshold".to_owned(),
                        marked_at_ms: 8_000,
                    }),
                ),
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry");
        assert_eq!(
            arr[0]["healthy"], false,
            "must be unhealthy after degraded mark"
        );
        assert!(
            arr[0]["error_message"]
                .as_str()
                .is_some_and(|e| e.contains("latency")),
            "error_message must contain the degradation reason"
        );
        assert_eq!(arr[0]["last_checked_at"], 8_000);
    }

    /// GET /v1/providers/health requires auth.
    #[tokio::test]
    async fn provider_health_requires_auth() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/providers/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
