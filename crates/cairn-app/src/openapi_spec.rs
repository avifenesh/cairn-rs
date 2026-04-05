//! Static OpenAPI 3.0 specification for the Cairn API.
//!
//! Served at `GET /v1/openapi.json`.  The Swagger UI at `GET /v1/docs`
//! loads this spec from the CDN-hosted swagger-ui bundle.

/// OpenAPI 3.0 specification as a static JSON string.
///
/// Groups endpoints by tag: Health, Sessions, Runs, Tasks, Approvals,
/// Providers, Memory, Events, Evals, Admin.
pub const OPENAPI_JSON: &str = r##"{
  "openapi": "3.0.3",
  "info": {
    "title": "Cairn API",
    "description": "Self-hostable control plane for production AI agent deployments.\n\nAll `/v1/` endpoints require `Authorization: Bearer <token>`. `/health`, `/v1/stream`, and `/v1/docs` are public.",
    "version": "0.1.0",
    "contact": {
      "name": "cairn-rs",
      "url": "https://github.com/avifenesh/cairn-rs"
    },
    "license": { "name": "MIT" }
  },
  "servers": [
    { "url": "http://localhost:3000", "description": "Local dev" }
  ],
  "components": {
    "securitySchemes": {
      "bearerAuth": {
        "type": "http",
        "scheme": "bearer",
        "description": "Admin or service-account bearer token"
      }
    },
    "schemas": {
      "Error": {
        "type": "object",
        "properties": {
          "code":    { "type": "string" },
          "message": { "type": "string" }
        },
        "required": ["code", "message"]
      },
      "ProjectKey": {
        "type": "object",
        "properties": {
          "tenant_id":    { "type": "string" },
          "workspace_id": { "type": "string" },
          "project_id":   { "type": "string" }
        }
      },
      "SessionRecord": {
        "type": "object",
        "properties": {
          "session_id":  { "type": "string" },
          "project":     { "$ref": "#/components/schemas/ProjectKey" },
          "state":       { "type": "string", "enum": ["open","completed","failed","archived"] },
          "version":     { "type": "integer" },
          "created_at":  { "type": "integer", "description": "Unix ms" },
          "updated_at":  { "type": "integer" }
        }
      },
      "RunRecord": {
        "type": "object",
        "properties": {
          "run_id":        { "type": "string" },
          "session_id":    { "type": "string" },
          "parent_run_id": { "type": "string", "nullable": true },
          "project":       { "$ref": "#/components/schemas/ProjectKey" },
          "state": {
            "type": "string",
            "enum": ["pending","running","paused","waiting_approval","waiting_dependency","completed","failed","canceled"]
          },
          "failure_class": { "type": "string", "nullable": true },
          "version":       { "type": "integer" },
          "created_at":    { "type": "integer" },
          "updated_at":    { "type": "integer" }
        }
      },
      "TaskRecord": {
        "type": "object",
        "properties": {
          "task_id":          { "type": "string" },
          "project":          { "$ref": "#/components/schemas/ProjectKey" },
          "parent_run_id":    { "type": "string", "nullable": true },
          "state": {
            "type": "string",
            "enum": ["queued","leased","running","completed","failed","canceled","paused","waiting_dependency","retryable_failed","dead_lettered"]
          },
          "lease_owner":      { "type": "string", "nullable": true },
          "lease_expires_at": { "type": "integer", "nullable": true },
          "version":          { "type": "integer" },
          "created_at":       { "type": "integer" },
          "updated_at":       { "type": "integer" }
        }
      },
      "ApprovalRecord": {
        "type": "object",
        "properties": {
          "approval_id":  { "type": "string" },
          "project":      { "$ref": "#/components/schemas/ProjectKey" },
          "run_id":       { "type": "string", "nullable": true },
          "task_id":      { "type": "string", "nullable": true },
          "requirement":  { "type": "string", "enum": ["required","advisory"] },
          "decision":     { "type": "string", "enum": ["approved","rejected"], "nullable": true },
          "created_at":   { "type": "integer" }
        }
      },
      "ListResponse": {
        "type": "object",
        "properties": {
          "items":    { "type": "array", "items": {} },
          "has_more": { "type": "boolean" }
        }
      },
      "EventEnvelope": {
        "type": "object",
        "properties": {
          "event_id":     { "type": "string" },
          "causation_id": { "type": "string", "nullable": true },
          "source":       { "type": "object" },
          "payload":      { "type": "object", "description": "RuntimeEvent payload" }
        }
      },
      "AppendResult": {
        "type": "object",
        "properties": {
          "event_id": { "type": "string" },
          "position": { "type": "integer" },
          "appended": { "type": "boolean" }
        }
      }
    }
  },
  "security": [{ "bearerAuth": [] }],
  "paths": {
    "/health": {
      "get": {
        "tags": ["Health"],
        "summary": "Liveness probe",
        "description": "Returns `{\"ok\":true}` when the server is running. No auth required.",
        "security": [],
        "operationId": "getHealth",
        "responses": {
          "200": { "description": "Server is alive", "content": { "application/json": { "schema": { "type": "object", "properties": { "ok": { "type": "boolean" } } } } } }
        }
      }
    },
    "/v1/health/detailed": {
      "get": {
        "tags": ["Health"],
        "summary": "Detailed health check",
        "description": "Returns per-component health: store, Ollama, event buffer, memory RSS.",
        "operationId": "getDetailedHealth",
        "responses": {
          "200": { "description": "Health report", "content": { "application/json": { "schema": { "type": "object" } } } }
        }
      }
    },
    "/v1/status": {
      "get": {
        "tags": ["Health"],
        "summary": "Runtime and store health",
        "operationId": "getStatus",
        "responses": { "200": { "description": "System status" } }
      }
    },
    "/v1/dashboard": {
      "get": {
        "tags": ["Health"],
        "summary": "Operator dashboard overview",
        "description": "Active runs, tasks, pending approvals, failed runs (24h), cost summary.",
        "operationId": "getDashboard",
        "responses": { "200": { "description": "Dashboard data" } }
      }
    },
    "/v1/rate-limit": {
      "get": {
        "tags": ["Health"],
        "summary": "Current rate-limit quota",
        "security": [],
        "operationId": "getRateLimit",
        "responses": { "200": { "description": "Quota status" } }
      }
    },
    "/v1/sessions": {
      "get": {
        "tags": ["Sessions"],
        "summary": "List active sessions",
        "operationId": "listSessions",
        "parameters": [
          { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 50 } },
          { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0  } }
        ],
        "responses": { "200": { "description": "Session list" } }
      },
      "post": {
        "tags": ["Sessions"],
        "summary": "Create a new session",
        "operationId": "createSession",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": {
            "type": "object",
            "properties": {
              "tenant_id":    { "type": "string" },
              "workspace_id": { "type": "string" },
              "project_id":   { "type": "string" },
              "session_id":   { "type": "string" }
            }
          }}}
        },
        "responses": {
          "201": { "description": "Created session", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SessionRecord" } } } }
        }
      }
    },
    "/v1/sessions/{id}/runs": {
      "get": {
        "tags": ["Sessions"],
        "summary": "List runs in a session",
        "operationId": "listSessionRuns",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Run list" } }
      }
    },
    "/v1/sessions/{id}/events": {
      "get": {
        "tags": ["Sessions"],
        "summary": "Entity-scoped event stream for a session",
        "operationId": "listSessionEvents",
        "parameters": [
          { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "cursor", "in": "query", "schema": { "type": "integer" } },
          { "name": "limit",  "in": "query", "schema": { "type": "integer" } }
        ],
        "responses": { "200": { "description": "Events page" } }
      }
    },
    "/v1/sessions/{id}/llm-traces": {
      "get": {
        "tags": ["Sessions"],
        "summary": "LLM call traces for a session",
        "operationId": "getSessionLlmTraces",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Trace list" } }
      }
    },
    "/v1/runs": {
      "get": {
        "tags": ["Runs"],
        "summary": "List runs",
        "operationId": "listRuns",
        "parameters": [
          { "name": "limit",  "in": "query", "schema": { "type": "integer" } },
          { "name": "offset", "in": "query", "schema": { "type": "integer" } }
        ],
        "responses": { "200": { "description": "Run list" } }
      },
      "post": {
        "tags": ["Runs"],
        "summary": "Start a new run",
        "operationId": "createRun",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object" } } } },
        "responses": {
          "201": { "description": "Created run", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RunRecord" } } } }
        }
      }
    },
    "/v1/runs/{id}": {
      "get": {
        "tags": ["Runs"],
        "summary": "Get run by ID",
        "operationId": "getRun",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Run record" }, "404": { "description": "Not found" } }
      }
    },
    "/v1/runs/{id}/pause": {
      "post": {
        "tags": ["Runs"],
        "summary": "Pause a running run",
        "operationId": "pauseRun",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "reason_kind": { "type": "string" }, "actor": { "type": "string" }, "resume_after_ms": { "type": "integer" } } } } } },
        "responses": { "200": { "description": "Paused run" } }
      }
    },
    "/v1/runs/{id}/resume": {
      "post": {
        "tags": ["Runs"],
        "summary": "Resume a paused run",
        "operationId": "resumeRun",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Resumed run" } }
      }
    },
    "/v1/runs/{id}/tasks": {
      "get": {
        "tags": ["Runs"],
        "summary": "List tasks for a run",
        "operationId": "listRunTasks",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Task list" } }
      }
    },
    "/v1/runs/{id}/approvals": {
      "get": {
        "tags": ["Runs"],
        "summary": "List approvals for a run",
        "operationId": "listRunApprovals",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Approval list" } }
      }
    },
    "/v1/runs/{id}/cost": {
      "get": {
        "tags": ["Runs"],
        "summary": "Cost breakdown for a run",
        "operationId": "getRunCost",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Cost summary" } }
      }
    },
    "/v1/runs/{id}/events": {
      "get": {
        "tags": ["Runs", "Events"],
        "summary": "Event stream for a run",
        "operationId": "listRunEvents",
        "parameters": [
          { "name": "id",     "in": "path",  "required": true, "schema": { "type": "string" } },
          { "name": "cursor", "in": "query", "schema": { "type": "integer" } },
          { "name": "limit",  "in": "query", "schema": { "type": "integer" } }
        ],
        "responses": { "200": { "description": "Events page" } }
      }
    },
    "/v1/runs/{id}/tool-invocations": {
      "get": {
        "tags": ["Runs"],
        "summary": "Tool invocations for a run",
        "operationId": "listRunToolInvocations",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Tool invocation list" } }
      }
    },
    "/v1/tasks": {
      "get": {
        "tags": ["Tasks"],
        "summary": "List all tasks (operator view)",
        "operationId": "listTasks",
        "parameters": [
          { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 100 } },
          { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0   } }
        ],
        "responses": { "200": { "description": "Task array" } }
      }
    },
    "/v1/tasks/{id}/claim": {
      "post": {
        "tags": ["Tasks"],
        "summary": "Claim a queued task",
        "operationId": "claimTask",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "worker_id": { "type": "string" }, "lease_duration_ms": { "type": "integer", "default": 30000 } }, "required": ["worker_id"] } } } },
        "responses": { "200": { "description": "Claimed task" }, "400": { "description": "Invalid transition" } }
      }
    },
    "/v1/tasks/{id}/release-lease": {
      "post": {
        "tags": ["Tasks"],
        "summary": "Release a task lease back to queued",
        "operationId": "releaseTaskLease",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Task back in queued state" } }
      }
    },
    "/v1/approvals/pending": {
      "get": {
        "tags": ["Approvals"],
        "summary": "List pending approvals",
        "operationId": "listPendingApprovals",
        "responses": { "200": { "description": "Pending approvals" } }
      }
    },
    "/v1/approvals/{id}/resolve": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Approve or reject an approval",
        "operationId": "resolveApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "decision": { "type": "string", "enum": ["approved","rejected"] }, "reason": { "type": "string" } }, "required": ["decision"] } } } },
        "responses": { "200": { "description": "Resolved approval" } }
      }
    },
    "/v1/providers": {
      "get": {
        "tags": ["Providers"],
        "summary": "List provider bindings",
        "operationId": "listProviders",
        "responses": { "200": { "description": "Provider list" } }
      }
    },
    "/v1/providers/health": {
      "get": {
        "tags": ["Providers"],
        "summary": "Provider health status",
        "operationId": "getProviderHealth",
        "responses": { "200": { "description": "Health records" } }
      }
    },
    "/v1/providers/ollama/models": {
      "get": {
        "tags": ["Providers"],
        "summary": "List locally available Ollama models",
        "operationId": "listOllamaModels",
        "responses": { "200": { "description": "Model list" } }
      }
    },
    "/v1/providers/ollama/generate": {
      "post": {
        "tags": ["Providers"],
        "summary": "Generate text via local Ollama (blocking)",
        "operationId": "ollamaGenerate",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "model": { "type": "string" }, "prompt": { "type": "string" }, "temperature": { "type": "number" }, "max_tokens": { "type": "integer" } } } } } },
        "responses": { "200": { "description": "Generated text + metadata" } }
      }
    },
    "/v1/providers/ollama/stream": {
      "post": {
        "tags": ["Providers"],
        "summary": "Stream tokens from Ollama via SSE",
        "operationId": "ollamaStream",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object" } } } },
        "responses": { "200": { "description": "SSE stream: token / done / error events" } }
      }
    },
    "/v1/providers/ollama/pull": {
      "post": {
        "tags": ["Providers"],
        "summary": "Pull an Ollama model",
        "operationId": "ollamaPull",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "model": { "type": "string" } } } } } },
        "responses": { "200": { "description": "Pull started" } }
      }
    },
    "/v1/memory/ingest": {
      "post": {
        "tags": ["Memory"],
        "summary": "Ingest a document into the knowledge store",
        "operationId": "memoryIngest",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "tenant_id": { "type": "string" }, "workspace_id": { "type": "string" }, "project_id": { "type": "string" }, "source_id": { "type": "string" }, "document_id": { "type": "string" }, "content": { "type": "string" }, "source_type": { "type": "string" } } } } } },
        "responses": { "200": { "description": "Ingestion result" } }
      }
    },
    "/v1/memory/search": {
      "get": {
        "tags": ["Memory"],
        "summary": "Lexical retrieval over the knowledge store",
        "operationId": "memorySearch",
        "parameters": [
          { "name": "query_text",   "in": "query", "required": true,  "schema": { "type": "string"  } },
          { "name": "tenant_id",    "in": "query", "required": false, "schema": { "type": "string"  } },
          { "name": "workspace_id", "in": "query", "required": false, "schema": { "type": "string"  } },
          { "name": "project_id",   "in": "query", "required": false, "schema": { "type": "string"  } },
          { "name": "limit",        "in": "query", "required": false, "schema": { "type": "integer" } }
        ],
        "responses": { "200": { "description": "Ranked search results" } }
      }
    },
    "/v1/memory/embed": {
      "post": {
        "tags": ["Memory"],
        "summary": "Embed texts via Ollama",
        "operationId": "memoryEmbed",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "texts": { "type": "array", "items": { "type": "string" } }, "model": { "type": "string" } } } } } },
        "responses": { "200": { "description": "Embedding vectors" } }
      }
    },
    "/v1/events": {
      "get": {
        "tags": ["Events"],
        "summary": "Cursor-based replay of the global event log",
        "description": "Returns up to `limit` events strictly after `after` position. Use `Last-Event-ID` on reconnect.",
        "operationId": "listEvents",
        "parameters": [
          { "name": "after", "in": "query", "schema": { "type": "integer", "description": "Return events after this position" } },
          { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 100 } }
        ],
        "responses": { "200": { "description": "Event page" } }
      }
    },
    "/v1/events/append": {
      "post": {
        "tags": ["Events"],
        "summary": "Append events (idempotent write)",
        "description": "Accepts an array of `EventEnvelope<RuntimeEvent>`. Causation-ID deduplication ensures at-least-once safety.",
        "operationId": "appendEvents",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/EventEnvelope" } } } } },
        "responses": {
          "201": { "description": "Append results", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/AppendResult" } } } } }
        }
      }
    },
    "/v1/stream": {
      "get": {
        "tags": ["Events"],
        "summary": "Real-time SSE event stream",
        "description": "Emits live events. On connect a `connected` event carries the current head position. Reconnect with `Last-Event-ID` to replay up to 1 000 missed events. No auth required.",
        "security": [],
        "operationId": "streamEvents",
        "responses": { "200": { "description": "SSE stream", "content": { "text/event-stream": {} } } }
      }
    },
    "/v1/evals/runs": {
      "get": {
        "tags": ["Evals"],
        "summary": "List eval runs",
        "operationId": "listEvalRuns",
        "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "default": 100 } }],
        "responses": { "200": { "description": "Eval run list" } }
      }
    },
    "/v1/traces": {
      "get": {
        "tags": ["Evals"],
        "summary": "All recent LLM call traces",
        "operationId": "listTraces",
        "parameters": [{ "name": "limit", "in": "query", "schema": { "type": "integer", "default": 500 } }],
        "responses": { "200": { "description": "LLM call traces" } }
      }
    },
    "/v1/costs": {
      "get": {
        "tags": ["Evals"],
        "summary": "Aggregate cost summary",
        "operationId": "getCosts",
        "responses": { "200": { "description": "Cost totals (calls, tokens, USD micros)" } }
      }
    },
    "/v1/admin/audit-log": {
      "get": {
        "tags": ["Admin"],
        "summary": "Audit log entries for the operator tenant",
        "operationId": "listAuditLog",
        "parameters": [
          { "name": "limit",    "in": "query", "schema": { "type": "integer", "default": 100 } },
          { "name": "since_ms", "in": "query", "schema": { "type": "integer" } }
        ],
        "responses": { "200": { "description": "Audit entries" } }
      }
    },
    "/v1/admin/tenants": {
      "post": {
        "tags": ["Admin"],
        "summary": "Create a new tenant",
        "operationId": "createTenant",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "tenant_id": { "type": "string" }, "name": { "type": "string" } }, "required": ["tenant_id", "name"] } } } },
        "responses": { "201": { "description": "Created tenant" } }
      }
    },
    "/v1/settings": {
      "get": {
        "tags": ["Admin"],
        "summary": "Deployment configuration",
        "operationId": "getSettings",
        "responses": { "200": { "description": "Settings including mode, backend, feature flags" } }
      }
    },
    "/v1/db/status": {
      "get": {
        "tags": ["Admin"],
        "summary": "Database health and migration state",
        "operationId": "getDbStatus",
        "responses": { "200": { "description": "Backend type, connected flag, migration count" } }
      }
    },
    "/v1/metrics": {
      "get": {
        "tags": ["Admin"],
        "summary": "JSON request metrics",
        "operationId": "getMetrics",
        "responses": { "200": { "description": "Rolling latency percentiles, request counts, error rate" } }
      }
    },
    "/v1/metrics/prometheus": {
      "get": {
        "tags": ["Admin"],
        "summary": "Prometheus-format metrics scrape endpoint",
        "operationId": "getMetricsPrometheus",
        "responses": { "200": { "description": "text/plain Prometheus exposition format" } }
      }
    },
    "/v1/openapi.json": {
      "get": {
        "tags": ["Admin"],
        "summary": "OpenAPI 3.0 specification",
        "security": [],
        "operationId": "getOpenApiSpec",
        "responses": { "200": { "description": "This document", "content": { "application/json": {} } } }
      }
    },
    "/v1/docs": {
      "get": {
        "tags": ["Admin"],
        "summary": "Swagger UI",
        "security": [],
        "operationId": "getSwaggerUi",
        "responses": { "200": { "description": "Interactive API explorer", "content": { "text/html": {} } } }
      }
    }
  }
}"##;

/// Swagger UI HTML — loads the CDN bundle and points it at `/v1/openapi.json`.
pub const SWAGGER_UI_HTML: &str = r##"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>Cairn API Docs</title>
  <link rel="stylesheet" href="https://unpkg.com/swagger-ui-dist@5/swagger-ui.css" />
  <style>
    body { margin: 0; background: #09090b; }
    .swagger-ui .topbar { background: #18181b; border-bottom: 1px solid #27272a; }
    .swagger-ui .topbar .download-url-wrapper { display: none; }
    .swagger-ui .info .title { color: #e4e4e7; }
    .swagger-ui .scheme-container { background: #18181b; }
  </style>
</head>
<body>
  <div id="swagger-ui"></div>
  <script src="https://unpkg.com/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
  <script>
    SwaggerUIBundle({
      url: "/v1/openapi.json",
      dom_id: "#swagger-ui",
      presets: [SwaggerUIBundle.presets.apis, SwaggerUIBundle.SwaggerUIStandalonePreset],
      layout: "BaseLayout",
      deepLinking: true,
      persistAuthorization: true,
      tryItOutEnabled: true,
    });
  </script>
</body>
</html>"##;
