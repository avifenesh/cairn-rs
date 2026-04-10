import { useState, useEffect, useRef, useMemo } from "react";
import {
  Search, Network, Info, ArrowRight, ExternalLink,
  RotateCcw, ZoomIn, ZoomOut,
} from "lucide-react";
import { clsx } from "clsx";

// ── Node/edge schema — mirrors cairn_graph::projections exactly ───────────────

interface NodeTypeDef {
  kind: string;
  label: string;
  description: string;
  color: string;
  bgColor: string;
  group: "runtime" | "memory" | "prompts" | "infra";
}

interface EdgeTypeDef {
  kind: string;
  label: string;
  description: string;
}

const NODE_TYPES: NodeTypeDef[] = [
  { kind: "session",         label: "Session",        description: "Conversation session container",    color: "border-blue-500/40 text-blue-400",       bgColor: "bg-blue-950/60 text-blue-300",       group: "runtime"  },
  { kind: "run",             label: "Run",             description: "Agent execution instance",          color: "border-indigo-500/40 text-indigo-400",   bgColor: "bg-indigo-950/60 text-indigo-300",   group: "runtime"  },
  { kind: "task",            label: "Task",            description: "Queued unit of work",               color: "border-violet-500/40 text-violet-400",   bgColor: "bg-violet-950/60 text-violet-300",   group: "runtime"  },
  { kind: "approval",        label: "Approval",        description: "Human-in-the-loop gate",            color: "border-amber-500/40 text-amber-400",     bgColor: "bg-amber-950/60 text-amber-300",     group: "runtime"  },
  { kind: "checkpoint",      label: "Checkpoint",      description: "Resumable execution snapshot",      color: "border-emerald-500/40 text-emerald-400", bgColor: "bg-emerald-950/60 text-emerald-300", group: "runtime"  },
  { kind: "mailbox_message", label: "Mailbox Msg",     description: "Agent-to-agent message",            color: "border-sky-500/40 text-sky-400",         bgColor: "bg-sky-950/60 text-sky-300",         group: "runtime"  },
  { kind: "tool_invocation", label: "Tool Call",       description: "External tool invocation",          color: "border-orange-500/40 text-orange-400",   bgColor: "bg-orange-950/60 text-orange-300",   group: "runtime"  },
  { kind: "route_decision",  label: "Route Decision",  description: "Provider routing decision",         color: "border-rose-500/40 text-rose-400",       bgColor: "bg-rose-950/60 text-rose-300",       group: "runtime"  },
  { kind: "provider_call",   label: "Provider Call",   description: "LLM provider API call",             color: "border-pink-500/40 text-pink-400",       bgColor: "bg-pink-950/60 text-pink-300",       group: "runtime"  },
  { kind: "memory",          label: "Memory",          description: "Knowledge store entry",             color: "border-purple-500/40 text-purple-400",   bgColor: "bg-purple-950/60 text-purple-300",   group: "memory"   },
  { kind: "document",        label: "Document",        description: "Ingested source document",          color: "border-teal-500/40 text-teal-400",       bgColor: "bg-teal-950/60 text-teal-300",       group: "memory"   },
  { kind: "chunk",           label: "Chunk",           description: "Indexed document fragment",         color: "border-green-500/40 text-green-400",     bgColor: "bg-green-950/60 text-green-300",     group: "memory"   },
  { kind: "source",          label: "Source",          description: "Signal / document source",          color: "border-cyan-500/40 text-cyan-400",       bgColor: "bg-cyan-950/60 text-cyan-300",       group: "memory"   },
  { kind: "ingest_job",      label: "Ingest Job",      description: "Document ingest pipeline run",      color: "border-lime-500/40 text-lime-400",       bgColor: "bg-lime-950/60 text-lime-300",       group: "memory"   },
  { kind: "signal",          label: "Signal",          description: "External signal event",             color: "border-zinc-500/40 text-gray-500 dark:text-zinc-400",       bgColor: "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-700 dark:text-zinc-300",       group: "memory"   },
  { kind: "prompt_asset",    label: "Prompt Asset",    description: "Prompt template asset",             color: "border-fuchsia-500/40 text-fuchsia-400", bgColor: "bg-fuchsia-950/60 text-fuchsia-300", group: "prompts"  },
  { kind: "prompt_version",  label: "Prompt Version",  description: "Versioned prompt snapshot",         color: "border-pink-500/40 text-pink-400",       bgColor: "bg-pink-950/60 text-pink-300",       group: "prompts"  },
  { kind: "prompt_release",  label: "Prompt Release",  description: "Deployed prompt release",           color: "border-red-500/40 text-red-400",         bgColor: "bg-red-950/60 text-red-300",         group: "prompts"  },
  { kind: "eval_run",        label: "Eval Run",        description: "Evaluation run record",             color: "border-yellow-500/40 text-yellow-400",   bgColor: "bg-yellow-950/60 text-yellow-300",   group: "prompts"  },
  { kind: "skill",           label: "Skill",           description: "Agent capability definition",       color: "border-indigo-400/40 text-indigo-300",   bgColor: "bg-indigo-950/40 text-indigo-200",   group: "infra"    },
  { kind: "channel_target",  label: "Channel",         description: "Notification channel target",       color: "border-sky-400/40 text-sky-300",         bgColor: "bg-sky-950/40 text-sky-200",         group: "infra"    },
];

