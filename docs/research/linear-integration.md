# Linear Integration Research

Research for implementing a Linear integration plugin in Cairn.
Sources: Linear developer docs (linear.app/developers), Linear SDK source (@linear/sdk v3.x).

---

## 1. Linear API Overview

Linear uses a **GraphQL API** (not REST).

- **Endpoint:** `https://api.linear.app/graphql`
- **Protocol:** POST with JSON body containing `query` and `variables`
- **Schema introspection:** Supported (use standard GraphQL introspection queries)
- **SDK:** `@linear/sdk` (TypeScript) -- but we will call GraphQL directly from Rust

### Authentication

Two methods:

**Personal API Key** (for development/self-hosted):
```
Authorization: <API_KEY>
```

**OAuth 2.0 Access Token** (for production integrations):
```
Authorization: Bearer <ACCESS_TOKEN>
```

Note: Personal API keys do NOT use the `Bearer` prefix. OAuth tokens DO use `Bearer`.

### Rate Limits

| Limit Type | API Key | OAuth App | Unauthenticated |
|---|---|---|---|
| Requests | 5,000/hour | 5,000/hour | 60/hour |
| Complexity | 250,000 pts/hour | 2,000,000 pts/hour | 10,000 pts/hour |
| Single query max | 10,000 pts | 10,000 pts | 10,000 pts |

Rate limit response headers:
- `X-RateLimit-Requests-Limit`, `X-RateLimit-Requests-Remaining`, `X-RateLimit-Requests-Reset`
- `X-Complexity`, `X-RateLimit-Complexity-Limit`, `X-RateLimit-Complexity-Remaining`, `X-RateLimit-Complexity-Reset`

Rate limited requests return HTTP 400 with `"code": "RATELIMITED"` in the GraphQL errors extensions.

---

## 2. OAuth 2.0 Flow

### URLs

| Purpose | URL |
|---|---|
| Authorization | `https://linear.app/oauth/authorize` |
| Token exchange | `https://api.linear.app/oauth/token` |
| Token revocation | `https://api.linear.app/oauth/revoke` |

### Authorization Request Parameters

| Parameter | Required | Description |
|---|---|---|
| `client_id` | Yes | Application client ID |
| `redirect_uri` | Yes | Callback URL |
| `response_type` | Yes | Must be `code` |
| `scope` | Yes | Comma-separated scopes |
| `state` | Recommended | CSRF protection token |
| `prompt` | No | Set to `consent` to force consent screen |
| `actor` | No | `user` (default) or `app` (agent mode) |
| `code_challenge` | No | PKCE challenge |
| `code_challenge_method` | No | PKCE method (S256) |

### Available Scopes

| Scope | Description |
|---|---|
| `read` | Read access (always present) |
| `write` | Modify user account data |
| `issues:create` | Create new issues and attachments |
| `comments:create` | Create comments on issues |
| `timeSchedule:write` | Manage schedules |
| `admin` | Full admin access |
| `app:assignable` | Allow issue delegation to the app (agent scope) |
| `app:mentionable` | Allow @mentioning the app (agent scope) |
| `customer:read` | Read customer data |
| `customer:write` | Write customer data |
| `initiative:read` | Read initiative data |
| `initiative:write` | Write initiative data |

### Token Response

```json
{
  "access_token": "<64-char hex string>",
  "token_type": "Bearer",
  "expires_in": 86399,
  "scope": "read write",
  "refresh_token": "<refresh-token>"
}
```

- Access tokens expire after **24 hours**
- Refresh tokens can be replayed within a **30-minute grace period**
- `actor=app` mode: resources created under the application identity (not user)

---

## 3. Webhooks

### Configuration

Webhooks can be created via:
- **UI:** Settings > API > New webhook
- **API:** `webhookCreate` mutation

```graphql
mutation {
  webhookCreate(input: {
    url: "https://your-server.com/hooks/linear"
    teamId: "TEAM_UUID"           # or use allPublicTeams: true
    resourceTypes: ["Issue", "Comment"]
    enabled: true
    label: "Cairn Agent Webhook"
    secret: "your-signing-secret"  # optional, for signature verification
  }) {
    success
    webhook {
      id
      enabled
    }
  }
}
```

### Requirements

