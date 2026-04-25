/**
 * RoutingPreview — F29 CE, highest-priority feature.
 *
 * Cross-references the `brain_model` and `generate_model` system-scope
 * defaults against the active provider connections so operators can
 * see at a glance which connection will serve each role. Replaces the
 * curl-debug workflow that blocked dogfood earlier.
 *
 * Rendered on the Providers page above the Connections table.
 */

import { useQueries, useQuery } from "@tanstack/react-query";
import { AlertTriangle, Router as RouteIcon } from "lucide-react";
import { clsx } from "clsx";
import { defaultApi } from "../lib/api";
import { sectionLabel } from "../lib/design-system";
import type { ProviderConnectionRecord, SettingDefault } from "../lib/types";

// ── Role registry ──────────────────────────────────────────────────────────
//
// Keep this in sync with the backend's routing keys (see
// `crates/cairn-providers/src/routing.rs` — `routing::brain_model()` /
// `routing::generate_model()`). Adding a new role = adding a row here.

interface RoleDef {
  key: string;
  label: string;
  blurb: string;
}

const ROUTING_ROLES: readonly RoleDef[] = [
  { key: "brain_model",    label: "Brain",    blurb: "Heavy reasoning / decide phase" },
  { key: "generate_model", label: "Generate", blurb: "Everyday generation / gather phase" },
] as const;

// ── Resolver ───────────────────────────────────────────────────────────────

interface RoutingRow {
  role: RoleDef;
  /** `null` when the setting has never been persisted. */
  modelId: string | null;
  /** First active connection advertising `modelId`, or `null`. */
  connection: ProviderConnectionRecord | null;
}

/** Match a model ID against a connection's `supported_models`. Strict
 *  equality — we intentionally don't fuzzy-match so an operator typo
 *  surfaces as a warning instead of silently routing to the wrong
 *  model. */
function findConnection(
  modelId: string,
  connections: readonly ProviderConnectionRecord[],
): ProviderConnectionRecord | null {
  for (const c of connections) {
    if (c.status !== "active") continue;
    if (c.supported_models.includes(modelId)) return c;
  }
  return null;
}

/** Pull a string model ID out of a `SettingDefault.value`. The JSON
 *  value may be `string | number | boolean | null | object` — we only
 *  accept non-empty strings. Returns null for every other shape. */
function coerceModelId(v: unknown): string | null {
  if (typeof v === "string" && v.length > 0) return v;
  return null;
}

// ── Component ──────────────────────────────────────────────────────────────

export function RoutingPreview() {
  // Provider connections scoped to the active tenant — same query key
  // as the parent page so we dedupe with TanStack's cache.
  const { data: connsResponse } = useQuery({
    queryKey: ["provider-connections"],
    queryFn:  () => defaultApi.listProviderConnections(),
    staleTime: 30_000,
  });
  const connections: ProviderConnectionRecord[] = connsResponse?.items ?? [];

  // One query per role. `useQueries` instead of chaining so the rows
  // render independently and one missing default doesn't block the
  // others.
  const settingsQueries = useQueries({
    queries: ROUTING_ROLES.map(role => ({
      queryKey: ["settings-default", "system", "system", role.key],
      queryFn:  () =>
        defaultApi.getSettingsDefault("system", "system", role.key),
      staleTime: 30_000,
      retry: false as const,
    })),
  });

  const anyLoading = settingsQueries.some(q => q.isLoading);
  const rows: RoutingRow[] = ROUTING_ROLES.map((role, i) => {
    const setting = settingsQueries[i]?.data as SettingDefault | null | undefined;
    const modelId = setting ? coerceModelId(setting.value) : null;
    return {
      role,
      modelId,
      connection: modelId ? findConnection(modelId, connections) : null,
    };
  });

  return (
    <section className="space-y-3" data-testid="routing-preview">
      <div className="flex items-center gap-2">
        <RouteIcon size={13} className="text-indigo-400" />
        <p className={clsx(sectionLabel, "mb-0")}>Routing Preview</p>
      </div>
      <p className="text-[11px] text-gray-400 dark:text-zinc-500">
        Which provider connection serves each routing role for the active
        tenant. Configure with <code className="font-mono text-[10px]">PUT /v1/settings/defaults/system/system/&lt;role&gt;</code>.
      </p>
      <div className="rounded-lg border border-gray-200 dark:border-zinc-800 overflow-hidden divide-y divide-gray-200 dark:divide-zinc-800/60">
        {rows.map(row => <RoutingRowView key={row.role.key} row={row} loading={anyLoading} />)}
      </div>
    </section>
  );
}

function RoutingRowView({ row, loading }: { row: RoutingRow; loading: boolean }) {
  const { role, modelId, connection } = row;

  // Three terminal states: (1) not configured, (2) configured but no
  // advertising connection, (3) fully wired. The third is the common
  // dogfood case — show connection ID + model on one line, no warning.
  let status: "unset" | "unwired" | "ok";
  if (!modelId) status = "unset";
  else if (!connection) status = "unwired";
  else status = "ok";

  return (
    <div
      className={clsx(
        "flex items-center gap-3 px-3 py-2",
        status === "unwired" && "bg-amber-500/5",
      )}
      data-testid={`routing-row-${role.key}`}
      data-status={status}
    >
      <div className="min-w-[5rem]">
        <p className="text-[12px] font-medium text-gray-900 dark:text-zinc-100">{role.label}</p>
        <p className="text-[10px] text-gray-400 dark:text-zinc-600">{role.blurb}</p>
      </div>
      <div className="flex-1 min-w-0 flex items-center gap-2 font-mono text-[12px]">
        {status === "unset" ? (
          <span
            className="text-gray-400 dark:text-zinc-600"
            data-testid={`routing-unset-${role.key}`}
          >
            not configured
          </span>
        ) : status === "unwired" ? (
          <>
            <span className="text-gray-700 dark:text-zinc-300 truncate" title={modelId!}>{modelId}</span>
            <span
              className="inline-flex items-center gap-1 text-amber-500 dark:text-amber-400 text-[11px] whitespace-nowrap"
              data-testid={`routing-warn-${role.key}`}
            >
              <AlertTriangle size={11} /> no active provider connection
            </span>
          </>
        ) : (
          <>
            <span className="text-indigo-500 dark:text-indigo-300 truncate" title={connection!.provider_connection_id}>
              {connection!.provider_connection_id}
            </span>
            <span className="text-gray-400 dark:text-zinc-500">&rarr;</span>
            <span className="text-gray-700 dark:text-zinc-300 truncate" title={modelId!}>
              {modelId}
            </span>
          </>
        )}
      </div>
      {loading && (
        <span className="text-[10px] text-gray-300 dark:text-zinc-600">loading…</span>
      )}
    </div>
  );
}

export default RoutingPreview;
