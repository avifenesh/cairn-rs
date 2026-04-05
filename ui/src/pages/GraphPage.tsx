import { useState } from "react";
import { Search, Network, Info, ArrowRight, ExternalLink } from "lucide-react";
import { clsx } from "clsx";

// ── Node/edge schema — mirrors cairn_graph::projections exactly ───────────────

interface NodeTypeDef {
  kind: string;
  label: string;
  description: string;
  color: string;       // Tailwind border/text color classes
  bgColor: string;     // badge background
  group: "runtime" | "memory" | "prompts" | "infra";
}

interface EdgeTypeDef {
  kind: string;
  label: string;
  description: string;
}

const NODE_TYPES: NodeTypeDef[] = [
  // Runtime group
  { kind: "session",        label: "Session",         description: "Conversation session container",    color: "border-blue-500/40 text-blue-400",    bgColor: "bg-blue-950/60 text-blue-300",    group: "runtime"  },
  { kind: "run",            label: "Run",             description: "Agent execution instance",          color: "border-indigo-500/40 text-indigo-400", bgColor: "bg-indigo-950/60 text-indigo-300", group: "runtime"  },
  { kind: "task",           label: "Task",            description: "Queued unit of work",               color: "border-violet-500/40 text-violet-400", bgColor: "bg-violet-950/60 text-violet-300", group: "runtime"  },
  { kind: "approval",       label: "Approval",        description: "Human-in-the-loop gate",            color: "border-amber-500/40 text-amber-400",   bgColor: "bg-amber-950/60 text-amber-300",   group: "runtime"  },
  { kind: "checkpoint",     label: "Checkpoint",      description: "Resumable execution snapshot",      color: "border-emerald-500/40 text-emerald-400", bgColor: "bg-emerald-950/60 text-emerald-300", group: "runtime" },
  { kind: "mailbox_message",label: "Mailbox Msg",     description: "Agent-to-agent message",            color: "border-sky-500/40 text-sky-400",       bgColor: "bg-sky-950/60 text-sky-300",       group: "runtime"  },
  { kind: "tool_invocation",label: "Tool Call",       description: "External tool invocation",          color: "border-orange-500/40 text-orange-400", bgColor: "bg-orange-950/60 text-orange-300", group: "runtime"  },
  { kind: "route_decision", label: "Route Decision",  description: "Provider routing decision",         color: "border-rose-500/40 text-rose-400",     bgColor: "bg-rose-950/60 text-rose-300",     group: "runtime"  },
  { kind: "provider_call",  label: "Provider Call",   description: "LLM provider API call",             color: "border-pink-500/40 text-pink-400",     bgColor: "bg-pink-950/60 text-pink-300",     group: "runtime"  },
  // Memory group
  { kind: "memory",         label: "Memory",          description: "Knowledge store entry",             color: "border-purple-500/40 text-purple-400", bgColor: "bg-purple-950/60 text-purple-300", group: "memory"   },
  { kind: "document",       label: "Document",        description: "Ingested source document",          color: "border-teal-500/40 text-teal-400",     bgColor: "bg-teal-950/60 text-teal-300",     group: "memory"   },
  { kind: "chunk",          label: "Chunk",           description: "Indexed document fragment",         color: "border-green-500/40 text-green-400",   bgColor: "bg-green-950/60 text-green-300",   group: "memory"   },
  { kind: "source",         label: "Source",          description: "Signal / document source",          color: "border-cyan-500/40 text-cyan-400",     bgColor: "bg-cyan-950/60 text-cyan-300",     group: "memory"   },
  { kind: "ingest_job",     label: "Ingest Job",      description: "Document ingest pipeline run",      color: "border-lime-500/40 text-lime-400",     bgColor: "bg-lime-950/60 text-lime-300",     group: "memory"   },
  { kind: "signal",         label: "Signal",          description: "External signal event",             color: "border-zinc-500/40 text-zinc-400",     bgColor: "bg-zinc-800/60 text-zinc-300",     group: "memory"   },
  // Prompts group
  { kind: "prompt_asset",   label: "Prompt Asset",    description: "Prompt template asset",             color: "border-fuchsia-500/40 text-fuchsia-400", bgColor: "bg-fuchsia-950/60 text-fuchsia-300", group: "prompts" },
  { kind: "prompt_version", label: "Prompt Version",  description: "Versioned prompt snapshot",         color: "border-pink-500/40 text-pink-400",     bgColor: "bg-pink-950/60 text-pink-300",     group: "prompts"  },
  { kind: "prompt_release", label: "Prompt Release",  description: "Deployed prompt release",           color: "border-red-500/40 text-red-400",       bgColor: "bg-red-950/60 text-red-300",       group: "prompts"  },
  { kind: "eval_run",       label: "Eval Run",        description: "Evaluation run record",             color: "border-yellow-500/40 text-yellow-400", bgColor: "bg-yellow-950/60 text-yellow-300", group: "prompts"  },
  // Infra group
  { kind: "skill",          label: "Skill",           description: "Agent capability definition",       color: "border-indigo-400/40 text-indigo-300", bgColor: "bg-indigo-950/40 text-indigo-200", group: "infra"    },
  { kind: "channel_target", label: "Channel",         description: "Notification channel target",       color: "border-sky-400/40 text-sky-300",       bgColor: "bg-sky-950/40 text-sky-200",       group: "infra"    },
];