- URL must be publicly accessible HTTPS (no localhost)
- Must respond with HTTP 200
- Response timeout: **5 seconds**
- Retry policy: 3 attempts at **1 minute**, **1 hour**, and **6 hours**

### Webhook IP Addresses (for allowlisting)

```
35.231.147.226
35.243.134.228
34.140.253.14
34.38.87.206
34.134.222.122
35.222.25.142
```

---

## 4. Webhook Headers

Every webhook request includes these HTTP headers:

| Header | Description | Example |
|---|---|---|
| `linear-delivery` | UUID v4 uniquely identifying this delivery | `a1b2c3d4-e5f6-...` |
| `linear-event` | Entity type that triggered the event | `Issue`, `Comment` |
| `linear-signature` | HMAC-SHA256 hex-encoded signature | `abcdef0123456789...` |
| `linear-timestamp` | Unix timestamp in milliseconds (string) | `1713100000000` |
| `content-type` | Always JSON | `application/json; charset=utf-8` |
| `user-agent` | Identifies Linear | `Linear-Webhook` |

**Important:** Header names are **lowercase** (as per HTTP/2 convention). The SDK constants confirm:
```typescript
LINEAR_WEBHOOK_SIGNATURE_HEADER = "linear-signature"
LINEAR_WEBHOOK_TS_HEADER = "linear-timestamp"
LINEAR_WEBHOOK_TS_FIELD = "webhookTimestamp"  // field in JSON body
```

---

## 5. Webhook Signature Verification

### Algorithm

**HMAC-SHA256** of the raw request body, using the webhook's signing secret as the key.

### Verification Steps (Rust pseudocode)

```rust
use hmac::{Hmac, Mac};
use sha2::Sha256;

fn verify_linear_webhook(
    raw_body: &[u8],
    signature_header: &str,   // from "linear-signature" header
    timestamp_header: &str,   // from "linear-timestamp" header
    secret: &str,
) -> bool {
    // 1. Compute HMAC-SHA256
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(raw_body);
    let computed = hex::encode(mac.finalize().into_bytes());

    // 2. Constant-time comparison
    let valid_sig = constant_time_eq(computed.as_bytes(), signature_header.as_bytes());

    // 3. Verify timestamp within 60 seconds
    let ts_ms: u64 = timestamp_header.parse().unwrap_or(0);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let valid_ts = (now_ms as i64 - ts_ms as i64).unsigned_abs() < 60_000;

    valid_sig && valid_ts
}
```

### Key Details

- The signature is a **hex-encoded** HMAC-SHA256 digest
- Linear signs the **exact raw body** -- any parsing/modification before verification will break it
- Timestamp tolerance: **60 seconds** (1 minute)
- The timestamp is available in both the `linear-timestamp` header AND the `webhookTimestamp` JSON body field; prefer the header
- Use constant-time comparison to prevent timing attacks

---

## 6. Webhook Event Types

### Data Change Events

These use the `EntityWebhookPayload` envelope with `action` of `"create"`, `"update"`, or `"remove"`:

| `type` value (in payload and `linear-event` header) | Entity |
|---|---|
| `Issue` | Issues |
| `Comment` | Issue/project/document comments |
| `Attachment` | Issue attachments |
| `IssueLabel` | Issue labels |
| `Reaction` | Comment reactions |
| `Project` | Projects |
| `ProjectUpdate` | Project updates |
| `Document` | Documents |
| `Initiative` | Initiatives |
| `InitiativeUpdate` | Initiative updates |
| `Cycle` | Cycles |
| `Customer` | Customers |
| `CustomerNeed` | Customer requests/needs |
| `User` | User profile changes |
| `AuditEntry` | Audit log entries |

### Special Events

| `type` value | Description |
|---|---|
| `IssueSLA` | Issue SLA breach/risk events |
| `OAuthApp` | OAuth app revoked |
| `AppUserNotification` | App user notification |
| `PermissionChange` | Team access changed for app user |
| `AgentSessionEvent` | Agent session created/updated (for agent integrations) |

---

## 7. Webhook Payload Structure

### Entity Webhook Envelope (`EntityWebhookPayload`)

Top-level fields present in all data-change webhook payloads:

