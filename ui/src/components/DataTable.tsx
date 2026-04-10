/**
 * DataTable — generic sortable/filterable/exportable table.
 *
 * Usage:
 *   <DataTable
 *     data={rows}
 *     columns={[
 *       { key: 'id',      header: 'ID',      render: (r) => r.id,  sortValue: (r) => r.id },
 *       { key: 'state',   header: 'State',   render: (r) => <Badge state={r.state} /> },
 *       { key: 'created', header: 'Created', sortValue: (r) => r.created_at },
 *     ]}
 *     filterFn={(row, q) => row.id.includes(q)}
 *     csvRow={(r) => [r.id, r.state, r.created_at]}
 *     csvHeaders={['ID', 'State', 'Created At']}
 *     filename="runs"
 *   />
 */

import { useState, useMemo, type ReactNode } from 'react';
import { ChevronUp, ChevronDown, ChevronsUpDown, Download, Search, ChevronLeft, ChevronRight } from 'lucide-react';
import { clsx } from 'clsx';

// ── Types ─────────────────────────────────────────────────────────────────────

export interface ColumnDef<T> {
  key:        string;
  header:     string;
  /** Rendered cell content. */
  render:     (row: T) => ReactNode;
  /** Return a comparable primitive for sorting. Omit to disable sort on this column. */
  sortValue?: (row: T) => string | number | null | undefined;
  /** Extra class on the <td>. */
  cellClass?: string;
  /** Extra class on the <th>. */
  headClass?: string;
}

export interface DataTableProps<T> {
  data:        T[];
  columns:     ColumnDef<T>[];
  /** Return true if the row matches the filter query. */
  filterFn?:   (row: T, query: string) => boolean;
  /** Return the CSV values for one row (strings). */
  csvRow?:     (row: T) => (string | number | null | undefined)[];
  /** CSV header row. */
  csvHeaders?: string[];
  /** Base filename for download (no .csv). */
  filename?:   string;
  /** Empty state message. */
  emptyText?:  string;
  /** Default rows per page. */
  defaultPageSize?: number;
  className?:  string;
  // ── Keyboard navigation ───────────────────────────────────────────────────
  /** 0-based index (in `data`) of the currently keyboard-focused row. */
  activeIndex?:  number;
  /** Set of row keys that are selected for bulk actions. */
  selectedIds?:  Set<string>;
  /** Called with the original data row and its index when clicked or Enter'd. */
  onRowClick?:   (row: T, index: number) => void;
  /** Derive a stable string key from a row (required when selectedIds is set). */
  getRowId?:     (row: T) => string;
}

type SortDir = 'asc' | 'desc';

// ── CSV helper ────────────────────────────────────────────────────────────────

function escapeCsv(v: string | number | null | undefined): string {
  if (v == null) return '';
  const s = String(v);
  if (s.includes(',') || s.includes('"') || s.includes('\n')) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}

