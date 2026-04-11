/**
 * design-system.ts — Single source of truth for cairn UI design tokens.
 *
 * All values derived from an audit of 39 pages + 23 shared components.
 * Every token maps to the BEST existing pattern, not an aspirational redesign.
 *
 * Usage: import { ds } from '../lib/design-system';
 *        <div className={ds.card.base}>…</div>
 *        <p className={ds.text.heading}>Title</p>
 */

// ─── Color Tokens ────────────────────────────────────────────────────────────

/** Semantic surface colors (background + border combos). */
export const surface = {
  /** Full-page scrollable background (DashboardPage, MetricsPage, PromptsPage). */
  page:       "bg-white dark:bg-zinc-950",
  /** Full-page background for dense data views (TasksPage, TracesPage, SettingsPage). */
  pageDense:  "bg-gray-50 dark:bg-zinc-900",
  /** Card / panel background. */
  card:       "bg-gray-50 dark:bg-zinc-900",
  /** Elevated surface inside cards (expanded sections, code blocks). */
  elevated:   "bg-white dark:bg-zinc-950",
  /** Table header background. */
  tableHead:  "bg-white dark:bg-zinc-950",
  /** Modal overlay. */
  overlay:    "bg-black/60",
  /** Modal / drawer body. */
  modal:      "bg-gray-50 dark:bg-zinc-900",
} as const;

/** Border tokens. */
export const border = {
  /** Standard panel / card border. */
  default:    "border-gray-200 dark:border-zinc-800",
  /** Subtle row separator. */
  subtle:     "border-gray-200/50 dark:border-zinc-800/50",
  /** Section divider within panels. */
  section:    "border-gray-200/60 dark:border-zinc-800/60",
  /** Interactive border on focus. */
  focus:      "border-indigo-500",
  /** Muted divider for toolbars. */
  muted:      "border-gray-200 dark:border-zinc-700",
} as const;

/** Semantic text colors. */
export const text = {
  /** Primary headings and titles. */
  heading:    "text-gray-900 dark:text-zinc-100",
  /** Secondary headings. */
  subheading: "text-gray-800 dark:text-zinc-200",
  /** Standard body text. */
  body:       "text-gray-700 dark:text-zinc-300",
  /** Secondary / descriptive text. */
  secondary:  "text-gray-500 dark:text-zinc-400",
  /** Labels and captions. */
  label:      "text-gray-400 dark:text-zinc-500",
  /** Muted text, placeholders, timestamps. */
  muted:      "text-gray-300 dark:text-zinc-600",
  /** Mono text in table cells. */
  mono:       "text-gray-700 dark:text-zinc-300 font-mono",
  /** Mono muted (IDs, hashes). */
  monoMuted:  "text-gray-400 dark:text-zinc-500 font-mono",
} as const;

/** Status / semantic colors — text + background + border combos. */
export const status = {
  success: {
    text:   "text-emerald-400",
    bg:     "bg-emerald-500/10",
    border: "border-emerald-500/20",
    accent: "border-l-emerald-500",
    dot:    "bg-emerald-500",
  },
  warning: {
    text:   "text-amber-400",
    bg:     "bg-amber-500/10",
    border: "border-amber-500/20",
    accent: "border-l-amber-500",
    dot:    "bg-amber-500",
  },
  danger: {
    text:   "text-red-400",
    bg:     "bg-red-500/10",
    border: "border-red-500/20",
    accent: "border-l-red-500",
    dot:    "bg-red-500",
  },
  info: {
    text:   "text-indigo-400",
    bg:     "bg-indigo-500/10",
    border: "border-indigo-500/20",
    accent: "border-l-indigo-500",
    dot:    "bg-indigo-500",
  },
  neutral: {
    text:   "text-gray-400 dark:text-zinc-500",
    bg:     "bg-gray-100/60 dark:bg-zinc-800/60",
    border: "border-gray-200 dark:border-zinc-700",
    accent: "border-l-gray-300 dark:border-l-zinc-700",
    dot:    "bg-zinc-600",
  },
} as const;

export type StatusVariant = keyof typeof status;

// ─── Typography Scale ────────────────────────────────────────────────────────

export const fontSize = {
  /** 10px — metadata, timestamps, subtle values. */
  xs2:    "text-[10px]",
  /** 11px — section labels, stat labels, badges, small UI text. */
  xs:     "text-[11px]",
  /** 12px — body text, buttons, form labels, table cells. */
  sm:     "text-[12px]",
  /** 13px — page titles, toolbar headings. */
  md:     "text-[13px]",
  /** 14px — prominent headings. */
  lg:     "text-[14px]",
  /** 20px — stat card values (compact variant). */
  stat:   "text-[20px]",
  /** 24px — stat card values (standard / text-2xl). */
  statLg: "text-2xl",
  /** 22px — dashboard headline numbers. */
  headline: "text-[22px]",
} as const;

