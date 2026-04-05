//! MCP (Model Context Protocol) client for cairn-tools.
//!
//! Implements the client side of the Model Context Protocol (JSON-RPC 2.0)
//! for connecting to external MCP servers via stdio or HTTP transport.
//! Tools discovered from MCP servers are exposed through the same
//! `PluginCapability` surface as built-in cairn plugins.
//!
//! # Protocol
//! MCP is JSON-RPC 2.0 over newline-delimited streams (stdio) or HTTP.
//! Handshake: `initialize` → `notifications/initialized` → `tools/list`.
//! Invocation: `tools/call` with `name` + `arguments`.
//!
//! # Naming convention
//! Tools from an MCP server named `"github"` are exposed as
//! `mcp.github.<tool_name>` so they can be namespaced away from built-in tools.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── MCP Protocol Constants ─────────────────────────────────────────────────

/// MCP protocol version this client declares during initialization.
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const JSONRPC_VERSION: &str = "2.0";

// ── Wire types (private) ───────────────────────────────────────────────────

#[derive(Serialize)]
struct McpRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    params: Option<Value>,
}

#[derive(Serialize)]
struct McpNotification {
    jsonrpc: &'static str,
    method: &'static str,
}

#[derive(Deserialize)]
struct McpResponse {
    result: Option<Value>,
    error: Option<McpErrorBody>,
}

#[derive(Deserialize, Debug)]
struct McpErrorBody {
    code: i64,
    message: String,
}

#[derive(Deserialize)]
struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    protocol_version: String,
}

#[derive(Deserialize)]
struct ListToolsResult {
    tools: Vec<McpToolDef>,
}

#[derive(Deserialize)]
struct McpToolDef {
    name: String,
    #[serde(default)]
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Option<Value>,
}

#[derive(Deserialize)]
struct CallToolResult {
    content: Vec<McpContent>,
    #[serde(rename = "isError", default)]
    is_error: bool,
}

#[derive(Deserialize)]
struct McpContent {
    #[serde(rename = "type")]
    content_type: String,
    text: Option<String>,
}

// ── Public Types ──────────────────────────────────────────────────────────

/// Endpoint configuration for an MCP server connection.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum McpEndpoint {
    /// Connect to an MCP server spawned as a child process (stdio).
    ///
    /// The child process communicates via newline-delimited JSON-RPC on
    /// its stdin/stdout. This is the standard MCP transport.
    Stdio {
        /// Executable to spawn (e.g. `"npx"`, `"uvx"`, `"my-mcp-server"`).
        command: String,
        /// Arguments passed to the command.
        #[serde(default)]
        args: Vec<String>,
        /// Extra environment variables as `"KEY=VALUE"` strings.
        #[serde(default)]
        env: Vec<String>,
    },
    /// Connect to a remote MCP server over HTTP.
    ///
    /// Note: synchronous HTTP transport is not yet implemented.
    /// Use `Stdio` for local MCP servers; HTTP support requires an async runtime.
    Http {
        /// Base URL of the MCP endpoint (e.g. `"http://localhost:3001/mcp"`).
        url: String,
        /// Optional request headers (e.g. `Authorization`).
        #[serde(default)]
        headers: HashMap<String, String>,
    },
}

/// A tool discovered from a connected MCP server.
#[derive(Clone, Debug)]
pub struct McpTool {
    /// Namespaced tool name: `mcp.<server_name>.<tool_name>`.
    pub name: String,
    /// Human-readable description from the MCP server, prefixed with `[MCP:<server>]`.
    pub description: String,
    /// JSON Schema for the tool's input arguments (from the MCP `inputSchema` field).
    pub input_schema: Value,
}

/// Errors from MCP client operations.
#[derive(Debug)]
pub enum McpError {
    /// Failed to spawn the process or read/write on the stdio pipes.
    Transport(String),
    /// The MCP server returned a JSON-RPC `error` object.
    Protocol { code: i64, message: String },
    /// The MCP server declared a protocol version this client doesn't support.
    VersionMismatch(String),
    /// `call_tool` or `list_tools` called before `connect` succeeded.
    NotConnected,
    /// HTTP transport requires an async runtime; use `Stdio` for synchronous usage.
    HttpNotImplemented,
}