const EDGE_TYPES: EdgeTypeDef[] = [
  { kind: "triggered",      label: "Triggered",       description: "Session/run triggered a downstream run or task"     },
  { kind: "spawned",        label: "Spawned",         description: "Run spawned a sub-run or child task"                },
  { kind: "depended_on",    label: "Depended On",     description: "Task depends on another task completing first"       },
  { kind: "approved_by",    label: "Approved By",     description: "Run or task was gated by an approval decision"      },
  { kind: "resumed_from",   label: "Resumed From",    description: "Execution resumed from a saved checkpoint"          },
  { kind: "sent_to",        label: "Sent To",         description: "Message sent to an agent or channel target"         },
  { kind: "read_from",      label: "Read From",       description: "Memory or chunk was read from a source"             },
  { kind: "cited",          label: "Cited",           description: "Run or task cited a memory chunk in a response"     },
  { kind: "derived_from",   label: "Derived From",    description: "Document or chunk derived from a parent document"   },
  { kind: "embedded_as",    label: "Embedded As",     description: "Document text embedded as a vector chunk"           },
  { kind: "evaluated_by",   label: "Evaluated By",    description: "Prompt release evaluated by an eval run"            },
  { kind: "released_as",    label: "Released As",     description: "Prompt version released as a deployable release"    },
  { kind: "rolled_back_to", label: "Rolled Back",     description: "Deployment rolled back to a previous release"       },
  { kind: "routed_to",      label: "Routed To",       description: "Request routed to a specific provider"              },
  { kind: "used_prompt",    label: "Used Prompt",     description: "Run or task used a specific prompt release"         },
  { kind: "used_tool",      label: "Used Tool",       description: "Run invoked a registered tool or skill"             },
  { kind: "called_provider",label: "Called Provider", description: "Task made an LLM provider call"                     },
];

const GROUP_LABELS: Record<NodeTypeDef["group"], string> = {
  runtime: "Runtime Execution",
  memory:  "Memory & Knowledge",
  prompts: "Prompts & Evals",
  infra:   "Infrastructure",
};
const GROUPS: NodeTypeDef["group"][] = ["runtime", "memory", "prompts", "infra"];

// ── Force simulation types + physics ─────────────────────────────────────────

type SimKind = "session" | "run" | "task";

interface SimNode {
  id:    string;
  kind:  SimKind;
  label: string;
  x: number; y: number;
  vx: number; vy: number;
}

interface SimEdge {
  id:     string;
  source: string;
  target: string;
}

const NODE_R: Record<SimKind, number>      = { session: 13, run: 9, task: 6 };
const NODE_FILL: Record<SimKind, string>   = { session: "#3b82f6", run: "#6366f1", task: "#8b5cf6" };
const NODE_STROKE: Record<SimKind, string> = { session: "#1d4ed8", run: "#3730a3", task: "#4c1d95" };

// Physics constants
const REPULSION = 3800;
const SPRING_K  = 0.05;
const REST_LEN  = 80;
const DAMPING   = 0.80;
const GRAVITY   = 0.015;
const SETTLE_V  = 0.08;