export const fontWeight = {
  normal:   "font-normal",
  medium:   "font-medium",
  semibold: "font-semibold",
  bold:     "font-bold",
} as const;

// ─── Spacing Scale ───────────────────────────────────────────────────────────

export const spacing = {
  /** Standard page layout — wide content pages. */
  pageWide:   "max-w-5xl mx-auto px-6 py-6 space-y-6",
  /** Standard page layout — narrow content pages. */
  pageNarrow: "max-w-4xl mx-auto px-5 py-5 space-y-6",
  /** Dense page layout — p-6, no max-width constraint. */
  pagePadded: "p-6 space-y-5",
  /** Card/panel internal padding. */
  card:       "p-4",
  /** Stat card grid — 2 cols mobile, 4 cols desktop. */
  statGrid:   "grid grid-cols-2 gap-3 lg:grid-cols-4",
  /** Two-column panel grid. */
  panelGrid:  "grid grid-cols-1 gap-4 lg:grid-cols-2",
  /** Three-column stat grid. */
  statGrid3:  "grid grid-cols-3 gap-3",
} as const;

// ─── Component Class Presets ─────────────────────────────────────────────────

/** Card / Panel presets. */
export const card = {
  /** Standard panel: bg + border + rounded + padding. */
  base: "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg p-4",
  /** Panel without padding (for table-bearing panels). */
  shell: "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden",
  /** Elevated panel (inside other panels). */
  inner: "bg-white dark:bg-zinc-950 border border-gray-200 dark:border-zinc-800 rounded-lg overflow-hidden",
} as const;

/** Section label — the canonical "UPPERCASE LABEL" above content blocks. */
export const sectionLabel =
  "text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider mb-3";

/** Toolbar presets. */
export const toolbar = {
  /** Standard toolbar strip. */
  base: "flex items-center gap-3 px-4 h-10 border-b border-gray-200 dark:border-zinc-800 shrink-0",
  /** Page title in toolbar. */
  title: "text-[13px] font-medium text-gray-800 dark:text-zinc-200",
  /** Item count next to title. */
  count: "ml-2 text-[12px] text-gray-400 dark:text-zinc-500 font-normal",
} as const;

/** Button presets. */
export const btn = {
  /** Primary action (indigo filled). */
  primary:
    "flex items-center gap-1.5 rounded bg-indigo-600 hover:bg-indigo-500 text-white text-[12px] font-medium px-3 py-1.5 transition-colors disabled:opacity-50",
  /** Secondary action (outlined). */
  secondary:
    "flex items-center gap-1.5 rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-gray-500 dark:text-zinc-400 text-[11px] px-2.5 py-1.5 hover:text-gray-800 dark:hover:text-zinc-200 hover:border-zinc-600 transition-colors disabled:opacity-40",
  /** Danger action (red filled). */
  danger:
    "flex items-center gap-1.5 rounded bg-red-600 hover:bg-red-500 text-white text-[12px] font-medium px-3 py-1.5 transition-colors disabled:opacity-50",
  /** Ghost button (text only). */
  ghost:
    "flex items-center gap-1 text-[12px] text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors disabled:opacity-40",
  /** Icon button (small square). */
  icon:
    "flex items-center justify-center w-7 h-7 rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors disabled:opacity-30",
} as const;

/** Badge presets — status pills with optional dot. */
export const badge = {
  /** Base badge structure. Add a status color set. */
  base: "inline-flex items-center gap-1.5 rounded text-[11px] font-medium px-1.5 py-0.5 whitespace-nowrap",
  /** Dot inside a badge. */
  dot: "w-1.5 h-1.5 rounded-full shrink-0",
  /** Badge with border (for outlined badges). */
  outlined: "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-medium border whitespace-nowrap",
} as const;

/** Form field presets. */
export const input = {
  /** Standard text input. */
  base:
    "w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200 px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors",
  /** Mono input (IDs, codes). */
  mono:
    "w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200 font-mono px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors",
  /** Select dropdown. */
  select:
    "rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-900 text-[12px] text-gray-700 dark:text-zinc-300 px-2 py-1 focus:outline-none focus:border-indigo-500 transition-colors",
  /** Textarea. */
  textarea:
    "w-full rounded border border-gray-200 dark:border-zinc-800 bg-gray-50 dark:bg-zinc-950 text-[12px] text-gray-800 dark:text-zinc-200 font-mono px-3 py-2 focus:outline-none focus:border-indigo-500 transition-colors resize-y leading-relaxed",
  /** Form label. */
  label: "text-[11px] text-gray-400 dark:text-zinc-500 block mb-1",
  /** Required asterisk. */
  required: "text-red-500",
  /** Helper text below inputs. */
  helper: "text-[10px] text-gray-300 dark:text-zinc-600 mt-1",
} as const;

