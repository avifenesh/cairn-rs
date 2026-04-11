import { useEffect, useMemo, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Search,
  Network,
  Info,
  ArrowRight,
  RotateCcw,
  ZoomIn,
  ZoomOut,
  RefreshCw,
  Loader2,
} from "lucide-react";
import { clsx } from "clsx";
import { StatCard } from "../components/StatCard";
import { defaultApi } from "../lib/api";
import { useScope } from "../hooks/useScope";
import { FeatureEmptyState } from "../components/FeatureEmptyState";
import type { GraphNodeKind, GraphNodeRecord, GraphTraceResponse } from "../lib/types";

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
  { kind: "run",             label: "Run",            description: "Agent execution instance",          color: "border-indigo-500/40 text-indigo-400",   bgColor: "bg-indigo-950/60 text-indigo-300",   group: "runtime"  },
  { kind: "task",            label: "Task",           description: "Queued unit of work",               color: "border-violet-500/40 text-violet-400",   bgColor: "bg-violet-950/60 text-violet-300",   group: "runtime"  },
  { kind: "approval",        label: "Approval",       description: "Human-in-the-loop gate",            color: "border-amber-500/40 text-amber-400",     bgColor: "bg-amber-950/60 text-amber-300",     group: "runtime"  },
  { kind: "checkpoint",      label: "Checkpoint",     description: "Resumable execution snapshot",      color: "border-emerald-500/40 text-emerald-400", bgColor: "bg-emerald-950/60 text-emerald-300", group: "runtime"  },
  { kind: "mailbox_message", label: "Mailbox Msg",    description: "Agent-to-agent message",            color: "border-sky-500/40 text-sky-400",         bgColor: "bg-sky-950/60 text-sky-300",         group: "runtime"  },
  { kind: "tool_invocation", label: "Tool Call",      description: "External tool invocation",          color: "border-orange-500/40 text-orange-400",   bgColor: "bg-orange-950/60 text-orange-300",   group: "runtime"  },
  { kind: "route_decision",  label: "Route Decision", description: "Provider routing decision",         color: "border-rose-500/40 text-rose-400",       bgColor: "bg-rose-950/60 text-rose-300",       group: "runtime"  },
  { kind: "provider_call",   label: "Provider Call",  description: "LLM provider API call",             color: "border-pink-500/40 text-pink-400",       bgColor: "bg-pink-950/60 text-pink-300",       group: "runtime"  },
  { kind: "memory",          label: "Memory",         description: "Knowledge store entry",             color: "border-purple-500/40 text-purple-400",   bgColor: "bg-purple-950/60 text-purple-300",   group: "memory"   },
  { kind: "document",        label: "Document",       description: "Ingested source document",          color: "border-teal-500/40 text-teal-400",       bgColor: "bg-teal-950/60 text-teal-300",       group: "memory"   },
  { kind: "chunk",           label: "Chunk",          description: "Indexed document fragment",         color: "border-green-500/40 text-green-400",     bgColor: "bg-green-950/60 text-green-300",     group: "memory"   },
  { kind: "source",          label: "Source",         description: "Signal / document source",          color: "border-cyan-500/40 text-cyan-400",       bgColor: "bg-cyan-950/60 text-cyan-300",       group: "memory"   },
  { kind: "ingest_job",      label: "Ingest Job",     description: "Document ingest pipeline run",      color: "border-lime-500/40 text-lime-400",       bgColor: "bg-lime-950/60 text-lime-300",       group: "memory"   },
  { kind: "signal",          label: "Signal",         description: "External signal event",             color: "border-zinc-500/40 text-gray-500 dark:text-zinc-400", bgColor: "bg-gray-100/60 dark:bg-zinc-800/60 text-gray-700 dark:text-zinc-300", group: "memory"   },
  { kind: "prompt_asset",    label: "Prompt Asset",   description: "Prompt template asset",             color: "border-fuchsia-500/40 text-fuchsia-400", bgColor: "bg-fuchsia-950/60 text-fuchsia-300", group: "prompts"  },
  { kind: "prompt_version",  label: "Prompt Version", description: "Versioned prompt snapshot",         color: "border-pink-500/40 text-pink-400",       bgColor: "bg-pink-950/60 text-pink-300",       group: "prompts"  },
  { kind: "prompt_release",  label: "Prompt Release", description: "Deployed prompt release",           color: "border-red-500/40 text-red-400",         bgColor: "bg-red-950/60 text-red-300",         group: "prompts"  },
  { kind: "eval_run",        label: "Eval Run",       description: "Evaluation run record",             color: "border-yellow-500/40 text-yellow-400",   bgColor: "bg-yellow-950/60 text-yellow-300",   group: "prompts"  },
  { kind: "skill",           label: "Skill",          description: "Agent capability definition",       color: "border-indigo-400/40 text-indigo-300",   bgColor: "bg-indigo-950/40 text-indigo-200",   group: "infra"    },
  { kind: "channel_target",  label: "Channel",        description: "Notification channel target",       color: "border-sky-400/40 text-sky-300",         bgColor: "bg-sky-950/40 text-sky-200",         group: "infra"    },
];

