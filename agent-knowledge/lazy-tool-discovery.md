# Lazy Tool Discovery Patterns for Agent Systems

**Generated**: 2026-04-07
**Sources**: MCP spec, Anthropic tool use docs, AutoGen, CrewAI, OpenAI Agents SDK

## TL;DR

- **MCP is the standard**: JSON-RPC `tools/list` + `tools/call` + `notifications/tools/list_changed`. Servers expose tools, clients discover them on demand.
- **Lazy = don't load all tools upfront**: OpenAI Agents SDK has `defer_loading=True` + `ToolSearchTool()` that lets the model load tool subsets on demand. This is the pattern cairn-rs should adopt.
- **Every framework uses the same tool shape**: `{name, description, inputSchema}`. JSON Schema for parameters is universal.
- **Static composition is the norm, dynamic is the differentiator**: LangChain/CrewAI/AutoGen all pass tools at agent creation. MCP and OpenAI Agents SDK support runtime discovery. cairn-rs should support both.

## Core Patterns

### 1. Tool Shape (Universal)

Every system uses the same shape:
```json
{
  "name": "unique_tool_name",
  "description": "What this tool does (LLM reads this to decide)",
  "inputSchema": { "type": "object", "properties": {...}, "required": [...] }
}
```

cairn-rs already has this in `PluginManifest` and `ToolDescriptorWire`. The new `ToolHandler` trait should align.

### 2. Discovery Patterns

**Static (most frameworks):** Tools assigned at agent creation, never change.
- LangChain: `agent = Agent(tools=[tool1, tool2])`
- CrewAI: `agent = Agent(tools=[search_tool, web_rag_tool])`
- AutoGen: `ToolUseAgent(model_client, tool_schema=tools)`

**Dynamic (MCP):** Tools discovered at runtime via `tools/list`, can change mid-session via `notifications/tools/list_changed`.

**Lazy (OpenAI Agents SDK):** Tools marked `defer_loading=True` are hidden from the LLM prompt until explicitly searched via `ToolSearchTool()`. This reduces token overhead when there are many tools.

### 3. Lazy Discovery Architecture (from OpenAI Agents SDK)

The key insight: **tool descriptions eat context window tokens**. With 50 tools, each with a 100-token description + schema, that's 5000 tokens of tool definitions in every prompt. Lazy discovery solves this:

1. **Immediate tools**: Core tools always in prompt (memory_search, memory_store)
2. **Deferred tools**: Registered but hidden (web_fetch, shell_exec, file_read, etc.)
3. **Tool search tool**: A meta-tool the LLM can call to discover deferred tools by capability query
4. **Namespaces**: Group related tools (e.g. "file_ops", "web_ops", "memory_ops") — load a namespace, not individual tools

OpenAI recommends: "keep each namespace fewer than 10 functions"

### 4. MCP Server Composition

MCP supports multiple servers, each exposing different tools:
- Host creates one client per server
- Each client does `tools/list` to discover that server's tools
- All tools are merged into a unified registry
- Server sends `notifications/tools/list_changed` when tools change

This maps to cairn-rs's plugin model: each plugin is an MCP server, the orchestrator is the MCP host.

### 5. Capability-Based Tool Selection

Beyond name matching, some systems filter tools by capability:
- **Conditional enablement** (OpenAI): `is_enabled(context, agent)` function per tool
- **Execution class** (cairn-rs already has this): `Sensitive` tools require approval
- **Operation kind matching**: embed tools for embed operations, generate tools for generate

## Recommended Architecture for cairn-rs

### Three-Tier Tool Registry

```
Tier 1: Builtin (always available, compiled in)
  memory_search, memory_store, complete_run, escalate

Tier 2: Registered (loaded at startup, in prompt)
  web_fetch, shell_exec, file_read, graph_query

Tier 3: Deferred (discoverable on demand)
  MCP server tools, plugin tools, custom tools
  NOT in prompt until LLM calls tool_search
```

### Tool Search Meta-Tool

Add a `tool_search` builtin tool that the LLM can call:
```json
{
  "name": "tool_search",
  "description": "Search for available tools by capability. Use when you need a tool not in your current set.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": { "type": "string", "description": "What capability you need" },
      "namespace": { "type": "string", "description": "Optional namespace filter" }
    },
    "required": ["query"]
  }
}
```

Returns matching deferred tools. The orchestrator adds them to the next prompt.

### MCP Integration Path

cairn-rs already has `cairn-plugin-proto` with JSON-RPC types. The MCP integration:
1. `PluginHost` connects to MCP servers via stdio or HTTP
2. On connect: `tools/list` → register as Tier 3 deferred tools
3. On `tools/call`: forward to MCP server, return result
4. On `notifications/tools/list_changed`: refresh tool list

## Tools to Build

### Essential (v1)
1. **memory_search** — semantic + lexical search over owned retrieval
2. **memory_store** — ingest new knowledge
3. **web_fetch** — HTTP GET with response capping
4. **shell_exec** — sandboxed command execution (Sensitive)
5. **tool_search** — meta-tool for lazy discovery

### Important (v1.1)
6. **file_read** — read file contents from project scope
7. **file_write** — write/patch files (Sensitive)
8. **graph_query** — query the knowledge graph
9. **eval_score** — score a run against a rubric
10. **notify_operator** — send notification to operator channel

### Plugin-sourced (v2)
- MCP server tools (any MCP-compatible tool server)
- Custom plugin tools (via cairn-plugin-proto)
