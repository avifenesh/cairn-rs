# Phase 0 Upstream Source Pointers

Status: generated  
Purpose: pin the preserved Phase 0 compatibility contract to exact upstream files and line numbers in `../cairn-sdk`, so Worker 1 evidence stays auditable even without direct legacy backend handler captures.

Current reading:

- these pointers intentionally reference frontend and protocol sources because the local upstream checkout still does not expose concrete handler implementations for the preserved `/v1/*` surfaces
- if direct backend capture becomes available later, it should supplement these pointers rather than erase the preserved frontend contract lineage

## HTTP Source Pointers

| Requirement | Base Route | Upstream Source Pointers |
|---|---|---|
| `GET /v1/feed?limit=20&unread=true` | `GET /v1/feed` | <code>frontend/src/lib/api/client.ts:93</code><br><code>frontend/src/lib/api/client.ts:96</code><br><code>frontend/src/lib/api/client.ts:97</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:103</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:104</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:105</code><br><code>docs/design/pieces/09-server-protocols.md:29</code> |
| `GET /v1/tasks?status=running&type=agent` | `GET /v1/tasks` | <code>frontend/src/lib/api/client.ts:105</code><br><code>frontend/src/lib/api/client.ts:108</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:107</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:108</code><br><code>docs/design/pieces/09-server-protocols.md:35</code><br><code>docs/design/pieces/09-server-protocols.md:36</code> |
| `GET /v1/approvals?status=pending` | `GET /v1/approvals` | <code>frontend/src/lib/api/client.ts:115</code><br><code>frontend/src/lib/api/client.ts:118</code><br><code>frontend/src/lib/api/client.ts:119</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:109</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:110</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:111</code><br><code>docs/design/pieces/09-server-protocols.md:39</code><br><code>docs/design/pieces/09-server-protocols.md:40</code> |
| `GET /v1/memories/search?q=test&limit=10` | `GET /v1/memories/search` | <code>frontend/src/lib/api/client.ts:160</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:118</code> |
| `POST /v1/assistant/message body={message,mode?,sessionId?}` | `POST /v1/assistant/message` | <code>frontend/src/lib/api/client.ts:129</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:114</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:171</code><br><code>docs/design/pieces/09-server-protocols.md:32</code> |
| `POST /v1/assistant/message body={message,mode?}` | `POST /v1/assistant/message` | <code>frontend/src/lib/api/client.ts:129</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:114</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:171</code><br><code>docs/design/pieces/09-server-protocols.md:32</code> |
| `GET /v1/stream?lastEventId=<id>` | `GET /v1/stream` | <code>docs/design/FRONTEND_AGENT_BRIEF.md:106</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:137</code><br><code>docs/design/pieces/09-server-protocols.md:31</code> |

## SSE Source Pointers

| Event | Upstream Source Pointers |
|---|---|
| `ready` | <code>frontend/src/lib/stores/sse.svelte.ts:71</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:141</code><br><code>frontend/src/lib/types.ts:154</code> |
| `feed_update` | <code>frontend/src/lib/stores/sse.svelte.ts:78</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:142</code><br><code>frontend/src/lib/types.ts:155</code> |
| `poll_completed` | <code>frontend/src/lib/stores/sse.svelte.ts:83</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:143</code><br><code>frontend/src/lib/types.ts:156</code> |
| `task_update` | <code>frontend/src/lib/stores/sse.svelte.ts:89</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:144</code><br><code>frontend/src/lib/types.ts:157</code><br><code>docs/design/pieces/10-frontend.md:144</code> |
| `approval_required` | <code>frontend/src/lib/stores/sse.svelte.ts:94</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:145</code><br><code>frontend/src/lib/types.ts:158</code> |
| `assistant_delta` | <code>frontend/src/lib/stores/sse.svelte.ts:100</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:146</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:173</code><br><code>frontend/src/lib/types.ts:159</code><br><code>docs/design/pieces/10-frontend.md:132</code> |
| `assistant_end` | <code>frontend/src/lib/stores/sse.svelte.ts:105</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:147</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:174</code><br><code>frontend/src/lib/types.ts:160</code><br><code>docs/design/pieces/10-frontend.md:136</code> |
| `assistant_reasoning` | <code>frontend/src/lib/stores/sse.svelte.ts:110</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:148</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:176</code><br><code>frontend/src/lib/types.ts:161</code><br><code>docs/design/pieces/10-frontend.md:140</code> |
| `assistant_tool_call` | <code>frontend/src/lib/stores/sse.svelte.ts:115</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:149</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:175</code><br><code>frontend/src/lib/types.ts:162</code> |
| `memory_proposed` | <code>frontend/src/lib/stores/sse.svelte.ts:121</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:150</code><br><code>frontend/src/lib/types.ts:163</code> |
| `agent_progress` | <code>frontend/src/lib/stores/sse.svelte.ts:150</code><br><code>docs/design/FRONTEND_AGENT_BRIEF.md:155</code><br><code>frontend/src/lib/types.ts:168</code> |
