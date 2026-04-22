//! Error mapping between `harness_core::ToolError` and cairn's `ToolError`.
//!
//! Every sub-crate wraps engine failures in `harness_core::ToolError` (37
//! stable error codes, structured `meta`). Cairn's orchestrator carries its
//! own `ToolError` enum for retry/cache decisions — we extend it with one
//! new variant `HarnessError` that passes the harness structure through
//! verbatim.

use cairn_tools::builtins::ToolError;
use harness_core::{ToolError as HarnessCoreError, ToolErrorCode};

/// Convert a `harness_core::ToolError` into cairn's `ToolError::HarnessError`.
///
/// All cairn-side consumers see the structured code + message + meta rather
/// than a stringified blob, which preserves machine-readable failure reasons
/// through the orchestrator.
impl From<HarnessCoreError> for CairnToolErrorShim {
    fn from(he: HarnessCoreError) -> Self {
        CairnToolErrorShim(ToolError::HarnessError {
            code: he.code,
            message: he.message,
            meta: he.meta,
        })
    }
}

/// Newtype wrapper used because cairn's `ToolError` lives in a sibling
/// crate and we cannot impl `From` directly there without an orphan-rule
/// violation. Call `.into_tool_error()` at the adapter boundary.
pub struct CairnToolErrorShim(pub ToolError);

impl CairnToolErrorShim {
    pub fn into_tool_error(self) -> ToolError {
        self.0
    }
}

/// Convenience: build cairn `ToolError::HarnessError` straight from the
/// three fields.
pub fn harness_err(code: ToolErrorCode, message: impl Into<String>) -> ToolError {
    ToolError::HarnessError {
        code,
        message: message.into(),
        meta: None,
    }
}

/// Convert a `harness_core::ToolError` into cairn `ToolError` inline.
pub fn map_harness(err: HarnessCoreError) -> ToolError {
    CairnToolErrorShim::from(err).into_tool_error()
}
