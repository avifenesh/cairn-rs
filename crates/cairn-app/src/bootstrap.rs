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