const EDGE_TYPES: EdgeTypeDef[] = [
  { kind: "triggered",    label: "Triggered",     description: "Session/run triggered a downstream run or task"     },
  { kind: "spawned",      label: "Spawned",       description: "Run spawned a sub-run or child task"                },
  { kind: "depended_on",  label: "Depended On",   description: "Task depends on another task completing first"       },
  { kind: "approved_by",  label: "Approved By",   description: "Run or task was gated by an approval decision"      },
  { kind: "resumed_from", label: "Resumed From",  description: "Execution resumed from a saved checkpoint"          },
  { kind: "sent_to",      label: "Sent To",       description: "Message sent to an agent or channel target"         },
  { kind: "read_from",    label: "Read From",     description: "Memory or chunk was read from a source"             },
  { kind: "cited",        label: "Cited",         description: "Run or task cited a memory chunk in a response"     },
  { kind: "derived_from", label: "Derived From",  description: "Document or chunk derived from a parent document"   },
  { kind: "embedded_as",  label: "Embedded As",   description: "Document text embedded as a vector chunk"           },
  { kind: "evaluated_by", label: "Evaluated By",  description: "Prompt release evaluated by an eval run"            },
  { kind: "released_as",  label: "Released As",   description: "Prompt version released as a deployable release"    },
  { kind: "rolled_back_to",label: "Rolled Back",  description: "Deployment rolled back to a previous release"       },
  { kind: "routed_to",    label: "Routed To",     description: "Request routed to a specific provider"              },
  { kind: "used_prompt",  label: "Used Prompt",   description: "Run or task used a specific prompt release"         },
  { kind: "used_tool",    label: "Used Tool",     description: "Run invoked a registered tool or skill"             },
  { kind: "called_provider",label: "Called Provider", description: "Task made an LLM provider call"                },
];

const GROUP_LABELS: Record<NodeTypeDef["group"], string> = {
  runtime: "Runtime Execution",
  memory:  "Memory & Knowledge",
  prompts: "Prompts & Evals",
  infra:   "Infrastructure",
};

const GROUPS: NodeTypeDef["group"][] = ["runtime", "memory", "prompts", "infra"];

// ── Stat card ─────────────────────────────────────────────────────────────────

function StatCard({ label, value, sub, accent = "border-l-zinc-700" }: {
  label: string; value: string | number; sub?: string; accent?: string;
}) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent)}>
      <p className="text-[11px] text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