function tick(nodes: SimNode[], edges: SimEdge[], cx: number, cy: number): number {
  const n = nodes.length;
  const fx = new Float32Array(n);
  const fy = new Float32Array(n);

  // Coulomb repulsion
  for (let i = 0; i < n; i++) {
    for (let j = i + 1; j < n; j++) {
      const dx = nodes[i].x - nodes[j].x || 0.01;
      const dy = nodes[i].y - nodes[j].y || 0.01;
      const d2  = Math.max(dx * dx + dy * dy, 25);
      const inv = REPULSION / (d2 * Math.sqrt(d2));
      fx[i] += inv * dx;  fy[i] += inv * dy;
      fx[j] -= inv * dx;  fy[j] -= inv * dy;
    }
  }

  // Hooke spring along edges
  const idx = new Map<string, number>(nodes.map((nd, i) => [nd.id, i]));
  for (const e of edges) {
    const si = idx.get(e.source), ti = idx.get(e.target);
    if (si == null || ti == null) continue;
    const dx = nodes[ti].x - nodes[si].x;
    const dy = nodes[ti].y - nodes[si].y;
    const d  = Math.sqrt(dx * dx + dy * dy) || 1;
    const f  = SPRING_K * (d - REST_LEN) / d;
    fx[si] += f * dx;  fy[si] += f * dy;
    fx[ti] -= f * dx;  fy[ti] -= f * dy;
  }

  // Center gravity
  for (let i = 0; i < n; i++) {
    fx[i] += (cx - nodes[i].x) * GRAVITY;
    fy[i] += (cy - nodes[i].y) * GRAVITY;
  }

  // Verlet integration
  let maxV = 0;
  for (let i = 0; i < n; i++) {
    nodes[i].vx = (nodes[i].vx + fx[i]) * DAMPING;
    nodes[i].vy = (nodes[i].vy + fy[i]) * DAMPING;
    nodes[i].x += nodes[i].vx;
    nodes[i].y += nodes[i].vy;
    const v = Math.abs(nodes[i].vx) + Math.abs(nodes[i].vy);
    if (v > maxV) maxV = v;
  }
  return maxV;
}

function buildGraph(maxNodes: number): { nodes: SimNode[]; edges: SimEdge[] } {
  const nodes: SimNode[] = [];
  const edges: SimEdge[] = [];
  const cx = 400, cy = 270;
  const rand = (lo: number, hi: number) => lo + Math.random() * (hi - lo);

  for (let s = 0; s < 5 && nodes.length < maxNodes; s++) {
    const sid = `session-${s}`;
    nodes.push({ id: sid, kind: "session", label: `Session ${s + 1}`,
      x: cx + rand(-280, 280), y: cy + rand(-180, 180), vx: 0, vy: 0 });

    const nRuns = 2 + Math.floor(Math.random() * 3);
    for (let r = 0; r < nRuns && nodes.length < maxNodes; r++) {
      const rid = `run-${s}-${r}`;
      nodes.push({ id: rid, kind: "run", label: `Run ${s + 1}.${r + 1}`,
        x: cx + rand(-280, 280), y: cy + rand(-180, 180), vx: 0, vy: 0 });
      edges.push({ id: `${sid}→${rid}`, source: sid, target: rid });

      const nTasks = 1 + Math.floor(Math.random() * 3);
      for (let t = 0; t < nTasks && nodes.length < maxNodes; t++) {
        const tid = `task-${s}-${r}-${t}`;
        nodes.push({ id: tid, kind: "task", label: `Task ${s + 1}.${r + 1}.${t + 1}`,
          x: cx + rand(-280, 280), y: cy + rand(-180, 180), vx: 0, vy: 0 });
        edges.push({ id: `${rid}→${tid}`, source: rid, target: tid });
      }
    }
  }
  return { nodes, edges };
}

// ── ForceGraph component ──────────────────────────────────────────────────────

const SVG_W = 800;
const SVG_H = 520;

interface XfState { tx: number; ty: number; k: number }

