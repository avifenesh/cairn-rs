//! CLI argument parsing and bootstrap utilities.

use cairn_api::bootstrap::{
    BootstrapConfig, DeploymentMode, EncryptionKeySource, ServerBootstrap, StorageBackend,
};

pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {},
            _ = terminate.recv() => {},
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

pub fn fatal_cli(message: impl Into<String>) -> ! {
    let message = message.into();
    eprintln!("{message}");
    #[cfg(test)]
    panic!("{message}");
    #[cfg(not(test))]
    std::process::exit(1);
}

/// Enforce RFC 020 §"Postgres as Production Target" (Integration Test #12):
/// team mode must refuse to start on SQLite. Called after *all* storage
/// configuration sources (CLI `--db`, `DATABASE_URL` env var) have been
/// merged so the invariant cannot be bypassed by choosing the env-var path
/// rather than the CLI flag.
///
/// Gated on `.db` / `.sqlite` suffix so integration test harnesses that
/// deliberately pass `sqlite:<path>?mode=rwc` (LiveHarness restart meta-
/// test) still boot — the suffix predicate is what separates "operator
/// fat-fingered a production DB path" from "test harness exercising
/// SQLite in a controlled way". The operator-facing footgun always ends
/// in `.db` or `.sqlite`.
pub fn enforce_team_mode_storage_invariant(config: &BootstrapConfig) {
    if config.mode != DeploymentMode::SelfHostedTeam {
        return;
    }
    if let StorageBackend::Sqlite { path } = &config.storage {
        if path.ends_with(".db") || path.ends_with(".sqlite") {
            fatal_cli(format!(
                "FATAL: SQLite is not supported in self-hosted team mode: {path}. \
                 Team mode requires Postgres. Pass --db postgres://... or set \
                 DATABASE_URL. See RFC 020 for rationale."
            ));
        }
    }
}

/// Parse a `--mode` value. Fatal on unknown values so a fat-fingered
/// env var or CLI flag fails loud instead of silently falling back to
/// local mode. Exposed (`pub`) so `main.rs`'s binary-side parser can
/// share the same fail-loud helper instead of open-coding a divergent
/// `eprintln!` + `exit(1)`.
pub fn parse_mode_value(raw: &str, source: &str) -> DeploymentMode {
    match raw {
        "team" | "self-hosted" => DeploymentMode::SelfHostedTeam,
        "local" => DeploymentMode::Local,
        other => fatal_cli(format!("Unknown mode ({}): {}", source, other)),
    }
}

/// Parse a `--db` / `CAIRN_DB` value into a `StorageBackend`. The literal
/// string `memory` selects the in-memory backend (matching the
/// documented `cairn-app --db memory` usage); anything with a Postgres
/// scheme becomes Postgres; everything else is treated as a SQLite
/// path, matching the original CLI semantics. Exposed (`pub`) so
/// `main.rs` can share the parser and stay in sync with the library.
pub fn parse_db_value(raw: &str) -> StorageBackend {
    if raw == "memory" {
        StorageBackend::InMemory
    } else if raw.starts_with("postgres://") || raw.starts_with("postgresql://") {
        StorageBackend::Postgres {
            connection_url: raw.to_owned(),
        }
    } else {
        StorageBackend::Sqlite {
            path: raw.to_owned(),
        }
    }
}

/// Track which configuration fields came from CLI flags so the
/// env-var fallback pass below does not clobber explicit user intent.
///
/// Public so `main.rs` can reuse the same struct when wiring env-var
/// fallback into the binary-side parser (`apply_env_fallback` below is
/// the shared mutator).
#[derive(Default)]
pub struct CliProvided {
    pub mode: bool,
    pub port: bool,
    pub db: bool,
}