const EDGE_TYPES: EdgeTypeDef[] = [
  { kind: "triggered",       label: "Triggered",       description: "Session/run triggered a downstream run or task" },
  { kind: "spawned",         label: "Spawned",         description: "Run spawned a sub-run or child task" },
  { kind: "depended_on",     label: "Depended On",     description: "Task depends on another task completing first" },
  { kind: "approved_by",     label: "Approved By",     description: "Run or task was gated by an approval decision" },
  { kind: "resumed_from",    label: "Resumed From",    description: "Execution resumed from a saved checkpoint" },
  { kind: "sent_to",         label: "Sent To",         description: "Message sent to an agent or channel target" },
  { kind: "read_from",       label: "Read From",       description: "Memory or chunk was read from a source" },
  { kind: "cited",           label: "Cited",           description: "Run or task cited a memory chunk in a response" },
  { kind: "derived_from",    label: "Derived From",    description: "Document or chunk derived from a parent document" },
  { kind: "embedded_as",     label: "Embedded As",     description: "Document text embedded as a vector chunk" },
  { kind: "evaluated_by",    label: "Evaluated By",    description: "Prompt release evaluated by an eval run" },
  { kind: "released_as",     label: "Released As",     description: "Prompt version released as a deployable release" },
  { kind: "rolled_back_to",  label: "Rolled Back",     description: "Deployment rolled back to a previous release" },
  { kind: "routed_to",       label: "Routed To",       description: "Request routed to a specific provider" },
  { kind: "used_prompt",     label: "Used Prompt",     description: "Run or task used a specific prompt release" },
  { kind: "used_tool",       label: "Used Tool",       description: "Run invoked a registered tool or skill" },
  { kind: "called_provider", label: "Called Provider", description: "Task made an LLM provider call" },
];

const GROUP_LABELS: Record<NodeTypeDef["group"], string> = {
  runtime: "Runtime Execution",
  memory: "Memory & Knowledge",
  prompts: "Prompts & Evals",
  infra: "Infrastructure",
};
const GROUPS: NodeTypeDef["group"][] = ["runtime", "memory", "prompts", "infra"];

// ── Force simulation types + physics ─────────────────────────────────────────

type SimKind = "session" | "run" | "task";

interface SimNode {
  id: string;
  kind: SimKind;
  label: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface SimEdge {
  id: string;
  source: string;
  target: string;
}

const NODE_R: Record<SimKind, number> = { session: 13, run: 9, task: 6 };
const NODE_FILL: Record<SimKind, string> = { session: "#3b82f6", run: "#6366f1", task: "#8b5cf6" };
const NODE_STROKE: Record<SimKind, string> = { session: "#1d4ed8", run: "#3730a3", task: "#4c1d95" };

const SVG_W = 800;
const SVG_H = 520;
const REPULSION = 3800;
const SPRING_K = 0.05;
const REST_LEN = 80;
const DAMPING = 0.8;
const GRAVITY = 0.015;
const SETTLE_V = 0.08;

interface XfState {
  tx: number;
  ty: number;
  k: number;
}

function tick(nodes: SimNode[], edges: SimEdge[], cx: number, cy: number): number {
  const n = nodes.length;
  const fx = new Float32Array(n);
  const fy = new Float32Array(n);

  for (let i = 0; i < n; i += 1) {
    for (let j = i + 1; j < n; j += 1) {
      const dx = nodes[i].x - nodes[j].x || 0.01;
      const dy = nodes[i].y - nodes[j].y || 0.01;
      const d2 = Math.max(dx * dx + dy * dy, 25);
      const inv = REPULSION / (d2 * Math.sqrt(d2));
      fx[i] += inv * dx;
      fy[i] += inv * dy;
      fx[j] -= inv * dx;
      fy[j] -= inv * dy;
    }
  }

  const idx = new Map<string, number>(nodes.map((node, index) => [node.id, index]));
  for (const edge of edges) {
    const si = idx.get(edge.source);
    const ti = idx.get(edge.target);
    if (si == null || ti == null) continue;
    const dx = nodes[ti].x - nodes[si].x;
    const dy = nodes[ti].y - nodes[si].y;
    const d = Math.sqrt(dx * dx + dy * dy) || 1;
    const force = (SPRING_K * (d - REST_LEN)) / d;
    fx[si] += force * dx;
    fy[si] += force * dy;
    fx[ti] -= force * dx;
    fy[ti] -= force * dy;
  }

  for (let i = 0; i < n; i += 1) {
    fx[i] += (cx - nodes[i].x) * GRAVITY;
    fy[i] += (cy - nodes[i].y) * GRAVITY;
  }

  let maxVelocity = 0;
  for (let i = 0; i < n; i += 1) {
    nodes[i].vx = (nodes[i].vx + fx[i]) * DAMPING;
    nodes[i].vy = (nodes[i].vy + fy[i]) * DAMPING;
    nodes[i].x += nodes[i].vx;
    nodes[i].y += nodes[i].vy;
    const velocity = Math.abs(nodes[i].vx) + Math.abs(nodes[i].vy);
    if (velocity > maxVelocity) maxVelocity = velocity;
  }

  return maxVelocity;
}

function hashString(value: string): number {
  let hash = 2166136261;
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index);
    hash = Math.imul(hash, 16777619);
  }
  return hash >>> 0;
}

