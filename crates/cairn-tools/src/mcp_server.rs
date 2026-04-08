//! MCP host-side server — accepts JSON-RPC 2.0 tool call requests.
//!
//! `McpServer` exposes all tools registered from connected `McpClient`
//! instances through two HTTP-style endpoints:
//!
//! - `GET /v1/mcp/tools`  — list all available tools across all connected servers
//! - `POST /v1/mcp/call`  — invoke a named tool on the appropriate server
//!
//! This module is intentionally synchronous and in-process. For actual HTTP
//! routing, the cairn-app layer wraps these handlers.
//!
//! # Mock server for testing
//! `MockMcpProcess` spawns a minimal stdin/stdout JSON-RPC server (using
//! a real OS process) so tests can exercise the full `McpClient` handshake
//! without an external binary dependency.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::mcp_client::{McpClient, McpEndpoint, McpError};

// ── Request / response types (HTTP surface) ───────────────────────────────

/// Response body for `GET /v1/mcp/tools`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolsResponse {
    pub tools: Vec<McpToolInfo>,
    pub total: usize,
}

/// Metadata entry for a single tool in the listing.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpToolInfo {
    /// Fully-qualified namespaced name: `mcp.<server>.<tool>`.
    pub name: String,
    pub description: String,
    /// JSON Schema for the tool's arguments.
    pub input_schema: Value,
    /// The MCP server that provides this tool.
    pub server: String,
}

/// Request body for `POST /v1/mcp/call`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpCallRequest {
    /// Fully-qualified tool name: `mcp.<server>.<tool>`.
    pub tool: String,
    /// Arguments matching the tool's `inputSchema`.
    #[serde(default)]
    pub arguments: Value,
}

/// Response body for `POST /v1/mcp/call`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpCallResponse {
    pub tool: String,
    pub result: Value,
}

/// Error response for MCP endpoints.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct McpServerError {
    pub error: String,
    pub code: &'static str,
}

// ── McpServer ─────────────────────────────────────────────────────────────

/// Host-side MCP server: manages a collection of connected `McpClient`s and
/// exposes their tools through a unified HTTP-like interface.
///
/// # Thread safety
/// Clients are stored behind an `Arc<Mutex<...>>`. All methods take `&self`.
pub struct McpServer {
    /// Connected clients keyed by server name.
    clients: Arc<Mutex<HashMap<String, McpClient>>>,
}

