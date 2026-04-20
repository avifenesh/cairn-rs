//! Shared test support for cairn-app integration tests.
//!
//! Each `tests/*.rs` file is its own crate; include this module via
//! `mod support;` to access the helpers below.

#![allow(dead_code)] // Not every test file uses every helper.

pub mod fake_fabric;

use std::sync::Arc;

use axum::Router;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_app::{AppBootstrap, AppState};
use cairn_runtime::InMemoryServices;
use cairn_store::InMemoryStore;

use crate::support::fake_fabric::build_fake_fabric;

/// Build a router + AppState wired to a read-only FakeFabric trio.
///
/// Most cairn-app integration tests bootstrap a router just to exercise
/// HTTP handler shape (auth, 404, response JSON, SSE framing). They do
/// not need live Valkey or real runtime mutation — a read-only fixture
/// is enough. Tests that do need mutation belong in
/// `crates/cairn-fabric/tests/integration/`.
pub async fn build_test_router_fake_fabric(config: BootstrapConfig) -> (Router, Arc<AppState>) {
    let store = Arc::new(InMemoryStore::new());
    let (runs, tasks, sessions) = build_fake_fabric(store.clone());
    let runtime = Arc::new(InMemoryServices::with_store_and_core(
        store, runs, tasks, sessions,
    ));
    AppBootstrap::router_with_injected_runtime(config, runtime, None)
        .await
        .expect("fake-fabric router bootstrap must succeed")
}

/// Router-only convenience when the test does not need the AppState
/// handle back. Equivalent to `AppBootstrap::router(config)` but uses
/// FakeFabric instead of trying to construct live Fabric.
pub async fn build_test_router(config: BootstrapConfig) -> Router {
    let (router, _state) = build_test_router_fake_fabric(config).await;
    router
}
