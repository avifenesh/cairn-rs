// ── Supported locales ─────────────────────────────────────────────────────────

export type Locale = 'en' | 'es' | 'de' | 'ja' | 'zh';

export const LOCALE_LABELS: Record<Locale, string> = {
  en: 'English',
  es: 'Español',
  de: 'Deutsch',
  ja: '日本語',
  zh: '中文',
};

// ── Translation keys ──────────────────────────────────────────────────────────

export type TranslationKey =
  // ── Navigation groups
  | 'nav.group.overview'
  | 'nav.group.operations'
  | 'nav.group.observability'
  | 'nav.group.infrastructure'
  // ── Navigation pages
  | 'nav.dashboard'
  | 'nav.sessions'
  | 'nav.runs'
  | 'nav.tasks'
  | 'nav.workers'
  | 'nav.orchestration'
  | 'nav.approvals'
  | 'nav.prompts'
  | 'nav.traces'
  | 'nav.memory'
  | 'nav.sources'
  | 'nav.costs'
  | 'nav.cost_calc'
  | 'nav.evals'
  | 'nav.graph'
  | 'nav.audit_log'
  | 'nav.logs'
  | 'nav.metrics'
  | 'nav.providers'
  | 'nav.plugins'
  | 'nav.credentials'
  | 'nav.deployment'
  | 'nav.channels'
  | 'nav.playground'
  | 'nav.api_docs'
  | 'nav.settings'
  | 'nav.account'
  | 'nav.sign_out'
  // ── Common actions
  | 'action.save'
  | 'action.cancel'
  | 'action.delete'
  | 'action.edit'
  | 'action.refresh'
  | 'action.search'
  | 'action.filter'
  | 'action.export'
  | 'action.import'
  | 'action.create'
  | 'action.close'
  | 'action.confirm'
  | 'action.back'
  | 'action.next'
  | 'action.loading'
  | 'action.retry'
  | 'action.copy'
  | 'action.copied'
  // ── Profile page section titles
  | 'profile.token_management'
  | 'profile.display_preferences'
  | 'profile.about'
  | 'profile.changelog'
  // ── Profile fields
  | 'profile.current_token'
  | 'profile.current_token_hint'
  | 'profile.change_token'
  | 'profile.theme'
  | 'profile.theme_hint'
  | 'profile.theme_dark'
  | 'profile.theme_light'
  | 'profile.theme_system'
  | 'profile.items_per_page'
  | 'profile.items_per_page_hint'
  | 'profile.date_format'
  | 'profile.date_format_hint'
  | 'profile.date_relative'
  | 'profile.date_absolute'
  | 'profile.timezone'
  | 'profile.timezone_hint'
  | 'profile.compact_mode'
  | 'profile.compact_mode_hint'
  | 'profile.auto_refresh'
  | 'profile.auto_refresh_hint'
  | 'profile.language'
  | 'profile.language_hint'
  // ── Status
  | 'status.healthy'
  | 'status.degraded'
  | 'status.loading'
  | 'status.error'
  | 'status.empty'
  | 'status.no_results'
  // ── 404
  | 'error.not_found'
  | 'error.not_found_detail'
  | 'error.go_dashboard';

// ── Translation maps ──────────────────────────────────────────────────────────

type TranslationMap = Record<TranslationKey, string>;