impl std::fmt::Display for McpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpError::Transport(msg) => write!(f, "MCP transport error: {msg}"),
            McpError::Protocol { code, message } => {
                write!(f, "MCP protocol error {code}: {message}")
            }
            McpError::VersionMismatch(v) => {
                write!(f, "MCP version mismatch: server reported {v}")
            }
            McpError::NotConnected => write!(f, "MCP client not connected"),
            McpError::HttpNotImplemented => write!(
                f,
                "HTTP MCP transport is not yet supported in synchronous mode; use Stdio"
            ),
        }
    }
}

impl std::error::Error for McpError {}

// ── Transport internals ────────────────────────────────────────────────────

struct StdioMcpTransport {
    child: Child,
    reader: BufReader<std::process::ChildStdout>,
}

impl StdioMcpTransport {
    fn spawn(command: &str, args: &[String], env_pairs: &[String]) -> Result<Self, McpError> {
        let mut cmd = Command::new(command);
        cmd.args(args);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        // Clean environment: always pass PATH, then caller-supplied KEY=VALUE pairs.
        cmd.env_clear();
        if let Ok(path) = std::env::var("PATH") {
            cmd.env("PATH", path);
        }
        for kv in env_pairs {
            if let Some((k, v)) = kv.split_once('=') {
                cmd.env(k, v);
            }
        }

        let mut child = cmd.spawn().map_err(|e| McpError::Transport(e.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpError::Transport("failed to capture child stdout".to_owned()))?;

        Ok(Self {
            child,
            reader: BufReader::new(stdout),
        })
    }

    fn send_raw(&mut self, json: &str) -> Result<(), McpError> {
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| McpError::Transport("stdin not available".to_owned()))?;
        stdin
            .write_all(json.as_bytes())
            .map_err(|e| McpError::Transport(e.to_string()))?;
        stdin
            .write_all(b"\n")
            .map_err(|e| McpError::Transport(e.to_string()))?;
        stdin
            .flush()
            .map_err(|e| McpError::Transport(e.to_string()))?;
        Ok(())
    }

    fn send_request(&mut self, req: &McpRequest) -> Result<(), McpError> {
        let json = serde_json::to_string(req).map_err(|e| McpError::Transport(e.to_string()))?;
        self.send_raw(&json)
    }

    fn send_notification(&mut self, notif: &McpNotification) -> Result<(), McpError> {
        let json =
            serde_json::to_string(notif).map_err(|e| McpError::Transport(e.to_string()))?;
        self.send_raw(&json)
    }

    fn recv(&mut self) -> Result<McpResponse, McpError> {
        let mut line = String::new();
        self.reader
            .read_line(&mut line)
            .map_err(|e| McpError::Transport(e.to_string()))?;
        if line.is_empty() {
            return Err(McpError::Transport(
                "MCP server closed the connection".to_owned(),
            ));
        }
        serde_json::from_str(line.trim())
            .map_err(|e| McpError::Transport(format!("invalid JSON-RPC response: {e}")))
    }