```json
{
  "action": "create",
  "type": "Issue",
  "createdAt": "2024-04-14T10:00:00.000Z",
  "organizationId": "org-uuid",
  "webhookId": "webhook-uuid",
  "webhookTimestamp": 1713088800000,
  "url": "https://linear.app/team/issue/ENG-123",
  "actor": {
    "id": "user-uuid",
    "name": "Jane Doe",
    "type": "user"
  },
  "data": { ... },
  "updatedFrom": { ... }
}
```

| Field | Type | Description |
|---|---|---|
| `action` | `"create" \| "update" \| "remove"` | What happened |
| `type` | string | Entity type name (matches `linear-event` header) |
| `createdAt` | ISO 8601 datetime | When the event was created |
| `organizationId` | UUID string | Organization that owns the entity |
| `webhookId` | UUID string | Which webhook config sent this |
| `webhookTimestamp` | number (ms) | Unix timestamp in milliseconds |
| `url` | string or null | URL to the entity in Linear |
| `actor` | object or null | Who performed the action (see Actor types below) |
| `data` | object | The full entity payload (type-specific) |
| `updatedFrom` | object or null | Previous values of changed fields (only for `update` action) |

### Actor Types

The `actor` field is a union of:
- `UserActorWebhookPayload` -- a human user (`type: "user"`)
- `OauthClientActorWebhookPayload` -- an OAuth app
- `IntegrationActorWebhookPayload` -- an integration

---

## 8. Issue Webhook Payload (`data` field)

When `type` is `"Issue"`, the `data` field contains `IssueWebhookPayload`:

```json
{
  "id": "issue-uuid",
  "identifier": "ENG-123",
  "number": 123,
  "title": "Implement Linear webhook handler",
  "description": "We need to handle Linear webhooks for...",
  "descriptionData": "...",
  "url": "https://linear.app/team/issue/ENG-123",
  "priority": 2,
  "priorityLabel": "High",
  "estimate": 3,
  "dueDate": "2024-04-20",
  "createdAt": "2024-04-14T10:00:00.000Z",
  "updatedAt": "2024-04-14T12:00:00.000Z",
  "completedAt": null,
  "canceledAt": null,
  "startedAt": null,
  "archivedAt": null,

  "stateId": "state-uuid",
  "state": {
    "id": "state-uuid",
    "name": "In Progress",
    "color": "#f2c94c",
    "type": "started"
  },

  "teamId": "team-uuid",
  "team": {
    "id": "team-uuid",
    "key": "ENG",
    "name": "Engineering"
  },

  "assigneeId": "user-uuid",
  "assignee": {
    "id": "user-uuid",
    "name": "Jane Doe",
    "email": "jane@example.com",
    "avatarUrl": "https://...",
    "url": "https://linear.app/..."
  },

  "creatorId": "user-uuid",
  "creator": {
    "id": "user-uuid",
    "name": "Bob Smith",
    "email": "bob@example.com",
    "avatarUrl": null,
    "url": "https://linear.app/..."
  },

  "labelIds": ["label-uuid-1", "label-uuid-2"],
  "labels": [
    {
      "id": "label-uuid-1",
      "name": "bug",
      "color": "#eb5757",
      "parentId": null
    },
    {
      "id": "label-uuid-2",
      "name": "backend",
      "color": "#4ea7fc",
      "parentId": null
    }
  ],

  "projectId": "project-uuid",
  "project": {
    "id": "project-uuid",
    "name": "Q2 Sprint",
    "url": "https://linear.app/..."
  },

  "cycleId": "cycle-uuid",
  "cycle": {
    "id": "cycle-uuid",
    "number": 5,
    "startsAt": "2024-04-01T00:00:00.000Z",
    "endsAt": "2024-04-14T00:00:00.000Z"
  },

  "parentId": null,
  "delegateId": null,
  "subscriberIds": ["user-uuid-1", "user-uuid-2"],
  "previousIdentifiers": [],
  "prioritySortOrder": -1234.56,
  "sortOrder": -5678.90,
  "reactionData": {},
  "trashed": false
}
```

### Workflow State Types

The `state.type` field indicates the category:
- `"triage"` -- Issue is in triage
- `"backlog"` -- Backlog state
- `"unstarted"` -- Not started
- `"started"` -- In progress
- `"completed"` -- Done
- `"canceled"` -- Canceled

