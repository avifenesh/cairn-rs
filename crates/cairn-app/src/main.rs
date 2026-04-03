//! Bootstrap binary for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces
//!
mod sse_hooks;

use cairn_api::bootstrap::{
    BootstrapConfig, DeploymentMode, EncryptionKeySource, ServerBootstrap, StorageBackend,
};

struct AppBootstrap;

impl ServerBootstrap for AppBootstrap {
    type Error = String;

    fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error> {
        Err(format!(
            "bootstrap blocked: server composition is still missing for {:?} at {}:{}",
            config.mode, config.listen_addr, config.listen_port
        ))
    }
}

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
                    if val.starts_with("postgres://") || val.starts_with("postgresql://") {
                        config.storage = StorageBackend::Postgres {
                            connection_url: val.clone(),
                        };
                    } else {
                        config.storage = StorageBackend::Sqlite {
                            path: val.clone(),
                        };
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

    if config.mode == DeploymentMode::SelfHostedTeam {
        if config.listen_addr == "127.0.0.1" {
            config.listen_addr = "0.0.0.0".to_owned();
        }
        // Team mode uses LocalAuto only if no explicit key — credentials_available() will reject it.
        if matches!(config.encryption_key, EncryptionKeySource::LocalAuto) {
            config.encryption_key = EncryptionKeySource::None;
        }
    }

    config
}

fn parse_args() -> BootstrapConfig {
    let args: Vec<String> = std::env::args().collect();
    parse_args_from(&args)
}

fn run_bootstrap<B>(bootstrap: &B, config: &BootstrapConfig) -> Result<(), B::Error>
where
    B: ServerBootstrap,
{
    bootstrap.start(config)
}

fn main() {
    let config = parse_args();
    let bootstrap = AppBootstrap;

    if let Err(err) = run_bootstrap(&bootstrap, &config) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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

    #[test]
    fn parse_args_defaults_to_local_mode() {
        let args = vec!["cairn-app".to_owned()];
        let config = parse_args_from(&args);

        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn parse_args_promotes_team_mode_to_public_bind() {
        let args = vec![
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
        ];
        let config = parse_args_from(&args);

        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn run_bootstrap_delegates_to_server_bootstrap() {
        let bootstrap = RecordingBootstrap::new();
        let config = BootstrapConfig::team("postgres://localhost/cairn");

        run_bootstrap(&bootstrap, &config).unwrap();

        assert_eq!(bootstrap.seen(), Some(config));
    }

    #[test]
    fn parse_args_db_flag_sets_postgres() {
        let args = vec![
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(matches!(config.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn parse_args_db_flag_sets_sqlite() {
        let args = vec![
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my_data.db".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(matches!(config.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let args = vec![
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
        ];
        let config = parse_args_from(&args);
        assert!(!config.credentials_available());
    }

    #[test]
    fn app_bootstrap_reports_explicit_blocker() {
        let bootstrap = AppBootstrap;
        let config = BootstrapConfig::default();

        let err = bootstrap.start(&config).unwrap_err();
        assert!(err.contains("server composition is still missing"));
    }
}