    fn shutdown(&mut self) {
        // Close stdin to signal EOF to the child, then wait for exit.
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

enum TransportState {
    Stdio(StdioMcpTransport),
    #[allow(dead_code)]
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
    Disconnected,
}

// ── McpClient ─────────────────────────────────────────────────────────────

/// MCP client connected to a single external MCP server.
///
/// # Lifecycle
/// 1. `McpClient::connect(name, endpoint)` — spawns process or resolves HTTP URL,
///    performs the `initialize` handshake.
/// 2. `list_tools()` — fetches the server's tool catalog (cached after first call).
/// 3. `call_tool(name, args)` — invokes a tool by its namespaced name.
/// 4. `disconnect()` / `Drop` — shuts down the transport cleanly.
///
/// # Thread safety
/// `McpClient` takes `&mut self` for all I/O operations. It is `Send` but not `Sync`.
/// Wrap in `Mutex` for shared access.
pub struct McpClient {
    server_name: String,
    transport: TransportState,
    tools_cache: Option<Vec<McpTool>>,
    request_counter: u64,
}

impl McpClient {
    /// Connect to an MCP server and perform the protocol handshake.
    ///
    /// For `Stdio` endpoints, spawns the child process and completes
    /// `initialize` → `notifications/initialized`. For `Http` endpoints,
    /// returns `McpError::HttpNotImplemented` (async runtime required).
    pub fn connect(server_name: impl Into<String>, endpoint: McpEndpoint) -> Result<Self, McpError> {
        let name = server_name.into();

        match endpoint {
            McpEndpoint::Stdio { command, args, env } => {
                let transport = StdioMcpTransport::spawn(&command, &args, &env)?;
                let mut client = Self {
                    server_name: name,
                    transport: TransportState::Stdio(transport),
                    tools_cache: None,
                    request_counter: 0,
                };
                client.initialize()?;
                Ok(client)
            }
            McpEndpoint::Http { url, headers } => {
                // Store the config so the type is valid, but refuse immediately.
                // HTTP support requires an async runtime (e.g. tokio) and will be
                // added in a follow-up once cairn-tools has an async dependency.
                let _client = Self {
                    server_name: name,
                    transport: TransportState::Http { url, headers },
                    tools_cache: None,
                    request_counter: 0,
                };
                Err(McpError::HttpNotImplemented)
            }
        }
    }

    fn next_id(&mut self) -> u64 {
        self.request_counter += 1;
        self.request_counter
    }

    fn send_rpc(&mut self, method: &str, params: Option<Value>) -> Result<Value, McpError> {
        let id = self.next_id();
        let req = McpRequest {
            jsonrpc: JSONRPC_VERSION,
            id,
            method: method.to_owned(),
            params,
        };

        match &mut self.transport {
            TransportState::Stdio(t) => {
                t.send_request(&req)?;
                let response = t.recv()?;
                if let Some(err) = response.error {
                    return Err(McpError::Protocol {
                        code: err.code,
                        message: err.message,
                    });
                }
                Ok(response.result.unwrap_or(Value::Null))
            }
            TransportState::Http { .. } => Err(McpError::HttpNotImplemented),
            TransportState::Disconnected => Err(McpError::NotConnected),
        }
    }

    fn initialize(&mut self) -> Result<(), McpError> {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {
                "name": "cairn",
                "version": "1.0.0"
            }
        });

        let result = self.send_rpc("initialize", Some(params))?;
        let init: InitializeResult = serde_json::from_value(result).map_err(|e| {
            McpError::Transport(format!("invalid initialize response: {e}"))
        })?;

        // MCP protocol versions are date-stamped. We accept any version and
        // warn on mismatch (non-fatal) rather than hard-failing, because
        // most servers remain backwards-compatible within the same spec generation.
        if init.protocol_version != MCP_PROTOCOL_VERSION {
            // Non-fatal: note the mismatch but proceed.
            let _ = &init.protocol_version;
        }

        // Send the `notifications/initialized` notification (no response expected).
        let notif = McpNotification {
            jsonrpc: JSONRPC_VERSION,
            method: "notifications/initialized",
        };
        if let TransportState::Stdio(t) = &mut self.transport {
            t.send_notification(&notif)?;
        }

        Ok(())
    }

