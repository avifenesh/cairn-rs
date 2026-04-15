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

fn fatal_cli(message: impl Into<String>) -> ! {
    let message = message.into();
    eprintln!("{message}");
    #[cfg(test)]
    panic!("{message}");
    #[cfg(not(test))]
    std::process::exit(1);
}

pub fn parse_args_from(args: &[String]) -> BootstrapConfig {
    let mut config = BootstrapConfig::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    config.mode = match args[i].as_str() {
                        "team" | "self-hosted" => DeploymentMode::SelfHostedTeam,
                        "local" => DeploymentMode::Local,
                        s => fatal_cli(format!("Unknown mode: {}", s)),
                    };
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    config.listen_port = args[i]
                        .parse::<u16>()
                        .unwrap_or_else(|_| fatal_cli(format!("Invalid port: {}", args[i])));
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
                    let val = &args[i];
                    if val.starts_with("postgres://") || val.starts_with("postgresql://") {
                        config.storage = StorageBackend::Postgres {
                            connection_url: val.clone(),
                        };
                    } else {
                        config.storage = StorageBackend::Sqlite { path: val.clone() };
                    }
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

    if config.tls_cert_path.is_some() && config.tls_key_path.is_some() {
        config.tls_enabled = true;
    }

    if config.mode == DeploymentMode::SelfHostedTeam {
        if config.listen_addr == "127.0.0.1" {
            config.listen_addr = "0.0.0.0".to_owned();
        }
        if let StorageBackend::Sqlite { path } = &config.storage {
            if path.ends_with(".sqlite") || path.ends_with(".db") {
                fatal_cli(format!(
                    "SQLite is not supported in self-hosted team mode: {}",
                    path
                ));
            }
        }
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
}