function seededBetween(seed: string, min: number, max: number): number {
  const ratio = (hashString(seed) % 10_000) / 9_999;
  return min + ratio * (max - min);
}

function toSimKind(kind: GraphNodeKind): SimKind | null {
  switch (kind) {
    case "session":
      return "session";
    case "run":
      return "run";
    case "task":
      return "task";
    default:
      return null;
  }
}

function shortNodeId(nodeId: string): string {
  return nodeId.length > 20 ? `${nodeId.slice(0, 8)}…${nodeId.slice(-6)}` : nodeId;
}

function liveNodeLabel(node: GraphNodeRecord, simKind: SimKind): string {
  const prefix = simKind === "session" ? "Session" : simKind === "run" ? "Run" : "Task";
  return `${prefix} ${shortNodeId(node.node_id)}`;
}

function buildRuntimeSimulationGraph(
  trace: GraphTraceResponse | undefined,
  maxNodes: number,
): { nodes: SimNode[]; edges: SimEdge[] } {
  if (!trace) return { nodes: [], edges: [] };

  const runtimeNodes = trace.nodes
    .map((node) => {
      const simKind = toSimKind(node.kind);
      return simKind ? { node, simKind } : null;
    })
    .filter((entry): entry is { node: GraphNodeRecord; simKind: SimKind } => entry !== null)
    .sort((left, right) => {
      if (right.node.created_at !== left.node.created_at) {
        return right.node.created_at - left.node.created_at;
      }
      return left.node.node_id.localeCompare(right.node.node_id);
    })
    .slice(0, maxNodes);

  const nodeIds = new Set(runtimeNodes.map(({ node }) => node.node_id));
  const nodes = runtimeNodes.map(({ node, simKind }) => ({
    id: node.node_id,
    kind: simKind,
    label: liveNodeLabel(node, simKind),
    x: seededBetween(`${node.node_id}:x`, 120, 680),
    y: seededBetween(`${node.node_id}:y`, 90, 430),
    vx: 0,
    vy: 0,
  }));

  const edges = trace.edges
    .filter((edge) => nodeIds.has(edge.source_node_id) && nodeIds.has(edge.target_node_id))
    .map((edge) => ({
      id: `${edge.source_node_id}:${edge.target_node_id}:${edge.kind}`,
      source: edge.source_node_id,
      target: edge.target_node_id,
    }));

  return { nodes, edges };
}

// ── ForceGraph component ──────────────────────────────────────────────────────