### Priority Values

| Value | Label |
|---|---|
| 0 | No priority |
| 1 | Urgent |
| 2 | High |
| 3 | Medium |
| 4 | Low |

---

## 9. Comment Webhook Payload

When `type` is `"Comment"`, the `data` field contains `CommentWebhookPayload`:

```json
{
  "id": "comment-uuid",
  "body": "This looks good, let's proceed.",
  "createdAt": "2024-04-14T10:30:00.000Z",
  "updatedAt": "2024-04-14T10:30:00.000Z",
  "editedAt": null,
  "issueId": "issue-uuid",
  "issue": {
    "id": "issue-uuid",
    "identifier": "ENG-123",
    "title": "...",
    "teamId": "team-uuid",
    "team": { "id": "team-uuid", "key": "ENG", "name": "Engineering" },
    "url": "https://linear.app/..."
  },
  "parentId": null,
  "user": {
    "id": "user-uuid",
    "name": "Jane Doe",
    "email": "jane@example.com",
    "avatarUrl": null,
    "url": "https://linear.app/..."
  },
  "botActor": null,
  "resolvedAt": null,
  "reactionData": {}
}
```

---

## 10. GraphQL API: Querying Issues

### Fetch a single issue by identifier

```graphql
query {
  issue(id: "ENG-123") {
    id
    identifier
    number
    title
    description
    url
    priority
    priorityLabel
    estimate
    dueDate
    createdAt
    updatedAt
    completedAt
    startedAt
    state {
      id
      name
      type
      color
    }
    team {
      id
      key
      name
    }
    assignee {
      id
      name
      email
    }
    creator {
      id
      name
    }
    labels {
      nodes {
        id
        name
        color
      }
    }
    project {
      id
      name
      url
    }
    cycle {
      id
      number
      startsAt
      endsAt
    }
    parent {
      id
      identifier
      title
    }
    children {
      nodes {
        id
        identifier
        title
        state { name type }
      }
    }
    comments {
      nodes {
        id
        body
        createdAt
        user { id name }
      }
    }
  }
}
```

### Fetch team issues with filtering

```graphql
query {
  team(id: "team-uuid") {
    issues(
      filter: {
        state: { type: { in: ["started", "unstarted"] } }
        priority: { lte: 2 }
      }
      first: 50
    ) {
      nodes {
        id
        identifier
        title
        priority
        state { name type }
        assignee { name }
      }
      pageInfo {
        hasNextPage
        endCursor
      }
    }
  }
}
```

### Fetch workflow states for a team

Needed to map state names to IDs for updating issue status:

```graphql
query {
  team(id: "team-uuid") {
    states {
      nodes {
        id
        name
        type
        color
        position
      }
    }
  }
}
```

---

## 11. GraphQL API: Mutations

### Create a comment on an issue

```graphql
mutation {
  commentCreate(input: {
    issueId: "ENG-123"
    body: "Agent analysis complete. The root cause is..."
  }) {
    success
    comment {
      id
      body
      createdAt
    }
  }
}
```

The `issueId` field accepts either a UUID or an issue identifier like `"ENG-123"`.

For agent apps using `actor=app`, you can also set:
- `createAsUser`: Display name for the comment author
- `displayIconUrl`: Avatar URL for the comment author

### Update an issue (change state/status)

```graphql
mutation {
  issueUpdate(
    id: "ENG-123"
    input: {
      stateId: "state-uuid"
    }
  ) {
    success
    issue {
      id
      identifier
      state {
        id
        name
        type
      }
    }
  }
}
```

### Update issue fields

```graphql
mutation {
  issueUpdate(
    id: "ENG-123"
    input: {
      assigneeId: "user-uuid"
      priority: 1
      addedLabelIds: ["label-uuid"]
      description: "Updated description..."
    }
  ) {
    success
    issue {
      id
      identifier
    }
  }
}
```

### IssueUpdateInput fields (all optional)