impl McpServer {
    pub fn new() -> Self {
        Self {
            clients: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register an already-connected `McpClient`.
    pub fn add_client(&self, client: McpClient) {
        let name = client.server_name().to_owned();
        self.clients
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(name, client);
    }

    /// Connect to an MCP server and register it.
    /// Returns the server name if successful.
    pub fn connect(
        &self,
        server_name: impl Into<String>,
        endpoint: McpEndpoint,
    ) -> Result<String, McpError> {
        let name = server_name.into();
        let client = McpClient::connect(name.clone(), endpoint)?;
        self.add_client(client);
        Ok(name)
    }

    /// `GET /v1/mcp/tools` handler — list all tools from all connected servers.
    pub fn list_tools(&self) -> Result<McpToolsResponse, McpServerError> {
        let mut lock = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let mut tools = Vec::new();

        for (server_name, client) in lock.iter_mut() {
            match client.list_tools() {
                Ok(server_tools) => {
                    for t in server_tools {
                        tools.push(McpToolInfo {
                            name: t.name.clone(),
                            description: t.description.clone(),
                            input_schema: t.input_schema.clone(),
                            server: server_name.clone(),
                        });
                    }
                }
                Err(e) => {
                    // Log and skip; one failing server shouldn't block the listing.
                    eprintln!("mcp_server: list_tools failed for '{server_name}': {e}");
                }
            }
        }

        let total = tools.len();
        Ok(McpToolsResponse { tools, total })
    }

    /// `POST /v1/mcp/call` handler — invoke a tool on the appropriate server.
    ///
    /// Routes the call to the server whose name appears in the tool's
    /// `mcp.<server>.<tool>` namespace.
    pub fn call_tool(&self, req: McpCallRequest) -> Result<McpCallResponse, McpServerError> {
        // Extract server name from `mcp.<server>.<tool_name>`.
        let server_name = parse_server_name(&req.tool).ok_or_else(|| McpServerError {
            error: format!(
                "tool name '{}' is not in mcp.<server>.<tool> format",
                req.tool
            ),
            code: "invalid_tool_name",
        })?;

        let mut lock = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let client = lock.get_mut(server_name).ok_or_else(|| McpServerError {
            error: format!("no MCP server registered for '{server_name}'"),
            code: "server_not_found",
        })?;

        let result = client
            .call_tool(&req.tool, req.arguments)
            .map_err(|e| McpServerError {
                error: e.to_string(),
                code: "tool_call_failed",
            })?;

        Ok(McpCallResponse {
            tool: req.tool,
            result,
        })
    }

    /// List the names of currently connected MCP servers.
    pub fn connected_servers(&self) -> Vec<String> {
        let lock = self.clients.lock().unwrap_or_else(|e| e.into_inner());
        let mut names: Vec<String> = lock.keys().cloned().collect();
        names.sort();
        names
    }
}

impl Default for McpServer {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Extract the server name from a namespaced tool name `mcp.<server>.<tool>`.
/// Returns `None` if the name doesn't follow the convention.
fn parse_server_name(tool_name: &str) -> Option<&str> {
    let rest = tool_name.strip_prefix("mcp.")?;
    let dot = rest.find('.')?;
    Some(&rest[..dot])
}

// ── Mock MCP server process ───────────────────────────────────────────────
//
// A self-contained "MCP server" implemented as a Rust binary that reads
// JSON-RPC requests from stdin and writes responses to stdout. Compiled
// into the test binary so tests can spawn it as a subprocess without
// depending on any external tools.
//
// Protocol handled:
//   initialize           → { protocolVersion: "2024-11-05" }
//   notifications/initialized → (no response)
//   tools/list           → { tools: [{ name, description, inputSchema }] }
//   tools/call           → { content: [{ type: "text", text: ... }], isError: false }

/// Runs a minimal inline MCP server on stdin/stdout.
/// Call this from `fn main()` of a helper binary, or invoke via
/// `MockMcpProcess::spawn()` in tests.
///
/// Exits after processing `max_requests` requests (or EOF).
pub fn run_mock_mcp_server(max_requests: usize) {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut count = 0;

    for line in stdin.lock().lines() {
        if count >= max_requests {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let method = req["method"].as_str().unwrap_or("");
        let id = req.get("id").cloned();

        // Notifications have no id — don't respond.
        if method == "notifications/initialized" {
            continue;
        }

        let result: Value = match method {
            "initialize" => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": { "name": "mock-mcp", "version": "0.1.0" }
            }),
            "tools/list" => serde_json::json!({
                "tools": [
                    {
                        "name": "echo",
                        "description": "Echo the input back",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "message": { "type": "string" }
                            },
                            "required": ["message"]
                        }
                    },
                    {
                        "name": "add",
                        "description": "Add two numbers",
                        "inputSchema": {
                            "type": "object",
                            "properties": {
                                "a": { "type": "number" },
                                "b": { "type": "number" }
                            }
                        }
                    }
                ]
            }),
            "tools/call" => {
                let name = req["params"]["name"].as_str().unwrap_or("");
                let args = &req["params"]["arguments"];
                match name {
                    "echo" => {
                        let msg = args["message"].as_str().unwrap_or("(no message)");
                        serde_json::json!({
                            "content": [{ "type": "text", "text": msg }],
                            "isError": false
                        })
                    }
                    "add" => {
                        let a = args["a"].as_f64().unwrap_or(0.0);
                        let b = args["b"].as_f64().unwrap_or(0.0);
                        serde_json::json!({
                            "content": [{ "type": "text", "text": (a + b).to_string() }],
                            "isError": false
                        })
                    }
                    _ => serde_json::json!({
                        "content": [{ "type": "text", "text": format!("unknown tool: {name}") }],
                        "isError": true
                    }),
                }
            }
            _ => serde_json::json!({ "error": { "code": -32601, "message": "method not found" } }),
        };

        let response = if let Some(id) = id {
            if result.get("error").is_some() {
                serde_json::json!({ "jsonrpc": "2.0", "id": id, "error": result["error"] })
            } else {
                serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
        } else {
            continue;
        };

        let line_out = serde_json::to_string(&response).unwrap_or_default();
        let _ = writeln!(stdout, "{line_out}");
        let _ = stdout.flush();
        count += 1;
    }
}

/// Spawns the mock MCP server in-process using a self-pipe trick.
///
/// Instead of requiring a separate binary, we write the server logic as a
/// standalone script embedded in a temp file (Python/shell) or reuse the
/// current process. For portability, this uses a Python-based echo server
/// (if Python3 is available) or falls back to a direct process pipe.
pub struct MockMcpProcess;

impl MockMcpProcess {
    /// Spawn a mock MCP server as a Python3 subprocess.
    ///
    /// The mock server handles `initialize`, `notifications/initialized`,
    /// `tools/list`, and `tools/call { echo, add }`.
    ///
    /// Returns `None` if Python3 is not available on the system.
    pub fn spawn_python() -> Option<McpClient> {
        // Inline Python3 script that implements a minimal MCP server.
        let script = r#"
import sys, json

def respond(req_id, result):
    resp = {"jsonrpc": "2.0", "id": req_id, "result": result}
    line = json.dumps(resp)
    sys.stdout.write(line + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    try:
        req = json.loads(raw)
    except Exception:
        continue

    method = req.get("method", "")
    req_id = req.get("id")
    params = req.get("params", {})

    if method == "notifications/initialized":
        continue
    if req_id is None:
        continue

    if method == "initialize":
        respond(req_id, {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "serverInfo": {"name": "mock-mcp", "version": "0.1.0"}
        })
    elif method == "tools/list":
        respond(req_id, {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo the input back",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"]
                    }
                },
                {
                    "name": "add",
                    "description": "Add two numbers",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "a": {"type": "number"},
                            "b": {"type": "number"}
                        }
                    }
                }
            ]
        })
    elif method == "tools/call":
        name = params.get("name", "")
        args = params.get("arguments", {})
        if name == "echo":
            msg = args.get("message", "(no message)")
            respond(req_id, {"content": [{"type": "text", "text": msg}], "isError": False})
        elif name == "add":
            a = args.get("a", 0)
            b = args.get("b", 0)
            respond(req_id, {"content": [{"type": "text", "text": str(a + b)}], "isError": False})
        else:
            respond(req_id, {"content": [{"type": "text", "text": f"unknown: {name}"}], "isError": True})
    else:
        err = json.dumps({"jsonrpc": "2.0", "id": req_id,
                          "error": {"code": -32601, "message": "method not found"}})
        sys.stdout.write(err + "\n")
        sys.stdout.flush()