function ForceGraph({ graph }: { graph: { nodes: SimNode[]; edges: SimEdge[] } }) {
  const nodesRef = useRef<SimNode[]>([]);
  const edgesRef = useRef<SimEdge[]>([]);
  const rafRef = useRef<number>(0);
  const svgRef = useRef<SVGSVGElement>(null);

  const [snap, setSnap] = useState<SimNode[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settled, setSettled] = useState(false);
  const [xf, setXf] = useState<XfState>({ tx: 0, ty: 0, k: 1 });

  const panRef = useRef({ active: false, sx: 0, sy: 0, stx: 0, sty: 0 });
  const didPanRef = useRef(false);

  function startLoop() {
    cancelAnimationFrame(rafRef.current);
    let frame = 0;

    function loop() {
      const maxVelocity = tick(nodesRef.current, edgesRef.current, SVG_W / 2, SVG_H / 2);
      frame += 1;
      if (frame % 2 === 0) {
        setSnap(nodesRef.current.map((node) => ({ ...node })));
      }
      if (maxVelocity > SETTLE_V) {
        rafRef.current = requestAnimationFrame(loop);
      } else {
        setSnap(nodesRef.current.map((node) => ({ ...node })));
        setSettled(true);
      }
    }

    rafRef.current = requestAnimationFrame(loop);
  }

  function resetSim() {
    cancelAnimationFrame(rafRef.current);
    nodesRef.current = graph.nodes.map((node) => ({ ...node }));
    edgesRef.current = graph.edges.map((edge) => ({ ...edge }));
    setSelectedId(null);
    setXf({ tx: 0, ty: 0, k: 1 });
    setSnap(nodesRef.current.map((node) => ({ ...node })));
    if (nodesRef.current.length === 0) {
      setSettled(true);
      return;
    }
    setSettled(false);
    startLoop();
  }

  useEffect(() => {
    resetSim();
    return () => cancelAnimationFrame(rafRef.current);
  }, [graph]);

  const connectedSet = useMemo(() => {
    if (!selectedId) return new Set<string>();
    const connected = new Set([selectedId]);
    for (const edge of edgesRef.current) {
      if (edge.source === selectedId) connected.add(edge.target);
      if (edge.target === selectedId) connected.add(edge.source);
    }
    return connected;
  }, [selectedId, snap]);

  const nodeMap = useMemo(() => new Map(snap.map((node) => [node.id, node])), [snap]);
  const hasSelected = selectedId !== null;
  const edges = edgesRef.current;
  const counts = useMemo(() => {
    return snap.reduce((acc, node) => {
      acc[node.kind] = (acc[node.kind] ?? 0) + 1;
      return acc;
    }, {} as Record<string, number>);
  }, [snap]);

  function handleWheel(event: React.WheelEvent<SVGSVGElement>) {
    event.preventDefault();
    const rect = svgRef.current?.getBoundingClientRect();
    if (!rect) return;
    const mouseX = event.clientX - rect.left;
    const mouseY = event.clientY - rect.top;
    const factor = event.deltaY < 0 ? 1.12 : 0.9;
    setXf((current) => {
      const k = Math.max(0.15, Math.min(5, current.k * factor));
      const scale = k / current.k;
      return {
        k,
        tx: mouseX - (mouseX - current.tx) * scale,
        ty: mouseY - (mouseY - current.ty) * scale,
      };
    });
  }

  function handleBgPointerDown(event: React.PointerEvent) {
    (event.currentTarget as Element).setPointerCapture(event.pointerId);
    panRef.current = {
      active: true,
      sx: event.clientX,
      sy: event.clientY,
      stx: xf.tx,
      sty: xf.ty,
    };
    didPanRef.current = false;
  }

  function handlePointerMove(event: React.PointerEvent) {
    if (!panRef.current.active) return;
    const dx = event.clientX - panRef.current.sx;
    const dy = event.clientY - panRef.current.sy;
    if (Math.abs(dx) > 3 || Math.abs(dy) > 3) didPanRef.current = true;
    setXf((current) => ({ ...current, tx: panRef.current.stx + dx, ty: panRef.current.sty + dy }));
  }

  function handlePointerUp() {
    panRef.current.active = false;
  }

  function handleNodeClick(event: React.MouseEvent, id: string) {
    event.stopPropagation();
    setSelectedId((current) => (current === id ? null : id));
    if (settled) {
      const node = nodesRef.current.find((entry) => entry.id === id);
      if (node) {
        const angle = Math.random() * 2 * Math.PI;
        node.vx = Math.cos(angle) * 2.5;
        node.vy = Math.sin(angle) * 2.5;
        setSettled(false);
        startLoop();
      }
    }
  }

  function handleSvgClick() {
    if (!didPanRef.current) setSelectedId(null);
  }

  return (
    <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden bg-white dark:bg-zinc-950 select-none">
      <div className="flex items-center gap-3 px-3 py-2 border-b border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
        <span className="text-[11px] text-gray-400 dark:text-zinc-500 font-mono tabular-nums">
          {snap.length} nodes · {edges.length} edges
        </span>
        {settled ? (
          <span className="text-[10px] text-emerald-600 font-mono">settled</span>
        ) : (
          <span className="text-[10px] text-amber-600 font-mono animate-pulse">simulating…</span>
        )}
        {selectedId && (
          <span className="text-[10px] text-indigo-400 font-mono truncate max-w-[220px]">
            ● {selectedId}
          </span>
        )}
        <div className="ml-auto flex items-center gap-1">
          <button
            onClick={() => setXf((current) => ({ ...current, k: Math.min(5, current.k * 1.25) }))}
            className="p-1.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
            title="Zoom in"
          >
            <ZoomIn size={12} />
          </button>
          <button
            onClick={() => setXf((current) => ({ ...current, k: Math.max(0.15, current.k * 0.8) }))}
            className="p-1.5 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
            title="Zoom out"
          >
            <ZoomOut size={12} />
          </button>
          <button
            onClick={() => setXf({ tx: 0, ty: 0, k: 1 })}
            className="px-1.5 py-1 rounded text-[10px] font-mono text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
            title="Reset zoom"
          >
            1:1
          </button>
          <div className="w-px h-4 bg-gray-200 dark:bg-zinc-800 mx-0.5" />
          <button
            onClick={resetSim}
            className="flex items-center gap-1 px-2 py-1 rounded text-[11px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors border border-gray-200 dark:border-zinc-800"
            title="Recenter live graph"
          >
            <RotateCcw size={10} />
            Reset
          </button>
        </div>
      </div>

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

        <rect
          x={0}
          y={0}
          width={SVG_W}
          height={SVG_H}
          fill="transparent"
          onPointerDown={handleBgPointerDown}
        />

        <g transform={`translate(${xf.tx},${xf.ty}) scale(${xf.k})`}>
          {edges.map((edge) => {
            const source = nodeMap.get(edge.source);
            const target = nodeMap.get(edge.target);
            if (!source || !target) return null;
            const active =
              hasSelected && connectedSet.has(edge.source) && connectedSet.has(edge.target);
            const sourceRadius = NODE_R[source.kind];
            const targetRadius = NODE_R[target.kind];
            const dx = target.x - source.x;
            const dy = target.y - source.y;
            const distance = Math.sqrt(dx * dx + dy * dy) || 1;
            const x1 = source.x + (dx / distance) * sourceRadius;
            const y1 = source.y + (dy / distance) * sourceRadius;
            const x2 = target.x - (dx / distance) * (targetRadius + 5);
            const y2 = target.y - (dy / distance) * (targetRadius + 5);

            return (
              <line
                key={edge.id}
                x1={x1}
                y1={y1}
                x2={x2}
                y2={y2}
                stroke={active ? "#6366f1" : "#3f3f46"}
                strokeWidth={active ? 1.5 : 0.8}
                strokeOpacity={hasSelected ? (active ? 0.9 : 0.12) : 0.4}
                markerEnd={active ? "url(#arr-hi)" : "url(#arr)"}
              />
            );
          })}

          {snap.map((node) => {
            const radius = NODE_R[node.kind];
            const selected = node.id === selectedId;
            const connected = connectedSet.has(node.id);
            const dim = hasSelected && !connected;
            const showLabel = node.kind === "session" || selected || connected;
            return (
              <g
                key={node.id}
                transform={`translate(${node.x},${node.y})`}
                style={{ cursor: "pointer" }}
                onClick={(event) => handleNodeClick(event, node.id)}
              >
                {selected && (
                  <circle
                    r={radius + 5}
                    fill="none"
                    stroke="#818cf8"
                    strokeWidth={1.5}
                    strokeOpacity={0.5}
                    strokeDasharray="3 2"
                  />
                )}
                <circle
                  r={radius}
                  fill={NODE_FILL[node.kind]}
                  stroke={selected ? "#a5b4fc" : NODE_STROKE[node.kind]}
                  strokeWidth={selected ? 2 : 1}
                  fillOpacity={dim ? 0.18 : 1}
                  strokeOpacity={dim ? 0.18 : 1}
                />
                {showLabel && (
                  <text
                    y={radius + 10}
                    textAnchor="middle"
                    fontSize={node.kind === "session" ? 9 : 8}
                    fill={dim ? "#3f3f46" : selected ? "#e4e4e7" : "#a1a1aa"}
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

      <div className="flex items-center gap-5 px-4 py-2 border-t border-gray-200 dark:border-zinc-800 bg-gray-50/40 dark:bg-zinc-900/40 flex-wrap">
        {(["session", "run", "task"] as SimKind[]).map((kind) => (
          <div key={kind} className="flex items-center gap-1.5">
            <svg width={NODE_R[kind] * 2 + 2} height={NODE_R[kind] * 2 + 2}>
              <circle
                cx={NODE_R[kind] + 1}
                cy={NODE_R[kind] + 1}
                r={NODE_R[kind]}
                fill={NODE_FILL[kind]}
                stroke={NODE_STROKE[kind]}
                strokeWidth={1}
              />
            </svg>
            <span className="text-[11px] text-gray-400 dark:text-zinc-500 capitalize">{kind}</span>
            {counts[kind] != null && (
              <span className="text-[10px] text-gray-300 dark:text-zinc-600 font-mono">{counts[kind]}</span>
            )}
          </div>
        ))}
        <span className="ml-auto text-[10px] text-gray-300 dark:text-zinc-600 hidden sm:block">
          scroll to zoom · drag to pan · click a node to highlight its connections
        </span>
      </div>
    </div>
  );
}

// ── Small shared atoms (schema view) ─────────────────────────────────────────

function NodeCard({
  node,
  selected,
  onClick,
}: {
  node: NodeTypeDef;
  selected: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={clsx(
        "w-full text-left rounded-lg border p-3 transition-all",
        selected
          ? clsx("ring-1 ring-indigo-500/60 bg-gray-100/60 dark:bg-zinc-800/60", node.color)
          : "border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 hover:bg-gray-100/60 dark:hover:bg-zinc-800/60 hover:border-gray-300 dark:hover:border-zinc-700",
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

function NodeDetail({
  node,
  liveCount,
  onClose,
}: {
  node: NodeTypeDef;
  liveCount: number;
  onClose: () => void;
}) {
  const relevantEdges = EDGE_TYPES.filter((edge) => {
    const kind = node.kind;
    if (kind === "run") return ["triggered", "spawned", "approved_by", "used_prompt", "used_tool", "called_provider", "resumed_from"].includes(edge.kind);
    if (kind === "task") return ["depended_on", "approved_by", "used_tool", "called_provider"].includes(edge.kind);
    if (kind === "session") return ["triggered"].includes(edge.kind);
    if (kind === "approval") return ["approved_by"].includes(edge.kind);
    if (kind === "checkpoint") return ["resumed_from"].includes(edge.kind);
    if (kind === "mailbox_message") return ["sent_to"].includes(edge.kind);
    if (kind === "tool_invocation") return ["used_tool"].includes(edge.kind);
    if (kind === "memory") return ["cited", "read_from"].includes(edge.kind);
    if (kind === "document") return ["derived_from", "embedded_as", "read_from"].includes(edge.kind);
    if (kind === "chunk") return ["embedded_as", "cited", "derived_from"].includes(edge.kind);
    if (kind === "source") return ["read_from"].includes(edge.kind);
    if (kind === "prompt_asset") return ["released_as"].includes(edge.kind);
    if (kind === "prompt_version") return ["released_as", "rolled_back_to"].includes(edge.kind);
    if (kind === "prompt_release") return ["used_prompt", "evaluated_by", "rolled_back_to"].includes(edge.kind);
    if (kind === "eval_run") return ["evaluated_by"].includes(edge.kind);
    if (kind === "provider_call") return ["called_provider", "routed_to"].includes(edge.kind);
    if (kind === "route_decision") return ["routed_to"].includes(edge.kind);
    if (kind === "skill") return ["used_tool"].includes(edge.kind);
    if (kind === "channel_target") return ["sent_to"].includes(edge.kind);
    if (kind === "signal") return ["read_from"].includes(edge.kind);
    if (kind === "ingest_job") return ["derived_from", "embedded_as"].includes(edge.kind);
    return false;
  });

  return (
    <aside className="w-72 shrink-0 border-l border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950 flex flex-col overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
        <div>
          <span className={clsx("text-[10px] font-mono rounded px-1.5 py-0.5", node.bgColor)}>{node.kind}</span>
          <p className="text-[13px] font-medium text-gray-800 dark:text-zinc-200 mt-1">{node.label}</p>
        </div>
        <button
          onClick={onClose}
          className="p-1 rounded text-gray-400 dark:text-zinc-600 hover:text-gray-700 dark:hover:text-zinc-300 hover:bg-gray-100 dark:hover:bg-zinc-800 transition-colors"
        >
          ×
        </button>
      </div>
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        <p className="text-[11px] text-gray-400 dark:text-zinc-500">{node.description}</p>
        <div>
          <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">Connected via</p>
          {relevantEdges.length === 0 ? (
            <p className="text-[11px] text-gray-300 dark:text-zinc-600 italic">No edges defined</p>
          ) : (
            <div className="space-y-1.5">
              {relevantEdges.map((edge) => (
                <div key={edge.kind} className="rounded bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 px-2.5 py-1.5">
                  <p className="text-[11px] font-mono text-gray-500 dark:text-zinc-400">{edge.kind}</p>
                  <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-0.5">{edge.description}</p>
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
          <p className="text-[13px] font-mono text-gray-700 dark:text-zinc-200">
            {liveCount}
          </p>
          <p className="text-[10px] text-gray-400 dark:text-zinc-600 mt-1">
            {liveCount === 1 ? "1 node in the current project trace" : `${liveCount} nodes in the current project trace`}
          </p>
        </div>
      </div>
    </aside>
  );
}

// ── Page ──────────────────────────────────────────────────────────────────────

type View = "simulation" | "schema";

export function GraphPage() {
  const [scope] = useScope();
  const [view, setView] = useState<View>("simulation");
  const [query, setQuery] = useState("");
  const [selectedNode, setSelectedNode] = useState<NodeTypeDef | null>(null);

  const {
    data: trace,
    isLoading,
    isError,
    error,
    refetch,
    isFetching,
  } = useQuery({
    queryKey: ["graph-trace", scope.tenant_id, scope.workspace_id, scope.project_id],
    queryFn: () => defaultApi.getGraphTrace({ ...scope, limit: 300 }),
    staleTime: 10_000,
    refetchInterval: 15_000,
  });

  const liveNodes = trace?.nodes ?? [];
  const liveEdges = trace?.edges ?? [];
  const runtimeGraph = useMemo(() => buildRuntimeSimulationGraph(trace, 100), [trace]);
  const liveCounts = useMemo(() => {
    return liveNodes.reduce((acc, node) => {
      acc[node.kind] = (acc[node.kind] ?? 0) + 1;
      return acc;
    }, {} as Record<string, number>);
  }, [liveNodes]);

  const lowerQuery = query.toLowerCase();
  const filteredNodes = useMemo(
    () =>
      NODE_TYPES.filter(
        (node) =>
          !lowerQuery ||
          node.kind.includes(lowerQuery) ||
          node.label.toLowerCase().includes(lowerQuery) ||
          node.description.toLowerCase().includes(lowerQuery),
      ),
    [lowerQuery],
  );

  const scopeLabel = `${scope.tenant_id}/${scope.workspace_id}/${scope.project_id}`;
  const runtimeNodeCount = runtimeGraph.nodes.length;
  const errorMessage = error instanceof Error ? error.message : "Unable to load live graph data.";

  return (
    <div className="flex flex-col h-full bg-white dark:bg-zinc-950">
      <div className="flex items-center gap-3 px-5 h-11 border-b border-gray-200 dark:border-zinc-800 shrink-0">
        <Network size={13} className="text-indigo-400 shrink-0" />
        <span className="text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider">Knowledge Graph</span>
        <span className="text-[10px] text-gray-300 dark:text-zinc-600">Implements RFC 015</span>

        <div className="ml-4 flex items-center gap-0.5 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 p-0.5">
          {(["simulation", "schema"] as View[]).map((nextView) => (
            <button
              key={nextView}
              onClick={() => setView(nextView)}
              className={clsx(
                "px-2.5 py-1 rounded text-[11px] font-medium transition-colors capitalize",
                view === nextView
                  ? "bg-gray-200 dark:bg-zinc-700 text-gray-900 dark:text-zinc-100"
                  : "text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300",
              )}
            >
              {nextView}
            </button>
          ))}
        </div>

        <div className="ml-auto flex items-center gap-3">
          <span className="text-[11px] text-gray-300 dark:text-zinc-600 font-mono truncate max-w-[260px]">
            {scopeLabel}
          </span>
          <button
            onClick={() => void refetch()}
            className="flex items-center gap-1 px-2 py-1 rounded text-[11px] border border-gray-200 dark:border-zinc-800 text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:hover:text-zinc-200 hover:bg-gray-50 dark:hover:bg-zinc-900 transition-colors"
          >
            <RefreshCw size={11} className={clsx(isFetching && "animate-spin")} />
            Refresh
          </button>
        </div>
      </div>

      <div className="flex flex-1 min-h-0 overflow-hidden">
        <div className="flex-1 overflow-y-auto min-w-0">
          <div className="max-w-4xl mx-auto px-5 py-5 space-y-5">
            {view === "simulation" ? (
              <>
                <div className="text-[12px] text-gray-400 dark:text-zinc-600 leading-relaxed">
                  Live session → run → task graph for the current project scope.
                  {runtimeNodeCount > 0 ? ` Rendering ${runtimeNodeCount} runtime nodes from ${liveNodes.length} total graph nodes.` : " Waiting for runtime activity to populate the trace."}
                </div>

                {isLoading ? (
                  <div className="rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60 p-8 flex items-center justify-center gap-3 text-sm text-gray-500 dark:text-zinc-400">
                    <Loader2 size={16} className="animate-spin" />
                    Loading live graph trace…
                  </div>
                ) : isError ? (
                  <div className="rounded-lg border border-red-800/40 bg-red-500/5 px-4 py-3 flex items-start gap-3">
                    <Info size={14} className="text-red-500 shrink-0 mt-0.5" />
                    <div className="flex-1 min-w-0">
                      <p className="text-[12px] font-medium text-red-400">Graph trace unavailable</p>
                      <p className="text-[11px] text-red-300/80 mt-0.5">{errorMessage}</p>
                    </div>
                    <button
                      onClick={() => void refetch()}
                      className="shrink-0 px-2 py-1 rounded text-[11px] border border-red-800/40 text-red-300 hover:bg-red-900/30 transition-colors"
                    >
                      Retry
                    </button>
                  </div>
                ) : runtimeGraph.nodes.length === 0 ? (
                  <FeatureEmptyState
                    icon={<Network size={20} className="text-gray-400 dark:text-zinc-500" />}
                    title="No graph data yet"
                    description="Graph nodes are created automatically when sessions and runs execute. Create a session to get started."
                    actionLabel="Go to Sessions"
                    actionHref="#sessions"
                  />
                ) : (
                  <ForceGraph graph={runtimeGraph} />
                )}
              </>
            ) : (
              <>
                <div className="flex items-start gap-8 py-3 px-4 rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50/60 dark:bg-zinc-900/60">
                  <StatCard compact variant="info" label="Node Types" value={NODE_TYPES.length} description="defined in schema" />
                  <StatCard compact variant="info" label="Edge Types" value={EDGE_TYPES.length} description="relationship kinds" />
                  <StatCard compact variant="success" label="Live Nodes" value={liveNodes.length} description={isError ? "trace unavailable" : scope.project_id} />
                  <StatCard compact variant="info" label="Live Edges" value={liveEdges.length} description={isError ? "trace unavailable" : "current scope"} />
                </div>

                {isError ? (
                  <div className="rounded-lg border border-red-800/40 bg-red-500/5 px-4 py-3 flex items-start gap-3">
                    <Info size={14} className="text-red-500 shrink-0 mt-0.5" />
                    <div className="flex-1 min-w-0">
                      <p className="text-[12px] font-medium text-red-400">Live graph lookup failed</p>
                      <p className="text-[11px] text-red-300/80 mt-0.5">{errorMessage}</p>
                    </div>
                  </div>
                ) : liveNodes.length === 0 ? (
                  <FeatureEmptyState
                    icon={<Network size={20} className="text-gray-400 dark:text-zinc-500" />}
                    title="No graph data yet"
                    description="Graph nodes are created automatically when sessions and runs execute. Create a session to get started."
                    actionLabel="Go to Sessions"
                    actionHref="#sessions"
                  />
                ) : (
                  <div className="rounded-lg border border-emerald-800/30 bg-emerald-500/5 px-4 py-3 flex items-start gap-3">
                    <Info size={14} className="text-emerald-500 shrink-0 mt-0.5" />
                    <div className="flex-1 min-w-0">
                      <p className="text-[12px] font-medium text-emerald-400">Live graph connected</p>
                      <p className="text-[11px] text-emerald-300/80 mt-0.5">
                        Showing {liveNodes.length} nodes and {liveEdges.length} edges for {scopeLabel}. Select any node type to inspect its live count in the side panel.
                      </p>
                    </div>
                  </div>
                )}

                <div className="relative">
                  <Search size={13} className="absolute left-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
                  <input
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                    placeholder="Filter node types… (e.g. 'run', 'prompt', 'memory')"
                    className="w-full rounded-lg border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[13px] text-gray-800 dark:text-zinc-200 placeholder-zinc-600 pl-9 pr-4 py-2 focus:outline-none focus:border-indigo-500 transition-colors"
                  />
                  {query && (
                    <button
                      onClick={() => setQuery("")}
                      className="absolute right-3 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 hover:text-gray-500 dark:hover:text-zinc-400 transition-colors"
                    >
                      ×
                    </button>
                  )}
                </div>

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
                        {filteredNodes.map((node) => (
                          <NodeCard
                            key={node.kind}
                            node={node}
                            selected={selectedNode?.kind === node.kind}
                            onClick={() => setSelectedNode((current) => (current?.kind === node.kind ? null : node))}
                          />
                        ))}
                      </div>
                    </div>
                  )
                ) : (
                  <div className="space-y-5">
                    {GROUPS.map((group) => {
                      const nodes = NODE_TYPES.filter((node) => node.group === group);
                      return (
                        <div key={group}>
                          <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-2">
                            {GROUP_LABELS[group]}
                            <span className="ml-2 text-gray-300 dark:text-zinc-600 normal-case font-normal">
                              {nodes.length} types
                            </span>
                          </p>
                          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-2">
                            {nodes.map((node) => (
                              <NodeCard
                                key={node.kind}
                                node={node}
                                selected={selectedNode?.kind === node.kind}
                                onClick={() => setSelectedNode((current) => (current?.kind === node.kind ? null : node))}
                              />
                            ))}
                          </div>
                        </div>
                      );
                    })}
                  </div>
                )}

                <div>
                  <p className="text-[10px] font-semibold text-gray-400 dark:text-zinc-600 uppercase tracking-wider mb-3">
                    Edge Types
                    <span className="ml-2 text-gray-300 dark:text-zinc-600 normal-case font-normal">
                      {EDGE_TYPES.length} relationship kinds
                    </span>
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
                        {EDGE_TYPES.map((edge, index) => (
                          <tr
                            key={edge.kind}
                            className={clsx(
                              "transition-colors hover:bg-gray-100/40 dark:hover:bg-zinc-800/40",
                              index % 2 === 0 ? "bg-gray-50 dark:bg-zinc-900" : "bg-white dark:bg-zinc-950",
                            )}
                          >
                            <td className="px-3 py-1.5 font-mono text-[11px] text-gray-400 dark:text-zinc-500 whitespace-nowrap">{edge.kind}</td>
                            <td className="px-3 py-1.5 text-gray-500 dark:text-zinc-400 whitespace-nowrap">{edge.label}</td>
                            <td className="px-3 py-1.5 text-gray-400 dark:text-zinc-600">{edge.description}</td>
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

        {view === "schema" && selectedNode && (
          <NodeDetail
            node={selectedNode}
            liveCount={liveCounts[selectedNode.kind] ?? 0}
            onClose={() => setSelectedNode(null)}
          />
        )}
      </div>
    </div>
  );
}

export default GraphPage;
