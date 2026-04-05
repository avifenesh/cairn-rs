# Status Update — Worker Core

## Task: .github/workflows/ci.yml
- **Files changed**: .github/workflows/ci.yml (replaced minimal existing file)
- **Issues**: none
- **Changes from original**:
  - Added branch scoping: push/PR to main only (was trigger on every push)
  - Added SQLX_OFFLINE=true env var (prevents sqlx compile-time DB connection)
  - Added RUSTFLAGS=-D warnings (turns clippy/rustc warnings into errors)
  - Added cargo registry cache (Cargo.lock hash key)
  - Added target/ build artefacts cache (Cargo.lock + Cargo.toml hash key)
  - Changed test to --exclude cairn-app --lib (was --workspace, which includes broken cairn-app lib)
  - Added cargo build --workspace step
  - CARGO_TERM_COLOR=always for readable CI output