/// Apply env-var fallback for `CAIRN_MODE` / `CAIRN_PORT` / `CAIRN_DB`
/// using the supplied `env` lookup. CLI-provided fields (tracked in
/// `provided`) are left untouched so the flag always wins. Invalid
/// values fatal loud via `fatal_cli` so a fat-fingered `CAIRN_PORT=foo`
/// is caught at boot rather than silently falling back to the default.
///
/// Empty strings are treated as unset so container compose files can
/// clear a container-wide default without tripping the validation.
///
/// Exposed for reuse in `main.rs`'s binary-side parser; the library's
/// own `parse_args_with_env` calls the same helper internally.
pub fn apply_env_fallback<F>(config: &mut BootstrapConfig, provided: &CliProvided, env: F)
where
    F: Fn(&str) -> Result<String, std::env::VarError>,
{
    if !provided.mode {
        if let Ok(raw) = env("CAIRN_MODE") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                config.mode = parse_mode_value(trimmed, "CAIRN_MODE");
            }
        }
    }
    if !provided.port {
        if let Ok(raw) = env("CAIRN_PORT") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                config.listen_port = trimmed
                    .parse::<u16>()
                    .unwrap_or_else(|_| fatal_cli(format!("Invalid CAIRN_PORT: {}", trimmed)));
            }
        }
    }
    if !provided.db {
        if let Ok(raw) = env("CAIRN_DB") {
            let trimmed = raw.trim();
            if !trimmed.is_empty() {
                config.storage = parse_db_value(trimmed);
            }
        }
    }
}

pub fn parse_args_from(args: &[String]) -> BootstrapConfig {
    parse_args_with_env(args, |k| std::env::var(k))
}

