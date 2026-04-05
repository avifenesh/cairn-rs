# AI Observability & LLM Ops Dashboard UX Patterns

> Deep research compiled April 2026. Covers 15+ sources across Langfuse, Helicone,
> Arize Phoenix, LangSmith, Braintrust, Portkey, Humanloop, and enterprise SaaS
> patterns. Focuses on concrete UX patterns, not marketing.
>
> Companion to: [ai-agent-operator-ui.md](ai-agent-operator-ui.md) (tech stack and
> architecture focus).

---

## Table of Contents

1. [Trace Visualization](#1-trace-visualization)
2. [Cost Monitoring Displays](#2-cost-monitoring-displays)
3. [Approval Workflow UIs](#3-approval-workflow-uis)
4. [Prompt Management Views](#4-prompt-management-views)
5. [Memory & RAG Quality Panels](#5-memory--rag-quality-panels)
6. [What Operators Need to Trust a Dashboard Daily](#6-what-operators-need-to-trust-a-dashboard-daily)
7. [Enterprise vs. Hobby: What Actually Differentiates](#7-enterprise-vs-hobby-what-actually-differentiates)
8. [Source Index](#source-index)

---

## 1. Trace Visualization

Trace visualization is the single most important view in any LLM observability tool.
Every platform converges on the same core layout, but execution quality varies wildly.

### 1.1 The Two-Panel Trace Detail

The universal pattern is a **split-pane layout**:

```
+---------------------------+--------------------------------------+
| SPAN TREE (left, ~35%)    | SPAN DETAIL (right, ~65%)            |
|                           |                                      |
| v Root trace              | [Input]  [Output]  [Metadata]  tabs  |
|   > LLM call (2.3s) $0.02|                                      |
|   v Tool: search (0.5s)  | {                                    |
|     > embed query (0.1s) |   "role": "user",                    |
|     > vector search(0.3s)|   "content": "Find the latest..."    |
|   > LLM call (1.1s) $0.01| }                                    |
|   > Tool: write (0.2s)   |                                      |
|                           | Tokens: 1,234 in / 567 out           |
|                           | Cost: $0.0043                        |
|                           | Model: claude-sonnet-4-5              |
|                           | Duration: 2.3s                       |
|                           | [Annotate] [Replay] [Share]          |
+---------------------------+--------------------------------------+
```

**Why it works**: The left tree provides structural orientation (where am I in the
execution flow), while the right panel provides depth (what happened at this step).
Users can scan the tree for anomalies (red error badges, unusually long durations)
and click to inspect.

**Key details that differentiate good implementations**:

- **Duration badges** on every span node, right-aligned. Not just text -- a subtle
  horizontal bar showing relative duration compared to sibling spans.
- **Cost badges** on LLM-type spans only (showing cost on non-LLM spans is noise).
- **Type icons** before span names: brain/sparkle for LLM, wrench for tool, search
  for retrieval, brackets for function, exclamation for error.
- **Collapse state**: Deep trees open with root + first level expanded, everything
  else collapsed. User controls persist within the session.

### 1.2 Waterfall / Timeline Views

Two distinct approaches exist, serving different mental models:

**Indented Tree** (Langfuse, LangSmith, Phoenix):
- Best for understanding logical structure and parent-child relationships.
- Shows "what called what" clearly.
- Duration shown as text badges, not visual bars.
- Dominates for debugging "why did the agent take this path."

**Waterfall Timeline** (AgentOps, Laminar, Logfire):
- Horizontal bars on a time axis, indented by depth.
- Shows parallelism and sequential dependencies at a glance.
- Duration is visual (bar length), not textual.
- Dominates for performance analysis: "where did time go."

**Flame Chart** (Logfire, SigNoz):
- Dense horizontal stacked bars, width proportional to duration.
- Familiar to developers from browser DevTools and profiling.
- Best for identifying latency hotspots in deep call stacks.
- Least useful for understanding agent reasoning flow.

**The pragmatic choice**: Ship the indented tree first (most common, easiest to
implement, best for agent debugging). Add a waterfall toggle for performance
analysis later.

### 1.3 What Each Span Node Must Show

Based on patterns across all platforms, the minimum viable span display is:

| Element | Format | Example |
|---|---|---|
| Type icon | Colored icon, 16px | (brain icon, blue) |
| Span name | Truncated text, monospace | `chat_completion` |
| Status | Badge: success/error/running | green dot / red dot / spinner |
| Duration | Right-aligned, gray text | `2.3s` |
| Cost | Right-aligned, only for LLM spans | `$0.02` |
| Token count | Tooltip on hover | `1.2k in / 567 out` |

**What to omit from the tree node** (show only in the detail panel):
- Full input/output text
- Model name and parameters
- Metadata and tags
- Scores and annotations

### 1.4 Session / Conversation Grouping

For multi-turn agent interactions, traces must group into sessions:

- **Langfuse**: `sessionId` propagated across traces. Session view shows
  chronological replay of the entire interaction.
- **Helicone**: Path-based grouping (`/parent/child`) with color-coded
  duration distribution charts. Same-path requests share colors.
- **LangSmith**: Threads using `session_id` metadata key.
- **AgentOps**: Session drawer for rapid navigation between past sessions.

**The pattern that works**: A session detail page that looks like a chat transcript
with expandable trace details for each turn. Each message bubble links to its
full trace. Running cost and token totals accumulate at the top.

### 1.5 Real-Time Trace Construction

The hardest UI problem: building a trace tree from a live event stream.

**Challenges**:
1. Events arrive out of order (child span may start before parent metadata).
2. Duration is unknown until the span completes.
3. Cost increments as tokens stream.
4. Errors must propagate up to parent nodes.

**Solution pattern** (used by Laminar, Langfuse):
- Client-side span buffer keyed by `trace_id + span_id`.
- Optimistic rendering: show partial tree, re-parent orphaned spans when their
  parent arrives.
- Duration badges show a running timer for in-progress spans.
- Cost counters animate upward as token counts arrive.
- Error state propagates: if a child fails, parent node gets a warning badge.
- DOM updates batched at animation frame rate to avoid jank.

---

## 2. Cost Monitoring Displays

Cost is the metric that gets AI tools approved or killed by finance teams. The UI
must serve two audiences: engineers debugging expensive traces, and managers
monitoring aggregate spend.

### 2.1 The Cost Dashboard Layout

The most effective pattern (seen across Portkey, Helicone, Langfuse):

```
+------------------------------------------------------------------+
| PERIOD SELECTOR: [Today] [7d] [30d] [Custom]                     |
+------------------------------------------------------------------+
|                                                                    |
|  $1,247.32          $847.21           $2,094.53                   |
|  This period        Previous period   Running total                |
|  +47.2% vs prev     (dimmed)         (bold, large)                |
|                                                                    |
+------------------------------------------------------------------+
|                                                                    |
|  [Cost Over Time - Area Chart]                                     |
|  - Stacked by model (Claude, GPT-4, etc.)                         |
|  - Budget threshold line (dashed red)                              |
|  - Hover shows daily breakdown                                     |
|                                                                    |
+------------------------------------------------------------------+
|                                                                    |
|  Model Breakdown          | By Feature/Endpoint                   |
|  +---------+------+----+  | +---------+--------+------+           |
|  | Model   | Cost | %  |  | | Feature | Cost   | Calls|           |
|  | Sonnet  | $892 | 71%|  | | Search  | $543   | 12k  |           |
|  | Haiku   | $241 | 19%|  | | Summary | $312   | 8k   |           |
|  | GPT-4o  | $114 | 9% |  | | Chat    | $204   | 45k  |           |
|  +---------+------+----+  | +---------+--------+------+           |
|                                                                    |
+------------------------------------------------------------------+
```

### 2.2 Per-Request Cost Visibility

This is what Helicone got right and what many tools miss:

- Every request in the log table has a **cost column** showing the dollar amount.
- The cost is computed from token counts and model pricing, not estimated.
- Sorting by cost descending immediately shows the most expensive requests.
- Clicking a request shows cost breakdown: input tokens, output tokens, cache
  hits, and the per-token rate used for calculation.

**The key insight**: Cost must be visible at every level of the hierarchy:
1. Dashboard: total spend, trend
2. Trace list: cost per trace
3. Trace detail: cost per span
4. Span detail: cost breakdown (input vs output tokens, pricing tier)

### 2.3 Budget Controls and Threshold Displays

**Portkey's approach** (most mature):
- Set spending thresholds per API key.
- System blocks requests when budget is exceeded.
- Dashboard shows budget utilization as a progress bar.
- Alerts fire at configurable percentages (80%, 90%, 100%).

**LiteLLM's approach**:
- Per-project and per-user budget limits.
- Virtual API keys tied to budget ceilings.
- Admin dashboard shows spend vs. limit per key.

**The UX pattern that works for budget displays**:

```
Budget: Engineering Team
[============================--------] 73% ($1,460 / $2,000)
                                       ↑ Alert at 80%
                                              ↑ Hard stop at 100%
```

- Green bar under 70%, yellow 70-90%, red 90%+.
- Projected spend line (dotted) based on current burn rate.
- "Days until budget exhausted" as a plain text callout.

### 2.4 Cache Impact Visualization

Helicone's caching dashboard shows three numbers prominently:
- **Cache hits** (count)
- **Cost saved** (dollars)
- **Time saved** (seconds/minutes)

This is presented as a row of metric cards with icons (dollar sign, bolt, chart).
The insight: showing savings is more motivating than showing raw cache statistics.
Operators care about "we saved $400 this week" not "cache hit rate is 34%."

### 2.5 Cost Segmentation Dimensions

The platforms that serve enterprise customers well let you slice cost by:

| Dimension | Why It Matters |
|---|---|
| Model | Which models drive spend |
| User / customer | Per-customer unit economics |
| Feature / endpoint | Which features are expensive |
| Environment | Separate dev from prod spend |
| Prompt version | Did the new prompt increase cost |
| Time period | Trend analysis and forecasting |
| Team / project | Internal chargeback |

**Langfuse** exports these dimensions to PostHog and Mixpanel for business
intelligence teams. **Helicone** uses Custom Properties for arbitrary segmentation.

---

## 3. Approval Workflow UIs

No LLM observability platform ships a first-class real-time approval workflow today.
This remains the biggest UX gap in the space. What exists falls into three
categories.

### 3.1 Annotation Queues (Post-Hoc Review)

**LangSmith's annotation queues** are the most developed:

- **Queue creation**: Form with name, description, optional dataset binding.
- **Two modes**: Single-run (sequential review) and pairwise (side-by-side A/B).
- **Review interface**: Central run display with collapsible annotation sidebar.
  Sidebar shows instructions and applicable rubric criteria.
- **Navigation**: Cyclical view -- annotators progress sequentially through items.
  Keyboard shortcuts (hotkeys A/B/E for pairwise) accelerate common actions.
- **Concurrency control**: Reservation system prevents duplicate reviews. Items
  are "locked" when viewed, auto-released after configurable timeout.
- **Reviewer thresholds**: Configure how many reviewers must score each item.
- **Visibility rules**: Reviewers cannot see each other's feedback (prevents bias),
  but comments are visible to the team.
- **Batch assignment**: Multi-select runs from tables, or auto-route via rules
  matching error conditions or quality thresholds.

**Langfuse's annotation workflow**:
- Annotate button on trace/session/observation detail views.
- Score configuration selection (standardized dimensions).
- Score value input aligned with pre-configured scoring templates.
- Progress tracking with live summary metrics.
- Batch processing via Annotation Queues for larger review sets.

### 3.2 Guardrails as Automated Approval

**Portkey's gateway guardrails** act as programmatic gatekeepers:

- 20+ deterministic checks plus LLM-based evaluators.
- Detection for: prompt injection, code presence, regex patterns, JSON schema
  violations, PII, gibberish.
- Actions on pass/fail: deny, allow, or append feedback.
- HTTP status codes signal outcomes (200 = pass, 246 = partial, 446 = blocked).
- Logs UI shows pass/fail counts per check with individual verdicts.

This is automated approval -- no human in the loop, but the UX pattern of showing
verdict badges with expandable reasoning applies directly to human approval UIs.

### 3.3 The Missing Pattern: Real-Time Approval

Based on synthesis across all platforms, this is the approval workflow that should
exist but does not yet:

```
+------------------------------------------------------------------+
| PENDING APPROVALS (3)                              [Filters] [v]  |
+------------------------------------------------------------------+
|                                                                    |
| [!] Agent wants to delete user account    2 min ago    HIGH RISK  |
|     Agent: support-bot | Session: #4521                           |
|     Proposed action: DELETE /api/users/8831                        |
|     Context: User requested account deletion in chat               |
|     Estimated cost: $0.00 (API call)                               |
|     [Approve] [Reject] [Modify] [View Full Trace]                 |
|                                                                    |
+------------------------------------------------------------------+
|                                                                    |
| [i] Agent wants to send email to customer  5 min ago   MEDIUM     |
|     Agent: outreach-bot | Session: #4518                          |
|     Proposed action: POST /api/email/send                          |
|     Context: Follow-up based on support conversation               |
|     Estimated cost: $0.003 (SES)                                   |
|     [Approve] [Reject] [Modify] [View Full Trace]                 |
|                                                                    |
+------------------------------------------------------------------+
```

**Key UX elements**:
1. **Risk badge**: Color-coded severity (red/yellow/green) based on action type.
2. **Context summary**: Why the agent wants to do this, in one sentence.
3. **Proposed action**: The exact API call or tool invocation, not a summary.
4. **Session link**: Jump to the full conversation that led here.
5. **Action buttons**: Approve (green), Reject (red), Modify (yellow, opens editor).
6. **Timeout indicator**: How long until the request auto-expires.
7. **Audit trail**: After decision, the card collapses to a one-line record:
   "Approved by @jane at 14:32 -- DELETE /api/users/8831".

**The critical interaction**: Approve/Reject must be one click, not a multi-step
form. The operator is already context-switched; adding friction guarantees the
queue gets ignored.

### 3.4 Approval Queue Anti-Patterns

Based on enterprise SaaS patterns and the LLM observability gap:

- **No priority ordering**: All items look the same urgency. Sort by risk level.
- **No expiry**: Stale approvals pile up. Auto-expire with configurable TTL.
- **No delegation**: Only one person can approve. Support team routing.
- **No batch actions**: Reviewing 50 low-risk items one by one. Offer "approve all
  low-risk" with a confirmation dialog.
- **Hidden queue**: Approval notifications buried in a submenu. Badge count must
  be visible in the top navigation at all times.
- **No mobile**: Operators are not always at their desks. Approval should work
  on a phone screen.

---

## 4. Prompt Management Views

Prompt management UIs must serve two workflows: iterative development (edit, test,
compare) and production deployment (version, label, roll back).

### 4.1 The Prompt Editor

**Langfuse's approach**:
- Dual prompt types: text templates and chat (structured message arrays).
- Variable syntax with `{{variable}}` placeholders.
- In-editor variable compilation: insert values, see real-time output preview.
- Form-based creation with type selection, template preview, and labeling.

**Helicone's approach**:
- Sandbox environment with live preview as parameters change.
- Granular control over temperature, model, and test inputs.
- Non-technical users can iterate without code changes.

**Arize Phoenix**:
- Prompt playground for replay: take a logged LLM call, modify the prompt,
  re-run it, and compare outputs side by side.

### 4.2 Version Comparison View

The most requested and least-well-implemented feature across platforms.

**What good looks like**:

```
+------------------------------------------------------------------+
| Prompt: summarize-ticket                                          |
| Comparing: v3 (production) vs v5 (staging)                       |
+------------------------------------------------------------------+
|                                                                    |
| LEFT (v3)                      | RIGHT (v5)                       |
| ============================== | ================================ |
| You are a support agent.       | You are a support agent.         |
| Summarize the following ticket | Summarize the following ticket   |
| in 2-3 sentences.             | in 2-3 sentences, focusing on    |
|                                | the customer's core issue and    |
|                                | any urgency indicators.          |
| ---                            | ---                              |
| Ticket: {{ticket_text}}        | Ticket: {{ticket_text}}          |
|                                | Priority: {{priority}}           |
+------------------------------------------------------------------+
|                                                                    |
| METRICS COMPARISON                                                 |
| +------------+--------+--------+--------+                         |
| | Metric     | v3     | v5     | Delta  |                         |
| | Avg cost   | $0.003 | $0.004 | +33%   |                         |
| | Avg latency| 1.2s   | 1.4s   | +17%   |                         |
| | Quality    | 0.82   | 0.91   | +11%   |                         |
| | Tokens out | 89     | 112    | +26%   |                         |
| +------------+--------+--------+--------+                         |
|                                                                    |
| [Promote v5 to Production] [Run Experiment] [Diff View]           |
+------------------------------------------------------------------+
```

**Key elements**:
- Side-by-side text diff with additions highlighted (green) and deletions
  highlighted (red). Standard diff coloring, not custom.
- Metrics comparison table below the diff, showing cost/quality/latency deltas.
- One-click promotion button to push the new version to production.
- Experiment runner to test the new version against a dataset before promoting.

### 4.3 Label-Based Deployment

Langfuse uses a label system (`production`, `staging`, `latest`) instead of numeric
versions for deployment targeting. This means:

- The production prompt is always fetched by label, not version number.
- Promoting a new version means moving the `production` label to point to it.
- Rolling back means moving the label back to the previous version.

**The UX advantage**: Non-technical operators understand "this is the production
version" better than "this is version 47."

### 4.4 Prompt Performance Tracking

Helicone's prompt experiments feature enables:

- Side-by-side evaluation with real production data.
- LLM-as-a-judge scoring for quantitative comparison.
- Confidence metrics before pushing to production.

**The pattern**: Prompt versions are not just text artifacts -- they are linked to
their runtime performance metrics. The version list shows not just "when was this
created" but "how well did it perform in production."

---

## 5. Memory & RAG Quality Panels

RAG quality monitoring is the newest frontier in LLM observability. Most platforms
are still building this out.

### 5.1 Retrieval Span Visualization

Within the trace tree, retrieval operations should be first-class spans:

```
v RAG: answer-question
  > embed query (0.1s, $0.0001)
  v vector search (0.3s)
    Retrieved 5 chunks:
    [1] score: 0.94 | doc: policy-v3.md | chunk: "Refund policy allows..."
    [2] score: 0.87 | doc: faq.md       | chunk: "Returns within 30..."
    [3] score: 0.71 | doc: terms.md     | chunk: "Liability limitations..."
    [4] score: 0.65 | doc: old-faq.md   | chunk: "Previous policy was..."
    [5] score: 0.42 | doc: blog.md      | chunk: "We recently updated..."
  > rerank (0.2s)
    Kept: [1], [2], [3]  |  Dropped: [4], [5]
  > LLM call (1.1s, $0.01)
    Context window: 3 chunks, 1,247 tokens
```

**Key elements**:
- Similarity scores visible at the chunk level.
- Source document identification (which doc, which chunk).
- Reranking step showing what was kept and what was dropped.
- Context window size: how many tokens of retrieved context went to the LLM.

### 5.2 Retrieval Quality Metrics

**Arize Phoenix** provides evaluation of retrieval quality through:
- Relevance scoring: Are retrieved documents actually relevant to the query?
- Faithfulness: Does the LLM response faithfully represent the retrieved content?
- Hallucination detection: Did the LLM generate claims not supported by context?

These metrics should be displayed as:

| Metric | Display Pattern |
|---|---|
| Relevance | Score bar (0-1) per retrieved chunk, color-coded |
| Faithfulness | Binary badge (faithful/unfaithful) with supporting evidence |
| Hallucination | Highlighted spans in the output text where claims lack support |
| Context precision | Ratio of relevant chunks to total chunks retrieved |
| Context recall | Ratio of relevant info captured vs. available in the corpus |

### 5.3 Embedding Drift Visualization

When embedding models change or the knowledge base evolves, retrieval quality
can degrade silently. The monitoring pattern:

- **Distribution chart**: Similarity score distribution over time. A leftward
  shift indicates degrading retrieval quality.
- **Threshold line**: Minimum acceptable similarity score. Items below the line
  are flagged.
- **Cluster visualization**: 2D projection (UMAP/t-SNE) of query embeddings
  colored by retrieval success. Clusters of failures indicate knowledge gaps.

### 5.4 Knowledge Base Coverage Panel

For operators monitoring a RAG system daily:

```
+------------------------------------------------------------------+
| KNOWLEDGE BASE HEALTH                                             |
+------------------------------------------------------------------+
| Documents: 1,247  |  Chunks: 45,892  |  Last updated: 2h ago     |
+------------------------------------------------------------------+
|                                                                    |
| Query Coverage (last 7d):                                          |
| - Fully answered: 78% (green)                                      |
| - Partially answered: 15% (yellow)                                 |
| - No relevant context found: 7% (red)                              |
|                                                                    |
| Top Unanswered Topics:                                             |
| 1. "International shipping rates" (23 queries, no relevant docs)   |
| 2. "Warranty extension process" (17 queries, low relevance)        |
| 3. "Bulk order discounts" (12 queries, outdated docs)              |
|                                                                    |
| [Add Documents] [Re-index] [View Failed Queries]                  |
+------------------------------------------------------------------+
```

This pattern does not exist in any current tool as a built-in feature. It requires
combining retrieval span data with evaluation metrics to infer coverage.

---

## 6. What Operators Need to Trust a Dashboard Daily

This section synthesizes patterns from platforms that have achieved daily-driver
status with operations teams, versus tools that get set up and then ignored.

### 6.1 The Five-Second Landing Page

When an operator opens the dashboard at 9 AM, they need to answer five questions
in five seconds:

1. **Is anything broken right now?** (error rate indicator)
2. **How much are we spending?** (running cost total)
3. **Are we within budget?** (budget utilization bar)
4. **Is quality holding?** (aggregate quality score or trend)
5. **Are there items needing my attention?** (approval/review badge count)

**The pattern**: A row of 4-5 large metric cards at the top of the dashboard,
each with a current value, trend arrow, and spark chart. Red/yellow/green
coloring for at-a-glance status.

```
+------------+  +------------+  +------------+  +------------+  +----------+
| Error Rate |  | Cost Today |  | Budget     |  | Quality    |  | Reviews  |
| 0.3%  ↓    |  | $127  ↑    |  | 64%        |  | 0.89  →    |  | 3 pending|
| [sparkline]|  | [sparkline]|  | [progress] |  | [sparkline]|  | [badge]  |
+------------+  +------------+  +------------+  +------------+  +----------+
```

### 6.2 Anomaly Detection, Not Just Metrics

Raw metrics require interpretation. What operators actually want:

- **"Error rate spiked 3x in the last hour"** -- not "error rate is 1.2%."
- **"Cost is 40% higher than same time last week"** -- not "cost is $127."
- **"Quality dropped after prompt v5 was deployed"** -- not "quality is 0.81."

**The pattern**: Alert cards that describe what changed, when, and the likely
cause. Humanloop describes this as turning evaluation baselines into production
guardrails -- the thresholds set during testing become the anomaly triggers in
production.

### 6.3 Time Controls That Actually Work

Every platform has a date range picker. Most implement it poorly.

**What works**:
- **Preset buttons**: Today, 7d, 30d, Custom. Not a calendar-first picker.
- **Relative mode by default**: "Last 7 days" updates on refresh. Absolute mode
  available for incident investigation.
- **Comparison toggle**: "Compare to previous period" as a single checkbox. Shows
  dotted overlay on all charts.
- **Auto-refresh indicator**: A subtle spinner or countdown showing when data
  will refresh. Manual refresh button always visible.

**What does not work**:
- Calendar pickers that require two clicks to select today.
- No timezone indicator (critical for distributed teams).
- Losing the selected time range when navigating between pages.
- No URL-encoded time range (cannot share a link to "look at this spike on Tuesday").

### 6.4 Filtering Without Friction

**Helicone's approach**: Custom Properties become table columns, making them
immediately scannable. Combined with AND/OR operators for cross-dimensional
filtering.

**Langfuse's approach**: Filter by trace name, user, tags, release version, and
custom metadata. Segments persist across navigation.

**The pattern that builds trust**:
1. **Saved filters / views**: Name and bookmark common filter combinations.
   "Production errors last 24h" should not require 4 clicks to reconstruct.
2. **Filter pills**: Active filters shown as dismissible pills at the top.
   Users always know what subset of data they are seeing.
3. **Deep links**: Every filter state encoded in the URL. Sharing a dashboard
   view means sharing a URL.
4. **Zero-result guidance**: When filters return no data, show "No traces match
   your filters" with a suggestion to broaden, not just an empty table.

### 6.5 Latency and Freshness Transparency

Operators distrust dashboards that feel stale. The tools that earn daily use:

- Show **"Last updated: 3 seconds ago"** in the dashboard footer.
- Show **data pipeline latency**: "Traces appear within ~5 seconds of completion."
- Show **ingestion status**: "Healthy" / "Delayed" / "Down" indicator.
- During outages, show **explicit degradation notices** rather than silently
  showing incomplete data.

### 6.6 The Audit Trail

Enterprise operators will not adopt a tool that does not answer "who did what when."

| Action | Audit Record |
|---|---|
| Prompt version promoted | "@jane promoted summarize-ticket v5 to production at 14:32" |
| Trace annotated | "@bob scored trace #8831 as 0.8 (quality) at 09:15" |
| Budget limit changed | "@admin set engineering-team budget to $2,000 at 11:00" |
| Guardrail triggered | "PII detection blocked response for trace #9012 at 16:45" |
| Approval decision | "@jane approved DELETE /api/users/8831 at 14:32, session #4521" |

The audit log should be a dedicated page, filterable by actor, action type, and
time range. Each entry links to the relevant artifact (trace, prompt, config).

---

## 7. Enterprise vs. Hobby: What Actually Differentiates

Based on pricing pages and feature matrices from Helicone, Langfuse, and Portkey,
the differentiation falls into five categories.

### 7.1 Access Control and Team Structure

| Capability | Hobby | Enterprise |
|---|---|---|
| Users | 1-3 | Unlimited |
| Organizations | 1 | Unlimited |
| RBAC (role-based access) | No | Yes |
| SAML SSO | No | Yes |
| Audit logs | No | Yes |

**UX implication**: Enterprise dashboards need role-aware views. An annotator
sees the review queue prominently. A budget owner sees cost dashboards first. An
engineer sees traces. The landing page should adapt to the user's role.

### 7.2 Data Retention and Scale

| Capability | Hobby | Enterprise |
|---|---|---|
| Data retention | 7 days (Helicone free) | Forever |
| Ingestion rate | 10 logs/min (Helicone free) | 30,000 logs/min |
| API rate limits | Tight | Unlimited |

**UX implication**: Hobby tools can get away with in-memory filtering. Enterprise
tools need server-side filtering, pagination, and search that works over millions
of traces without the browser locking up.

### 7.3 Compliance and Deployment

| Capability | Hobby | Enterprise |
|---|---|---|
| SOC-2 compliance | No | Yes |
| HIPAA compliance | No | Yes |
| On-premises deployment | No | Yes |
| Custom MSAs | No | Yes |

**UX implication**: Enterprise dashboards must support data masking (PII redaction
in trace displays), configurable data residency indicators, and compliance
badges showing which certifications the instance meets.

### 7.4 Support and Customization

| Capability | Hobby | Enterprise |
|---|---|---|
| Support channel | Community/forum | Dedicated engineer, private Slack |
| Custom dashboards | No | Yes |
| Bulk pricing | No | Yes |
| InfoSec reviews | No | Yes |

### 7.5 What Actually Matters for UI Design

The enterprise vs. hobby distinction is not about adding more features. It is
about three things:

1. **Trust signals**: Audit trails, compliance badges, data freshness indicators,
   role-based views. Operators will not put a tool in the daily rotation unless
   they trust the data.

2. **Scale gracefully**: A dashboard that works with 100 traces/day must also work
   with 100,000 traces/day. This means server-side everything (filtering, sorting,
   search, aggregation) and progressive loading patterns.

3. **Team workflows**: Hobby tools serve an individual developer debugging their
   own code. Enterprise tools serve a team where one person writes the prompt,
   another reviews the quality, another monitors the cost, and a fourth approves
   high-risk actions. The UI must support handoff between these roles.

---

## Source Index

| # | Source | URL | Key Contribution |
|---|---|---|---|
| 1 | Langfuse Tracing Docs | langfuse.com/docs/tracing | Trace tree structure, nested observations |
| 2 | Langfuse Sessions | langfuse.com/docs/tracing-features/sessions | Session replay, multi-turn grouping |
| 3 | Langfuse Analytics | langfuse.com/docs/analytics/overview | Cost segmentation dimensions, BI export |
| 4 | Langfuse Scores | langfuse.com/docs/scores/overview | Evaluation metrics, regression detection |
| 5 | Langfuse Annotation | langfuse.com/docs/scores/annotation | Annotation workflow, scoring widgets |
| 6 | Langfuse Prompts | langfuse.com/docs/prompts/get-started | Version control, label-based deployment |
| 7 | Langfuse Datasets | langfuse.com/docs/datasets/overview | Dataset browser, experiment comparison |
| 8 | Langfuse Trace URLs | langfuse.com/docs/tracing-features/url | Deep linking, public sharing |
| 9 | Helicone Quick Start | docs.helicone.ai/getting-started/quick-start | Request logging views |
| 10 | Helicone Sessions | docs.helicone.ai/features/sessions | Path-based grouping, waterfall timeline |
| 11 | Helicone Caching | docs.helicone.ai/features/advanced-usage/caching | Cost savings visualization |
| 12 | Helicone User Metrics | docs.helicone.ai/features/advanced-usage/user-metrics | Per-user cost tracking |
| 13 | Helicone Custom Props | docs.helicone.ai/features/advanced-usage/custom-properties | Filtering and segmentation |
| 14 | Helicone Pricing | helicone.ai/pricing | Enterprise vs hobby feature matrix |
| 15 | Helicone Prompt Mgmt | helicone.ai/blog/prompt-management | Version control, A/B testing, sandbox |
| 16 | Helicone Observability | helicone.ai/blog/llm-observability | Cost per-request, anomaly detection |
| 17 | Arize Phoenix Docs | arize.com/docs/phoenix | Span replay, evaluation tabs, annotation |
| 18 | LangSmith Announcement | blog.langchain.com/announcing-langsmith | Run trees, playground, performance views |
| 19 | LangSmith Evaluation | docs.langchain.com/langsmith/evaluate-llm-application | Experiments table, scoring interface |
| 20 | LangSmith Observability | docs.langchain.com/langsmith/observability | Card-based nav, automation rules |
| 21 | LangSmith Annotation Queues | docs.langchain.com/langsmith/annotation-queues | Queue management, pairwise review, reservations |
| 22 | Braintrust Tracing | braintrust.dev/docs/guides/tracing | Icon-based span types, score embedding |
| 23 | Portkey Observability | portkey.ai/docs/product/observability | 21+ metrics, budget limits, metadata |
| 24 | Portkey Traces | portkey.ai/docs/product/observability/traces | OTel spans, waterfall rendering, cost per span |
| 25 | Humanloop Monitoring | humanloop.com/blog/llm-monitoring | Evaluation baselines as guardrails, alert patterns |
