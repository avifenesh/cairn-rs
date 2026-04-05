# Status Update — Worker Core

## Task: docs/deployment.md
- **Files created**: docs/deployment.md
- **Files changed**: none
- **Issues**: none
- **Notable**:
  - TLS is documented with --tls-cert and --tls-key flags (confirmed from cairn-app/src/lib.rs:13689/13695)
  - Noted that --mode team REQUIRES TLS (cairn-app/src/lib.rs:3704 check)
  - CAIRN_PORT and CAIRN_MODE env vars are documentation-only (app uses CLI flags --port and --mode)
  - Systemd unit uses EnvironmentFile for CAIRN_ADMIN_TOKEN to keep secret off command line
  - NoNewPrivileges/ProtectSystem/PrivateTmp hardening directives included
  - Upgrade section notes that migrations run automatically