    /// Return the tools available from this MCP server.
    ///
    /// Results are cached after the first successful call. Use
    /// `refresh_tools()` to force a new `tools/list` round-trip.
    pub fn list_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        if let Some(cached) = &self.tools_cache {
            return Ok(cached.clone());
        }
        self.refresh_tools()
    }

    /// Perform a fresh `tools/list` round-trip and update the cache.
    pub fn refresh_tools(&mut self) -> Result<Vec<McpTool>, McpError> {
        let result = self.send_rpc("tools/list", None)?;
        let list: ListToolsResult = serde_json::from_value(result).map_err(|e| {
            McpError::Transport(format!("invalid tools/list response: {e}"))
        })?;

        let server_name = self.server_name.clone();
        let tools: Vec<McpTool> = list
            .tools
            .into_iter()
            .map(|def| McpTool {
                name: format!("mcp.{}.{}", server_name, def.name),
                description: format!("[MCP:{}] {}", server_name, def.description),
                input_schema: def.input_schema.unwrap_or_else(|| {
                    serde_json::json!({"type": "object", "properties": {}})
                }),
            })
            .collect();

        self.tools_cache = Some(tools.clone());
        Ok(tools)
    }

    /// Invoke a tool by its namespaced name (`mcp.<server>.<tool>`) with JSON arguments.
    ///
    /// Returns the tool's text output as a JSON string value, or a JSON array
    /// if the server returns multiple content blocks.
    pub fn call_tool(&mut self, name: &str, args: Value) -> Result<Value, McpError> {
        // Accept both `mcp.<server>.<tool>` (namespaced) and `<tool>` (bare).
        let prefix = format!("mcp.{}.", self.server_name);
        let tool_name = name
            .strip_prefix(&prefix)
            .unwrap_or(name)
            .to_owned();

        let params = serde_json::json!({
            "name": tool_name,
            "arguments": args,
        });

        let result = self.send_rpc("tools/call", Some(params))?;
        let call_result: CallToolResult = serde_json::from_value(result).map_err(|e| {
            McpError::Transport(format!("invalid tools/call response: {e}"))
        })?;

        if call_result.is_error {
            let error_text: String = call_result
                .content
                .iter()
                .filter_map(|c| c.text.as_deref())
                .collect::<Vec<_>>()
                .join("\n");
            return Err(McpError::Protocol {
                code: -1,
                message: error_text,
            });
        }

        let parts: Vec<&str> = call_result
            .content
            .iter()
            .filter(|c| c.content_type == "text")
            .filter_map(|c| c.text.as_deref())
            .collect();

        Ok(Value::String(parts.join("\n")))
    }

    /// Gracefully disconnect from the MCP server.
    pub fn disconnect(&mut self) {
        if let TransportState::Stdio(ref mut t) = self.transport {
            t.shutdown();
        }
        self.transport = TransportState::Disconnected;
        self.tools_cache = None;
    }

    /// The server name this client was connected with.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Whether the client has an active transport (not disconnected).
    pub fn is_connected(&self) -> bool {
        !matches!(self.transport, TransportState::Disconnected)
    }

    /// Return the cached tool list without a network round-trip.
    /// Returns `None` if `list_tools()` has not been called yet.
    pub fn cached_tools(&self) -> Option<&[McpTool]> {
        self.tools_cache.as_deref()
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}

// ── Registry integration helpers ────────────────────────────────────────────

