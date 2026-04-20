//! Valkey testcontainers harness for integration tests.
//!
//! Public because cairn-app's HTTP integration suite also boots an
//! `AppBootstrap::router` against a live Valkey, and duplicating the
//! container bootstrap in two places would mean two places to maintain
//! when the testcontainers API bumps.
//!
//! Gated behind the `test-harness` cargo feature so production builds
//! never link `testcontainers`.
//!
//! # Contract
//!
//! - One Valkey container per test-binary invocation, shared across all
//!   tests in that binary via `OnceCell`. Dropped when the binary exits
//!   (`ContainerAsync` kills + removes on `Drop`).
//! - **No FLUSHDB between tests.** Parallel tests must use uuid-scoped
//!   keyspaces (tenant / workspace / project) so FF's `{p:N}` hash tags
//!   route to disjoint slots. FLUSHDB on a shared container would wipe
//!   sibling tests' in-flight state.
//! - `CAIRN_TEST_VALKEY_URL` override: when set, skip the container and
//!   point at an external Valkey. Useful for CI sidecars and docker-less
//!   environments.

use std::sync::Arc;

use testcontainers::{
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
    ContainerAsync, GenericImage,
};
use tokio::sync::OnceCell;

/// Handle on the shared Valkey container. The inner `ContainerAsync` is
/// held only so `Drop` doesn't run until the test binary exits.
pub struct ValkeyContainer {
    _container: Option<ContainerAsync<GenericImage>>,
    host: String,
    port: u16,
}

impl ValkeyContainer {
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn endpoint(&self) -> (String, u16) {
        (self.host.clone(), self.port)
    }
}

static SHARED: OnceCell<Arc<ValkeyContainer>> = OnceCell::const_new();

/// Return the shared Valkey container endpoint, booting one if this is
/// the first call in the test binary.
///
/// Honours `CAIRN_TEST_VALKEY_URL` — when set, no container starts; the
/// host and port are parsed from the URL. Port defaults to 6379 if not
/// present in the URL.
pub async fn valkey_endpoint() -> (String, u16) {
    shared_container().await.endpoint()
}

/// Return the shared container handle. Prefer [`valkey_endpoint`] for
/// most uses; this returns the full `ValkeyContainer` for cases that
/// need to hold a reference across setup paths.
pub async fn shared_container() -> Arc<ValkeyContainer> {
    SHARED
        .get_or_init(|| async {
            if let Ok(url) = std::env::var("CAIRN_TEST_VALKEY_URL") {
                let parsed = url::Url::parse(&url).expect("invalid CAIRN_TEST_VALKEY_URL");
                let host = parsed.host_str().unwrap_or("localhost").to_owned();
                let port = parsed.port().unwrap_or(6379);
                return Arc::new(ValkeyContainer {
                    _container: None,
                    host,
                    port,
                });
            }

            let container = GenericImage::new("valkey/valkey", "8-alpine")
                .with_exposed_port(6379.tcp())
                .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
                .start()
                .await
                .expect("failed to start valkey container");

            let host = container
                .get_host()
                .await
                .expect("container host unavailable")
                .to_string();
            let port = container
                .get_host_port_ipv4(6379.tcp())
                .await
                .expect("container port unavailable");

            Arc::new(ValkeyContainer {
                _container: Some(container),
                host,
                port,
            })
        })
        .await
        .clone()
}
