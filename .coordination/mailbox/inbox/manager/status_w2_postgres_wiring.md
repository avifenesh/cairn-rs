# STATUS: postgres_wiring

**Task:** Wire --db postgres flag to actual PgStore connection  
**Build:** CLEAN — 0 errors

## Changes

### `crates/cairn-app/Cargo.toml`
- Added `features = ["postgres"]` to `cairn-store` dependency
- Added `sqlx = { version = "0.8", features = ["runtime-tokio", "postgres"] }`

### `crates/cairn-app/src/main.rs`

**New imports:** `cairn_store::{DbAdapter, pg::{PgAdapter, PgEventLog, PgMigrationRunner}}`, `sqlx::postgres::PgPoolOptions`

**New types:**
- `PgBackend { event_log: Arc<PgEventLog>, adapter: Arc<PgAdapter> }`
- `AppState.pg: Option<Arc<PgBackend>>`

**`main()` startup block (when `StorageBackend::Postgres`):**
1. `PgPoolOptions::new().connect(url).await` — abort on failure
2. `PgMigrationRunner::run_pending()` — applies schema migrations, aborts on failure
3. Creates `PgBackend`, sets `AppState.pg = Some(...)`

**Handler changes:**
- `status_handler`: checks `pg.adapter.health_check()` when Pg configured
- `list_events_handler`: uses `pg.event_log.read_stream()` for durable replay when Pg present
- `append_events_handler`: **dual-write** — appends to Pg (durability) AND InMemory (read models + SSE broadcast)

**New route:** `GET /v1/db/status` — reports backend ("postgres"/"in_memory"), connectivity, migration count, schema currency

## Usage
```bash
cargo run -p cairn-app -- --db postgres://user:pass@localhost/cairn
# → connects, runs migrations, dual-writes events to Postgres + InMemory
```