/** Table presets. */
export const table = {
  /** Table header cell. */
  th: "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800 text-left",
  /** Table header cell — right-aligned. */
  thRight: "px-3 py-2 text-[11px] font-medium text-gray-400 dark:text-zinc-500 uppercase tracking-wider whitespace-nowrap border-b border-gray-200 dark:border-zinc-800 text-right",
  /** Table cell. */
  td: "px-3 py-1.5",
  /** Column header row background. */
  headBg: "bg-white dark:bg-zinc-950",
  /** Even row. */
  rowEven: "bg-gray-50 dark:bg-zinc-900",
  /** Odd row. */
  rowOdd: "bg-gray-50/50 dark:bg-zinc-900/50",
  /** Row hover. */
  rowHover: "hover:bg-white/5 transition-colors",
  /** Row border. */
  rowBorder: "border-b border-gray-200/50 dark:border-zinc-800/50 last:border-0",
} as const;

/** Modal / Drawer presets. */
export const modal = {
  /** Backdrop overlay. */
  backdrop: "fixed inset-0 z-50 flex items-center justify-center bg-black/60",
  /** Modal container. */
  container: "bg-gray-50 dark:bg-zinc-900 border border-gray-200 dark:border-zinc-800 rounded-lg shadow-xl",
  /** Modal header. */
  header: "flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-zinc-800",
  /** Modal header title. */
  headerTitle: "text-[13px] font-medium text-gray-800 dark:text-zinc-200",
  /** Modal body. */
  body: "px-4 py-4 space-y-4",
  /** Modal footer. */
  footer: "flex items-center justify-end gap-2 px-4 py-3 border-t border-gray-200 dark:border-zinc-800",
  /** Drawer (right side panel). */
  drawer: "fixed right-0 top-0 bottom-0 z-50 bg-white dark:bg-zinc-950 border-l border-gray-200 dark:border-zinc-800 flex flex-col shadow-2xl",
} as const;

/** Tab presets. */
export const tab = {
  /** Tab container. */
  bar: "flex items-center gap-0 border-b border-gray-200 dark:border-zinc-800",
  /** Active tab. */
  active: "px-3 h-10 text-[12px] font-medium border-b-2 text-gray-900 dark:text-zinc-100 border-indigo-500",
  /** Inactive tab. */
  inactive: "px-3 h-10 text-[12px] font-medium border-b-2 border-transparent text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors",
  /** Pill tab (active). */
  pillActive: "flex items-center gap-1.5 px-2.5 py-1 rounded text-[12px] font-medium bg-gray-200 dark:bg-zinc-700 text-gray-800 dark:text-zinc-200",
  /** Pill tab (inactive). */
  pillInactive: "flex items-center gap-1.5 px-2.5 py-1 rounded text-[12px] font-medium text-gray-400 dark:text-zinc-500 hover:text-gray-700 dark:hover:text-zinc-300 transition-colors",
} as const;

/** Skeleton / loading pulse presets. */
export const skeleton = {
  /** Base pulse block. */
  base: "rounded bg-gray-100 dark:bg-zinc-800 animate-pulse",
  /** Skeleton line (narrow). */
  line: "h-2.5 rounded bg-gray-100 dark:bg-zinc-800",
  /** Skeleton block (taller). */
  block: "h-6 rounded bg-gray-100 dark:bg-zinc-800",
} as const;

// ─── Auto-refresh control preset ─────────────────────────────────────────────

export const autoRefresh = {
  /** Select dropdown for refresh interval. */
  select:
    "appearance-none rounded border border-gray-200 dark:border-zinc-700 bg-gray-50 dark:bg-zinc-900 text-[11px] font-mono pl-5 pr-2 h-7 text-gray-500 dark:text-zinc-400 focus:outline-none focus:border-indigo-500 transition-colors",
  /** Icon overlay position. */
  iconWrap: "absolute left-1.5 top-1/2 -translate-y-1/2 pointer-events-none",
} as const;

// ─── Convenience export ──────────────────────────────────────────────────────

/** Default export: all tokens under one namespace. */
export const ds = {
  surface,
  border,
  text,
  status,
  fontSize,
  fontWeight,
  spacing,
  card,
  sectionLabel,
  toolbar,
  btn,
  badge,
  input,
  table,
  modal,
  tab,
  skeleton,
  autoRefresh,
} as const;