const en: TranslationMap = {
  // Navigation groups
  'nav.group.overview':        'Overview',
  'nav.group.operations':      'Operations',
  'nav.group.observability':   'Observability',
  'nav.group.infrastructure':  'Infrastructure',
  // Navigation pages
  'nav.dashboard':    'Dashboard',
  'nav.sessions':     'Sessions',
  'nav.runs':         'Runs',
  'nav.tasks':        'Tasks',
  'nav.workers':      'Workers',
  'nav.orchestration':'Orchestration',
  'nav.approvals':    'Approvals',
  'nav.prompts':      'Prompts',
  'nav.traces':       'Traces',
  'nav.memory':       'Memory',
  'nav.sources':      'Sources',
  'nav.costs':        'Costs',
  'nav.cost_calc':    'Calculator',
  'nav.evals':        'Evals',
  'nav.graph':        'Graph',
  'nav.audit_log':    'Audit Log',
  'nav.logs':         'Logs',
  'nav.metrics':      'Metrics',
  'nav.providers':    'Providers',
  'nav.plugins':      'Plugins',
  'nav.credentials':  'Credentials',
  'nav.deployment':   'Deployment',
  'nav.channels':     'Channels',
  'nav.playground':   'Playground',
  'nav.api_docs':     'API Docs',
  'nav.settings':     'Settings',
  'nav.account':      'Account',
  'nav.sign_out':     'Sign out',
  // Common actions
  'action.save':      'Save',
  'action.cancel':    'Cancel',
  'action.delete':    'Delete',
  'action.edit':      'Edit',
  'action.refresh':   'Refresh',
  'action.search':    'Search',
  'action.filter':    'Filter',
  'action.export':    'Export',
  'action.import':    'Import',
  'action.create':    'Create',
  'action.close':     'Close',
  'action.confirm':   'Confirm',
  'action.back':      'Back',
  'action.next':      'Next',
  'action.loading':   'Loading…',
  'action.retry':     'Retry',
  'action.copy':      'Copy',
  'action.copied':    'Copied',
  // Profile sections
  'profile.token_management':    'Token Management',
  'profile.display_preferences': 'Display Preferences',
  'profile.about':               'About',
  'profile.changelog':           'Changelog',
  // Profile fields
  'profile.current_token':       'Current token',
  'profile.current_token_hint':  'Used for all API requests',
  'profile.change_token':        'Change token',
  'profile.theme':               'Theme',
  'profile.theme_hint':          'Overrides the OS preference when set explicitly',
  'profile.theme_dark':          'Dark',
  'profile.theme_light':         'Light',
  'profile.theme_system':        'System',
  'profile.items_per_page':      'Items per page',
  'profile.items_per_page_hint': 'Rows shown in data tables',
  'profile.date_format':         'Date format',
  'profile.date_format_hint':    'How timestamps are displayed',
  'profile.date_relative':       'Relative (2m ago)',
  'profile.date_absolute':       'Absolute (2026-04-05)',
  'profile.timezone':            'Timezone',
  'profile.timezone_hint':       'For absolute timestamps; blank = browser default',
  'profile.compact_mode':        'Compact mode',
  'profile.compact_mode_hint':   'Reduces padding in tables and cards for data-dense views',
  'profile.auto_refresh':        'Auto-refresh',
  'profile.auto_refresh_hint':   'Master switch for per-page automatic data polling. When off, pages only update on manual refresh.',
  'profile.language':            'Language',
  'profile.language_hint':       'UI display language',
  // Status
  'status.healthy':    'Healthy',
  'status.degraded':   'Degraded',
  'status.loading':    'Loading…',
  'status.error':      'Error',
  'status.empty':      'No data',
  'status.no_results': 'No results',
  // 404
  'error.not_found':        'Page not found',
  'error.not_found_detail': "The page you're looking for doesn't exist or was moved.",
  'error.go_dashboard':     'Go to Dashboard',
};

// Partial stubs — only keys that differ from English need entries.
// Falls back to English for any missing key.

const es: Partial<TranslationMap> = {
  'nav.group.overview':        'Resumen',
  'nav.group.operations':      'Operaciones',
  'nav.group.observability':   'Observabilidad',
  'nav.group.infrastructure':  'Infraestructura',
  'nav.dashboard':    'Panel',
  'nav.sessions':     'Sesiones',
  'nav.runs':         'Ejecuciones',
  'nav.tasks':        'Tareas',
  'nav.workers':      'Trabajadores',
  'nav.approvals':    'Aprobaciones',
  'nav.credentials':  'Credenciales',
  'nav.settings':     'Configuración',
  'nav.account':      'Cuenta',
  'nav.sign_out':     'Cerrar sesión',
  'action.save':      'Guardar',
  'action.cancel':    'Cancelar',
  'action.delete':    'Eliminar',
  'action.edit':      'Editar',
  'action.refresh':   'Actualizar',
  'action.search':    'Buscar',
  'action.create':    'Crear',
  'action.close':     'Cerrar',
  'action.loading':   'Cargando…',
  'action.copy':      'Copiar',
  'action.copied':    'Copiado',
  'profile.language': 'Idioma',
  'status.healthy':   'Saludable',
  'status.degraded':  'Degradado',
  'status.loading':   'Cargando…',
  'status.empty':     'Sin datos',
  'error.not_found':        'Página no encontrada',
  'error.not_found_detail': 'La página que buscas no existe o fue movida.',
  'error.go_dashboard':     'Ir al panel',
};