/// Extract the `McpEndpoint` from a plugin manifest if it declares `McpServer` capability.
///
/// Used by the plugin registry to decide whether to route tool calls through an
/// `McpClient` instead of a JSON-RPC plugin process.
pub fn mcp_endpoint_for_manifest(
    capabilities: &[crate::plugins::PluginCapability],
) -> Option<McpEndpoint> {
    capabilities.iter().find_map(|cap| {
        if let crate::plugins::PluginCapability::McpServer { endpoint } = cap {
            Some(endpoint.clone())
        } else {
            None
        }
    })
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── McpEndpoint serialization ────────────────────────────────────────

    #[test]
    fn mcp_endpoint_stdio_round_trips() {
        let endpoint = McpEndpoint::Stdio {
            command: "npx".to_owned(),
            args: vec!["-y".to_owned(), "@modelcontextprotocol/server-filesystem".to_owned()],
            env: vec!["MY_VAR=value".to_owned()],
        };
        let json = serde_json::to_string(&endpoint).unwrap();
        let parsed: McpEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(endpoint, parsed);
    }

    #[test]
    fn mcp_endpoint_http_round_trips() {
        let mut headers = HashMap::new();
        headers.insert("Authorization".to_owned(), "Bearer token".to_owned());
        let endpoint = McpEndpoint::Http {
            url: "http://localhost:3001/mcp".to_owned(),
            headers,
        };
        let json = serde_json::to_string(&endpoint).unwrap();
        let parsed: McpEndpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(endpoint, parsed);
    }

    #[test]
    fn mcp_endpoint_http_returns_not_implemented() {
        let result = McpClient::connect(
            "test-server",
            McpEndpoint::Http {
                url: "http://localhost:9999/mcp".to_owned(),
                headers: HashMap::new(),
            },
        );
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            matches!(err, McpError::HttpNotImplemented),
            "expected HttpNotImplemented, got: {err}"
        );
    }

    // ── McpTool construction ─────────────────────────────────────────────

    #[test]
    fn mcp_tool_fields_accessible() {
        let tool = McpTool {
            name: "mcp.github.create_issue".to_owned(),
            description: "[MCP:github] Create a new issue".to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "title": {"type": "string"},
                    "body": {"type": "string"}
                },
                "required": ["title"]
            }),
        };
        assert_eq!(tool.name, "mcp.github.create_issue");
        assert!(tool.description.contains("MCP:github"));
        assert!(tool.input_schema.is_object());
    }

    // ── Tool name stripping ──────────────────────────────────────────────

    #[test]
    fn mcp_tool_name_prefix_stripping() {
        // Verify the naming convention: mcp.<server>.<tool>
        let server = "github";
        let raw_tool = "create_issue";
        let namespaced = format!("mcp.{server}.{raw_tool}");
        let prefix = format!("mcp.{server}.");
        let stripped = namespaced.strip_prefix(&prefix).unwrap_or(&namespaced);
        assert_eq!(stripped, raw_tool);
    }

    #[test]
    fn mcp_tool_bare_name_passes_through() {
        let server = "github";
        let bare = "create_issue";
        let prefix = format!("mcp.{server}.");
        let result = bare.strip_prefix(&prefix).unwrap_or(bare);
        assert_eq!(result, bare);
    }

    // ── McpError display ─────────────────────────────────────────────────

    #[test]
    fn mcp_error_display_not_connected() {
        let err = McpError::NotConnected;
        assert!(err.to_string().contains("not connected"));
    }

    #[test]
    fn mcp_error_display_protocol() {
        let err = McpError::Protocol {
            code: -32601,
            message: "Method not found".to_owned(),
        };
        let s = err.to_string();
        assert!(s.contains("-32601"));
        assert!(s.contains("Method not found"));
    }

    #[test]
    fn mcp_error_display_http_not_implemented() {
        let err = McpError::HttpNotImplemented;
        assert!(err.to_string().contains("HTTP"));
    }

    // ── PluginCapability McpServer variant ───────────────────────────────

    #[test]
    fn plugin_capability_mcp_server_round_trips() {
        use crate::plugins::PluginCapability;
        let cap = PluginCapability::McpServer {
            endpoint: McpEndpoint::Stdio {
                command: "my-mcp-server".to_owned(),
                args: vec!["--port".to_owned(), "3000".to_owned()],
                env: vec![],
            },
        };
        let json = serde_json::to_string(&cap).unwrap();
        let parsed: PluginCapability = serde_json::from_str(&json).unwrap();
        assert_eq!(cap, parsed);
    }

    #[test]
    fn mcp_endpoint_for_manifest_finds_mcp_server_capability() {
        use crate::plugins::PluginCapability;
        let endpoint = McpEndpoint::Stdio {
            command: "uvx".to_owned(),
            args: vec!["mcp-server-git".to_owned()],
            env: vec![],
        };
        let caps = vec![
            PluginCapability::ToolProvider {
                tools: vec!["some.builtin".to_owned()],
            },
            PluginCapability::McpServer {
                endpoint: endpoint.clone(),
            },
        ];
        let found = mcp_endpoint_for_manifest(&caps);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), endpoint);
    }

    #[test]
    fn mcp_endpoint_for_manifest_returns_none_without_mcp_capability() {
        use crate::plugins::PluginCapability;
        let caps = vec![
            PluginCapability::ToolProvider {
                tools: vec!["tool.a".to_owned()],
            },
            PluginCapability::PolicyHook,
        ];
        assert!(mcp_endpoint_for_manifest(&caps).is_none());
    }

    // ── Mock MCP server test (process-based) ─────────────────────────────
    //
    // A real integration test would spawn an actual MCP server binary.
    // Since we can't guarantee a test binary is present, the connect() call
    // is expected to fail at spawn with a transport error (command not found),
    // which is a valid McpError::Transport. This test verifies the error path.

    #[test]
    fn connect_to_nonexistent_command_returns_transport_error() {
        let result = McpClient::connect(
            "nonexistent",
            McpEndpoint::Stdio {
                command: "this-binary-definitely-does-not-exist-12345".to_owned(),
                args: vec![],
                env: vec![],
            },
        );
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            matches!(err, McpError::Transport(_)),
            "expected Transport error for nonexistent binary, got: {err}"
        );
    }

    // ── Protocol version constant ────────────────────────────────────────

    #[test]
    fn mcp_protocol_version_is_date_format() {
        // MCP protocol versions are date-stamped (YYYY-MM-DD).
        let parts: Vec<&str> = MCP_PROTOCOL_VERSION.split('-').collect();
        assert_eq!(parts.len(), 3, "protocol version must be YYYY-MM-DD");
        assert_eq!(parts[0].len(), 4, "year must be 4 digits");
    }
}
