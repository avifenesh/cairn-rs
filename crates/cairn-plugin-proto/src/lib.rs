//! Language-neutral plugin protocol boundaries and shared types.
//!
//! Defines the JSON-RPC 2.0 wire format, manifest schema, and capability
//! declarations per RFC 007. This crate is intentionally free of host-side
//! runtime dependencies — it describes what goes on the wire.

pub mod capabilities;
pub mod manifest;
pub mod wire;

pub use capabilities::{CapabilityFamily, InvocationStatus};
pub use manifest::{CapabilityWire, LimitsWire, PluginManifestWire};
pub use wire::{
    ActorWire, HostInfo, InitializeParams, InitializeResult, JsonRpcError, JsonRpcErrorBody,
    JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, PluginInfo, RuntimeLinkageWire,
    ScopeWire, ToolDescriptorWire, ToolsInvokeParams, ToolsInvokeResult, ToolsListResult,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_is_one_zero() {
        let params = InitializeParams {
            protocol_version: "1.0".to_owned(),
            host: HostInfo {
                name: "cairn".to_owned(),
                version: "0.1.0".to_owned(),
            },
        };
        assert_eq!(params.protocol_version, "1.0");
    }
}