const de: Partial<TranslationMap> = {
  'nav.group.overview':        'Übersicht',
  'nav.group.operations':      'Betrieb',
  'nav.group.observability':   'Beobachtbarkeit',
  'nav.group.infrastructure':  'Infrastruktur',
  'nav.dashboard':    'Übersicht',
  'nav.sessions':     'Sitzungen',
  'nav.runs':         'Läufe',
  'nav.tasks':        'Aufgaben',
  'nav.workers':      'Arbeiter',
  'nav.approvals':    'Genehmigungen',
  'nav.settings':     'Einstellungen',
  'nav.account':      'Konto',
  'nav.sign_out':     'Abmelden',
  'action.save':      'Speichern',
  'action.cancel':    'Abbrechen',
  'action.delete':    'Löschen',
  'action.edit':      'Bearbeiten',
  'action.refresh':   'Aktualisieren',
  'action.search':    'Suchen',
  'action.create':    'Erstellen',
  'action.close':     'Schließen',
  'action.loading':   'Laden…',
  'action.copy':      'Kopieren',
  'action.copied':    'Kopiert',
  'profile.language': 'Sprache',
  'status.healthy':   'Gesund',
  'status.degraded':  'Beeinträchtigt',
  'status.loading':   'Laden…',
  'status.empty':     'Keine Daten',
  'error.not_found':        'Seite nicht gefunden',
  'error.not_found_detail': 'Die gesuchte Seite existiert nicht oder wurde verschoben.',
  'error.go_dashboard':     'Zum Dashboard',
};

const ja: Partial<TranslationMap> = {
  'nav.group.overview':        '概要',
  'nav.group.operations':      'オペレーション',
  'nav.group.observability':   '可観測性',
  'nav.group.infrastructure':  'インフラ',
  'nav.dashboard':    'ダッシュボード',
  'nav.sessions':     'セッション',
  'nav.runs':         '実行',
  'nav.tasks':        'タスク',
  'nav.workers':      'ワーカー',
  'nav.approvals':    '承認',
  'nav.settings':     '設定',
  'nav.account':      'アカウント',
  'nav.sign_out':     'サインアウト',
  'action.save':      '保存',
  'action.cancel':    'キャンセル',
  'action.delete':    '削除',
  'action.edit':      '編集',
  'action.refresh':   '更新',
  'action.search':    '検索',
  'action.create':    '作成',
  'action.close':     '閉じる',
  'action.loading':   '読み込み中…',
  'action.copy':      'コピー',
  'action.copied':    'コピーしました',
  'profile.language': '言語',
  'status.healthy':   '正常',
  'status.degraded':  '低下',
  'status.loading':   '読み込み中…',
  'status.empty':     'データなし',
  'error.not_found':        'ページが見つかりません',
  'error.not_found_detail': 'お探しのページは存在しないか、移動されました。',
  'error.go_dashboard':     'ダッシュボードへ',
};

const zh: Partial<TranslationMap> = {
  'nav.group.overview':        '概览',
  'nav.group.operations':      '运营',
  'nav.group.observability':   '可观测性',
  'nav.group.infrastructure':  '基础设施',
  'nav.dashboard':    '仪表板',
  'nav.sessions':     '会话',
  'nav.runs':         '运行',
  'nav.tasks':        '任务',
  'nav.workers':      '工作者',
  'nav.approvals':    '审批',
  'nav.settings':     '设置',
  'nav.account':      '账户',
  'nav.sign_out':     '退出登录',
  'action.save':      '保存',
  'action.cancel':    '取消',
  'action.delete':    '删除',
  'action.edit':      '编辑',
  'action.refresh':   '刷新',
  'action.search':    '搜索',
  'action.create':    '创建',
  'action.close':     '关闭',
  'action.loading':   '加载中…',
  'action.copy':      '复制',
  'action.copied':    '已复制',
  'profile.language': '语言',
  'status.healthy':   '健康',
  'status.degraded':  '降级',
  'status.loading':   '加载中…',
  'status.empty':     '暂无数据',
  'error.not_found':        '页面未找到',
  'error.not_found_detail': '您查找的页面不存在或已移动。',
  'error.go_dashboard':     '前往仪表板',
};

// ── Registry ──────────────────────────────────────────────────────────────────

const TRANSLATIONS: Record<Locale, Partial<TranslationMap>> = { en, es, de, ja, zh };

// ── Core translate function ───────────────────────────────────────────────────

/**
 * Translate a key for a given locale.
 * Falls back to English if the key is missing in the requested locale.
 * Returns the key itself if missing from English (should never happen in prod).
 */
export function translate(key: TranslationKey, locale: Locale): string {
  return (
    TRANSLATIONS[locale][key] ??
    en[key] ??
    key
  );
}

/**
 * Build a bound translate function for a fixed locale.
 * Convenient when locale doesn't change during a render.
 */
export function makeT(locale: Locale): (key: TranslationKey) => string {
  return (key) => translate(key, locale);
}

// ── Default (English) t() ────────────────────────────────────────────────────

/** Module-level t() defaulting to English. Prefer useLocale().t in components. */
export function t(key: TranslationKey): string {
  return en[key] ?? key;
}
