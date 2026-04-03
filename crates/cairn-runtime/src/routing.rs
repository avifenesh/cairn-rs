//! Provider route resolution service per RFC 009.
//!
//! Resolves which provider binding to use for a given operation and
//! runtime context, producing a durable RouteDecisionRecord.

use async_trait::async_trait;
use cairn_domain::providers::{OperationKind, RouteDecisionRecord};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::ProjectKey;

use crate::error::RuntimeError;

/// Route resolver service boundary.
///
/// Per RFC 009, resolves provider bindings using selector precedence,
/// fallback chains, and capability checks. The resolve method produces
/// a durable RouteDecisionRecord that links to route attempts.
#[async_trait]
pub trait RouteResolverService: Send + Sync {
    /// Resolve a provider route for the given operation and context.
    async fn resolve(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
        context: &SelectorContext,
    ) -> Result<RouteDecisionRecord, RuntimeError>;
}