function triggerDownload(csv: string, filename: string) {
  const blob = new Blob([csv], { type: 'text/csv;charset=utf-8;' });
  const url  = URL.createObjectURL(blob);
  const a    = document.createElement('a');
  a.href     = url;
  a.download = `${filename}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}

// ── Sort icon ─────────────────────────────────────────────────────────────────

function SortIcon({ active, dir }: { active: boolean; dir: SortDir }) {
  if (!active) return <ChevronsUpDown size={11} className="text-gray-300 dark:text-zinc-700 ml-1 inline shrink-0" />;
  return dir === 'asc'
    ? <ChevronUp   size={11} className="text-indigo-400 ml-1 inline shrink-0" />
    : <ChevronDown size={11} className="text-indigo-400 ml-1 inline shrink-0" />;
}

// ── Main component ────────────────────────────────────────────────────────────

const PAGE_SIZES = [10, 25, 50, 100];

export function DataTable<T>({
  data,
  columns,
  filterFn,
  csvRow,
  csvHeaders,
  filename = 'export',
  emptyText = 'No data',
  defaultPageSize = 25,
  className,
  activeIndex,
  selectedIds,
  onRowClick,
  getRowId,
}: DataTableProps<T>) {
  const [query,      setQuery]      = useState('');
  const [sortKey,    setSortKey]    = useState<string | null>(null);
  const [sortDir,    setSortDir]    = useState<SortDir>('asc');
  const [page,       setPage]       = useState(0);
  const [pageSize,   setPageSize]   = useState(defaultPageSize);

  // ── Filter ──────────────────────────────────────────────────────────────────
  const filtered = useMemo(() => {
    if (!query.trim() || !filterFn) return data;
    const q = query.trim().toLowerCase();
    return data.filter(row => filterFn(row, q));
  }, [data, query, filterFn]);

  // ── Sort ────────────────────────────────────────────────────────────────────
  const sorted = useMemo(() => {
    if (!sortKey) return filtered;
    const col = columns.find(c => c.key === sortKey);
    if (!col?.sortValue) return filtered;
    return [...filtered].sort((a, b) => {
      const av = col.sortValue!(a) ?? '';
      const bv = col.sortValue!(b) ?? '';
      const cmp = av < bv ? -1 : av > bv ? 1 : 0;
      return sortDir === 'asc' ? cmp : -cmp;
    });
  }, [filtered, sortKey, sortDir, columns]);

  // ── Pagination ───────────────────────────────────────────────────────────────
  const totalPages = Math.max(1, Math.ceil(sorted.length / pageSize));
  const safePage   = Math.min(page, totalPages - 1);
  const pageRows   = sorted.slice(safePage * pageSize, (safePage + 1) * pageSize);

  function handleSort(key: string) {
    if (sortKey === key) {
      setSortDir(d => d === 'asc' ? 'desc' : 'asc');
    } else {
      setSortKey(key);
      setSortDir('asc');
    }
    setPage(0);
  }

  function handleFilter(q: string) {
    setQuery(q);
    setPage(0);
  }

  // ── CSV export ───────────────────────────────────────────────────────────────
  function handleExport() {
    if (!csvRow) return;
    const rows: string[] = [];
    if (csvHeaders) rows.push(csvHeaders.map(escapeCsv).join(','));
    for (const row of sorted) {
      rows.push(csvRow(row).map(escapeCsv).join(','));
    }
    triggerDownload(rows.join('\r\n'), filename);
  }

  return (
    <div className={clsx('flex flex-col gap-0', className)}>
      {/* ── Toolbar ──────────────────────────────────────────────────────── */}
      <div className="flex items-center gap-2 mb-2">
        {/* Search */}
        {filterFn && (
          <div className="relative flex-1 max-w-xs">
            <Search size={12} className="absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400 dark:text-zinc-600 pointer-events-none" />
            <input
              value={query}
              onChange={e => handleFilter(e.target.value)}
              placeholder="Filter…"
              className="w-full h-8 pl-7 pr-3 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800
                         text-xs text-gray-700 dark:text-zinc-300 placeholder-zinc-600
                         focus:outline-none focus:border-indigo-500 focus:ring-1 focus:ring-indigo-500
                         transition-colors"
            />
          </div>
        )}

        <span className="text-[11px] text-gray-400 dark:text-zinc-600 ml-1">
          {filtered.length !== data.length
            ? `${filtered.length} / ${data.length}`
            : `${data.length} rows`}
        </span>

        {/* CSV export */}
        {csvRow && (
          <button
            onClick={handleExport}
            className="ml-auto flex items-center gap-1.5 h-8 px-2.5 rounded-md
                       bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 text-[11px] text-gray-400 dark:text-zinc-500
                       hover:bg-white/5 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors"
            title="Export filtered data as CSV"
          >
            <Download size={12} /> CSV
          </button>
        )}
      </div>

      {/* ── Table ────────────────────────────────────────────────────────── */}
      <div className="bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden">
        {/* Header */}
        <div className="border-b border-gray-200 dark:border-zinc-800 bg-white dark:bg-zinc-950">
          <table className="w-full">
            <thead>
              <tr>
                {columns.map(col => {
                  const sortable = !!col.sortValue;
                  const active   = sortKey === col.key;
                  return (
                    <th
                      key={col.key}
                      onClick={sortable ? () => handleSort(col.key) : undefined}
                      className={clsx(
                        'px-4 h-8 text-left text-[10px] font-medium text-gray-400 dark:text-zinc-600 uppercase tracking-wider select-none whitespace-nowrap',
                        sortable && 'cursor-pointer hover:text-gray-500 dark:hover:text-zinc-400 transition-colors',
                        active && 'text-indigo-400',
                        col.headClass,
                      )}
                    >
                      {col.header}
                      {sortable && <SortIcon active={active} dir={sortDir} />}
                    </th>
                  );
                })}
              </tr>
            </thead>
          </table>
        </div>

        {/* Body */}
        {pageRows.length === 0 ? (
          <div className="px-4 py-12 text-center text-xs text-gray-400 dark:text-zinc-600">{emptyText}</div>
        ) : (
          <table className="w-full">
            <tbody>
              {pageRows.map((row, i) => {
                // Absolute index in `data` (pre-filter/sort) for keyboard nav matching.
                const absIdx   = safePage * pageSize + i;
                const rowKey   = getRowId ? getRowId(row) : undefined;
                const isActive = activeIndex !== undefined && activeIndex === absIdx;
                const isSelected = rowKey !== undefined && selectedIds ? selectedIds.has(rowKey) : false;
                return (
                <tr
                  key={i}
                  onClick={() => onRowClick?.(row, absIdx)}
                  className={clsx(
                    'border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0 transition-colors',
                    onRowClick ? 'cursor-pointer' : '',
                    isActive
                      ? 'bg-zinc-700/70 ring-1 ring-inset ring-zinc-500/60'
                      : isSelected
                        ? 'bg-indigo-950/40 hover:bg-indigo-950/60'
                        : i % 2 === 0
                          ? 'bg-gray-50 dark:bg-zinc-900 hover:bg-white/5'
                          : 'bg-gray-50/50 dark:bg-zinc-900/50 hover:bg-white/5',
                  )}
                >
                  {columns.map(col => (
                    <td key={col.key} className={clsx('px-4 h-9', col.cellClass)}>
                      {col.render(row)}
                    </td>
                  ))}
                </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </div>

      {/* ── Pagination ───────────────────────────────────────────────────── */}
      {sorted.length > 10 && (
        <div className="flex items-center justify-between mt-2 px-1">
          <div className="flex items-center gap-1.5">
            <span className="text-[11px] text-gray-400 dark:text-zinc-600">Rows per page</span>
            <select
              value={pageSize}
              onChange={e => { setPageSize(Number(e.target.value)); setPage(0); }}
              className="h-7 px-1.5 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 text-[11px] text-gray-500 dark:text-zinc-400
                         focus:outline-none focus:border-indigo-500 transition-colors"
            >
              {PAGE_SIZES.map(n => <option key={n} value={n}>{n}</option>)}
            </select>
          </div>

          <div className="flex items-center gap-2">
            <span className="text-[11px] text-gray-400 dark:text-zinc-600">
              {safePage * pageSize + 1}–{Math.min((safePage + 1) * pageSize, sorted.length)} of {sorted.length}
            </span>
            <button
              onClick={() => setPage(p => Math.max(0, p - 1))}
              disabled={safePage === 0}
              className="flex items-center justify-center w-7 h-7 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800
                         text-gray-400 dark:text-zinc-500 hover:bg-white/5 disabled:opacity-30 disabled:cursor-not-allowed
                         transition-colors"
            >
              <ChevronLeft size={13} />
            </button>
            <span className="text-[11px] text-gray-400 dark:text-zinc-500 tabular-nums min-w-[3rem] text-center">
              {safePage + 1} / {totalPages}
            </span>
            <button
              onClick={() => setPage(p => Math.min(totalPages - 1, p + 1))}
              disabled={safePage >= totalPages - 1}
              className="flex items-center justify-center w-7 h-7 rounded-md bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800
                         text-gray-400 dark:text-zinc-500 hover:bg-white/5 disabled:opacity-30 disabled:cursor-not-allowed
                         transition-colors"
            >
              <ChevronRight size={13} />
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