function ForceGraph({ maxNodes = 100 }: { maxNodes?: number }) {
  const nodesRef = useRef<SimNode[]>([]);
  const edgesRef = useRef<SimEdge[]>([]);
  const rafRef   = useRef<number>(0);
  const svgRef   = useRef<SVGSVGElement>(null);

  const [snap, setSnap]             = useState<SimNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settled, setSettled]       = useState(false);
  const [xf, setXf]                 = useState<XfState>({ tx: 0, ty: 0, k: 1 });

  // Pan tracking (refs to avoid stale closures in pointermove)
  const panRef = useRef({ active: false, sx: 0, sy: 0, stx: 0, sty: 0 });
  const didPanRef = useRef(false);

  // ── Sim lifecycle ───────────────────────────────────────────────────────────

  function startLoop() {
    cancelAnimationFrame(rafRef.current);
    let frame = 0;
    function loop() {
      const maxV = tick(nodesRef.current, edgesRef.current, SVG_W / 2, SVG_H / 2);
      frame++;
      if (frame % 2 === 0) setSnap(nodesRef.current.map(n => ({ ...n })));
      if (maxV > SETTLE_V) {
        rafRef.current = requestAnimationFrame(loop);
      } else {
        setSnap(nodesRef.current.map(n => ({ ...n })));
        setSettled(true);
      }
    }
    rafRef.current = requestAnimationFrame(loop);
  }

  function resetSim() {
    cancelAnimationFrame(rafRef.current);
    const { nodes, edges } = buildGraph(maxNodes);
    nodesRef.current = nodes;
    edgesRef.current = edges;
    setSelectedId(null);
    setSettled(false);
    setXf({ tx: 0, ty: 0, k: 1 });
    setSnap(nodes.map(n => ({ ...n })));
    startLoop();
  }

  useEffect(() => {
    resetSim();
    return () => cancelAnimationFrame(rafRef.current);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // ── Connected set (for highlight) ───────────────────────────────────────────

  const connectedSet = useMemo(() => {
    if (!selectedId) return new Set<string>();
    const s = new Set([selectedId]);
    for (const e of edgesRef.current) {
      if (e.source === selectedId) s.add(e.target);
      if (e.target === selectedId) s.add(e.source);
    }
    return s;
  }, [selectedId]); // edgesRef is stable

  // ── Zoom ────────────────────────────────────────────────────────────────────

  function handleWheel(e: React.WheelEvent<SVGSVGElement>) {
    e.preventDefault();
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect) return;
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;
    const factor = e.deltaY < 0 ? 1.12 : 0.9;
    setXf(t => {
      const k = Math.max(0.15, Math.min(5, t.k * factor));
      const f = k / t.k;
      return { k, tx: mx - (mx - t.tx) * f, ty: my - (my - t.ty) * f };
    });
  }

  // ── Pan ─────────────────────────────────────────────────────────────────────

  function handleBgPointerDown(e: React.PointerEvent) {
    (e.currentTarget as Element).setPointerCapture(e.pointerId);
    panRef.current = { active: true, sx: e.clientX, sy: e.clientY, stx: xf.tx, sty: xf.ty };
    didPanRef.current = false;
  }

  function handlePointerMove(e: React.PointerEvent) {
    if (!panRef.current.active) return;
    const dx = e.clientX - panRef.current.sx;
    const dy = e.clientY - panRef.current.sy;
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) didPanRef.current = true;
    setXf(t => ({ ...t, tx: panRef.current.stx + dx, ty: panRef.current.sty + dy }));
  }

  function handlePointerUp() {
    panRef.current.active = false;
  }

  // ── Node interaction ────────────────────────────────────────────────────────

  function handleNodeClick(e: React.MouseEvent, id: string) {
    e.stopPropagation();
    setSelectedId(prev => prev === id ? null : id);
    // Give the node a small velocity kick to restart sim
    if (settled) {
      const node = nodesRef.current.find(n => n.id === id);
      if (node) {
        const a = Math.random() * 2 * Math.PI;
        node.vx = Math.cos(a) * 2.5;
        node.vy = Math.sin(a) * 2.5;
        setSettled(false);
        startLoop();
      }
    }
  }

  function handleSvgClick() {
    if (!didPanRef.current) setSelectedId(null);
  }

  // ── Derived render data ─────────────────────────────────────────────────────

  const nodeMap     = useMemo(() => new Map(snap.map(n => [n.id, n])), [snap]);
  const hasSelected = selectedId !== null;
  const edges       = edgesRef.current;

  // Page counts for footer
  const counts = useMemo(() => {
    const s = snap.reduce((a, n) => { a[n.kind] = (a[n.kind] ?? 0) + 1; return a; },
      {} as Record<string, number>);
    return s;
  }, [snap]);

  // ── Render ──────────────────────────────────────────────────────────────────

  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden bg-white dark:bg-zinc-950 select-none">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-3 py-2 border-b border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
        <span className="text-[11px] text-gray-400 dark:text-zinc-500 font-mono tabular-nums">
          {snap.length} nodes · {edges.length} edges
        </span>
        {settled
          ? <span className="text-[10px] text-emerald-600 font-mono">settled</span>
          : <span className="text-[10px] text-amber-600 font-mono animate-pulse">simulating…</span>}
        {selectedId && (
          <span className="text-[10px] text-indigo-400 font-mono truncate max-w-[160px]">
            ● {selectedId}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          <button
            onClick={() => setXf(t => { const k = Math.min(5, t.k * 1.25); return { ...t, k }; })}
            className="p-1.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
            title="Zoom in"
          ><ZoomIn size={12} /></button>
          <button
            onClick={() => setXf(t => { const k = Math.max(0.15, t.k * 0.8); return { ...t, k }; })}
            className="p-1.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
            title="Zoom out"
          ><ZoomOut size={12} /></button>
          <button
            onClick={() => setXf({ tx: 0, ty: 0, k: 1 })}
            className="px-1.5 py-1 rounded text-[10px] font-mono text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors"
            title="Reset zoom"
          >1:1</button>
          <div className="w-px h-4 bg-gray-100 dark:bg-zinc-800 mx-0.5" />
          <button
            onClick={resetSim}
            className="flex items-center gap-1 px-2 py-1 rounded text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors border border-gray-200 dark:border-zinc-800"
            title="Regenerate demo data"
          ><RotateCcw size={10} /> Reset</button>
        </div>
      </div>

      {/* SVG canvas */}
      <svg
        ref={svgRef}
        viewBox={`0 0 ${SVG_W} ${SVG_H}`}
        width="100%"
        style={{ height: SVG_H, cursor: panRef.current.active ? "grabbing" : "grab", display: "block" }}
        onWheel={handleWheel}
        onPointerMove={handlePointerMove}
        onPointerUp={handlePointerUp}
        onPointerLeave={handlePointerUp}
        onClick={handleSvgClick}
      >
        <defs>
          <marker id="arr" markerWidth="5" markerHeight="4" refX="4.5" refY="2" orient="auto">
            <path d="M0,0 L5,2 L0,4 Z" fill="#3f3f46" fillOpacity={0.7} />
          </marker>
          <marker id="arr-hi" markerWidth="5" markerHeight="4" refX="4.5" refY="2" orient="auto">
            <path d="M0,0 L5,2 L0,4 Z" fill="#6366f1" fillOpacity={0.95} />
          </marker>
        </defs>

        {/* Background (pan target) */}
        <rect
          x={0} y={0} width={SVG_W} height={SVG_H}
          fill="transparent"
          onPointerDown={handleBgPointerDown}
        />

        <g transform={`translate(${xf.tx},${xf.ty}) scale(${xf.k})`}>
          {/* Edges */}
          {edges.map(e => {
            const s = nodeMap.get(e.source), t = nodeMap.get(e.target);
            if (!s || !t) return null;
            const active = hasSelected && connectedSet.has(e.source) && connectedSet.has(e.target);
            const sr = NODE_R[s.kind], tr = NODE_R[t.kind];
            // Offset endpoints to edge of circles
            const dx = t.x - s.x, dy = t.y - s.y;
            const d  = Math.sqrt(dx * dx + dy * dy) || 1;
            const x1 = s.x + (dx / d) * sr;
            const y1 = s.y + (dy / d) * sr;
            const x2 = t.x - (dx / d) * (tr + 5);
            const y2 = t.y - (dy / d) * (tr + 5);
            return (
              <line key={e.id}
                x1={x1} y1={y1} x2={x2} y2={y2}
                stroke={active ? "#6366f1" : "#3f3f46"}
                strokeWidth={active ? 1.5 : 0.8}
                strokeOpacity={hasSelected ? (active ? 0.9 : 0.12) : 0.4}
                markerEnd={active ? "url(#arr-hi)" : "url(#arr)"}
              />
            );
          })}

          {/* Nodes */}
          {snap.map(node => {
            const r  = NODE_R[node.kind];
            const sel = node.id === selectedId;
            const con = connectedSet.has(node.id);
            const dim = hasSelected && !con;
            const showLabel = node.kind === "session" || sel || con;
            return (
              <g key={node.id}
                transform={`translate(${node.x},${node.y})`}
                style={{ cursor: "pointer" }}
                onClick={e => handleNodeClick(e, node.id)}
              >
                {sel && (
                  <circle r={r + 5}
                    fill="none"
                    stroke="#818cf8"
                    strokeWidth={1.5}
                    strokeOpacity={0.5}
                    strokeDasharray="3 2"
                  />
                )}
                <circle
                  r={r}
                  fill={NODE_FILL[node.kind]}
                  stroke={sel ? "#a5b4fc" : NODE_STROKE[node.kind]}
                  strokeWidth={sel ? 2 : 1}
                  fillOpacity={dim ? 0.18 : 1}
                  strokeOpacity={dim ? 0.18 : 1}
                />
                {showLabel && (
                  <text
                    y={r + 10}
                    textAnchor="middle"
                    fontSize={node.kind === "session" ? 9 : 8}
                    fill={dim ? "#3f3f46" : sel ? "#e4e4e7" : "#a1a1aa"}
                    style={{ pointerEvents: "none", userSelect: "none" }}
                  >
                    {node.label}
                  </text>
                )}
              </g>
            );
          })}
        </g>
      </svg>

      {/* Footer legend */}
      <div className="flex items-center gap-5 px-4 py-2 border-t border-gray-200 dark:border-zinc-800 bg-gray-50/40 dark:bg-zinc-900/40 flex-wrap">
        {(["session", "run", "task"] as SimKind[]).map(k => (
          <div key={k} className="flex items-center gap-1.5">
            <svg width={NODE_R[k] * 2 + 2} height={NODE_R[k] * 2 + 2}>
              <circle
                cx={NODE_R[k] + 1} cy={NODE_R[k] + 1} r={NODE_R[k]}
                fill={NODE_FILL[k]} stroke={NODE_STROKE[k]} strokeWidth={1}
              />
            </svg>
            <span className="text-[11px] text-gray-400 dark:text-zinc-500 capitalize">{k}</span>
            {counts[k] != null && (
              <span className="text-[10px] text-gray-300 dark:text-zinc-600 font-mono">{counts[k]}</span>
            )}
          </div>
        ))}
        <span className="ml-auto text-[10px] text-gray-300 dark:text-zinc-600 hidden sm:block">
          scroll to zoom · drag to pan · click node to highlight connections
        </span>
      </div>
    </div>
  );
}

