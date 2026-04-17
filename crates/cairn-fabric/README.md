# cairn-fabric

Cairn ↔ FlowFabric integration layer. Bridges Cairn's event-sourced runtime to
FlowFabric's Valkey-native execution engine (active tasks, aggregates, state
map, signal routing, suspension, budgets).

- Library crate: `src/` — services, bridges, boot/runtime wiring
- Unit tests: `cargo test -p cairn-fabric --lib` (205 tests, no Valkey required)
- Live integration tests: `cargo test -p cairn-fabric --test integration -- --ignored` (require Valkey + flowfabric library loaded)

## Running the live integration suite

The recommended entry point is the bundled runner script, which boots a
disposable Valkey container, builds the FlowFabric Lua library, loads it via
`FUNCTION LOAD REPLACE`, and executes the `--ignored` tests end-to-end.

```bash
./scripts/run-fabric-integration-tests.sh
```

### Flags

| Flag             | Effect                                                              |
|------------------|---------------------------------------------------------------------|
| `--ci`           | Fail-fast, quieter cargo output, non-zero exit on any test failure. |
| `--keep-valkey`  | Leave the container running after the script exits (for debugging). |
| `-h`, `--help`   | Print usage and exit.                                               |

### Environment variables

| Variable                | Default                                                | Purpose                                                  |
|-------------------------|--------------------------------------------------------|----------------------------------------------------------|
| `CAIRN_TEST_VALKEY_URL` | `redis://localhost:6379`                              | Connection URL consumed by `TestHarness`.                |
| `VALKEY_IMAGE`          | `valkey/valkey:8-alpine`                               | Docker image.                                            |
| `VALKEY_PORT`           | `6379`                                                 | Host port mapped to container `6379`.                    |
| `VALKEY_CONTAINER`      | `cairn-fabric-integ-valkey`                            | Container name (idempotent reuse).                       |
| `FF_PATH`               | `/tmp/FlowFabric`                                      | FlowFabric checkout location (script manages the clone). |
| `FF_BRANCH`             | `feat/execution-engine`                                | Branch to fetch before checkout.                         |
| `FF_REPO`               | `https://github.com/avifenesh/FlowFabric.git`          | Clone URL when `FF_PATH` is missing.                     |
| `FF_REV`                | pinned SHA (see script)                                | **Must match** `rev` in `crates/cairn-fabric/Cargo.toml`. Lua bundle loaded into Valkey drifts from Rust-side FCALL signatures otherwise. |

### What the script does

1. **Pre-flight** — checks `docker`, `cargo`, `git`, and a reachable docker daemon.
2. **FF checkout** — clones `FF_REPO` to `FF_PATH` if missing, then `git fetch` + `git checkout --detach $FF_REV` so Lua-side and Rust-side are on the exact same commit.
3. **Valkey** — starts `VALKEY_IMAGE` on `VALKEY_PORT`, reusing an existing container if already running.
4. **Build** — `cargo build --release -p ff-script` in the FF checkout (the crate's `build.rs` concatenates `lua/*.lua` into a single bundled `flowfabric.lua` under `target/release/build/ff-script-*/out/`).
5. **Load library** — `docker cp` the bundle into the container, then `valkey-cli -x FUNCTION LOAD REPLACE` and verify with `FCALL ff_version 0`.
6. **Run tests** — `cargo test -p cairn-fabric --test integration -- --ignored --nocapture` with `CAIRN_TEST_VALKEY_URL` exported.
7. **Cleanup** — trap removes the container on exit unless `--keep-valkey` was passed.

### Running a single test

```bash
./scripts/run-fabric-integration-tests.sh --keep-valkey  # one-time setup
CAIRN_TEST_VALKEY_URL=redis://localhost:6379 \
  cargo test -p cairn-fabric --test integration test_create_and_read_run -- --ignored --nocapture
```

## Troubleshooting

**`docker daemon not reachable`**
Docker Desktop (or an equivalent engine) must be running. Under WSL2, enable
*Settings → Resources → WSL Integration* for your distro in Docker Desktop.

**`FUNCTION LOAD failed`**
A stale library is usually the culprit. Flush and retry:

```bash
docker exec cairn-fabric-integ-valkey valkey-cli FUNCTION FLUSH
./scripts/run-fabric-integration-tests.sh
```

If the failure persists, inspect the generated bundle — it is produced by
`ff-script/build.rs` and lives under
`/tmp/FlowFabric/target/release/build/ff-script-*/out/flowfabric.lua`. A
malformed or truncated bundle means the FF checkout is dirty; `git status` it
and rebuild.

**`ff_version FCALL returned empty`**
The library loaded but did not register `ff_version`. This usually means the
build produced a Lua file without the `#!lua name=flowfabric` preamble — the
FF `build.rs` writes it first, so a partial rebuild (aborted compile) can
leave a stale OUT_DIR. Remove the build artifact and retry:

```bash
rm -rf /tmp/FlowFabric/target/release/build/ff-script-*
./scripts/run-fabric-integration-tests.sh
```

**`valkey did not respond to PING`**
The container started but the server is not accepting traffic. Pull fresh
logs:

```bash
docker logs cairn-fabric-integ-valkey
```

A common cause is port `6379` being held by a local Redis/Valkey process —
set `VALKEY_PORT=6380` (and `CAIRN_TEST_VALKEY_URL=redis://localhost:6380`)
to side-step it.

**`FF checkout on '<other-branch>'`**
The script warns but does not fix this. `cd /tmp/FlowFabric && git checkout feat/execution-engine`
or set `FF_BRANCH` to the branch you intend to test against.

**Port conflict / stale container**
The container name is stable (`cairn-fabric-integ-valkey`). If an old one is
wedged, remove it manually:

```bash
docker rm -f cairn-fabric-integ-valkey
```

## Related

- FlowFabric source: <https://github.com/avifenesh/FlowFabric> (branch `feat/execution-engine`)
- Integration test harness: `tests/integration.rs` (`TestHarness::setup`)
- Runtime wiring: `src/boot.rs` — `ff_script::loader::ensure_library` is called on startup
