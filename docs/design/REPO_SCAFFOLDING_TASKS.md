# Cairn Rust Rewrite: Repo Scaffolding Tasks

Status: execution draft  
Audience: workers starting implementation in `cairn-rs`

## Purpose

This document defines the concrete repo-scaffolding tasks that should happen before deeper feature work fans out.

It complements:

- [`EIGHT_WORKER_EXECUTION_PLAN.md`](./EIGHT_WORKER_EXECUTION_PLAN.md)
- [`MILESTONE_BOARD_WEEKS_1_4.md`](./MILESTONE_BOARD_WEEKS_1_4.md)

## Target Workspace Shape

The initial workspace should move toward:

- `Cargo.toml`
- `rust-toolchain.toml`
- `.cargo/config.toml`
- `crates/cairn-domain`
- `crates/cairn-store`
- `crates/cairn-runtime`
- `crates/cairn-tools`
- `crates/cairn-memory`
- `crates/cairn-graph`
- `crates/cairn-agent`
- `crates/cairn-evals`
- `crates/cairn-signal`
- `crates/cairn-channels`
- `crates/cairn-api`
- `crates/cairn-plugin-proto`
- `crates/cairn-app`
- `tests/compat`
- `tests/fixtures`
- `scripts/`
- `.coordination/`

## Root Scaffolding Tasks

- create workspace `Cargo.toml` with all planned members
- pin Rust toolchain version
- add rustfmt/clippy policy config
- add basic CI workflow for build, test, and format
- add top-level workspace README section for code layout once crates exist

## Crate Scaffolding Tasks

For every crate:

- create `Cargo.toml`
- create `src/lib.rs` or `src/main.rs`
- add crate README comment/header describing ownership
- add one smoke test or compile test

## Shared Infrastructure Tasks

- define common error strategy
- define crate dependency rules
- define feature-flag policy
- define test fixture directory conventions
- define migration file naming rules
- define versioning policy for plugin protocol and API fixtures

## Worker-Assigned Scaffold Tasks

### Worker 1

- create `tests/compat`
- create `tests/fixtures`
- add fixture naming and preserved-surface conventions
- add comparison harness layout for `../cairn-sdk`

### Worker 2

- scaffold `crates/cairn-domain`
- add module layout for:
  - ids
  - commands
  - events
  - tenancy
  - lifecycle
  - policy

### Worker 3

- scaffold `crates/cairn-store`
- create migration folder layout
- create projection module layout
- define DB adapter boundaries for Postgres and SQLite

### Worker 4

- scaffold `crates/cairn-runtime`
- create service modules for:
  - sessions
  - runs
  - tasks
  - approvals
  - checkpoints
  - mailbox
  - recovery

### Worker 5

- scaffold `crates/cairn-tools`
- scaffold plugin host modules
- add execution-class module layout for:
  - `supervised_process`
  - `sandboxed_process`

### Worker 6

- scaffold `crates/cairn-memory`
- scaffold `crates/cairn-graph`
- create module layout for:
  - ingest
  - retrieval
  - diagnostics
  - graph projections
  - graph queries

### Worker 7

- scaffold `crates/cairn-agent`
- scaffold `crates/cairn-evals`
- create module layout for:
  - prompts
  - releases
  - selectors
  - eval matrices
  - scorecards
  - orchestrator

### Worker 8

- scaffold `crates/cairn-api`
- scaffold `crates/cairn-signal`
- scaffold `crates/cairn-channels`
- scaffold `crates/cairn-app`
- create API module layout for:
  - HTTP routes
  - SSE
  - auth
  - operator read models
  - bootstrap

## First PR Sequence

The recommended first PR sequence is:

1. workspace root and empty crates
2. domain and store interfaces
3. runtime spine skeleton
4. API/SSE shell and compatibility harness
5. tool/plugin host skeleton
6. memory/graph skeleton
7. agent/eval skeleton
8. signal/channel/app skeleton

## Definition Of Scaffolding Done

Scaffolding is done when:

- the workspace compiles
- every planned crate exists
- every worker has a concrete write surface in the repo
- fixture and migration directories exist
- CI can validate formatting and compilation
- no worker has to invent repo layout locally