// ── Node type card ────────────────────────────────────────────────────────────

function NodeCard({ node, selected, onClick }: {
  node: NodeTypeDef; selected: boolean; onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "w-full text-left rounded-lg border p-3 transition-all",
        selected
          ? clsx("ring-1 ring-indigo-500/60 bg-zinc-800/60", node.color)
          : clsx("border-zinc-800 bg-zinc-900 hover:bg-zinc-800/60 hover:border-zinc-700"),
      )}
    >
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <span className={clsx("text-[10px] font-mono font-medium rounded px-1.5 py-0.5", node.bgColor)}>
          {node.kind}
        </span>
        {selected && <ArrowRight size={11} className="text-indigo-400 shrink-0 mt-0.5" />}
      </div>
      <p className="text-[12px] font-medium text-zinc-200 truncate">{node.label}</p>
      <p className="text-[11px] text-zinc-600 mt-0.5 leading-snug">{node.description}</p>
    </button>
  );
}

// ── Detail panel ──────────────────────────────────────────────────────────────

function NodeDetail({ node, onClose }: { node: NodeTypeDef; onClose: () => void }) {
  // Show which edges are relevant to this node kind based on its group
  const relevantEdges = EDGE_TYPES.filter((e) => {
    const k = node.kind;
    if (k === "run")             return ["triggered","spawned","approved_by","used_prompt","used_tool","called_provider","resumed_from"].includes(e.kind);
    if (k === "task")            return ["depended_on","approved_by","used_tool","called_provider"].includes(e.kind);
    if (k === "session")         return ["triggered"].includes(e.kind);
    if (k === "approval")        return ["approved_by"].includes(e.kind);
    if (k === "checkpoint")      return ["resumed_from"].includes(e.kind);
    if (k === "mailbox_message") return ["sent_to"].includes(e.kind);
    if (k === "tool_invocation") return ["used_tool"].includes(e.kind);
    if (k === "memory")          return ["cited","read_from"].includes(e.kind);
    if (k === "document")        return ["derived_from","embedded_as","read_from"].includes(e.kind);
    if (k === "chunk")           return ["embedded_as","cited","derived_from"].includes(e.kind);
    if (k === "source")          return ["read_from"].includes(e.kind);
    if (k === "prompt_asset")    return ["released_as"].includes(e.kind);
    if (k === "prompt_version")  return ["released_as","rolled_back_to"].includes(e.kind);
    if (k === "prompt_release")  return ["used_prompt","evaluated_by","rolled_back_to"].includes(e.kind);
    if (k === "eval_run")        return ["evaluated_by"].includes(e.kind);
    if (k === "provider_call")   return ["called_provider","routed_to"].includes(e.kind);
    if (k === "route_decision")  return ["routed_to"].includes(e.kind);
    if (k === "skill")           return ["used_tool"].includes(e.kind);
    if (k === "channel_target")  return ["sent_to"].includes(e.kind);
    if (k === "signal")          return ["read_from"].includes(e.kind);
    if (k === "ingest_job")      return ["derived_from","embedded_as"].includes(e.kind);
    return false;
  });

  return (
    <aside className="w-72 shrink-0 border-l border-zinc-800 bg-zinc-950 flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 border-b border-zinc-800">
        <div>
          <span className={clsx("text-[10px] font-mono rounded px-1.5 py-0.5", node.bgColor)}>
            {node.kind}
          </span>
          <p className="text-[13px] font-medium text-zinc-200 mt-1">{node.label}</p>
        </div>
        <button onClick={onClose}
          className="p-1 rounded text-zinc-600 hover:text-zinc-300 hover:bg-zinc-800 transition-colors">
          ×
        </button>
      </div>

      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        <div>
          <p className="text-[11px] text-zinc-500 mb-1">{node.description}</p>
        </div>

        <div>
          <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">
            Connected via
          </p>
          {relevantEdges.length === 0 ? (
            <p className="text-[11px] text-zinc-700 italic">No edges defined</p>
          ) : (
            <div className="space-y-1.5">
              {relevantEdges.map((e) => (
                <div key={e.kind} className="rounded bg-zinc-900 border border-zinc-800 px-2.5 py-1.5">
                  <p className="text-[11px] font-mono text-zinc-400">{e.kind}</p>
                  <p className="text-[10px] text-zinc-600 mt-0.5">{e.description}</p>
                </div>
              ))}
            </div>
          )}
        </div>

        <div className="rounded-lg bg-zinc-900 border border-zinc-800 px-3 py-2.5">
          <div className="flex items-center gap-1.5 mb-1">
            <Info size={10} className="text-zinc-600" />
            <span className="text-[10px] text-zinc-600 font-medium">Live count</span>
          </div>
          <p className="text-[13px] font-mono text-zinc-600 italic">— backend offline</p>
        </div>
      </div>
    </aside>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

export function GraphPage() {
  const [query, setQuery]           = useState("");
  const [selectedNode, setSelectedNode] = useState<NodeTypeDef | null>(null);

  const lowerQuery = query.toLowerCase();
  const filteredNodes = NODE_TYPES.filter((n) =>
    !lowerQuery ||
    n.kind.includes(lowerQuery) ||
    n.label.toLowerCase().includes(lowerQuery) ||
    n.description.toLowerCase().includes(lowerQuery),
  );

  function handleNodeClick(node: NodeTypeDef) {
    setSelectedNode((prev) => (prev?.kind === node.kind ? null : node));
  }

  return (
    <div className="flex flex-col h-full bg-zinc-950">
      {/* ── Toolbar ─────────────────────────────────────────────────── */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-zinc-800 shrink-0">
        <Network size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-zinc-500 uppercase tracking-wider">
          Knowledge Graph
        </span>
        <div className="ml-auto flex items-center gap-3">
          <span className="text-[11px] text-zinc-700">RFC 004</span>
        </div>
      </div>

      {/* ── Body ─────────────────────────────────────────────────────── */}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        <div className="flex-1 overflow-y-auto min-w-0">
          <div className="max-w-4xl mx-auto px-5 py-5 space-y-6">

            {/* Stat strip */}
            <div className="flex items-start gap-8 py-3 px-4 rounded-lg border border-zinc-800 bg-zinc-900/60">
              <StatCard label="Node Types"  value={NODE_TYPES.length}  sub="defined in schema"  accent="border-l-indigo-500" />
              <StatCard label="Edge Types"  value={EDGE_TYPES.length}  sub="relationship kinds" accent="border-l-indigo-500" />
              <StatCard label="Live Nodes"  value="—"                  sub="backend offline"    accent="border-l-zinc-700"   />
              <StatCard label="Live Edges"  value="—"                  sub="backend offline"    accent="border-l-zinc-700"   />
            </div>

            {/* Offline notice */}
            <div className="rounded-lg border border-amber-800/40 bg-amber-500/5 px-4 py-3 flex items-start gap-3">
              <Info size={14} className="text-amber-500 shrink-0 mt-0.5" />
              <div className="flex-1 min-w-0">
                <p className="text-[12px] font-medium text-amber-400">
                  Graph backend not connected
                </p>
                <p className="text-[11px] text-amber-700 mt-0.5">
                  Live node/edge data requires RFC 004 HTTP endpoints (
                  <code className="text-amber-600">GET /v1/graph/nodes</code>,{" "}
                  <code className="text-amber-600">POST /v1/graph/query</code>).
                  The schema below reflects the compiled type system.
                </p>
              </div>
              <a
                href="#memory"
                onClick={() => { window.location.hash = "memory"; }}
                className="shrink-0 flex items-center gap-1 text-[11px] text-amber-600 hover:text-amber-400 transition-colors"
              >
                Memory search <ExternalLink size={10} />
              </a>
            </div>

            {/* Search */}
            <div className="relative">
              <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-zinc-600 pointer-events-none" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Filter node types… (e.g. 'run', 'prompt', 'memory')"
                className="w-full rounded-lg border border-zinc-800 bg-zinc-900 text-[13px] text-zinc-200
                           placeholder-zinc-600 pl-9 pr-4 py-2 focus:outline-none focus:border-indigo-500 transition-colors"
              />
              {query && (
                <button onClick={() => setQuery("")}
                  className="absolute right-3 top-1/2 -translate-y-1/2 text-zinc-600 hover:text-zinc-400 transition-colors">
                  ×
                </button>
              )}
            </div>

            {/* Node type grid — grouped */}
            {query ? (
              /* Flat filtered results */
              filteredNodes.length === 0 ? (
                <p className="text-[13px] text-zinc-600 italic py-4 text-center">
                  No node types match &ldquo;{query}&rdquo;
                </p>
              ) : (
                <div>
                  <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-3">
                    {filteredNodes.length} result{filteredNodes.length !== 1 ? "s" : ""}
                  </p>
                  <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
                    {filteredNodes.map((n) => (
                      <NodeCard
                        key={n.kind}
                        node={n}
                        selected={selectedNode?.kind === n.kind}
                        onClick={() => handleNodeClick(n)}
                      />
                    ))}
                  </div>
                </div>
              )
            ) : (
              /* Grouped view */
              <div className="space-y-5">
                {GROUPS.map((group) => {
                  const nodes = NODE_TYPES.filter((n) => n.group === group);
                  return (
                    <div key={group}>
                      <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-2">
                        {GROUP_LABELS[group]}
                        <span className="ml-2 text-zinc-700 normal-case font-normal">
                          {nodes.length} types
                        </span>
                      </p>
                      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
                        {nodes.map((n) => (
                          <NodeCard
                            key={n.kind}
                            node={n}
                            selected={selectedNode?.kind === n.kind}
                            onClick={() => handleNodeClick(n)}
                          />
                        ))}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            {/* Edge types reference */}
            <div>
              <p className="text-[10px] font-semibold text-zinc-600 uppercase tracking-wider mb-3">
                Edge Types
                <span className="ml-2 text-zinc-700 normal-case font-normal">
                  {EDGE_TYPES.length} relationship kinds
                </span>
              </p>
              <div className="rounded-lg border border-zinc-800 overflow-hidden">
                <table className="min-w-full text-[12px]">
                  <thead className="bg-zinc-900">
                    <tr>
                      <th className="px-3 py-2 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider border-b border-zinc-800 w-40">Kind</th>
                      <th className="px-3 py-2 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider border-b border-zinc-800 w-32">Label</th>
                      <th className="px-3 py-2 text-left text-[10px] font-medium text-zinc-600 uppercase tracking-wider border-b border-zinc-800">Description</th>
                    </tr>
                  </thead>
                  <tbody className="divide-y divide-zinc-800/50">
                    {EDGE_TYPES.map((e, i) => (
                      <tr key={e.kind} className={clsx(
                        "transition-colors hover:bg-zinc-800/40",
                        i % 2 === 0 ? "bg-zinc-900" : "bg-[#111113]",
                      )}>
                        <td className="px-3 py-1.5 font-mono text-[11px] text-zinc-500 whitespace-nowrap">
                          {e.kind}
                        </td>
                        <td className="px-3 py-1.5 text-zinc-400 whitespace-nowrap">
                          {e.label}
                        </td>
                        <td className="px-3 py-1.5 text-zinc-600">
                          {e.description}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </div>

          </div>
        </div>

        {/* Node detail panel */}
        {selectedNode && (
          <NodeDetail node={selectedNode} onClose={() => setSelectedNode(null)} />
        )}
      </div>
    </div>
  );
}

export default GraphPage;
