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
    "description": "Self-hostable control plane for production AI agent deployments.\n\nAll `/v1/` endpoints require `Authorization: Bearer <token>`. `/health` and `/v1/docs` are public. `/v1/stream` requires bearer auth via the `?token=` query parameter (browsers cannot set custom headers on SSE connections).\n\n**Database:** Set `DATABASE_URL=postgres://user:pass@host/db` for persistent storage, or `--db memory` for ephemeral in-memory mode.\n\n**Rate limiting:** All responses include `X-RateLimit-Limit`, `X-RateLimit-Remaining`, and `X-RateLimit-Reset` headers. Token-authenticated requests: 1000 req/min. IP-only: 100 req/min. Exceeded requests return `429` with `Retry-After`.",
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
      "CommandOutcome": {
        "type": "object",
        "description": "F47: one bash-class tool invocation distilled from a tool_result frame. `exit_code` is always emitted; the value is `null` when the tool_result did not structurally expose one. The extractor never fabricates exit codes — do not infer success from a missing code.",
        "properties": {
          "tool_name": { "type": "string", "description": "Tool name from the proposal (e.g. `bash`, `shell_exec`)." },
          "cmd":       { "type": "string", "description": "For bash-class tools, the `command` argument. Truncated to 500 chars." },
          "exit_code": { "type": "integer", "nullable": true, "description": "Exit code surfaced by the tool_result, or `null` when not structurally exposed." }
        },
        "required": ["tool_name", "cmd", "exit_code"]
      },
      "CompletionVerification": {
        "type": "object",
        "description": "F47 PR1 sidecar attached to the `orchestrate_finished` SSE event on `termination=completed` runs. Warning / error lines extracted from tool_result text give operators an independent signal alongside the LLM's free-text `summary`. Non-authoritative: the extractor reports what tool outputs say, not whether the run succeeded.",
        "properties": {
          "warnings": {
            "type": "array",
            "description": "Tool-output lines matched by the warning signal (e.g. `warning: unused import`). Full matched line, truncated to 500 chars. Capped at 50 entries.",
            "items": { "type": "string" }
          },
          "errors": {
            "type": "array",
            "description": "Tool-output lines matched by the error signal (e.g. `error[E0308]:`, `error:`). Same truncation / cap rules as warnings.",
            "items": { "type": "string" }
          },
          "commands": {
            "type": "array",
            "description": "Per-bash-class-tool invocations: command text and (optional) exit code.",
            "items": { "$ref": "#/components/schemas/CommandOutcome" }
          },
          "tool_results_scanned": {
            "type": "integer",
            "description": "How many InvokeTool results were scanned to produce this summary. `0` means Done reached with no recorded tool calls."
          },
          "extractor_version": {
            "type": "integer",
            "description": "Version of the extractor logic (1 = F47 PR1). Bumped when the matching or truncation policy changes."
          }
        },
        "required": ["warnings", "errors", "commands", "tool_results_scanned", "extractor_version"]
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
      "DependencyKind": {
        "type": "string",
        "enum": ["success_only"],
        "description": "Edge-kind taxonomy for task dependencies. Mirrors FF 0.2's `dependency_kind` FCALL argument. Today only `success_only` is supported (downstream becomes eligible when upstream terminates successfully; any non-success outcome cascades as skipped)."
      },
      "TaskDependency": {
        "type": "object",
        "properties": {
          "dependent_task_id":   { "type": "string" },
          "depends_on_task_id":  { "type": "string" },
          "project":             { "$ref": "#/components/schemas/ProjectKey" },
          "created_at_ms":       { "type": "integer" },
          "dependency_kind":     { "$ref": "#/components/schemas/DependencyKind" },
          "data_passing_ref":    { "type": "string", "nullable": true, "maxLength": 256, "pattern": "^[A-Za-z0-9._:/-]*$" }
        },
        "required": ["dependent_task_id","depends_on_task_id","project","created_at_ms"]
      },
      "TaskDependencyRecord": {
        "type": "object",
        "properties": {
          "dependency":     { "$ref": "#/components/schemas/TaskDependency" },
          "resolved_at_ms": { "type": "integer", "nullable": true }
        },
        "required": ["dependency"]
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
          "created_at":   { "type": "integer" },
          "updated_at":   { "type": "integer" }
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
      },
      "TemplateSummary": {
        "type": "object",
        "properties": {
          "id":          { "type": "string" },
          "name":        { "type": "string" },
          "description": { "type": "string" },
          "category":    { "type": "string", "enum": ["chatbot","code_assistant","data_pipeline","customer_support"] },
          "file_count":  { "type": "integer" }
        },
        "required": ["id", "name", "description", "category", "file_count"]
      },
      "TemplateFile": {
        "type": "object",
        "properties": {
          "path":        { "type": "string", "description": "Relative file path within the template" },
          "description": { "type": "string" },
          "content":     { "type": "string" }
        },
        "required": ["path", "description", "content"]
      },
      "Template": {
        "type": "object",
        "properties": {
          "id":          { "type": "string" },
          "name":        { "type": "string" },
          "description": { "type": "string" },
          "category":    { "type": "string", "enum": ["chatbot","code_assistant","data_pipeline","customer_support"] },
          "files":       { "type": "array", "items": { "$ref": "#/components/schemas/TemplateFile" } }
        },
        "required": ["id", "name", "description", "category", "files"]
      },
      "ApplyTemplateRequest": {
        "type": "object",
        "properties": {
          "project_id": { "type": "string" }
        },
        "required": ["project_id"]
      },
      "ApplyTemplateResult": {
        "type": "object",
        "properties": {
          "template_id":   { "type": "string" },
          "project_id":    { "type": "string" },
          "files_created": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["template_id", "project_id", "files_created"]
      },
      "UsageReport": {
        "type": "object",
        "properties": {
          "tenant_id":            { "type": "string" },
          "tier":                 { "type": "string", "enum": ["free","pro","enterprise"] },
          "sessions_used":        { "type": "integer" },
          "max_sessions":         { "type": "integer" },
          "runs_today":           { "type": "integer" },
          "max_runs_per_day":     { "type": "integer" },
          "tokens_this_month":    { "type": "integer", "format": "int64" },
          "max_tokens_per_month": { "type": "integer", "format": "int64" },
          "features_enabled":     { "type": "array", "items": { "type": "string" } }
        },
        "required": ["tenant_id", "tier"]
      },
      "ResourceUsage": {
        "type": "object",
        "properties": {
          "used":         { "type": "integer" },
          "limit":        { "type": "integer" },
          "remaining":    { "type": "integer" },
          "percent_used": { "type": "number", "format": "double" }
        }
      },
      "DetailedUsageReport": {
        "type": "object",
        "properties": {
          "tenant_id": { "type": "string" },
          "tier":      { "type": "string", "enum": ["free","pro","enterprise"] },
          "sessions":  { "$ref": "#/components/schemas/ResourceUsage" },
          "runs":      { "$ref": "#/components/schemas/ResourceUsage" },
          "tokens":    { "$ref": "#/components/schemas/ResourceUsage" }
        },
        "required": ["tenant_id", "tier", "sessions", "runs", "tokens"]
      },
      "SystemInfo": {
        "type": "object",
        "properties": {
          "version":         { "type": "string" },
          "deployment_mode": { "type": "string", "enum": ["local","self_hosted_team"] },
          "store_backend":   { "type": "string", "enum": ["memory","postgres"] },
          "uptime_secs":     { "type": "integer" },
          "capabilities":    { "type": "object" },
          "environment":     { "type": "object" }
        }
      },
      "SystemRole": {
        "type": "object",
        "properties": {
          "role":        { "type": "string", "description": "Process role: all, api, worker" },
          "serves_http": { "type": "boolean" },
          "runs_workers": { "type": "boolean" }
        },
        "required": ["role", "serves_http", "runs_workers"]
      },
      "EventCountResponse": {
        "type": "object",
        "properties": {
          "total":   { "type": "integer", "format": "int64" },
          "by_type": { "type": "object", "additionalProperties": { "type": "integer" } }
        },
        "required": ["total", "by_type"]
      },
      "RebuildProjectionsResponse": {
        "type": "object",
        "properties": {
          "ok":               { "type": "boolean" },
          "events_replayed":  { "type": "integer" },
          "duration_ms":      { "type": "integer" }
        }
      },
      "ExportBundleRequest": {
        "type": "object",
        "properties": {
          "project_id": { "type": "string", "nullable": true },
          "format":     { "type": "string", "enum": ["json","yaml"], "default": "json" }
        }
      },
      "ApplyBundleRequest": {
        "type": "object",
        "properties": {
          "project_id":        { "type": "string" },
          "bundle":            { "type": "object", "description": "Full CairnBundle envelope" },
          "conflict_strategy": { "type": "string", "enum": ["skip","overwrite","rename"], "default": "skip" },
          "existing_ids":      { "type": "array", "items": { "type": "string" }, "default": [] }
        },
        "required": ["project_id", "bundle"]
      },
      "WorkspaceUsageReport": {
        "type": "object",
        "properties": {
          "workspace_id":        { "type": "string" },
          "active_runs":         { "type": "integer" },
          "max_concurrent_runs": { "type": "integer" },
          "runs_this_hour":      { "type": "integer" },
          "max_runs_per_hour":   { "type": "integer" },
          "tokens_today":        { "type": "integer", "format": "int64" },
          "max_tokens_per_day":  { "type": "integer", "format": "int64" },
          "storage_mb":          { "type": "integer", "format": "int64" },
          "max_storage_mb":      { "type": "integer", "format": "int64" }
        },
        "required": ["workspace_id"]
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
    "/v1/runs/{id}/claim": {
      "post": {
        "tags": ["Runs"],
        "summary": "Claim a run's execution lease (Fabric-only semantic; no-op on in-memory path)",
        "description": "Activates the run's FF execution so downstream FCALLs (pause / enter_waiting_approval / resolve_approval / signals) accept it. Unlike POST /v1/tasks/{id}/claim, this endpoint takes no body: runs are not worker-pulled, so the caller never advertises worker identity here — the Fabric runtime uses its own configured worker_instance_id + lease_ttl_ms. NOT idempotent — re-claiming an already-active run fails at FF's grant gate (`execution_not_eligible`) and surfaces as a 500. Callers must claim once per lifecycle. A second claim after a suspend/resume cycle is legitimate and dispatches through FF's `ff_claim_resumed_execution` path.",
        "operationId": "claimRun",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": {
          "200": { "description": "Run record after active-lease activation" },
          "404": { "description": "Run not found" },
          "500": { "description": "Underlying runtime error (including re-claim of an already-active run)" }
        }
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
    "/v1/runs/{id}/telemetry": {
      "get": {
        "tags": ["Runs", "Observability"],
        "summary": "Live-aggregated per-run telemetry (provider calls + tool invocations + totals)",
        "description": "Returns the run state + stuck flag, every provider call with model/tokens/cost/latency, every tool invocation with duration, and running totals suitable for the operator observability panel. Aggregated at read time from the InMemory projection.",
        "operationId": "getRunTelemetry",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": {
          "200": { "description": "Run telemetry payload" },
          "404": { "description": "Run not found or not visible to tenant" }
        }
      }
    },
    "/v1/projects/{tenant}/{workspace}/{project}/costs": {
      "get": {
        "tags": ["Costs", "Observability"],
        "summary": "Lifetime cost rollup for a project (F29 CD-2)",
        "description": "Returns the lifetime cost, token, and provider-call totals for every session under the given (tenant, workspace, project) triple. Zeros are returned when the project has not emitted any provider calls yet. Time-range slicing is a follow-up; v1 is lifetime-total.",
        "operationId": "getProjectCosts",
        "parameters": [
          { "name": "tenant",    "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "workspace", "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "project",   "in": "path", "required": true, "schema": { "type": "string" } }
        ],
        "responses": {
          "200": { "description": "Project cost summary" },
          "403": { "description": "Tenant scope mismatch" }
        }
      }
    },
    "/v1/workspaces/{tenant}/{workspace}/costs": {
      "get": {
        "tags": ["Costs", "Observability"],
        "summary": "Lifetime cost rollup for a workspace (F29 CD-2)",
        "description": "Returns the lifetime cost, token, and provider-call totals aggregated across every project in the given (tenant, workspace). Zeros are returned when the workspace has not emitted any provider calls yet.",
        "operationId": "getWorkspaceCosts",
        "parameters": [
          { "name": "tenant",    "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "workspace", "in": "path", "required": true, "schema": { "type": "string" } }
        ],
        "responses": {
          "200": { "description": "Workspace cost summary" },
          "403": { "description": "Tenant scope mismatch" }
        }
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
    "/v1/tasks/{id}/dependencies": {
      "get": {
        "tags": ["Tasks"],
        "summary": "List unresolved prerequisite tasks blocking this task",
        "operationId": "listTaskDependencies",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "TaskDependencyRecord array (empty means no active blockers)" } }
      },
      "post": {
        "tags": ["Tasks"],
        "summary": "Declare a task-level dependency (FF flow edge)",
        "description": "Both tasks must share the same session (FF flows are session-scoped). Cross-session, cross-project, or self-dependency declares are rejected with 422. Re-declaring an existing edge with a different `dependency_kind` or `data_passing_ref` returns 409 dependency_conflict; identical replay is idempotent 201. `data_passing_ref` is an opaque caller-supplied string forwarded to FF edge storage — cairn never dereferences it. Downstream consumers are responsible for interpreting the value.",
        "operationId": "addTaskDependency",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "properties": {
                  "depends_on_task_id": { "type": "string", "description": "Prerequisite task id (must share session with the dependent task)." },
                  "dependency_kind":    { "type": "string", "enum": ["success_only"], "default": "success_only", "description": "Edge kind. Today only `success_only` is supported." },
                  "data_passing_ref":   { "type": "string", "maxLength": 256, "pattern": "^[A-Za-z0-9._:/-]*$", "nullable": true, "description": "Opaque reference stored on the FF edge. Charset limited for round-trip safety; empty string treated as absent." }
                },
                "required": ["depends_on_task_id"]
              }
            }
          }
        },
        "responses": {
          "201": { "description": "Dependency record (TaskDependencyRecord)" },
          "404": { "description": "One of the tasks not found" },
          "409": { "description": "Edge already exists with different kind/ref (dependency_conflict)" },
          "422": { "description": "Cross-session, self-dependency, or invalid data_passing_ref" }
        }
      }
    },
    "/v1/approvals": {
      "get": {
        "tags": ["Approvals"],
        "summary": "List approvals (unified — plan + tool-call, F45)",
        "description": "Merged operator inbox across both approval kinds. Every item carries a `kind` discriminator (`plan` | `tool_call`). Plan-approval rows flatten `ApprovalRecord`; tool-call rows flatten `ToolCallApprovalRecord`. Supersedes the pre-F45 `/v1/tool-call-approvals` list, which now 308-redirects here.",
        "operationId": "listApprovals",
        "parameters": [
          { "name": "kind",         "in": "query", "schema": { "type": "string", "enum": ["plan","tool_call"] }, "description": "Narrow to one kind; absent = both." },
          { "name": "state",        "in": "query", "schema": { "type": "string", "enum": ["pending","approved","rejected","timeout"] } },
          { "name": "run_id",       "in": "query", "schema": { "type": "string" } },
          { "name": "session_id",   "in": "query", "schema": { "type": "string" }, "description": "Tool-call native; excludes plan approvals when set." },
          { "name": "tenant_id",    "in": "query", "schema": { "type": "string" } },
          { "name": "workspace_id", "in": "query", "schema": { "type": "string" } },
          { "name": "project_id",   "in": "query", "schema": { "type": "string" } },
          { "name": "limit",        "in": "query", "schema": { "type": "integer", "default": 100 } },
          { "name": "offset",       "in": "query", "schema": { "type": "integer", "default": 0 } }
        ],
        "responses": { "200": { "description": "Merged approval records, newest first" } }
      }
    },
    "/v1/approvals/pending": {
      "get": {
        "tags": ["Approvals"],
        "summary": "List pending plan approvals",
        "operationId": "listPendingApprovals",
        "responses": { "200": { "description": "Pending plan approvals" } }
      }
    },
    "/v1/approvals/{id}": {
      "get": {
        "tags": ["Approvals"],
        "summary": "Fetch any approval by id (unified, F45)",
        "description": "Resolves tool-call first, then plan. Response carries a `kind` discriminator.",
        "operationId": "getApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": {
          "200": { "description": "Unified approval record" },
          "404": { "description": "Not found (or cross-tenant)" }
        }
      }
    },
    "/v1/approvals/{id}/approve": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Approve an approval (kind-aware)",
        "description": "For plan approvals the body is ignored. For tool-call approvals `scope` is required: `{type:\"once\"}` resolves this call only; `{type:\"session\", match_policy?}` widens to matching calls in the same session (omitted `match_policy` inherits the proposal's). `approved_tool_args` overrides any prior amendment. `operator_id` in the body must match the authenticated principal when present (else 400 `identity_mismatch`).",
        "operationId": "approveApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": false, "content": { "application/json": { "schema": { "type": "object", "properties": {
          "operator_id": { "type": "string" },
          "scope": { "type": "object", "oneOf": [
            { "type": "object", "properties": { "type": { "type": "string", "enum": ["once"] } }, "required": ["type"] },
            { "type": "object", "properties": { "type": { "type": "string", "enum": ["session"] }, "match_policy": { "type": "object" } }, "required": ["type"] }
          ] },
          "approved_tool_args": {}
        } } } } },
        "responses": {
          "200": { "description": "Approved" },
          "400": { "description": "operator_id in body does not match authenticated principal" },
          "404": { "description": "Unknown id" },
          "409": { "description": "Approval already resolved" },
          "422": { "description": "tool-call approval missing required `scope`" }
        }
      }
    },
    "/v1/approvals/{id}/reject": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Reject an approval (kind-aware)",
        "operationId": "rejectApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": false, "content": { "application/json": { "schema": { "type": "object", "properties": {
          "operator_id": { "type": "string" },
          "reason":      { "type": "string" }
        } } } } },
        "responses": {
          "200": { "description": "Rejected" },
          "400": { "description": "operator_id mismatch" },
          "404": { "description": "Unknown id" },
          "409": { "description": "Approval already resolved" }
        }
      }
    },
    "/v1/approvals/{id}/deny": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Alias of /reject (legacy route)",
        "operationId": "denyApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Rejected" } }
      }
    },
    "/v1/approvals/{id}/amend": {
      "patch": {
        "tags": ["Approvals"],
        "summary": "Amend tool-call arguments (tool-call kind only)",
        "description": "Non-resolving — operator must still approve/reject. Returns 422 `unsupported_on_plan_approval` when the id points at a plan approval, and 403 `self_amend_forbidden` if the proposal's `tool_name` is `amend_approval`.",
        "operationId": "amendApproval",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": {
          "operator_id":   { "type": "string" },
          "new_tool_args": {}
        }, "required": ["new_tool_args"] } } } },
        "responses": {
          "200": { "description": "Amended; state remains pending" },
          "400": { "description": "operator_id mismatch" },
          "403": { "description": "Cannot amend amend_approval tool calls" },
          "404": { "description": "Unknown id" },
          "409": { "description": "Proposal already resolved" },
          "422": { "description": "Amend not supported on plan approvals" }
        }
      }
    },
    "/v1/tool-call-approvals": {
      "get": {
        "tags": ["Approvals"],
        "summary": "Deprecated — 308-redirects to /v1/approvals?kind=tool_call",
        "description": "Deprecated in F45. Clients should call `/v1/approvals?kind=tool_call`. This path 308-redirects (preserves method + body); response carries `Deprecation: true`.",
        "operationId": "listToolCallApprovals",
        "deprecated": true,
        "responses": { "308": { "description": "Permanent Redirect to /v1/approvals" } }
      }
    },
    "/v1/tool-call-approvals/{call_id}": {
      "get": {
        "tags": ["Approvals"],
        "summary": "Deprecated — 308-redirects to /v1/approvals/{id}",
        "operationId": "getToolCallApproval",
        "deprecated": true,
        "parameters": [{ "name": "call_id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "308": { "description": "Permanent Redirect" } }
      }
    },
    "/v1/tool-call-approvals/{call_id}/approve": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Deprecated — 308-redirects to /v1/approvals/{id}/approve",
        "operationId": "approveToolCallApproval",
        "deprecated": true,
        "parameters": [{ "name": "call_id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "308": { "description": "Permanent Redirect" } }
      }
    },
    "/v1/tool-call-approvals/{call_id}/reject": {
      "post": {
        "tags": ["Approvals"],
        "summary": "Deprecated — 308-redirects to /v1/approvals/{id}/reject",
        "operationId": "rejectToolCallApproval",
        "deprecated": true,
        "parameters": [{ "name": "call_id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "308": { "description": "Permanent Redirect" } }
      }
    },
    "/v1/tool-call-approvals/{call_id}/amend": {
      "patch": {
        "tags": ["Approvals"],
        "summary": "Deprecated — 308-redirects to /v1/approvals/{id}/amend",
        "operationId": "amendToolCallApproval",
        "deprecated": true,
        "parameters": [{ "name": "call_id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "308": { "description": "Permanent Redirect" } }
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
    "/v1/providers/connections": {
      "get": {
        "tags": ["Providers"],
        "summary": "List provider connections",
        "operationId": "listProviderConnections",
        "parameters": [
          { "name": "tenant_id", "in": "query", "required": true, "schema": { "type": "string" } },
          { "name": "limit",     "in": "query", "schema": { "type": "integer", "default": 50 } },
          { "name": "offset",    "in": "query", "schema": { "type": "integer", "default": 0 } }
        ],
        "responses": { "200": { "description": "Provider connection list" } }
      },
      "post": {
        "tags": ["Providers"],
        "summary": "Register a provider connection (entitlement-gated)",
        "description": "Creates a new provider connection. Requires a tier that supports external providers (returns 403 in local_eval tier).",
        "operationId": "createProviderConnection",
        "requestBody": { "required": true, "content": { "application/json": { "schema": { "type": "object", "properties": { "tenant_id": { "type": "string" }, "provider_connection_id": { "type": "string" }, "provider_family": { "type": "string" }, "adapter_type": { "type": "string" } }, "required": ["tenant_id", "provider_connection_id", "provider_family", "adapter_type"] } } } },
        "responses": {
          "201": { "description": "Provider connection created" },
          "403": { "description": "Entitlement tier does not allow external provider connections" }
        }
      }
    },
    "/v1/models/catalog": {
      "get": {
        "tags": ["Models"],
        "summary": "List the bundled model catalog (LiteLLM + cairn overlay)",
        "description": "Read-only projection of the bundled LiteLLM pricing catalog plus any cairn TOML overlay and operator overrides. Supports filter, search, capability filters, cost ceiling, free-only shortcut, and pagination. Callable by any authenticated operator — the UI provider wizard and cost calculator read from here.",
        "operationId": "listModelCatalog",
        "parameters": [
          { "name": "provider",          "in": "query", "schema": { "type": "string" }, "description": "Exact-match provider family (e.g. openai, anthropic, openrouter)." },
          { "name": "tier",              "in": "query", "schema": { "type": "string", "enum": ["brain", "mid", "light"] }, "description": "Routing tier." },
          { "name": "search",            "in": "query", "schema": { "type": "string" }, "description": "Case-insensitive substring across id, display_name, and provider." },
          { "name": "supports_tools",    "in": "query", "schema": { "type": "boolean" } },
          { "name": "supports_json_mode","in": "query", "schema": { "type": "boolean" } },
          { "name": "reasoning",         "in": "query", "schema": { "type": "boolean" } },
          { "name": "max_cost_per_1m",   "in": "query", "schema": { "type": "number" }, "description": "Upper bound on cost_per_1m_input (USD)." },
          { "name": "free_only",         "in": "query", "schema": { "type": "boolean" }, "description": "When true, only models with zero input+output cost." },
          { "name": "limit",             "in": "query", "schema": { "type": "integer", "default": 100, "maximum": 1000 } },
          { "name": "offset",            "in": "query", "schema": { "type": "integer", "default": 0 } }
        ],
        "responses": {
          "200": {
            "description": "Filtered, paginated model list",
            "content": { "application/json": { "schema": { "type": "object", "properties": {
              "items":   { "type": "array", "items": { "type": "object" } },
              "total":   { "type": "integer" },
              "hasMore": { "type": "boolean" }
            }, "required": ["items", "total", "hasMore"] } } }
          },
          "422": { "description": "Validation error (invalid limit/offset/tier)" },
          "503": { "description": "model_catalog_unavailable — bundled catalog is empty" }
        }
      }
    },
    "/v1/models/catalog/providers": {
      "get": {
        "tags": ["Models"],
        "summary": "Unique provider families in the model catalog, with counts",
        "description": "Lets the UI build a provider-filter dropdown without a full catalog scan. Cached after the first call for the process lifetime; admin CRUD overrides do NOT invalidate this cache.",
        "operationId": "listCatalogProviders",
        "responses": {
          "200": {
            "description": "Providers with entry counts",
            "content": { "application/json": { "schema": { "type": "object", "properties": {
              "providers": { "type": "array", "items": { "type": "object", "properties": {
                "name":  { "type": "string" },
                "count": { "type": "integer" }
              }, "required": ["name", "count"] } }
            }, "required": ["providers"] } } }
          },
          "503": { "description": "model_catalog_unavailable" }
        }
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
    "/v1/chat/stream": {
      "post": {
        "tags": ["Chat"],
        "summary": "Stream tokens from any configured LLM provider via SSE",
        "operationId": "chatStream",
        "description": "Routes to the first available provider: Bedrock, Ollama, OpenAI-compat brain, worker, or OpenRouter.",
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
    "/v1/evals/rubrics": {
      "get": {
        "tags": ["Evals"],
        "summary": "List rubrics for a tenant (issue #138)",
        "operationId": "listEvalRubrics",
        "parameters": [{ "name": "tenant_id", "in": "query", "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Eval rubric list" } }
      }
    },
    "/v1/evals/baselines": {
      "get": {
        "tags": ["Evals"],
        "summary": "List baselines for a tenant (issue #138)",
        "operationId": "listEvalBaselines",
        "parameters": [{ "name": "tenant_id", "in": "query", "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Eval baseline list" } }
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
    "/v1/settings/defaults/{scope}/{scope_id}/{key}": {
      "get": {
        "tags": ["Admin"],
        "summary": "Fetch a single stored default setting by exact scope",
        "description": "Returns the stored default at the exact `(scope, scope_id, key)` triple. This endpoint is exact-lookup — for fallback resolution across the scope cascade use `GET /v1/settings/defaults/resolve/{key}?project=...`. 404 when no value has been persisted at this triple.",
        "operationId": "getDefaultSetting",
        "parameters": [
          { "name": "scope", "in": "path", "required": true, "schema": { "type": "string", "enum": ["system", "tenant", "workspace", "project"] } },
          { "name": "scope_id", "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "key", "in": "path", "required": true, "schema": { "type": "string" } }
        ],
        "responses": {
          "200": { "description": "`{ scope, scope_id, key, value, source }`" },
          "404": { "description": "Setting not set at this scope" }
        }
      },
      "put": {
        "tags": ["Admin"],
        "summary": "Set a scoped default setting",
        "description": "Persists `value` as the default for `key` at the given scope. Scope layers cascade: System < Tenant < Workspace < Project (project overrides tenant overrides system). For `scope=system`, the conventional `scope_id` is `system`. For Tenant/Workspace/Project, `scope_id` is the respective entity id. Model-id keys (`brain_model`, `generate_model`, `stream_model`, `embed_model`) validate the value against the LiteLLM catalog union'd with every in-scope provider connection's `supported_models` — 422 with an actionable message if unknown.",
        "operationId": "setDefaultSetting",
        "parameters": [
          { "name": "scope", "in": "path", "required": true, "schema": { "type": "string", "enum": ["system", "tenant", "workspace", "project"] } },
          { "name": "scope_id", "in": "path", "required": true, "schema": { "type": "string" }, "description": "Scope entity id (`system` for system scope)." },
          { "name": "key", "in": "path", "required": true, "schema": { "type": "string" }, "description": "Setting key (e.g. `brain_model`, `generate_model`, `max_tokens`, `temperature`)." }
        ],
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "type": "object", "required": ["value"], "properties": { "value": { "description": "Setting value — type depends on the key (string for models, number for tokens/temperature)." } } } } }
        },
        "responses": {
          "200": { "description": "Persisted setting" },
          "400": { "description": "Invalid scope or malformed body" },
          "422": { "description": "Value failed per-key validation (unknown model, out-of-range numeric, oversized string)" }
        }
      },
      "delete": {
        "tags": ["Admin"],
        "summary": "Clear a scoped default setting",
        "operationId": "clearDefaultSetting",
        "parameters": [
          { "name": "scope", "in": "path", "required": true, "schema": { "type": "string", "enum": ["system", "tenant", "workspace", "project"] } },
          { "name": "scope_id", "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "key", "in": "path", "required": true, "schema": { "type": "string" } }
        ],
        "responses": { "200": { "description": "Cleared; resolution now falls through to the next scope layer or the hardcoded default" } }
      }
    },
    "/v1/settings/defaults/all": {
      "get": {
        "tags": ["Admin"],
        "summary": "List every persisted default setting across all scopes",
        "description": "Flat list of all settings explicitly set via `PUT /v1/settings/defaults/...`. Unset keys are omitted. For the effective value of a specific key with fallback resolution, use `GET /v1/settings/defaults/resolve/{key}?project=...`.",
        "operationId": "listAllDefaultSettings",
        "responses": { "200": { "description": "`{ settings: [...], total: n }`" } }
      }
    },
    "/v1/settings/defaults/resolve/{key}": {
      "get": {
        "tags": ["Admin"],
        "summary": "Resolve the effective default for a key",
        "description": "Walks the scope cascade (Project → Workspace → Tenant → System → env → hardcoded) and returns the first layer's value. Requires `?project=<project_id>` to anchor the resolution.",
        "operationId": "resolveDefaultSetting",
        "parameters": [
          { "name": "key", "in": "path", "required": true, "schema": { "type": "string" } },
          { "name": "project", "in": "query", "required": true, "schema": { "type": "string" }, "description": "Project id anchoring the scope cascade." }
        ],
        "responses": { "200": { "description": "`{ key, value }`" } }
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
    },
    "/v1/system/info": {
      "get": {
        "tags": ["System"],
        "summary": "Comprehensive system information",
        "description": "Returns build metadata, runtime capabilities, deployment mode, store backend, uptime, and sanitised environment config (secrets masked).",
        "operationId": "getSystemInfo",
        "responses": {
          "200": { "description": "System info", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SystemInfo" } } } }
        }
      }
    },
    "/v1/system/role": {
      "get": {
        "tags": ["System"],
        "summary": "Current process deployment role",
        "description": "Returns the RFC 011 process role (all, api, worker) and which subsystems are active.",
        "operationId": "getSystemRole",
        "responses": {
          "200": { "description": "Process role", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/SystemRole" } } } }
        }
      }
    },
    "/v1/templates": {
      "get": {
        "tags": ["Templates"],
        "summary": "List starter templates",
        "description": "Returns summaries of all registered starter templates (RFC 012). Built-in templates include simple-chatbot, code-reviewer, and data-analyst.",
        "operationId": "listTemplates",
        "responses": {
          "200": { "description": "Template summaries", "content": { "application/json": { "schema": { "type": "array", "items": { "$ref": "#/components/schemas/TemplateSummary" } } } } }
        }
      }
    },
    "/v1/templates/{id}": {
      "get": {
        "tags": ["Templates"],
        "summary": "Get template detail",
        "description": "Returns the full template including all file contents (prompts, configs, eval suites).",
        "operationId": "getTemplate",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" }, "description": "Template ID (e.g. simple-chatbot, code-reviewer, data-analyst)" }],
        "responses": {
          "200": { "description": "Full template with files", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Template" } } } },
          "404": { "description": "Template not found" }
        }
      }
    },
    "/v1/templates/{id}/apply": {
      "post": {
        "tags": ["Templates"],
        "summary": "Apply template to a project",
        "description": "Scaffolds a project by creating the template's file tree under `projects/{project_id}/`. Returns the list of created file paths.",
        "operationId": "applyTemplate",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApplyTemplateRequest" } } }
        },
        "responses": {
          "200": { "description": "Files created", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApplyTemplateResult" } } } },
          "404": { "description": "Template not found" }
        }
      }
    },
    "/v1/entitlements": {
      "get": {
        "tags": ["Entitlements"],
        "summary": "Current plan and usage limits",
        "description": "Returns the tenant's plan tier, current usage counters, limits, and enabled features (RFC 014). Pass `?tenant_id=` to query a specific tenant; defaults to 'default'.",
        "operationId": "getEntitlements",
        "parameters": [
          { "name": "tenant_id", "in": "query", "required": false, "schema": { "type": "string", "default": "default" } }
        ],
        "responses": {
          "200": { "description": "Usage report", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/UsageReport" } } } },
          "404": { "description": "No plan assigned to tenant" }
        }
      }
    },
    "/v1/entitlements/usage": {
      "get": {
        "tags": ["Entitlements"],
        "summary": "Detailed usage breakdown",
        "description": "Per-resource usage with remaining capacity and percentage used. Useful for dashboard gauges and quota warnings.",
        "operationId": "getEntitlementUsage",
        "parameters": [
          { "name": "tenant_id", "in": "query", "required": false, "schema": { "type": "string", "default": "default" } }
        ],
        "responses": {
          "200": { "description": "Detailed usage", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/DetailedUsageReport" } } } },
          "404": { "description": "No plan assigned to tenant" }
        }
      }
    },
    "/v1/admin/rebuild-projections": {
      "post": {
        "tags": ["Admin"],
        "summary": "Rebuild all read-model projections",
        "description": "Performs a snapshot → replay cycle: exports the current event log and replays every event through `apply_projection`. Use after schema changes or bug fixes that affect projection logic.",
        "operationId": "rebuildProjections",
        "responses": {
          "200": { "description": "Rebuild result", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RebuildProjectionsResponse" } } } }
        }
      }
    },
    "/v1/admin/event-count": {
      "get": {
        "tags": ["Admin"],
        "summary": "Event log cardinality",
        "description": "Total event count and per-type breakdown. Useful for health checks and spotting unexpected event distributions.",
        "operationId": "getEventCount",
        "responses": {
          "200": { "description": "Event counts", "content": { "application/json": { "schema": { "$ref": "#/components/schemas/EventCountResponse" } } } }
        }
      }
    },
    "/v1/admin/event-log": {
      "get": {
        "tags": ["Admin"],
        "summary": "Raw event log viewer",
        "description": "Paginated raw event log with optional position-based cursor. Max 500 per page.",
        "operationId": "getEventLog",
        "parameters": [
          { "name": "from",  "in": "query", "schema": { "type": "integer", "default": 0 }, "description": "Start position (1-based)" },
          { "name": "limit", "in": "query", "schema": { "type": "integer", "default": 100, "maximum": 500 } }
        ],
        "responses": {
          "200": { "description": "Event page with has_more flag", "content": { "application/json": { "schema": { "type": "object", "properties": { "events": { "type": "array", "items": { "$ref": "#/components/schemas/EventEnvelope" } }, "has_more": { "type": "boolean" } } } } } }
        }
      }
    },
    "/v1/admin/snapshot": {
      "post": {
        "tags": ["Admin"],
        "summary": "Export full event log snapshot",
        "description": "Downloads the complete in-memory event log as a JSON file attachment. Use for backups before destructive operations.",
        "operationId": "createSnapshot",
        "responses": {
          "200": { "description": "JSON snapshot file", "content": { "application/json": { "schema": { "type": "object", "description": "StoreSnapshot with all events in position order" } } } }
        }
      }
    },
    "/v1/admin/restore": {
      "post": {
        "tags": ["Admin"],
        "summary": "Restore from snapshot",
        "description": "Clears all in-memory state and replays the uploaded event log. Irreversible — take a snapshot first.",
        "operationId": "restoreSnapshot",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "type": "object", "description": "StoreSnapshot previously exported via /v1/admin/snapshot" } } }
        },
        "responses": {
          "200": { "description": "Restore result", "content": { "application/json": { "schema": { "type": "object", "properties": { "ok": { "type": "boolean" }, "event_count": { "type": "integer" }, "replayed": { "type": "integer" } } } } } }
        }
      }
    },
    "/v1/admin/rotate-token": {
      "post": {
        "tags": ["Admin"],
        "summary": "Rotate admin bearer token at runtime",
        "description": "Replaces the active admin token with a new one. The old token is immediately revoked. Requires the current token in the Authorization header. The new token must be at least 16 characters.",
        "operationId": "rotateAdminToken",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "type": "object", "required": ["new_token"], "properties": { "new_token": { "type": "string", "minLength": 16, "description": "New admin bearer token (min 16 chars)" } } } } }
        },
        "responses": {
          "200": { "description": "Token rotated", "content": { "application/json": { "schema": { "type": "object", "properties": { "status": { "type": "string", "example": "rotated" } } } } } },
          "400": { "description": "new_token too short (min 16 chars)" }
        }
      }
    },
    "/v1/admin/backup": {
      "post": {
        "tags": ["Admin"],
        "summary": "Create SQLite database backup",
        "description": "Copies the active SQLite database file to a timestamped backup. Only available when the SQLite backend is active (returns 404 otherwise).",
        "operationId": "createBackup",
        "responses": {
          "200": { "description": "Backup created", "content": { "application/json": { "schema": { "type": "object", "properties": { "status": { "type": "string", "example": "backed_up" }, "path": { "type": "string" }, "size_bytes": { "type": "integer" } } } } } },
          "404": { "description": "SQLite backend not active" }
        }
      }
    },
    "/v1/webhooks/github": {
      "post": {
        "tags": ["Webhooks"],
        "summary": "Receive GitHub webhook events",
        "description": "Verifies HMAC-SHA256 signature, parses the event, and dispatches based on configured event-to-action mappings. Auth is via webhook signature, not bearer token.",
        "operationId": "githubWebhook",
        "responses": {
          "200": { "description": "Event processed or ignored" },
          "401": { "description": "Invalid or missing signature" },
          "503": { "description": "GitHub App not configured" }
        }
      }
    },
    "/v1/webhooks/github/actions": {
      "get": {
        "tags": ["Webhooks"],
        "summary": "List GitHub webhook event-to-action mappings",
        "operationId": "listWebhookActions",
        "responses": {
          "200": { "description": "Current action mappings", "content": { "application/json": { "schema": { "type": "object" } } } }
        }
      },
      "put": {
        "tags": ["Webhooks"],
        "summary": "Replace GitHub webhook event-to-action mappings",
        "operationId": "setWebhookActions",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "type": "object", "properties": { "actions": { "type": "array", "items": { "type": "object", "properties": { "event_pattern": { "type": "string" }, "label_filter": { "type": "string" }, "repo_filter": { "type": "string" }, "action": { "type": "string", "enum": ["create_and_orchestrate", "acknowledge", "ignore"] } } } } } } } }
        },
        "responses": {
          "200": { "description": "Actions updated" },
          "503": { "description": "GitHub App not configured" }
        }
      }
    },
    "/v1/webhooks/github/scan": {
      "post": {
        "tags": ["Webhooks"],
        "summary": "Scan a repo for open issues and queue them for sequential processing",
        "description": "Lists open issues from a GitHub repo via the App API, creates a session+run for each, and processes them one at a time through the orchestrator. Each issue gets its own PR with an approval gate before merge.",
        "operationId": "githubScan",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "type": "object", "required": ["repo", "installation_id"], "properties": { "repo": { "type": "string", "description": "owner/repo" }, "installation_id": { "type": "integer", "description": "GitHub App installation ID" }, "labels": { "type": "string", "description": "Comma-separated label filter" }, "limit": { "type": "integer", "description": "Max issues to scan (default 30, max 100)" } } } } }
        },
        "responses": {
          "200": { "description": "Issues queued for processing" },
          "503": { "description": "GitHub App not configured" }
        }
      }
    },
    "/v1/webhooks/github/queue": {
      "get": {
        "tags": ["Webhooks"],
        "summary": "View the current issue processing queue",
        "operationId": "githubQueue",
        "responses": {
          "200": { "description": "Queue status", "content": { "application/json": { "schema": { "type": "object" } } } }
        }
      }
    },
    "/v1/bundles/export": {
      "post": {
        "tags": ["Bundles"],
        "summary": "Export project artifacts as a portable bundle",
        "description": "Exports prompt assets, releases, and knowledge documents from a project into the RFC 013 CairnBundle format.",
        "operationId": "exportBundle",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ExportBundleRequest" } } }
        },
        "responses": {
          "200": { "description": "CairnBundle envelope", "content": { "application/json": { "schema": { "type": "object" } } } }
        }
      }
    },
    "/v1/bundles/apply": {
      "post": {
        "tags": ["Bundles"],
        "summary": "Apply a bundle to a project",
        "description": "Validates, plans, and applies a CairnBundle into the target project with conflict resolution. Supports skip/overwrite/rename strategies.",
        "operationId": "applyBundle",
        "requestBody": {
          "required": true,
          "content": { "application/json": { "schema": { "$ref": "#/components/schemas/ApplyBundleRequest" } } }
        },
        "responses": {
          "200": { "description": "Import result with per-artifact outcomes", "content": { "application/json": { "schema": { "type": "object", "properties": { "artifacts_imported": { "type": "integer" }, "artifacts_skipped": { "type": "integer" }, "outcomes": { "type": "array", "items": { "type": "object" } } } } } } },
          "400": { "description": "Bundle validation failed" }
        }
      }
    },
    "/v1/overview": {
      "get": {
        "tags": ["Health"],
        "summary": "High-level operator overview",
        "description": "Combines status and dashboard: store backend, deployment mode, uptime, active counts, cost summary, feature flags.",
        "operationId": "getOverview",
        "responses": { "200": { "description": "Overview data" } }
      }
    },
    "/v1/prompts/assets": {
      "get": {
        "tags": ["Prompts"],
        "summary": "List prompt assets",
        "operationId": "listPromptAssets",
        "parameters": [
          { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 100 } },
          { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
        ],
        "responses": { "200": { "description": "Prompt asset list" } }
      }
    },
    "/v1/prompts/releases": {
      "get": {
        "tags": ["Prompts"],
        "summary": "List prompt releases",
        "operationId": "listPromptReleases",
        "parameters": [
          { "name": "limit",  "in": "query", "schema": { "type": "integer", "default": 100 } },
          { "name": "offset", "in": "query", "schema": { "type": "integer", "default": 0 } }
        ],
        "responses": { "200": { "description": "Prompt release list" } }
      }
    },
    "/v1/notifications": {
      "get": {
        "tags": ["Notifications"],
        "summary": "List notifications",
        "operationId": "listNotifications",
        "responses": { "200": { "description": "Notification list" } }
      }
    },
    "/v1/notifications/read-all": {
      "post": {
        "tags": ["Notifications"],
        "summary": "Mark all notifications as read",
        "operationId": "markAllNotificationsRead",
        "responses": { "200": { "description": "Marked read" } }
      }
    },
    "/v1/notifications/{id}/read": {
      "post": {
        "tags": ["Notifications"],
        "summary": "Mark a single notification as read",
        "operationId": "markNotificationRead",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Marked read" } }
      }
    },
    "/v1/decisions": {
      "get": {
        "tags": ["Decisions"],
        "summary": "List recent decisions (RFC 019)",
        "operationId": "listDecisions",
        "responses": { "200": { "description": "Decision list" } }
      }
    },
    "/v1/decisions/cache": {
      "get": {
        "tags": ["Decisions"],
        "summary": "List active cached decisions (learned rules)",
        "operationId": "listDecisionCache",
        "responses": { "200": { "description": "Cached decisions" } }
      }
    },
    "/v1/decisions/{id}": {
      "get": {
        "tags": ["Decisions"],
        "summary": "Get decision with full reasoning chain",
        "operationId": "getDecision",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Decision detail" } }
      }
    },
    "/v1/decisions/{id}/invalidate": {
      "post": {
        "tags": ["Decisions"],
        "summary": "Invalidate a specific cached decision",
        "operationId": "invalidateDecision",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Invalidated" } }
      }
    },
    "/v1/decisions/invalidate": {
      "post": {
        "tags": ["Decisions"],
        "summary": "Bulk invalidate by scope and kind",
        "operationId": "bulkInvalidateDecisions",
        "responses": { "200": { "description": "Invalidation count" } }
      }
    },
    "/v1/decisions/evaluate": {
      "post": {
        "tags": ["Decisions"],
        "summary": "Evaluate a decision request (RFC 019 8-step pipeline)",
        "description": "Drives a DecisionRequest through scope, visibility, guardrail, budget, cache, approval, cache-write, and return steps. Cached decisions are persisted to the event log so they survive restart (RFC 020 §'Decision Cache Survival').",
        "operationId": "evaluateDecision",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["kind"],
                "properties": {
                  "kind": { "type": "object" },
                  "principal": { "type": "object" },
                  "subject": { "type": "object" },
                  "tenant_id": { "type": "string" },
                  "workspace_id": { "type": "string" },
                  "project_id": { "type": "string" },
                  "correlation_id": { "type": "string" }
                }
              }
            }
          }
        },
        "responses": {
          "200": { "description": "Decision evaluated; body carries decision_id, outcome, source, cached, cache_hit." },
          "400": { "description": "Malformed request." }
        }
      }
    },
    "/v1/decisions/invalidate-by-rule": {
      "post": {
        "tags": ["Decisions"],
        "summary": "Invalidate decisions referencing a guardrail rule",
        "operationId": "invalidateByRule",
        "responses": { "200": { "description": "Invalidation count" } }
      }
    },
    "/v1/runs/{id}/approve": {
      "post": {
        "tags": ["Plan Review"],
        "summary": "Approve a plan artifact (RFC 018)",
        "operationId": "approvePlan",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Approved, next_step: create_execute_run" } }
      }
    },
    "/v1/runs/{id}/reject": {
      "post": {
        "tags": ["Plan Review"],
        "summary": "Reject a plan artifact",
        "operationId": "rejectPlan",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Rejected" } }
      }
    },
    "/v1/runs/{id}/revise": {
      "post": {
        "tags": ["Plan Review"],
        "summary": "Request plan revision, creates new Plan-mode run",
        "operationId": "revisePlan",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "201": { "description": "New plan run created" } }
      }
    },
    "/v1/sqeq/initialize": {
      "post": {
        "tags": ["SQ/EQ Protocol"],
        "summary": "Initialize SQ/EQ transport session (RFC 021)",
        "operationId": "sqeqInitialize",
        "responses": { "200": { "description": "Session established" } }
      }
    },
    "/v1/sqeq/submit": {
      "post": {
        "tags": ["SQ/EQ Protocol"],
        "summary": "Submit a command via SQ/EQ",
        "operationId": "sqeqSubmit",
        "responses": { "202": { "description": "Submission accepted" } }
      }
    },
    "/v1/sqeq/events": {
      "get": {
        "tags": ["SQ/EQ Protocol"],
        "summary": "SSE event stream with scope filtering",
        "operationId": "sqeqEvents",
        "responses": { "200": { "description": "Event stream" } }
      }
    },
    "/.well-known/agent.json": {
      "get": {
        "tags": ["A2A"],
        "summary": "A2A Agent Card (RFC 021)",
        "operationId": "a2aAgentCard",
        "responses": { "200": { "description": "Agent Card JSON" } }
      }
    },
    "/v1/a2a/tasks": {
      "post": {
        "tags": ["A2A"],
        "summary": "Submit an A2A task",
        "operationId": "a2aSubmitTask",
        "responses": { "201": { "description": "Task submitted" } }
      }
    },
    "/v1/a2a/tasks/{id}": {
      "get": {
        "tags": ["A2A"],
        "summary": "Get A2A task status",
        "operationId": "a2aGetTask",
        "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
        "responses": { "200": { "description": "Task status" } }
      }
    },
    "/v1/projects/{project}/local-paths": {
      "delete": {
        "tags": ["Projects"],
        "summary": "Detach a local_fs path from a project",
        "description": "Removes a local-filesystem pseudo-repo previously attached via `POST /v1/projects/{project}/repos` with `host=local_fs`. Separate from the `/repos/{owner}/{repo}` endpoint because arbitrary filesystem paths can't be split into two path segments.",
        "operationId": "detachProjectLocalPath",
        "parameters": [{ "name": "project", "in": "path", "required": true, "schema": { "type": "string" } }],
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["path"],
                "properties": { "path": { "type": "string" } }
              }
            }
          }
        },
        "responses": {
          "204": { "description": "Detached" },
          "404": { "description": "No such path attached to this project" }
        }
      }
    },
    "/v1/integrations/github/verify-installation": {
      "post": {
        "tags": ["Integrations"],
        "summary": "Verify a GitHub App installation",
        "description": "Mints a JWT from the provided app_id + private_key, exchanges it for an installation access token, and reports the installation's owner and repo count. Does not mutate server state.",
        "operationId": "verifyGithubInstallation",
        "requestBody": {
          "required": true,
          "content": {
            "application/json": {
              "schema": {
                "type": "object",
                "required": ["app_id", "private_key", "installation_id"],
                "properties": {
                  "app_id":          { "type": "integer" },
                  "private_key":     { "type": "string", "description": "PEM-encoded RSA private key" },
                  "installation_id": { "type": "integer" }
                }
              }
            }
          }
        },
        "responses": {
          "200": {
            "description": "Verification succeeded",
            "content": {
              "application/json": {
                "schema": {
                  "type": "object",
                  "properties": {
                    "verified":   { "type": "boolean" },
                    "owner":      { "type": "string" },
                    "repo_count": { "type": "integer" },
                    "expires_at": { "type": "string" }
                  }
                }
              }
            }
          },
          "400": { "description": "Invalid request (bad PEM, empty key, etc.)" },
          "502": { "description": "GitHub API error — credentials or installation ID rejected" }
        }
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
