import { useQuery } from "@tanstack/react-query";
import { Activity, BookOpen, Loader2, Wrench } from "lucide-react";
import { ErrorFallback } from "../components/ErrorFallback";
import { defaultApi } from "../lib/api";

function StatCard({ label, value, sub }: { label: string; value: number; sub?: string }) {
  return (
    <div className="border-l-2 border-indigo-500 pl-3 py-0.5">
      <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-wider">{label}</p>
      <p className="text-[20px] font-semibold text-gray-900 dark:text-zinc-100 tabular-nums leading-tight">{value}</p>
      {sub && <p className="text-[11px] text-gray-400 dark:text-zinc-600 mt-0.5">{sub}</p>}
    </div>
  );
}

function displayName(skill: { id?: string; name?: string }) {
  return skill.name?.trim() || skill.id?.trim() || "Unnamed skill";
}

export function SkillsPage() {
  const { data, isLoading, isError, error, refetch, isFetching } = useQuery({
    queryKey: ["skills"],
    queryFn: () => defaultApi.listSkills(),
    staleTime: 15_000,
  });

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 px-6 py-5 text-[12px] text-gray-400 dark:text-zinc-600">
        <Loader2 size={12} className="animate-spin" /> Loading skills…
      </div>
    );
  }

  if (isError) {
    return <ErrorFallback error={error} resource="skills" onRetry={() => void refetch()} />;
  }

  const items = data?.items ?? [];
  const summary = data?.summary ?? { total: 0, enabled: 0, disabled: 0 };
  const active = data?.currently_active ?? [];

  return (
    <div className="p-6 space-y-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <p className="text-[11px] text-gray-400 dark:text-zinc-500 uppercase tracking-widest">Infrastructure / Skills</p>
          <h1 className="text-[24px] font-semibold text-gray-900 dark:text-zinc-100 mt-1">Skills</h1>
          <p className="text-[13px] text-gray-400 dark:text-zinc-500 mt-1">
            Operator-visible inventory of installed and currently active skills.
          </p>
        </div>
        <button
          onClick={() => void refetch()}
          disabled={isFetching}
          className="flex items-center gap-1.5 rounded-md border border-gray-200 dark:border-zinc-800 px-3 py-1.5 text-[12px] text-gray-500 dark:text-zinc-400 hover:text-gray-800 dark:hover:text-zinc-100 hover:bg-gray-50 dark:hover:bg-zinc-900 disabled:opacity-50 transition-colors"
        >
          {isFetching ? <Loader2 size={12} className="animate-spin" /> : <Activity size={12} />}
          Refresh
        </button>
      </div>

      <div className="grid grid-cols-1 gap-4 md:grid-cols-3">
        <StatCard label="Installed" value={summary.total} />
        <StatCard label="Enabled" value={summary.enabled} />
        <StatCard label="Disabled" value={summary.disabled} />
      </div>

      <div className="grid grid-cols-1 gap-5 xl:grid-cols-[1.3fr,0.7fr]">
        <section className="rounded-xl border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 overflow-hidden">
          <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
            <div>
              <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">Installed skills</p>
              <p className="text-[11px] text-gray-400 dark:text-zinc-600">Inventory returned by `GET /v1/skills`.</p>
            </div>
            <span className="text-[11px] font-mono text-gray-400 dark:text-zinc-600">{items.length}</span>
          </div>

          {items.length === 0 ? (
            <div className="px-4 py-8 text-center">
              <BookOpen size={18} className="mx-auto text-gray-300 dark:text-zinc-700 mb-2" />
              <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No skills discovered</p>
              <p className="text-[12px] text-gray-400 dark:text-zinc-600 mt-1">
                Installed skills will appear here when the host reports them.
              </p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200 dark:divide-zinc-800">
              {items.map((skill, index) => (
                <div key={`${skill.id ?? skill.name ?? "skill"}-${index}`} className="px-4 py-3">
                  <div className="flex items-start justify-between gap-3">
                    <div className="min-w-0">
                      <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-100 truncate">
                        {displayName(skill)}
                      </p>
                      {typeof skill.description === "string" && skill.description.length > 0 && (
                        <p className="text-[12px] text-gray-400 dark:text-zinc-500 mt-0.5 leading-relaxed">
                          {skill.description}
                        </p>
                      )}
                    </div>
                    <span
                      className={`shrink-0 rounded-full border px-2 py-0.5 text-[10px] font-medium ${
                        skill.enabled === false
                          ? "border-gray-200 dark:border-zinc-700 text-gray-500 dark:text-zinc-400 bg-gray-100 dark:bg-zinc-800"
                          : "border-emerald-200 dark:border-emerald-900/50 text-emerald-600 dark:text-emerald-400 bg-emerald-50 dark:bg-emerald-950/20"
                      }`}
                    >
                      {skill.enabled === false ? "disabled" : "enabled"}
                    </span>
                  </div>
                  {typeof skill.id === "string" && skill.id.length > 0 && (
                    <p className="mt-2 text-[11px] font-mono text-gray-400 dark:text-zinc-600">{skill.id}</p>
                  )}
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="rounded-xl border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 overflow-hidden">
          <div className="px-4 py-3 border-b border-gray-200 dark:border-zinc-800">
            <p className="text-[13px] font-medium text-gray-900 dark:text-zinc-100">Currently active</p>
            <p className="text-[11px] text-gray-400 dark:text-zinc-600">Skills the host reports as active right now.</p>
          </div>

          {active.length === 0 ? (
            <div className="px-4 py-8 text-center">
              <Wrench size={18} className="mx-auto text-gray-300 dark:text-zinc-700 mb-2" />
              <p className="text-[13px] font-medium text-gray-500 dark:text-zinc-400">No active skills</p>
              <p className="text-[12px] text-gray-400 dark:text-zinc-600 mt-1">
                Active skill sessions will show up here when reported.
              </p>
            </div>
          ) : (
            <div className="divide-y divide-gray-200 dark:divide-zinc-800">
              {active.map((entry, index) => (
                <div key={`${entry}-${index}`} className="px-4 py-3 text-[12px] font-mono text-gray-500 dark:text-zinc-400">
                  {entry}
                </div>
              ))}
            </div>
          )}
        </section>
      </div>
    </div>
  );
}