| Field | Type | Description |
|---|---|---|
| `title` | string | Issue title |
| `description` | string | Markdown description |
| `stateId` | UUID | Workflow state (status) |
| `assigneeId` | UUID | Assigned user |
| `priority` | int (0-4) | Priority level |
| `estimate` | int | Complexity estimate |
| `dueDate` | date string | Due date (YYYY-MM-DD) |
| `projectId` | UUID | Project association |
| `cycleId` | UUID | Cycle association |
| `parentId` | UUID or identifier | Parent issue |
| `teamId` | UUID | Move to different team |
| `labelIds` | UUID[] | Replace all labels |
| `addedLabelIds` | UUID[] | Add labels |
| `removedLabelIds` | UUID[] | Remove labels |
| `delegateId` | UUID | Delegate to agent |
| `subscriberIds` | UUID[] | Subscribers |
| `trashed` | bool | Move to/from trash |

---

## 12. Agent Framework (Linear-native Agent Protocol)

Linear has a first-party agent framework. Agent apps are OAuth applications with `actor=app` mode that receive `AgentSessionEvent` webhooks. This is the recommended path for Cairn's Linear integration.

### Agent Session Lifecycle

1. **User delegates issue to agent** (or @mentions agent in a comment)
2. Linear creates an `AgentSession` and sends `AgentSessionEvent` webhook with `action: "created"`
3. Agent must emit a `thought` activity within **10 seconds** to acknowledge
4. Agent performs work, emitting activities (thoughts, actions, errors, responses)
5. Agent completes session or user dismisses it

### AgentSessionEvent Webhook Payload

```json
{
  "type": "AgentSessionEvent",
  "action": "created",
  "organizationId": "org-uuid",
  "webhookId": "webhook-uuid",
  "webhookTimestamp": 1713088800000,
  "createdAt": "2024-04-14T10:00:00.000Z",
  "appUserId": "app-user-uuid",
  "oauthClientId": "oauth-client-uuid",
  "agentSession": {
    "id": "session-uuid",
    "status": "pending",
    "type": "delegation",
    "createdAt": "2024-04-14T10:00:00.000Z",
    "organizationId": "org-uuid",
    "appUserId": "app-user-uuid",
    "issueId": "issue-uuid",
    "issue": {
      "id": "issue-uuid",
      "identifier": "ENG-123",
      "title": "Implement feature X",
      "description": "Full issue description...",
      "teamId": "team-uuid",
      "team": { "id": "team-uuid", "key": "ENG", "name": "Engineering" },
      "url": "https://linear.app/..."
    },
    "commentId": null,
    "comment": null,
    "creator": {
      "id": "user-uuid",
      "name": "Jane Doe",
      "email": "jane@example.com"
    },
    "url": "https://linear.app/..."
  },
  "agentActivity": null,
  "promptContext": "## Issue\n**ENG-123: Implement feature X**\n\n...",
  "guidance": [
    {
      "body": "Always write tests for new features.",
      "origin": { "type": "team", "name": "Engineering" }
    }
  ],
  "previousComments": []
}
```

### Key AgentSessionEvent Fields

| Field | Type | Description |
|---|---|---|
| `action` | string | `"created"`, `"updated"` |
| `agentSession` | object | Full session data including issue |
| `agentSession.issue` | object | Issue with `id`, `identifier`, `title`, `description`, `team`, `url` |
| `agentSession.status` | string | `pending`, `active`, `awaitingInput`, `complete`, `error`, `stale` |
| `agentSession.type` | string | `"delegation"` (assigned) or mention type |
| `promptContext` | string or null | Pre-formatted context string for the agent (only on `created`) |
| `guidance` | array or null | Team/workspace guidance rules for the agent |
| `previousComments` | array or null | Prior comments in thread (for mention sessions) |
| `appUserId` | UUID | The agent's app user ID |
| `oauthClientId` | UUID | The OAuth client ID |

### Agent Activities (emitting thoughts/progress)

```graphql
mutation {
  agentActivityCreate(input: {
    agentSessionId: "session-uuid"
    content: {
      type: "thought"
      body: "Analyzing the issue requirements..."
    }
    ephemeral: false
  }) {
    success
    agentActivity {
      id
    }
  }
}
```

**Activity Types** (`AgentActivityType` enum):
| Type | Description |
|---|---|
| `thought` | Agent's internal reasoning (shown to user) |
| `action` | An action the agent is taking |
| `response` | Final response/output |
| `error` | Error occurred |
| `elicitation` | Asking the user for input |
| `prompt` | User-initiated prompt |