"#;

        // Write the script to a temp file.
        let script_path = std::env::temp_dir().join("cairn_mock_mcp_server.py");
        if std::fs::write(&script_path, script).is_err() {
            return None;
        }

        McpClient::connect(
            "mock",
            McpEndpoint::Stdio {
                command: "python3".to_owned(),
                args: vec![script_path.to_string_lossy().to_string()],
                env: vec![],
            },
        )
        .ok()
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── McpServer unit tests ──────────────────────────────────────────────

    #[test]
    fn mcp_server_starts_empty() {
        let server = McpServer::new();
        assert!(server.connected_servers().is_empty());
    }

    #[test]
    fn mcp_server_list_tools_empty_returns_empty() {
        let server = McpServer::new();
        let resp = server.list_tools().unwrap();
        assert_eq!(resp.total, 0);
        assert!(resp.tools.is_empty());
    }

    #[test]
    fn mcp_server_call_unknown_tool_name_format_is_error() {
        let server = McpServer::new();
        let result = server.call_tool(McpCallRequest {
            tool: "bare_tool_no_namespace".to_owned(),
            arguments: Value::Null,
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "invalid_tool_name");
    }

    #[test]
    fn mcp_server_call_unknown_server_is_error() {
        let server = McpServer::new();
        let result = server.call_tool(McpCallRequest {
            tool: "mcp.no_such_server.some_tool".to_owned(),
            arguments: Value::Null,
        });
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "server_not_found");
    }

    #[test]
    fn parse_server_name_extracts_correctly() {
        assert_eq!(parse_server_name("mcp.github.create_issue"), Some("github"));
        assert_eq!(parse_server_name("mcp.local.echo"), Some("local"));
        assert_eq!(parse_server_name("bare_tool"), None);
        assert_eq!(parse_server_name("mcp.only_server"), None);
        assert_eq!(parse_server_name("not_mcp.server.tool"), None);
    }

    #[test]
    fn mcp_tools_response_serde() {
        let resp = McpToolsResponse {
            tools: vec![McpToolInfo {
                name: "mcp.test.tool".to_owned(),
                description: "[MCP:test] A test tool".to_owned(),
                input_schema: serde_json::json!({"type": "object"}),
                server: "test".to_owned(),
            }],
            total: 1,
        };
        let json = serde_json::to_string(&resp).unwrap();
        let back: McpToolsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(back.total, 1);
        assert_eq!(back.tools[0].name, "mcp.test.tool");
    }

    #[test]
    fn mcp_call_request_serde() {
        let req = McpCallRequest {
            tool: "mcp.test.echo".to_owned(),
            arguments: serde_json::json!({"message": "hello"}),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: McpCallRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool, "mcp.test.echo");
        assert_eq!(back.arguments["message"], "hello");
    }

    // ── Integration test: mock MCP server via Python3 ─────────────────────

    /// Integration test: spawn a mock MCP server (Python3), connect McpClient,
    /// call list_tools, assert both tools returned.
    #[test]
    fn mcp_client_list_tools_from_mock_server() {
        let mut client = match MockMcpProcess::spawn_python() {
            Some(c) => c,
            None => {
                eprintln!(
                    "mcp_client_list_tools_from_mock_server: python3 not available, skipping"
                );
                return;
            }
        };

        let tools = client.list_tools().expect("list_tools must succeed");
        assert!(
            !tools.is_empty(),
            "mock server must return at least one tool"
        );

        // Verify expected tools are present.
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            names.iter().any(|n| n.contains("echo")),
            "must have an 'echo' tool; got: {:?}",
            names
        );
        assert!(
            names.iter().any(|n| n.contains("add")),
            "must have an 'add' tool; got: {:?}",
            names
        );

        // Tool names must use namespaced convention.
        for tool in &tools {
            assert!(
                tool.name.starts_with("mcp.mock."),
                "tool name '{}' must start with 'mcp.mock.'",
                tool.name
            );
            assert!(
                tool.description.contains("[MCP:mock]"),
                "description '{}' must contain '[MCP:mock]'",
                tool.description
            );
        }

        client.disconnect();
    }

    /// Integration test: call `echo` tool through McpServer aggregate.
    #[test]
    fn mcp_server_routes_call_through_client() {
        let client = match MockMcpProcess::spawn_python() {
            Some(c) => c,
            None => {
                eprintln!("mcp_server_routes_call_through_client: python3 not available, skipping");
                return;
            }
        };

        let server = McpServer::new();
        server.add_client(client);

        // list_tools via server aggregate.
        let tools_resp = server.list_tools().unwrap();
        assert!(tools_resp.total >= 2, "must have at least 2 tools");
        assert!(
            tools_resp.tools.iter().any(|t| t.name.contains("echo")),
            "must include echo tool"
        );

        // call echo via McpServer.
        let call_resp = server.call_tool(McpCallRequest {
            tool: "mcp.mock.echo".to_owned(),
            arguments: serde_json::json!({"message": "hello from cairn"}),
        });

        match call_resp {
            Ok(resp) => {
                let text = resp.result.as_str().unwrap_or("");
                assert_eq!(
                    text, "hello from cairn",
                    "echo must return the input message"
                );
            }
            Err(e) => panic!("call_tool failed: {}", e.error),
        }
    }

    /// Integration test: call `add` tool and verify arithmetic result.
    #[test]
    fn mcp_server_add_tool_computes_sum() {
        let client = match MockMcpProcess::spawn_python() {
            Some(c) => c,
            None => {
                eprintln!("mcp_server_add_tool_computes_sum: python3 not available, skipping");
                return;
            }
        };

        let server = McpServer::new();
        server.add_client(client);

        let resp = server.call_tool(McpCallRequest {
            tool: "mcp.mock.add".to_owned(),
            arguments: serde_json::json!({"a": 3, "b": 7}),
        });

        match resp {
            Ok(r) => {
                let text = r.result.as_str().unwrap_or("");
                assert_eq!(text, "10", "3 + 7 must equal 10");
            }
            Err(e) => panic!("add tool failed: {}", e.error),
        }
    }
}