// ── Small shared atoms (schema view) ─────────────────────────────────────────

function StatCard({ label, value, sub, accent = "border-l-zinc-700" }: {
  label: string; value: string | number; sub?: string; accent?: string;
}) {
  return (
    <div className={clsx("border-l-2 pl-3 py-0.5", accent)}>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

function NodeCard({ node, selected, onClick }: {
  node: NodeTypeDef; selected: boolean; onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "w-full text-left rounded-lg border p-3 transition-all",
        selected
          ? clsx("ring-1 ring-indigo-500/60 bg-gray-100/60 dark:bg-zinc-800/60", node.color)
          : "border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 hover:bg-gray-100/60 dark:hover:bg-gray-100/60 dark:bg-zinc-800/60 hover:border-gray-200 dark:border-zinc-700",
      )}
    >
      <div className="flex items-start justify-between gap-2 mb-1.5">
        <span className={clsx("text-[10px] font-mono font-medium rounded px-1.5 py-0.5", node.bgColor)}>
          {node.kind}
        </span>
        {selected && <ArrowRight size={11} className="text-indigo-400 shrink-0 mt-0.5" />}
      </div>
      <p className="text-[12px] font-medium text-gray-800 dark:text-zinc-200 truncate">{node.label}</p>
      <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5 leading-snug">{node.description}</p>
    </button>
  );
}