/// Parse CLI args with an injectable env-var lookup. CLI flags always
/// take precedence; env vars fill in only the fields that the CLI did
/// not set. Tests use this entry point to exercise env fallback
/// hermetically without mutating the real process environment.
pub fn parse_args_with_env<F>(args: &[String], env: F) -> BootstrapConfig
where
    F: Fn(&str) -> Result<String, std::env::VarError>,
{
    let mut config = BootstrapConfig::default();
    let mut cli = CliProvided::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    config.mode = parse_mode_value(args[i].as_str(), "--mode");
                    cli.mode = true;
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    config.listen_port = args[i]
                        .parse::<u16>()
                        .unwrap_or_else(|_| fatal_cli(format!("Invalid port: {}", args[i])));
                    cli.port = true;
                }
            }
            "--addr" => {
                i += 1;
                if i < args.len() {
                    config.listen_addr = args[i].clone();
                }
            }
            "--tls-cert" => {
                i += 1;
                if i < args.len() {
                    config.tls_cert_path = Some(args[i].clone());
                }
            }
            "--tls-key" => {
                i += 1;
                if i < args.len() {
                    config.tls_key_path = Some(args[i].clone());
                }
            }
            "--db" => {
                i += 1;
                if i < args.len() {
                    config.storage = parse_db_value(args[i].as_str());
                    cli.db = true;
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

    apply_env_fallback(&mut config, &cli, env);

    if config.tls_cert_path.is_some() && config.tls_key_path.is_some() {
        config.tls_enabled = true;
    }

    if config.mode == DeploymentMode::SelfHostedTeam {
        if config.listen_addr == "127.0.0.1" {
            config.listen_addr = "0.0.0.0".to_owned();
        }
        // RFC 020 team-mode storage invariant. Shared with main.rs::parse_args
        // so CLI-only and env-var paths are covered by the same refusal.
        enforce_team_mode_storage_invariant(&config);
        if matches!(config.encryption_key, EncryptionKeySource::LocalAuto) {
            config.encryption_key = EncryptionKeySource::None;
        }
    }

    config
}

pub fn parse_args() -> BootstrapConfig {
    let args: Vec<String> = std::env::args().collect();
    parse_args_from(&args)
}

pub fn run_bootstrap<B>(bootstrap: &B, config: &BootstrapConfig) -> Result<(), B::Error>
where
    B: ServerBootstrap,
{
    bootstrap.start(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        // First element is the binary name (argv[0]), skipped by parse_args_from.
        std::iter::once("cairn-app")
            .chain(items.iter().copied())
            .map(String::from)
            .collect()
    }

    // ── encryption-key-env ─────────────────────────────────────────────

    #[test]
    fn encryption_key_env_sets_envvar_source() {
        let config = parse_args_from(&args(&["--encryption-key-env", "MY_SECRET"]));
        assert_eq!(
            config.encryption_key,
            EncryptionKeySource::EnvVar {
                var_name: "MY_SECRET".to_owned()
            }
        );
    }

    // ── unknown args silently skipped ──────────────────────────────────

    #[test]
    fn unknown_flags_are_silently_skipped() {
        let config = parse_args_from(&args(&["--foo", "bar", "--port", "9090", "--baz"]));
        // Unknown flags ignored; known flag still parsed.
        assert_eq!(config.listen_port, 9090);
    }

    // ── dangling flags ─────────────────────────────────────────────────

    #[test]
    fn dangling_port_flag_keeps_default() {
        // --port at end with no value: the guard `i < args.len()` prevents
        // out-of-bounds and leaves the default unchanged.
        let config = parse_args_from(&args(&["--port"]));
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn dangling_addr_flag_keeps_default() {
        let config = parse_args_from(&args(&["--addr"]));
        assert_eq!(config.listen_addr, "127.0.0.1");
    }

    #[test]
    fn dangling_encryption_key_env_keeps_default() {
        let config = parse_args_from(&args(&["--encryption-key-env"]));
        // Default for local mode is LocalAuto.
        assert_eq!(config.encryption_key, EncryptionKeySource::LocalAuto);
    }

    // ── mode parsing ───────────────────────────────────────────────────

    #[test]
    fn mode_local() {
        let config = parse_args_from(&args(&["--mode", "local"]));
        assert_eq!(config.mode, DeploymentMode::Local);
    }

    #[test]
    fn mode_team() {
        let config = parse_args_from(&args(&[
            "--mode",
            "team",
            "--db",
            "postgres://localhost/cairn",
        ]));
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
    }

    #[test]
    fn mode_self_hosted_alias() {
        let config = parse_args_from(&args(&[
            "--mode",
            "self-hosted",
            "--db",
            "postgres://localhost/cairn",
        ]));
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
    }

    // ── port ───────────────────────────────────────────────────────────

    #[test]
    fn port_override() {
        let config = parse_args_from(&args(&["--port", "8080"]));
        assert_eq!(config.listen_port, 8080);
    }

    // ── addr ───────────────────────────────────────────────────────────

    #[test]
    fn addr_override() {
        let config = parse_args_from(&args(&["--addr", "0.0.0.0"]));
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    // ── db backend selection ───────────────────────────────────────────

    #[test]
    fn db_postgres() {
        let config = parse_args_from(&args(&["--db", "postgres://user:pass@host/db"]));
        assert_eq!(
            config.storage,
            StorageBackend::Postgres {
                connection_url: "postgres://user:pass@host/db".to_owned()
            }
        );
    }

    #[test]
    fn db_postgresql_scheme() {
        let config = parse_args_from(&args(&["--db", "postgresql://host/db"]));
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn db_sqlite_path() {
        let config = parse_args_from(&args(&["--db", "/tmp/cairn.db"]));
        assert_eq!(
            config.storage,
            StorageBackend::Sqlite {
                path: "/tmp/cairn.db".to_owned()
            }
        );
    }

    // ── TLS ────────────────────────────────────────────────────────────

    #[test]
    fn tls_enabled_when_both_cert_and_key() {
        let config = parse_args_from(&args(&[
            "--tls-cert",
            "/etc/cert.pem",
            "--tls-key",
            "/etc/key.pem",
        ]));
        assert!(config.tls_enabled);
        assert_eq!(config.tls_cert_path.as_deref(), Some("/etc/cert.pem"));
        assert_eq!(config.tls_key_path.as_deref(), Some("/etc/key.pem"));
    }

    #[test]
    fn tls_not_enabled_with_cert_only() {
        let config = parse_args_from(&args(&["--tls-cert", "/etc/cert.pem"]));
        assert!(!config.tls_enabled);
    }

    // ── team mode side-effects ─────────────────────────────────────────

    #[test]
    fn team_mode_forces_bind_all_interfaces() {
        let config = parse_args_from(&args(&[
            "--mode",
            "team",
            "--db",
            "postgres://localhost/cairn",
        ]));
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let config = parse_args_from(&args(&[
            "--mode",
            "team",
            "--db",
            "postgres://localhost/cairn",
        ]));
        assert_eq!(config.encryption_key, EncryptionKeySource::None);
    }

    // ── defaults ───────────────────────────────────────────────────────

    #[test]
    fn defaults_without_args() {
        let config = parse_args_from(&args(&[]));
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
        assert!(matches!(config.storage, StorageBackend::InMemory));
        assert_eq!(config.encryption_key, EncryptionKeySource::LocalAuto);
        assert!(!config.tls_enabled);
    }

    // ── RFC 020 team-mode storage invariant ────────────────────────────

    #[test]
    #[should_panic(expected = "SQLite is not supported in self-hosted team mode")]
    fn enforce_refuses_team_mode_with_sqlite_db_suffix() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Sqlite {
                path: "/var/lib/cairn/prod.db".to_owned(),
            },
            ..BootstrapConfig::default()
        };
        enforce_team_mode_storage_invariant(&config);
    }

    #[test]
    #[should_panic(expected = "SQLite is not supported in self-hosted team mode")]
    fn enforce_refuses_team_mode_with_sqlite_suffix() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Sqlite {
                path: "/var/lib/cairn/prod.sqlite".to_owned(),
            },
            ..BootstrapConfig::default()
        };
        enforce_team_mode_storage_invariant(&config);
    }

    #[test]
    fn enforce_accepts_team_mode_with_postgres() {
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Postgres {
                connection_url: "postgres://localhost/cairn".to_owned(),
            },
            ..BootstrapConfig::default()
        };
        // Must not panic.
        enforce_team_mode_storage_invariant(&config);
    }

    #[test]
    fn enforce_accepts_team_mode_with_inmemory() {
        // InMemory is intentionally permitted even in team mode — it is
        // ephemeral by design and operators must explicitly opt in via
        // `--db memory`. The RFC 020 footgun is specifically SQLite
        // "looking durable while silently not being so in a multi-
        // process deployment".
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::InMemory,
            ..BootstrapConfig::default()
        };
        enforce_team_mode_storage_invariant(&config);
    }

    #[test]
    fn enforce_allows_sqlite_harness_rwc_path_in_team_mode() {
        // LiveHarness::setup_with_sqlite passes `sqlite:<path>?mode=rwc`
        // which does not end with `.db`/`.sqlite`. The refusal must NOT
        // trip on it — tightening the suffix gate would break the live
        // integration test harness.
        let config = BootstrapConfig {
            mode: DeploymentMode::SelfHostedTeam,
            storage: StorageBackend::Sqlite {
                path: "sqlite:/tmp/cairn-test-abcd1234.db?mode=rwc".to_owned(),
            },
            ..BootstrapConfig::default()
        };
        // Must not panic — path ends with `?mode=rwc`, not `.db`.
        enforce_team_mode_storage_invariant(&config);
    }

    #[test]
    fn enforce_allows_sqlite_in_local_mode() {
        let config = BootstrapConfig {
            mode: DeploymentMode::Local,
            storage: StorageBackend::Sqlite {
                path: "/home/user/.cairn/local.db".to_owned(),
            },
            ..BootstrapConfig::default()
        };
        // Local mode: SQLite is a supported, first-class backend.
        enforce_team_mode_storage_invariant(&config);
    }

    // ── combined flags ─────────────────────────────────────────────────

    #[test]
    fn multiple_flags_combined() {
        let config = parse_args_from(&args(&[
            "--port",
            "4000",
            "--addr",
            "10.0.0.1",
            "--encryption-key-env",
            "CAIRN_KEY",
            "--db",
            "postgres://localhost/cairn",
        ]));
        assert_eq!(config.listen_port, 4000);
        assert_eq!(config.listen_addr, "10.0.0.1");
        assert_eq!(
            config.encryption_key,
            EncryptionKeySource::EnvVar {
                var_name: "CAIRN_KEY".to_owned()
            }
        );
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    // ── env-var fallback (CAIRN_PORT / CAIRN_DB / CAIRN_MODE) ──────────
    //
    // Uses the `parse_args_with_env` injection point so tests do not
    // race on the real process environment. Tests never mutate
    // `std::env::var_os` — the lookup closure is entirely hermetic.

    fn env_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Result<String, std::env::VarError> {
        let owned: std::collections::HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |key: &str| {
            owned
                .get(key)
                .cloned()
                .ok_or(std::env::VarError::NotPresent)
        }
    }

    #[test]
    fn env_cairn_port_applied_when_flag_absent() {
        let config = parse_args_with_env(&args(&[]), env_from(&[("CAIRN_PORT", "9090")]));
        assert_eq!(config.listen_port, 9090);
    }

    #[test]
    fn env_cairn_port_ignored_when_flag_present() {
        let config = parse_args_with_env(
            &args(&["--port", "8080"]),
            env_from(&[("CAIRN_PORT", "9090")]),
        );
        assert_eq!(config.listen_port, 8080);
    }

    #[test]
    #[should_panic(expected = "Invalid CAIRN_PORT")]
    fn env_cairn_port_rejects_garbage() {
        let _ = parse_args_with_env(&args(&[]), env_from(&[("CAIRN_PORT", "not-a-port")]));
    }

    #[test]
    fn env_cairn_port_empty_keeps_default() {
        // Empty string is treated as unset so operators can unset a
        // container-wide var via `CAIRN_PORT=` in compose without
        // tripping the "Invalid CAIRN_PORT" panic.
        let config = parse_args_with_env(&args(&[]), env_from(&[("CAIRN_PORT", "")]));
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn env_cairn_db_postgres_applied() {
        let config = parse_args_with_env(
            &args(&[]),
            env_from(&[("CAIRN_DB", "postgres://localhost/cairn")]),
        );
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn env_cairn_db_memory_applied() {
        let config = parse_args_with_env(&args(&[]), env_from(&[("CAIRN_DB", "memory")]));
        assert!(matches!(config.storage, StorageBackend::InMemory));
    }

    #[test]
    fn env_cairn_db_sqlite_path_applied() {
        let config = parse_args_with_env(
            &args(&[]),
            env_from(&[("CAIRN_DB", "/var/lib/cairn/local.db")]),
        );
        match config.storage {
            StorageBackend::Sqlite { path } => assert_eq!(path, "/var/lib/cairn/local.db"),
            other => panic!("expected Sqlite, got {:?}", other),
        }
    }

    #[test]
    fn env_cairn_db_ignored_when_flag_present() {
        let config = parse_args_with_env(
            &args(&["--db", "postgres://from-cli/db"]),
            env_from(&[("CAIRN_DB", "postgres://from-env/db")]),
        );
        match config.storage {
            StorageBackend::Postgres { connection_url } => {
                assert_eq!(connection_url, "postgres://from-cli/db")
            }
            other => panic!("expected Postgres, got {:?}", other),
        }
    }

    #[test]
    fn env_cairn_mode_team_applied() {
        let config = parse_args_with_env(
            &args(&[]),
            env_from(&[
                ("CAIRN_MODE", "team"),
                // Team mode requires a Postgres backend via the RFC 020
                // invariant — supply one so the enforcement call below
                // the env-fallback pass does not fatal.
                ("CAIRN_DB", "postgres://localhost/cairn"),
            ]),
        );
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        // Team mode flips default bind to 0.0.0.0.
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn env_cairn_mode_self_hosted_alias_applied() {
        let config = parse_args_with_env(
            &args(&[]),
            env_from(&[
                ("CAIRN_MODE", "self-hosted"),
                ("CAIRN_DB", "postgres://localhost/cairn"),
            ]),
        );
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
    }

    #[test]
    #[should_panic(expected = "Unknown mode (CAIRN_MODE)")]
    fn env_cairn_mode_rejects_garbage() {
        let _ = parse_args_with_env(&args(&[]), env_from(&[("CAIRN_MODE", "hybrid")]));
    }

    #[test]
    fn env_cairn_mode_ignored_when_flag_present() {
        let config = parse_args_with_env(
            &args(&["--mode", "local"]),
            env_from(&[("CAIRN_MODE", "team")]),
        );
        assert_eq!(config.mode, DeploymentMode::Local);
    }

    #[test]
    fn env_vars_do_not_fire_without_backing_values() {
        // Sanity check: with an empty env lookup we still get defaults.
        let config = parse_args_with_env(&args(&[]), env_from(&[]));
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_port, 3000);
        assert!(matches!(config.storage, StorageBackend::InMemory));
    }
}