**Activity Signals** (`AgentActivitySignal` enum):
| Signal | Description |
|---|---|
| `continue` | Agent will continue working |
| `stop` | Agent is done |
| `auth` | Agent needs authentication |
| `select` | Agent wants user to select from options |

### Updating Agent Session

```graphql
mutation {
  agentSessionUpdate(
    id: "session-uuid"
    input: {
      plan: { steps: ["Analyze", "Implement", "Test"] }
      addedExternalUrls: [
        { label: "Pull Request", url: "https://github.com/..." }
      ]
    }
  ) {
    success
  }
}
```

---

## 13. Webhook Management via API

### Create webhook

```graphql
mutation {
  webhookCreate(input: {
    url: "https://cairn.example.com/v1/hooks/linear"
    resourceTypes: ["Issue", "Comment", "AgentSessionEvent"]
    allPublicTeams: true
    enabled: true
    label: "Cairn Integration"
    secret: "webhook-signing-secret"
  }) {
    success
    webhook {
      id
      url
      enabled
      secret
      resourceTypes
    }
  }
}
```

### List webhooks

```graphql
query {
  webhooks {
    nodes {
      id
      url
      enabled
      label
      resourceTypes
      team { id name }
      createdAt
    }
  }
}
```

### Delete webhook

```graphql
mutation {
  webhookDelete(id: "webhook-uuid") {
    success
  }
}
```

---

## 14. Implementation Plan for Cairn

### Two Integration Modes

**Mode A: Standard Webhooks (simpler)**
- Register a webhook for `Issue` and `Comment` events
- On `Issue` create/update, trigger Cairn agent runs based on labels/state
- Agent reads issue via GraphQL, does work, comments back, updates state
- Does NOT use Linear's agent framework -- just API calls

**Mode B: Linear Agent Protocol (richer UX)**
- Register as an OAuth app with `actor=app`, scopes: `read`, `write`, `app:assignable`, `app:mentionable`
- Listen for `AgentSessionEvent` webhooks
- Use `agentActivityCreate` to emit thoughts/progress visible in Linear UI
- Use `agentSessionUpdate` to share plan and external links (PRs, etc.)
- Richer UX: users see agent thinking in real-time within Linear

### Recommended: Mode B (Agent Protocol)

Advantages:
- Native agent UX in Linear (real-time activity feed)
- Issue delegation workflow (users assign issues to the agent)
- `promptContext` field provides pre-formatted context
- `guidance` rules let teams configure agent behavior from Linear
- Agent does not count as a billable user
- 10-second acknowledgment requirement ensures responsive UX

### Required Cairn Components

1. **Webhook receiver** (`POST /v1/hooks/linear`)
   - Parse raw body, verify HMAC-SHA256 signature
   - Route by `type` field to appropriate handler
   - Must respond within 5 seconds (queue work, ack fast)

2. **Linear API client** (GraphQL over HTTP)
   - `query issue(id)` -- fetch issue details
   - `mutation commentCreate` -- post agent comments
   - `mutation issueUpdate` -- change state, assignee, labels
   - `mutation agentActivityCreate` -- emit agent thoughts
   - `mutation agentSessionUpdate` -- update session plan/links
   - `query team.states` -- resolve state names to IDs

3. **OAuth token management**
   - Store encrypted access + refresh tokens per workspace
   - Auto-refresh tokens before 24h expiry
   - Support `actor=app` mode for agent identity

4. **Event-to-action mapping** (operator-configurable)
   - "When issue created with label `agent-fix` -> trigger code fix agent"
   - "When issue delegated to Cairn agent -> start autonomous session"
   - "When comment mentions @cairn -> respond to question"

### Rust Implementation Notes

- Use `hmac` + `sha2` crates for webhook signature verification
- Use `reqwest` for GraphQL HTTP calls (POST JSON to `https://api.linear.app/graphql`)
- The `linear-signature` header value is hex-encoded (not base64)
- Always read raw body bytes before any JSON parsing (signature verification needs exact bytes)
- The `linear-timestamp` header is a string containing milliseconds since epoch
- Issue identifiers like `"ENG-123"` can be used directly in mutations (`issueId`, `id` params)
- Priority is numeric: 0=none, 1=urgent, 2=high, 3=medium, 4=low (inverse of what you might expect)