function NodeDetail({ node, onClose }: { node: NodeTypeDef; onClose: () => void }) {
  const relevantEdges = EDGE_TYPES.filter(e => {
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
    <aside className="w-72 shrink-0 border-l border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
        <div>
          <span className={clsx("text-[10px] font-mono rounded px-1.5 py-0.5", node.bgColor)}>{node.kind}</span>
          <p className="text-[13px] font-medium text-gray-800 dark:text-zinc-200 mt-1">{node.label}</p>
        </div>
        <button onClick={onClose}
          className="p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-gray-100 dark:bg-zinc-800 transition-colors">×</button>
      </div>
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        <p className="text-[11px] text-gray-400 dark:text-zinc-500">{node.description}</p>
        <div>
          <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Connected via</p>
          {relevantEdges.length === 0 ? (
            <p className="text-[11px] text-gray-300 dark:text-zinc-600 italic">No edges defined</p>
          ) : (
            <div className="space-y-1.5">
              {relevantEdges.map(e => (
                <div key={e.kind} className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5">
                  <p className="text-[11px] font-mono text-gray-500 dark:text-zinc-400">{e.kind}</p>
                  <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5">{e.description}</p>
                </div>
              ))}
            </div>
          )}
        </div>
        <div className="rounded-lg bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-3 py-2.5">
          <div className="flex items-center gap-1.5 mb-1">
            <Info size={10} className="text-gray-400 dark:text-zinc-600" />
            <span className="text-[10px] text-gray-400 dark:text-zinc-600 font-medium">Live count</span>
          </div>
          <p className="text-[13px] font-mono text-gray-400 dark:text-zinc-600 italic">— backend offline</p>
        </div>
      </div>
    </aside>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

type View = "simulation" | "schema";

export function GraphPage() {
  const [view, setView]               = useState<View>("simulation");
  const [query, setQuery]             = useState("");
  const [selectedNode, setSelectedNode] = useState<NodeTypeDef | null>(null);

  const lowerQuery   = query.toLowerCase();
  const filteredNodes = NODE_TYPES.filter(n =>
    !lowerQuery ||
    n.kind.includes(lowerQuery) ||
    n.label.toLowerCase().includes(lowerQuery) ||
    n.description.toLowerCase().includes(lowerQuery),
  );

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      {/* Toolbar */}
      <div className="flex items-center gap-3 px-5 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <Network size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Knowledge Graph</span>

        {/* View switcher */}
        <div className="ml-4 flex items-center gap-0.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 p-0.5">
          {(["simulation", "schema"] as View[]).map(v => (
            <button key={v} onClick={() => setView(v)}
              className={clsx(
                "px-2.5 py-1 rounded text-[11px] font-medium transition-colors capitalize",
                view === v
                  ? "bg-gray-200 dark:bg-zinc-700 text-gray-900 dark:text-zinc-100"
                  : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
              )}>
              {v}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-3">
          <span className="text-[11px] text-gray-300 dark:text-zinc-600">RFC 004</span>
        </div>
      </div>

      {/* Body */}
      <div className="flex flex-1 min-h-0 overflow-hidden">
        <div className="flex-1 overflow-y-auto min-w-0">
          <div className="max-w-4xl mx-auto px-5 py-5 space-y-5">

            {view === "simulation" ? (
              <>
                <div className="text-[12px] text-gray-400 dark:text-zinc-600 leading-relaxed">
                  Force-directed graph of demo sessions → runs → tasks.
                  Scroll to zoom · drag background to pan · click a node to highlight its connections.
                </div>
                <ForceGraph maxNodes={100} />
              </>
            ) : (
              <>
                {/* Stat strip */}
                <div className="flex items-start gap-8 py-3 px-4 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
                  <StatCard label="Node Types"  value={NODE_TYPES.length}  sub="defined in schema"  accent="border-l-indigo-500" />
                  <StatCard label="Edge Types"  value={EDGE_TYPES.length}  sub="relationship kinds" accent="border-l-indigo-500" />
                  <StatCard label="Live Nodes"  value="—"                  sub="backend offline"    accent="border-l-zinc-700"   />
                  <StatCard label="Live Edges"  value="—"                  sub="backend offline"    accent="border-l-zinc-700"   />
                </div>

                {/* Offline notice */}
                <div className="rounded-lg border border-amber-800/40 bg-amber-500/5 px-4 py-3 flex items-start gap-3">
                  <Info size={14} className="text-amber-500 shrink-0 mt-0.5" />
                  <div className="flex-1 min-w-0">
                    <p className="text-[12px] font-medium text-amber-400">Graph backend not connected</p>
                    <p className="text-[11px] text-amber-700 mt-0.5">
                      Run a workflow to populate live graph data. The visualization above
                      uses demo data. Ingest documents or run an orchestration to see real nodes.
                    </p>
                  </div>
                  <a href="#memory" onClick={() => { window.location.hash = "memory"; }}
                    className="shrink-0 flex items-center gap-1 text-[11px] text-amber-600 hover:text-amber-400 transition-colors">
                    Memory search <ExternalLink size={10} />
                  </a>
                </div>

                {/* Search */}
                <div className="relative">
                  <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
                  <input value={query} onChange={e => setQuery(e.target.value)}
                    placeholder="Filter node types… (e.g. 'run', 'prompt', 'memory')"
                    className="w-full rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-800 dark:text-zinc-200
                               placeholder-zinc-600 pl-9 pr-4 py-2 focus:outline-none focus:border-indigo-500 transition-colors" />
                  {query && (
                    <button onClick={() => setQuery("")}
                      className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors">×</button>
                  )}
                </div>

                {/* Node grid */}
                {query ? (
                  filteredNodes.length === 0 ? (
                    <p className="text-[13px] text-gray-400 dark:text-zinc-600 italic py-4 text-center">
                      No node types match &ldquo;{query}&rdquo;
                    </p>
                  ) : (
                    <div>
                      <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-3">
                        {filteredNodes.length} result{filteredNodes.length !== 1 ? "s" : ""}
                      </p>
                      <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
                        {filteredNodes.map(n => (
                          <NodeCard key={n.kind} node={n}
                            selected={selectedNode?.kind === n.kind}
                            onClick={() => setSelectedNode(p => p?.kind === n.kind ? null : n)} />
                        ))}
                      </div>
                    </div>
                  )
                ) : (
                  <div className="space-y-5">
                    {GROUPS.map(group => {
                      const nodes = NODE_TYPES.filter(n => n.group === group);
                      return (
                        <div key={group}>
                          <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">
                            {GROUP_LABELS[group]}
                            <span className="ml-2 text-gray-300 dark:text-zinc-600 normal-case font-normal">{nodes.length} types</span>
                          </p>
                          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
                            {nodes.map(n => (
                              <NodeCard key={n.kind} node={n}
                                selected={selectedNode?.kind === n.kind}
                                onClick={() => setSelectedNode(p => p?.kind === n.kind ? null : n)} />
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}

                {/* Edge types table */}
                <div>
                  <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-3">
                    Edge Types
                    <span className="ml-2 text-gray-300 dark:text-zinc-600 normal-case font-normal">{EDGE_TYPES.length} relationship kinds</span>
                  </p>
                  <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden">
                    <table className="min-w-full text-[12px]">
                      <thead className="bg-gray-50 dark:bg-zinc-900">
                        <tr>
                          <th className="px-3 py-2 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-b border-gray-200 dark:border-zinc-800 w-40">Kind</th>
                          <th className="px-3 py-2 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-b border-gray-200 dark:border-zinc-800 w-32">Label</th>
                          <th className="px-3 py-2 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider border-b border-gray-200 dark:border-zinc-800">Description</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-gray-200 dark:divide-zinc-800/50">
                        {EDGE_TYPES.map((e, i) => (
                          <tr key={e.kind} className={clsx("transition-colors hover:bg-gray-100/40 dark:hover:bg-gray-100/40 dark:bg-zinc-800/40",
                            i % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-[#111113]")}>
                            <td className="px-3 py-1.5 font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap">{e.kind}</td>
                            <td className="px-3 py-1.5 text-gray-500 dark:text-zinc-400 whitespace-nowrap">{e.label}</td>
                            <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-600">{e.description}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                </div>
              </>
            )}

          </div>
        </div>

        {/* Node detail panel (schema view only) */}
        {view === "schema" && selectedNode && (
          <NodeDetail node={selectedNode} onClose={() => setSelectedNode(null)} />
        )}
      </div>
    </div>
  );
}

export default GraphPage;
