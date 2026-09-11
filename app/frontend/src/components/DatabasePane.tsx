import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from 'react'
import { Check, ChevronDown, ChevronLeft, ChevronRight, ClipboardPaste, Copy, Database, Download, Edit3, Eye, EyeOff, FileCode2, Folder, GripVertical, HardDrive, KeyRound, LoaderCircle, Lock, MoreHorizontal, PanelLeftClose, PanelLeftOpen, Play, Plus, Power, RefreshCw, Search, Server, Settings2, ShieldCheck, Table2, TextSelect, Trash2, Upload, X } from 'lucide-react'
import { open as openNativeDialog, type DialogFilter } from '@tauri-apps/plugin-dialog'
import { DatabaseMenu, DatabaseResultGrid } from './DatabaseUi'
import type { DatabaseMenuItem, DatabaseResultSelection } from './DatabaseUi'
import { DatabaseFileDialog } from './DatabaseFileDialog'
import type { DatabaseFileDialogCompletion, DatabaseFileDialogFilter, DatabaseFileDialogProps } from './DatabaseFileDialog'
import { databaseInsertSql, queryResultTarget } from '../database-insert-sql'
import { fileDialogJoin } from '../file-dialog-path'
import { databaseWriteKinds } from '../database-write-kind'
import type { DatabaseWriteKind, DatabaseWriteKinds } from '../database-write-kind'
import { pasteAtCaret } from '../database-field-edit'
import { useI18n } from '../i18n'
import { databaseSqlCompletionContext, databaseSqlCompletionItems } from '../database-sql-completion'
import type { DatabaseSqlCompletionItem } from '../database-sql-completion'
import type { DatabaseColumnInfo, DatabaseDriver, DatabaseForeignKeyInfo, DatabaseIndexInfo, DatabasePaneUiRequest, DatabaseProfile, DatabaseQueryResult, DatabaseSqlExportProgress, SessionStatus } from '../types'

export interface DatabasePaneHandle { confirmClose(): Promise<boolean>; reconnect(): Promise<void> }
const DATABASE_PAGE_SIZES = [50, 100, 200] as const
// Floating notices (and the connection-test result) clear themselves after this long.
const DATABASE_NOTICE_MS = 5000

// Filter lists for file pickers; import and export stay in sync by sharing them.
const SQL_FILE_FILTERS = (t: ReturnType<typeof useI18n>['t']): DatabaseFileDialogFilter[] => [
  { label: t('fileDialog.sqlFiles'), extensions: ['sql'] },
  { label: t('fileDialog.allFiles'), extensions: [] }
]
const SQLITE_FILE_FILTERS = (t: ReturnType<typeof useI18n>['t']): DatabaseFileDialogFilter[] => [
  { label: t('fileDialog.sqliteFiles'), extensions: ['db', 'sqlite', 'sqlite3'] },
  { label: t('fileDialog.allFiles'), extensions: [] }
]

function nativeDialogFilters(filters: DatabaseFileDialogFilter[]): DialogFilter[] {
  return filters.filter((filter) => filter.extensions.length > 0).map((filter) => ({ name: filter.label, extensions: filter.extensions }))
}

// Export/import notices report file sizes, which grow past raw byte counts very quickly.
function databaseByteSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1 }
  return `${value.toFixed(1)} ${units[unit]}`
}

// Suggested dump file name: the connection or table name, minus characters no file system wants.
function databaseFileBase(name: string): string {
  return (name || 'database').replace(/[\\/:*?"<>|]/g, '-')
}

function DatabasePageSizeSelect({ value, disabled, label, onChange }: { value: number; disabled: boolean; label: string; onChange(value: number): void }): React.JSX.Element {
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)
  useEffect(() => {
    if (!open) return
    const closeOutside = (event: PointerEvent): void => { if (!root.current?.contains(event.target as Node)) setOpen(false) }
    const closeOnEscape = (event: KeyboardEvent): void => { if (event.key === 'Escape') setOpen(false) }
    window.addEventListener('pointerdown', closeOutside)
    window.addEventListener('keydown', closeOnEscape)
    return () => { window.removeEventListener('pointerdown', closeOutside); window.removeEventListener('keydown', closeOnEscape) }
  }, [open])
  return <div ref={root} className={'database-page-size-select' + (open ? ' open' : '')}>
    <button type="button" disabled={disabled} aria-label={label} aria-haspopup="listbox" aria-expanded={open} onClick={() => setOpen((current) => !current)}><span>{value}</span><ChevronDown size={13} /></button>
    {open && <div className="database-page-size-options" role="listbox" aria-label={label}>{DATABASE_PAGE_SIZES.map((size) => <button type="button" key={size} role="option" aria-selected={size === value} className={size === value ? 'selected' : ''} onClick={() => { setOpen(false); if (size !== value) onChange(size) }}><span>{size}</span>{size === value && <Check size={13} />}</button>)}</div>}
  </div>
}

interface DatabasePaneProps {
  ref?: React.Ref<DatabasePaneHandle>
  onConfirm(options: { title: string; message: string; detail?: string; kind: 'warning' | 'danger'; confirmLabel: string; grantLabel?: string; onGrant?(): void }): Promise<boolean>
  visible: boolean
  paneId?: string
  initialProfile?: DatabaseProfile
  initialProfileId?: string
  initialReadOnly?: boolean
  onProfileSelected?(profile: DatabaseProfile, readOnly: boolean): Promise<void>
  onConnectionStateChange?(status: SessionStatus, error?: string): void
  managementOnly?: boolean
  onManageConnections?(): void
  onOpen?(profile: DatabaseProfile, readOnly: boolean): Promise<void>
}

export function DatabasePane({ ref, onConfirm, visible, paneId = '', initialProfile, initialProfileId = '', initialReadOnly = true, onProfileSelected, onConnectionStateChange, managementOnly = false, onManageConnections, onOpen }: DatabasePaneProps): React.JSX.Element {
  const { t } = useI18n()
  const [profiles, setProfiles] = useState<DatabaseProfile[]>(initialProfile ? [initialProfile] : [])
  const [profileId, setProfileId] = useState(initialProfile?.id ?? initialProfileId)
  const [runtimeId, setRuntimeId] = useState('')
  const [sql, setSql] = useState('select 1')
  const [result, setResult] = useState<DatabaseQueryResult | null>(null)
  const [tables, setTables] = useState<string[]>([])
  const [selectedTable, setSelectedTable] = useState('')
  const [columns, setColumns] = useState<DatabaseColumnInfo[]>([])
  const [indexes, setIndexes] = useState<DatabaseIndexInfo[]>([])
  const [foreignKeys, setForeignKeys] = useState<DatabaseForeignKeyInfo[]>([])
  const [error, setError] = useState('')
  const [name, setName] = useState(initialProfile?.name ?? '')
  const [groupName, setGroupName] = useState(initialProfile?.groupName ?? '')
  const [path, setPath] = useState(initialProfile?.driver === 'sqlite' ? initialProfile.host : '')
  const [driver, setDriver] = useState<DatabaseDriver>(initialProfile?.driver ?? 'sqlite')
  const [host, setHost] = useState(initialProfile && initialProfile.driver !== 'sqlite' ? initialProfile.host : '127.0.0.1')
  const [port, setPort] = useState(initialProfile?.port || (initialProfile?.driver === 'postgresql' ? 5432 : 3306))
  const [username, setUsername] = useState(initialProfile?.username ?? '')
  const [password, setPassword] = useState('')
  const [passwordEdited, setPasswordEdited] = useState(false)
  const [passwordVisible, setPasswordVisible] = useState(false)
  const [credentialSaveFailed, setCredentialSaveFailed] = useState(false)
  const [databaseName, setDatabaseName] = useState(initialProfile?.databaseName ?? '')
  const [sslEnabled, setSslEnabled] = useState(initialProfile?.sslEnabled ?? false)
  const [readOnly, setReadOnly] = useState(initialReadOnly)
  const [newColumnName, setNewColumnName] = useState('')
  const [newColumnType, setNewColumnType] = useState('TEXT')
  const [newIndexName, setNewIndexName] = useState('')
  const [newIndexColumn, setNewIndexColumn] = useState('')
  const [view, setView] = useState<'overview' | 'query' | 'structure' | 'settings'>('overview')
  const [filter, setFilter] = useState('')
  const [notice, setNotice] = useState('')
  const [noticeRevision, setNoticeRevision] = useState(0)
  // Notices float over the pane instead of taking layout space, so they clear themselves;
  // repeating the same text bumps the revision and restarts the timer.
  const showNotice = useCallback((text: string): void => { setNotice(text); setNoticeRevision((value) => value + 1) }, [])
  // Write grants last until the connection is dropped; they are granted per kind (DML / DDL).
  const [writeGrants, setWriteGrants] = useState<{ dml: boolean; ddl: boolean }>({ dml: false, ddl: false })
  const [busy, setBusy] = useState(false)
  const [connectionFilter, setConnectionFilter] = useState('')
  const [tableSidebarCollapsed, setTableSidebarCollapsed] = useState(false)
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set())
  const [draggedProfile, setDraggedProfile] = useState('')
  const [profileDrop, setProfileDrop] = useState<{ id: string; position: 'before' | 'after' } | null>(null)
  const [menu, setMenu] = useState<{ x: number; y: number; anchor: HTMLElement; items: DatabaseMenuItem[] } | null>(null)
  const [fileDialog, setFileDialog] = useState<DatabaseFileDialogProps | null>(null)
  const [permissionsOpen, setPermissionsOpen] = useState(false)
  const [tableResult, setTableResult] = useState<DatabaseQueryResult | null>(null)
  const [tablePage, setTablePage] = useState(0)
  const [tablePageSize, setTablePageSize] = useState(50)
  const [queryPage, setQueryPage] = useState(0)
  const [queryPageSize, setQueryPageSize] = useState(50)
  const [executedSql, setExecutedSql] = useState('')
  const [structureLoaded, setStructureLoaded] = useState('')
  const [structureEdit, setStructureEdit] = useState<'column' | 'index' | null>(null)
  const [uniqueIndex, setUniqueIndex] = useState(false)
  const [queryTime, setQueryTime] = useState<number | null>(null)
  const [columnsByTable, setColumnsByTable] = useState<Record<string, string[]>>({})
  const [sqlCursor, setSqlCursor] = useState(sql.length)
  const [completionIndex, setCompletionIndex] = useState(0)
  const [completionDismissed, setCompletionDismissed] = useState(true)
  const [editorFocused, setEditorFocused] = useState(false)
  const rootRef = useRef<HTMLDivElement>(null)
  const permissionsRef = useRef<HTMLDivElement>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const editorRef = useRef<HTMLTextAreaElement>(null)
  const dragCleanup = useRef<() => void>(() => {})
  const closingConfirmation = useRef(false)
  const mounted = useRef(true)
  const ownedRuntime = useRef('')
  const connecting = useRef(false)
  const schemaLoads = useRef(new Set<string>())
  const pendingViewLoad = useRef<'overview' | 'structure' | null>(null)
  const actionInFlight = useRef(false)
  const initialConnectionStarted = useRef(false)
  const mountId = useRef(`db-ui-${Math.random().toString(36).slice(2)}`).current
  const controls = useRef<Record<string, () => Promise<unknown> | unknown>>({})
  const snapshotRef = useRef<() => unknown>(() => null)
  useEffect(() => {
    mounted.current = true
    return () => {
      mounted.current = false
      dragCleanup.current()
      const id = ownedRuntime.current
      ownedRuntime.current = ''
      if (id) void window.api.databaseRuntimes.disconnect(id).catch(() => {})
    }
  }, [])
  useEffect(() => {
    if (!visible) return
    let cancelled = false
    void window.api.databaseProfiles.list().then((items) => { if (!cancelled) setProfiles(items) })
      .catch((e) => { if (!cancelled) setError(String(e)) })
    return () => { cancelled = true }
  }, [visible])
  useEffect(() => {
    if (!permissionsOpen) return
    const closeOutside = (event: PointerEvent): void => { if (!permissionsRef.current?.contains(event.target as Node)) setPermissionsOpen(false) }
    const closeOnEscape = (event: KeyboardEvent): void => { if (event.key === 'Escape') { event.preventDefault(); setPermissionsOpen(false) } }
    window.addEventListener('pointerdown', closeOutside)
    window.addEventListener('keydown', closeOnEscape, true)
    return () => { window.removeEventListener('pointerdown', closeOutside); window.removeEventListener('keydown', closeOnEscape, true) }
  }, [permissionsOpen])
  // `nextReadOnly` and `restart` let the permission toggle rebuild a live runtime before
  // React commits the new state, so read-only never applies to a stale closure.
  const connect = async (nextReadOnly = readOnly, restart = false): Promise<void> => {
    const selected = profiles.find((item) => item.id === profileId)
    if (!selected || ownedRuntime.current || connecting.current) return
    if (runtimeId && !restart) return
    if (managementOnly) { await onOpen?.(selected, nextReadOnly); return }
    initialConnectionStarted.current = true
    connecting.current = true
    onConnectionStateChange?.('connecting')
    setError('')
    try {
      const currentProfiles = await window.api.databaseProfiles.list()
      if (!mounted.current) return
      setProfiles(currentProfiles)
      const profile = currentProfiles.find((item) => item.id === profileId)
      if (!profile) {
        const message = t('database.connectionRemoved')
        setError(message); onConnectionStateChange?.('error', message); return
      }
      await onProfileSelected?.(profile, nextReadOnly)
      if (!mounted.current) return
      setName(profile.name); setDriver(profile.driver); setGroupName(profile.groupName)
      setHost(profile.driver === 'sqlite' ? '127.0.0.1' : profile.host)
      setPath(profile.driver === 'sqlite' ? profile.host : '')
      setPort(profile.port); setUsername(profile.username); setDatabaseName(profile.databaseName); setSslEnabled(profile.sslEnabled)
      const runtime = await window.api.databaseRuntimes.connect({
        driver: profile.driver, profileId: profile.id,
        host: profile.driver === 'sqlite' ? undefined : profile.host, port: profile.port,
        username: profile.username, database: profile.databaseName,
        sslEnabled: profile.sslEnabled, readOnly: nextReadOnly, sqlitePath: profile.driver === 'sqlite' ? profile.host : undefined
      })
      if (!mounted.current) { await window.api.databaseRuntimes.disconnect(runtime.id); return }
      ownedRuntime.current = runtime.id
      setRuntimeId(runtime.id); setView('query')
      onConnectionStateChange?.('connected')
      const items = await window.api.databaseRuntimes.listTables(runtime.id)
      if (mounted.current && ownedRuntime.current === runtime.id) setTables(items)
    } catch (e) {
      if (mounted.current) {
        const message = String(e)
        setError(message)
        onConnectionStateChange?.(ownedRuntime.current ? 'connected' : 'error', ownedRuntime.current ? undefined : message)
      }
    }
    finally { connecting.current = false }
  }
  const confirm = (message: string, danger = true): Promise<boolean> => onConfirm({ title: t('database.title'), message, kind: danger ? 'danger' : 'warning', confirmLabel: t('database.continue') })
  const writeGranted = (kind: DatabaseWriteKind): boolean => kind === 'dml' ? writeGrants.dml : writeGrants.ddl
  const permissionSummary = readOnly
    ? t('database.permissionsSummaryReadOnly')
    : writeGrants.dml && writeGrants.ddl
      ? t('database.permissionsSummaryAllGranted')
      : writeGrants.dml
        ? t('database.permissionsSummaryDmlGranted')
        : writeGrants.ddl
          ? t('database.permissionsSummaryDdlGranted')
          : t('database.permissionsSummaryConfirmEach')
  // A granted kind skips the per-statement dialog until the connection ends.
  const approveWrite = async (kind: DatabaseWriteKind, message: string): Promise<boolean> => writeGranted(kind) || await confirm(message)
  // Read-only is a per-connection choice and is exclusive with write grants. MySQL and
  // PostgreSQL only need the frontend gate, but SQLite carries the flag into the OS-level open
  // mode, so a live SQLite runtime has to be rebuilt.
  const applyReadOnly = async (next: boolean): Promise<void> => {
    if (next === readOnly) return
    const profile = profiles.find((item) => item.id === profileId)
    setReadOnly(next)
    if (next) setWriteGrants({ dml: false, ddl: false })
    if (profile) await onProfileSelected?.(profile, next)
    const rebuild = Boolean(runtimeId) && driver === 'sqlite'
    showNotice(t(next ? rebuild ? 'database.readOnlyReconnected' : 'database.readOnlyNotice' : rebuild ? 'database.writeReconnected' : 'database.writeNotice'))
    if (!rebuild) return
    await disconnect()
    if (mounted.current) await connect(next, true)
  }
  const toggleWriteGrant = async (kind: DatabaseWriteKind): Promise<void> => {
    if (writeGranted(kind)) {
      setWriteGrants((current) => ({ ...current, [kind]: false }))
      return
    }
    if (!await confirm(kind === 'dml' ? t('database.confirmGrantDml') : t('database.confirmGrantDdl'))) return
    // A write grant only makes sense without read-only, so granting releases it. On SQLite that
    // rebuilds the runtime, and the grant is applied after the rebuild so it is not revoked by it.
    const reconnected = readOnly && Boolean(runtimeId) && driver === 'sqlite'
    if (readOnly) await applyReadOnly(false)
    setWriteGrants((current) => ({ ...current, [kind]: true }))
    showNotice(kind === 'dml'
      ? t(reconnected ? 'database.grantDmlReconnected' : 'database.grantDmlNotice')
      : t(reconnected ? 'database.grantDdlReconnected' : 'database.grantDdlNotice'))
  }
  // The dialog describes the kind of change (DML/DDL) instead of showing the SQL, and the
  // missing kinds can be granted for the rest of the connection without leaving the dialog.
  const approveStatement = async (kinds: DatabaseWriteKinds): Promise<boolean> => {
    const missing: DatabaseWriteKind[] = []
    if (kinds.dml && !writeGranted('dml')) missing.push('dml')
    if (kinds.ddl && !writeGranted('ddl')) missing.push('ddl')
    if (missing.length === 0) return true
    const both = missing.length === 2
    const ddl = missing.includes('ddl')
    let grant = false
    const accepted = await onConfirm({
      title: t('database.title'),
      message: t(both ? 'database.confirmBoth' : ddl ? 'database.confirmDdl' : 'database.confirmWrite'),
      detail: t(both ? 'database.confirmBothDetail' : ddl ? 'database.confirmDdlDetail' : 'database.confirmWriteDetail'),
      kind: 'danger',
      confirmLabel: t('database.allowOnce'),
      grantLabel: t('database.grantForSession'),
      onGrant: () => { grant = true },
    })
    if (!accepted) return false
    if (grant) {
      setWriteGrants((current) => { const next = { ...current }; for (const kind of missing) next[kind] = true; return next })
      showNotice(t(ddl ? 'database.grantDdlNotice' : 'database.grantDmlNotice'))
    }
    return true
  }
  const loadQueryPage = async (statement: string, page: number, pageSize = queryPageSize): Promise<void> => {
    const start = performance.now()
    const next = await window.api.databaseRuntimes.execute(runtimeId, statement, pageSize, page * pageSize)
    setResult(next); setQueryPage(page); setQueryTime(Math.round(performance.now() - start))
  }
  const executedWriteKinds = databaseWriteKinds(executedSql)
  const executedStatementWrites = executedWriteKinds.dml || executedWriteKinds.ddl
  // Paging re-sends the statement, so it must never silently re-run a write.
  const rerunQueryPage = (page: number, pageSize: number): Promise<void> =>
    executedStatementWrites ? Promise.resolve() : loadQueryPage(executedSql, page, pageSize)
  const execute = async (): Promise<void> => {
    if (!runtimeId) return
    setView('query')
    const editor = editorRef.current
    const statement = editor && editor.selectionStart !== editor.selectionEnd ? sql.slice(editor.selectionStart, editor.selectionEnd) : sql
    if (!statement.trim()) return
    const writeKinds = databaseWriteKinds(statement)
    if (writeKinds.dml || writeKinds.ddl) {
      if (readOnly) throw new Error(t('database.readOnly'))
      if (!await approveStatement(writeKinds)) return
    }
    setResult(null); setQueryTime(null); setQueryPage(0); setExecutedSql(statement)
    await loadQueryPage(statement, 0)
  }
  // The dump happens in the native process and streams straight to the chosen file, so a large
  // database never becomes a JavaScript string and the webview never holds it in memory.
  const defaultExportPath = async (filename: string): Promise<string> => {
    const fallback = filename
    try {
      const databaseDirectory = driver === 'sqlite' && path.trim()
        ? await window.api.files.parentLocal(path.trim())
        : ''
      const directory = databaseDirectory || await window.api.files.home()
      return fileDialogJoin(directory, filename)
    } catch {
      return fallback
    }
  }
  const exportDatabase = (table?: string): void => {
    if (!runtimeId) return
    const defaultFileName = `${databaseFileBase(table || activeProfile?.name || 'database')}.sql`
    void defaultExportPath(defaultFileName).then((defaultPath) => {
      if (!mounted.current || !runtimeId) return
      setFileDialog({
        mode: 'save',
        title: t('database.exportSqlFileTitle'),
        defaultPath,
        defaultFileName,
        filters: SQL_FILE_FILTERS(t),
        showSchemaOnly: true,
        onCancel: () => setFileDialog(null),
        onConfirm: async (path, schemaOnly, onProgress): Promise<DatabaseFileDialogCompletion> => {
          const summary = await window.api.databaseRuntimes.exportSql(runtimeId, path, table, schemaOnly, onProgress)
          const details = [
            ...(!table ? [{ label: t('database.exportTables'), value: String(summary.tables) }] : []),
            { label: t('database.exportRows'), value: String(summary.rows) },
            { label: t('database.exportSize'), value: databaseByteSize(summary.bytes) },
            { label: t('database.exportPath'), value: path },
            ...(schemaOnly ? [{ label: t('database.exportMode'), value: t('database.schemaOnlyExport') }] : [])
          ]
          showNotice(table
            ? t('database.exportedTable', { table, rows: summary.rows, size: databaseByteSize(summary.bytes), path })
            : t('database.exported', { tables: summary.tables, rows: summary.rows, size: databaseByteSize(summary.bytes), path }))
          return { title: t('database.exportCompleted'), details }
        }
      })
    })
  }
  // Export the visible result snapshot as INSERT statements. The target comes from the executed
  // query when possible, and otherwise uses a neutral name independent of the left table list.
  const exportResultSql = (): void => {
    const value = view === 'overview' ? tableResult : result
    if (!value?.columns.length || !value.rows.length) return
    const targetTable = queryResultTarget(executedSql || sql)
    const text = databaseInsertSql(targetTable, value.columns, value.rows, driver)
    if (!text) return
    const defaultFileName = `${databaseFileBase(targetTable)}-result.sql`
    void defaultExportPath(defaultFileName).then((defaultPath) => {
      if (!mounted.current) return
      setFileDialog({
        mode: 'save',
        title: t('database.exportResultSqlFileTitle'),
        defaultPath,
        defaultFileName,
        filters: SQL_FILE_FILTERS(t),
        onCancel: () => setFileDialog(null),
        onConfirm: async (path, _schemaOnly, onProgress): Promise<DatabaseFileDialogCompletion> => {
          const summary = await window.api.databaseRuntimes.writeQuerySql(path, text, value.rows.length, onProgress)
          showNotice(t('database.exportedQuery', { rows: summary.rows, size: databaseByteSize(summary.bytes), path }))
          return {
            title: t('database.exportQueryCompleted'),
            details: [
              { label: t('database.exportRows'), value: String(summary.rows) },
              { label: t('database.exportSize'), value: databaseByteSize(summary.bytes) },
              { label: t('database.exportPath'), value: path },
              { label: t('database.exportTarget'), value: targetTable }
            ]
          }
        }
      })
    })
  }
  const copyCell = useCallback((value: string): void => { void window.api.system.writeClipboard(value).then(() => showNotice(t('database.copied'))).catch((e) => setError(String(e))) }, [t, showNotice])
  // The grid only knows values, so the INSERT target comes from the selected table and
  // is spelled out in the menu label.
  const copyRowsAsInsert = (selection: DatabaseResultSelection): void => {
    if (!selectedTable || !selection.rows.length) return
    void window.api.system.writeClipboard(databaseInsertSql(selectedTable, selection.columns, selection.rows, driver))
      .then(() => showNotice(t('database.copiedInsert', { count: selection.rows.length })))
      .catch((e) => setError(String(e)))
  }
  const resultMenu = (selection: DatabaseResultSelection): DatabaseMenuItem[] => [
    ...(selection.cell ? [{ label: t('database.copyCell'), icon: <Copy size={14} />, action: () => copyCell(selection.cell) }] : []),
    { label: selectedTable ? t('database.copyInsertRows', { count: selection.rows.length, table: selectedTable }) : t('database.copyInsertNoTable'), icon: <FileCode2 size={14} />, disabled: !selectedTable, action: () => copyRowsAsInsert(selection) },
  ]
  // A SQL script may contain both data and schema changes, so it needs both grants before it can
  // run unattended.
  const importDatabase = (): void => {
    if (!runtimeId) return
    if (readOnly) { setError(t('database.readOnly')); return }
    setFileDialog({
      mode: 'open',
      title: t('database.importSqlFileTitle'),
      filters: SQL_FILE_FILTERS(t),
      onCancel: () => setFileDialog(null),
      onConfirm: (path) => {
        setFileDialog(null)
        void runAction(async () => {
          if (!(writeGranted('dml') && writeGranted('ddl'))) {
            const accepted = await onConfirm({
              title: t('database.title'),
              message: t('database.confirmImport'),
              detail: t('database.confirmImportDetail'),
              kind: 'danger',
              confirmLabel: t('database.continue'),
            })
            if (!accepted) return
          }
          await window.api.databaseRuntimes.importSqlFile(runtimeId, path)
          setTables(await window.api.databaseRuntimes.listTables(runtimeId))
          showNotice(t('database.imported', { path }))
        })
      }
    })
  }
  const chooseSqlitePath = async (): Promise<void> => {
    try {
      const selected = await openNativeDialog({ title: t('database.chooseDatabaseFile'), defaultPath: path || undefined, filters: nativeDialogFilters(SQLITE_FILE_FILTERS(t)), multiple: false, directory: false })
      if (typeof selected === 'string') setPath(selected)
    } catch (caught) {
      setError(String(caught))
    }
  }
  const inspectTable = async (table: string): Promise<void> => {
    if (!runtimeId || !table) return
    setColumns([]); setIndexes([]); setForeignKeys([]); setStructureLoaded('')
    const info = await window.api.databaseRuntimes.describeTable(runtimeId, table)
    const indexInfo = driver !== 'postgresql' ? await window.api.databaseRuntimes.listIndexes(runtimeId, table) : []
    const keys = driver === 'sqlite' ? await window.api.databaseRuntimes.listForeignKeys(runtimeId, table) : []
    setColumns(info); setIndexes(indexInfo); setForeignKeys(keys); setStructureLoaded(table)
    setColumnsByTable((current) => ({ ...current, [table]: info.map((column) => column.name) }))
  }
  const addColumn = async (): Promise<void> => {
    if (!runtimeId || !selectedTable || readOnly || driver !== 'sqlite' || !newColumnName.trim() || !newColumnType.trim()) return
    if (!await approveWrite('ddl', t('database.confirmAddColumn'))) return
    await window.api.databaseRuntimes.addColumn(runtimeId, selectedTable, newColumnName.trim(), newColumnType.trim())
    setNewColumnName(''); setStructureEdit(null); await inspectTable(selectedTable)
  }
  const dropColumn = async (column: string): Promise<void> => {
    if (readOnly || driver !== 'sqlite' || !await approveWrite('ddl', t('database.confirmDropColumn', { name: column }))) return
    await window.api.databaseRuntimes.dropColumn(runtimeId, selectedTable, column); await inspectTable(selectedTable)
  }
  const addIndex = async (): Promise<void> => {
    if (!runtimeId || !selectedTable || readOnly || driver !== 'sqlite' || !newIndexName.trim() || !newIndexColumn.trim()) return
    if (!await approveWrite('ddl', t('database.confirmAddIndex'))) return
    await window.api.databaseRuntimes.createIndex(runtimeId, selectedTable, newIndexName.trim(), newIndexColumn.trim(), uniqueIndex)
    setNewIndexName(''); setNewIndexColumn(''); setStructureEdit(null); await inspectTable(selectedTable)
  }
  const saveProfile = async (): Promise<DatabaseProfile | undefined> => {
    if (!validEndpoint || !name.trim()) return
    const needsCredentialUpdate = driver !== 'sqlite' && (passwordEdited || !canReuseCredential)
    const needsCredentialRemoval = Boolean(activeProfile?.hasSavedCredential) && (driver === 'sqlite' || (needsCredentialUpdate && !password))
    // Remove a secret bound to the old destination before publishing new settings.
    // If removal fails, leave the old profile intact and surface the error.
    const removeBeforeSave = Boolean(activeProfile?.hasSavedCredential) && (!canReuseCredential || needsCredentialRemoval)
    if (removeBeforeSave && activeProfile) {
      await window.api.databaseProfiles.forgetCredential(activeProfile.id)
      setProfiles((items) => items.map((item) => item.id === activeProfile.id ? { ...item, hasSavedCredential: false } : item))
    }
    let saved = await window.api.databaseProfiles.save({
      ...activeProfile, id: profileId || undefined, name: name.trim(), groupName: groupName.trim(), driver,
      host: driver === 'sqlite' ? path.trim() : host.trim(), port: driver === 'sqlite' ? 0 : port,
      username: username.trim(), databaseName: databaseName.trim(), sslEnabled
    })
    // Retain the assigned ID if the OS credential store fails, so retry updates this record.
    setProfileId(saved.id)
    setProfiles((items) => items.some((item) => item.id === saved.id) ? items.map((item) => item.id === saved.id ? saved : item) : [...items, saved])
    try {
      if (needsCredentialUpdate && password) {
        await window.api.databaseProfiles.saveCredential(saved.id, password)
        saved = { ...saved, hasSavedCredential: true }
      } else if (needsCredentialRemoval && !removeBeforeSave) {
        await window.api.databaseProfiles.forgetCredential(saved.id)
        saved = { ...saved, hasSavedCredential: false }
      }
    } catch {
      setCredentialSaveFailed(true)
      // Keep the pending password (including explicit empty) for a deliberate retry.
      setPasswordEdited(true)
      throw new Error(t('database.passwordSaveFailed'))
    }
    setProfiles((items) => items.map((item) => item.id === saved.id ? saved : item))
    setName(saved.name); setGroupName(saved.groupName); setUsername(saved.username); setDatabaseName(saved.databaseName)
    setPath(saved.driver === 'sqlite' ? saved.host : ''); setHost(saved.driver === 'sqlite' ? '127.0.0.1' : saved.host)
    setPassword(''); setPasswordEdited(false); setPasswordVisible(false); setCredentialSaveFailed(false)
    showNotice(t('database.saved'))
    return saved
  }
  const testConnection = async (): Promise<void> => {
    if (!validEndpoint || runtimeId) return
    await window.api.databaseConnectionTest({
      driver, profileId: canReuseCredential && !passwordEdited ? profileId : undefined,
      host: driver === 'sqlite' ? undefined : host.trim(), port,
      username: username.trim(), password: driver === 'sqlite' ? undefined : canReuseCredential && !passwordEdited ? undefined : password,
      database: databaseName.trim(), sslEnabled, readOnly: true,
      sqlitePath: driver === 'sqlite' ? path.trim() : undefined
    })
    if (mounted.current) showNotice(t('database.testSuccess'))
  }
  const uiSnapshot = (): unknown => ({ paneId, mountId, controls: Object.keys(controls.current).map((ref) => ({ ref, enabled: true, value: ref === 'sql' ? sql : ref === 'selectedTable' ? selectedTable : undefined })), tables, selectedTable, runtimeConnected: Boolean(runtimeId), result: view === 'overview' ? tableResult : view === 'query' ? result : null, error: error || null })
  snapshotRef.current = uiSnapshot
  useEffect(() => {
    if (!notice) return
    const timer = setTimeout(() => setNotice(''), DATABASE_NOTICE_MS)
    return () => clearTimeout(timer)
  }, [notice, noticeRevision])
  const pageControls: Record<string, () => Promise<void>> = {}
  // Paging re-sends the query, so it stays unavailable for write statements the
  // same way the on-screen buttons are disabled.
  if (!busy && runtimeId && (view === 'overview' || view === 'query') && !(view === 'query' && executedStatementWrites)) {
    const page = view === 'overview' ? tablePage : queryPage
    const hasMore = view === 'overview' ? tableResult?.hasMore : result?.hasMore
    const loadPage = (nextPage: number): Promise<void> => runAction(() => view === 'overview' ? loadTablePage(selectedTable, nextPage, tablePageSize) : rerunQueryPage(nextPage, queryPageSize), true)
    if (page > 0) pageControls.previousPage = () => loadPage(page - 1)
    if (hasMore) pageControls.nextPage = () => loadPage(page + 1)
  }
  const profileControls = Object.fromEntries(profiles.map((profile) => ['profile:' + profile.id, () => { if (!runtimeId) return pickProfile(profile.id) }]))
  const tableControls = Object.fromEntries(tables.map((table) => ['table:' + table, () => runAction(() => browseTable(table), true)]))
  controls.current = { sql: () => setSql, selectedTable: () => selectedTable, name: () => setName, path: () => setPath, host: () => setHost, port: () => setPort, username: () => setUsername, databaseName: () => setDatabaseName, driver: () => setDriver, sslEnabled: () => setSslEnabled, ...(runtimeId ? {} : { readOnly: () => runAction(() => applyReadOnly(true)) }), newColumnName: () => setNewColumnName, newColumnType: () => setNewColumnType, newIndexName: () => setNewIndexName, newIndexColumn: () => setNewIndexColumn, execute: () => runAction(execute, true), connect: () => connect(), disconnect: () => disconnect(), testConnection: () => testConnection(), saveProfile: () => saveProfile(), addColumn: () => runAction(addColumn, true), addIndex: () => runAction(addIndex, true), viewOverview: () => { if (runtimeId && selectedTable) return runAction(() => browseTable(selectedTable), true) }, viewQuery: () => { if (runtimeId) setView('query') }, viewStructure: () => { if (runtimeId && selectedTable) return runAction(showStructure, true) }, viewSettings: () => onManageConnections?.(), ...profileControls, ...tableControls, ...pageControls }
  useEffect(() => { if (!paneId || managementOnly) return; void window.api.databasePaneUi.mount(paneId, mountId, true); const stop = window.api.databasePaneUi.onRequest((request: DatabasePaneUiRequest) => { if (request.paneId !== paneId || request.mountId !== mountId) return; void (async () => { try { if (request.action.action === 'snapshot') await window.api.databasePaneUi.respond(request.requestId, paneId, mountId, { value: snapshotRef.current() }); else if (request.action.action === 'fill') { const ref = request.action.ref; const value = request.action.value; if (ref === 'sql') setSql(value); else if (ref === 'selectedTable') { if (actionInFlight.current || connecting.current) throw new Error(t('database.working')); setSelectedTable(value); setTableResult(null); setTablePage(0); setStructureLoaded(''); setColumns([]); setIndexes([]); setForeignKeys([]) } else if (ref === 'name') setName(value); else if (ref === 'path') setPath(value); else if (ref === 'host') setHost(value); else if (ref === 'port') setPort(Number(value)); else if (ref === 'username') setUsername(value); else if (ref === 'databaseName') setDatabaseName(value); else if (ref === 'driver') setDriver(value as DatabaseDriver); else if (ref === 'sslEnabled') setSslEnabled(value === 'true'); else if (ref === 'readOnly') { if (runtimeId) throw new Error(t('database.readOnlyLocked')); await applyReadOnly(value === 'true') } else if (ref === 'newColumnName') setNewColumnName(value); else if (ref === 'newColumnType') setNewColumnType(value); else if (ref === 'newIndexName') setNewIndexName(value); else if (ref === 'newIndexColumn') setNewIndexColumn(value); else throw new Error('该控件不支持填写'); await window.api.databasePaneUi.respond(request.requestId, paneId, mountId, { value: { accepted: true, ref } }) } else { const action = controls.current[request.action.ref]; if (!action) throw new Error('未知或禁用的数据库窗格控件'); const value = await action(); await window.api.databasePaneUi.respond(request.requestId, paneId, mountId, { value: { accepted: true, ref: request.action.ref, result: value ?? null } }) } } catch (e) { await window.api.databasePaneUi.respond(request.requestId, paneId, mountId, { error: String(e) }) } })() }); return () => { stop(); void window.api.databasePaneUi.mount(paneId, mountId, false) } }, [paneId, mountId, managementOnly])
  const selectProfile = (id: string): void => { setPassword(''); setPasswordEdited(false); setPasswordVisible(false); setCredentialSaveFailed(false); setProfileId(id); const p = profiles.find((item) => item.id === id); if (!p) return; setName(p.name); setGroupName(p.groupName); setPath(p.driver === 'sqlite' ? p.host : ''); setDriver(p.driver); setHost(p.driver === 'sqlite' ? '127.0.0.1' : p.host); setPort(p.port || (p.driver === 'postgresql' ? 5432 : 3306)); setUsername(p.username); setDatabaseName(p.databaseName); setSslEnabled(p.sslEnabled) }
  // A write grant belongs to one connection, so it lives until the connection is dropped.
  const disconnect = async (): Promise<void> => { if (runtimeId) await window.api.databaseRuntimes.disconnect(runtimeId); ownedRuntime.current = ''; setRuntimeId(''); setPermissionsOpen(false); setWriteGrants({ dml: false, ddl: false }); setTables([]); setColumnsByTable({}); setResult(null); setTableResult(null); setTablePage(0); setStructureLoaded(''); setSelectedTable(''); setTableSidebarCollapsed(false); setView('overview'); onConnectionStateChange?.('disconnected') }
  const activeProfile = profiles.find((profile) => profile.id === profileId)
  const driverLabels: Record<DatabaseDriver, string> = { sqlite: 'SQLite', mysql: 'MySQL / MariaDB', postgresql: 'PostgreSQL' }
  const validEndpoint = driver === 'sqlite' ? Boolean(path.trim()) : Boolean(host.trim()) && Number.isInteger(port) && port >= 1 && port <= 65535
  const canReuseCredential = Boolean(activeProfile?.hasSavedCredential && activeProfile.driver === driver
    && activeProfile.host === host.trim() && activeProfile.port === port && activeProfile.username === username.trim()
    && activeProfile.databaseName === databaseName.trim() && activeProfile.sslEnabled === sslEnabled && !credentialSaveFailed)
  const isDirty = passwordEdited || credentialSaveFailed || !activeProfile || activeProfile.name !== name || activeProfile.groupName !== groupName || activeProfile.driver !== driver
    || activeProfile.host !== (driver === 'sqlite' ? path : host)
    || (driver !== 'sqlite' && (activeProfile.port !== port || activeProfile.username !== username || activeProfile.databaseName !== databaseName || activeProfile.sslEnabled !== sslEnabled))
  const runAction = async (action: () => Promise<unknown>, reportError = false): Promise<void> => {
    if (actionInFlight.current) {
      if (reportError) throw new Error(t('database.working'))
      return
    }
    actionInFlight.current = true
    setBusy(true)
    setError('')
    showNotice('')
    try { await action() } catch (e) { if (mounted.current) setError(String(e)); if (reportError) throw e } finally { actionInFlight.current = false; if (mounted.current) setBusy(false) }
  }
  const newProfile = (): void => {
    setPassword(''); setPasswordEdited(false); setPasswordVisible(false); setCredentialSaveFailed(false)
    setProfileId(''); setName(''); setGroupName(''); setPath(''); setDriver('sqlite'); setHost('127.0.0.1'); setPort(3306)
    setUsername(''); setDatabaseName(''); setSslEnabled(false); setReadOnly(true); setError(''); showNotice('')
  }
  const settingsVisible = managementOnly
  const hasUnsavedChanges = managementOnly && (activeProfile ? isDirty : Boolean(name || groupName || path || passwordEdited || username || databaseName || sslEnabled || driver !== 'sqlite' || host !== '127.0.0.1'))
  const confirmDiscard = async (): Promise<boolean> => {
    if (busy || closingConfirmation.current) return false
    if (!hasUnsavedChanges) return true
    closingConfirmation.current = true
    try { return await onConfirm({ title: t('database.unsavedTitle'), message: t('database.unsavedMessage'), kind: 'warning', confirmLabel: t('database.discard') }) }
    finally { closingConfirmation.current = false }
  }
  useImperativeHandle(ref, () => ({ confirmClose: confirmDiscard, reconnect: () => runAction(connect) }))
  const pickProfile = async (id: string): Promise<void> => {
    if (id === profileId || !await confirmDiscard()) return
    selectProfile(id); setError(''); showNotice('')
  }
  const startNew = async (): Promise<void> => { if (await confirmDiscard()) newProfile() }
  const saveAndConnect = async (): Promise<void> => {
    const profile = isDirty ? await saveProfile() : activeProfile
    if (profile) await onOpen?.(profile, readOnly)
  }
  const loadTablePage = async (table: string, page: number, pageSize = tablePageSize): Promise<void> => {
    if (!runtimeId) return
    const quote = driver === 'mysql' ? '\u0060' : '"'
    const statement = 'SELECT * FROM ' + quote + table.replaceAll(quote, quote + quote) + quote
    const next = await window.api.databaseRuntimes.execute(runtimeId, statement, pageSize, page * pageSize)
    setTableResult(next); setTablePage(page)
  }
  const browseTable = async (table: string): Promise<void> => {
    if (!runtimeId) return
    setSelectedTable(table); setView('overview'); setTableResult(null); setTablePage(0); setStructureEdit(null)
    await loadTablePage(table, 0)
  }
  const showStructure = async (): Promise<void> => {
    setView('structure'); setStructureEdit(null)
    if (selectedTable && structureLoaded !== selectedTable) await inspectTable(selectedTable)
  }
  const refreshTables = async (): Promise<void> => {
    const items = await window.api.databaseRuntimes.listTables(runtimeId); setTables(items)
    if (selectedTable && !items.includes(selectedTable)) { setSelectedTable(''); setTableResult(null); setTablePage(0); setColumns([]); setIndexes([]); setForeignKeys([]); setStructureLoaded(''); setView('query') }
  }
  const changeTablePageSize = (pageSize: number): void => {
    if (!selectedTable) { setTablePageSize(pageSize); setTablePage(0); return }
    void runAction(async () => { await loadTablePage(selectedTable, 0, pageSize); setTablePageSize(pageSize) })
  }
  const changeQueryPageSize = (pageSize: number): void => {
    if (!executedSql || !result?.columns.length) { setQueryPageSize(pageSize); setQueryPage(0); return }
    void runAction(async () => { await rerunQueryPage(0, pageSize); setQueryPageSize(pageSize) })
  }

  const completionContext = useMemo(() => databaseSqlCompletionContext(sql, sqlCursor, tables, selectedTable), [sql, sqlCursor, tables, selectedTable])
  const completions = useMemo(
    () => completionDismissed || !editorFocused ? [] : databaseSqlCompletionItems(completionContext, tables, columnsByTable),
    [completionContext, tables, columnsByTable, completionDismissed, editorFocused]
  )
  const applyCompletion = (item: DatabaseSqlCompletionItem): void => {
    const nextSql = sql.slice(0, item.start) + item.label + sql.slice(item.end)
    const nextCursor = item.start + item.label.length
    setSql(nextSql); setSqlCursor(nextCursor); setCompletionDismissed(true)
    requestAnimationFrame(() => { editorRef.current?.focus(); editorRef.current?.setSelectionRange(nextCursor, nextCursor) })
  }

  useEffect(() => {
    schemaLoads.current.clear()
    setColumnsByTable({})
  }, [runtimeId])

  useEffect(() => {
    const table = completionContext?.table
    if (!runtimeId || !table || Object.hasOwn(columnsByTable, table) || schemaLoads.current.has(table)) return
    const requestedRuntime = runtimeId
    schemaLoads.current.add(table)
    void window.api.databaseRuntimes.describeTable(runtimeId, table).then((info) => {
      if (mounted.current && ownedRuntime.current === requestedRuntime) setColumnsByTable((current) => ({ ...current, [table]: info.map((column) => column.name) }))
    }).catch(() => undefined).finally(() => { schemaLoads.current.delete(table) })
  }, [runtimeId, completionContext?.table, columnsByTable])

  useEffect(() => {
    if (!runtimeId || busy || !selectedTable || pendingViewLoad.current !== view) return
    pendingViewLoad.current = null
    if (view === 'overview' && !tableResult) void runAction(() => loadTablePage(selectedTable, 0))
    if (view === 'structure' && structureLoaded !== selectedTable) void runAction(() => inspectTable(selectedTable))
  }, [view, busy, runtimeId, selectedTable, tableResult, structureLoaded])

  useEffect(() => {
    if (!visible || managementOnly || !(initialProfile || initialProfileId) || initialConnectionStarted.current || profileId !== (initialProfile?.id ?? initialProfileId) || !profiles.some((profile) => profile.id === profileId)) return
    initialConnectionStarted.current = true
    void runAction(connect)
  }, [visible, managementOnly, initialProfile, initialProfileId, profiles, profileId])

  const filteredProfiles = profiles.filter((profile) => [profile.name, profile.host, profile.groupName, driverLabels[profile.driver]].some((value) => value.toLowerCase().includes(connectionFilter.toLowerCase())))
  const groups = Array.from(new Set(profiles.map((profile) => profile.groupName)))
  const reorderProfile = async (id: string, targetId: string, position: 'before' | 'after'): Promise<void> => {
    const source = profiles.find((profile) => profile.id === id)
    const target = profiles.find((profile) => profile.id === targetId)
    if (!source || !target || id === targetId || source.groupName !== target.groupName) return
    const previous = profiles
    const group = profiles.filter((profile) => profile.groupName === source.groupName && profile.id !== id)
    const index = group.findIndex((profile) => profile.id === targetId)
    group.splice(index + (position === 'after' ? 1 : 0), 0, source)
    let next = 0
    const reordered = profiles.map((profile) => profile.groupName === source.groupName ? group[next++]! : profile)
    setProfiles(reordered)
    try { setProfiles(await window.api.databaseProfiles.reorder(reordered.map((profile) => profile.id))) }
    catch (e) { setProfiles(previous); throw e }
  }
  const startProfileDrag = (event: React.PointerEvent, id: string): void => {
    if (event.button !== 0 || busy || connectionFilter) return
    event.preventDefault(); event.stopPropagation(); dragCleanup.current()
    const source = profiles.find((profile) => profile.id === id)
    const startX = event.clientX, startY = event.clientY, pointerId = event.pointerId
    let active = false, drop: { id: string; position: 'before' | 'after' } | null = null
    const move = (e: PointerEvent): void => {
      if (e.pointerId !== pointerId) return
      if (!active && Math.hypot(e.clientX - startX, e.clientY - startY) < 5) return
      active = true; setDraggedProfile(id); e.preventDefault()
      const row = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>('[data-database-profile]')
      const target = profiles.find((profile) => profile.id === row?.dataset.databaseProfile)
      drop = row && listRef.current?.contains(row) && target && target.id !== id && target.groupName === source?.groupName
        ? { id: target.id, position: e.clientY < row.getBoundingClientRect().top + row.getBoundingClientRect().height / 2 ? 'before' : 'after' } : null
      setProfileDrop(drop)
      const list = listRef.current; if (list) { const box = list.getBoundingClientRect(); if (e.clientY < box.top + 24) list.scrollTop -= 12; else if (e.clientY > box.bottom - 24) list.scrollTop += 12 }
    }
    const cleanup = (): void => {
      window.removeEventListener('pointermove', move); window.removeEventListener('pointerup', up); window.removeEventListener('pointercancel', cancel); window.removeEventListener('keydown', key); window.removeEventListener('blur', cancel)
      setDraggedProfile(''); setProfileDrop(null); dragCleanup.current = () => {}
    }
    const up = (e: PointerEvent): void => { if (e.pointerId !== pointerId) return; cleanup(); if (active && drop) void runAction(() => reorderProfile(id, drop!.id, drop!.position)) }
    const cancel = (): void => cleanup()
    const key = (e: KeyboardEvent): void => { if (e.key === 'Escape') { e.preventDefault(); cleanup() } }
    dragCleanup.current = cleanup
    window.addEventListener('pointermove', move, { passive: false }); window.addEventListener('pointerup', up); window.addEventListener('pointercancel', cancel); window.addEventListener('keydown', key); window.addEventListener('blur', cancel)
  }
  const openMenu = (anchor: HTMLElement, items: DatabaseMenuItem[], x?: number, y?: number): void => {
    if (busy) return
    const bounds = anchor.getBoundingClientRect(); setMenu({ x: x ?? bounds.right - 220, y: y ?? bounds.bottom + 4, anchor, items })
  }
  const profileMenu = (profile: DatabaseProfile): DatabaseMenuItem[] => [
    { label: t('common.editConnection'), icon: <Edit3 size={14} />, action: () => { void pickProfile(profile.id) } },
    { label: t('app.duplicate'), icon: <Copy size={14} />, action: () => { void (async () => { if (await confirmDiscard()) { selectProfile(profile.id); setProfileId(''); setName(t('database.copyName', { name: profile.name })); showNotice(t('database.copyPasswordHint')) } })() } },
    { label: t('database.deleteConnection'), icon: <Trash2 size={14} />, danger: true, action: () => { void runAction(async () => {
      if (!await onConfirm({ title: t('database.deleteConnection'), message: profile.name, detail: t('database.confirmDelete'), kind: 'danger', confirmLabel: t('database.deleteConnection') })) return
      await window.api.databaseProfiles.remove(profile.id); setProfiles((items) => items.filter((item) => item.id !== profile.id)); if (profile.id === profileId) newProfile()
    }) } }
  ]
  // Import and export both run natively against a chosen file path, so there is no "which table"
  // question to answer; the schema in the file decides what a dump restores.
  // A single table dump carries its own schema, so it restores on its own.
  const tableMenu = (table: string): DatabaseMenuItem[] => [
    { label: t('database.exportTable'), icon: <FileCode2 size={14} />, disabled: !runtimeId, action: () => exportDatabase(table) }
  ]

  // Right-click on a text field opens the app's own menu; the WebView menu is
  // suppressed globally so no browser or platform menu can appear here.
  const pasteIntoField = async (field: HTMLInputElement | HTMLTextAreaElement, start: number, end: number): Promise<void> => {
    try {
      const content = await window.api.system.readClipboard()
      if (content.type !== 'text') return
      // The offsets were captured when the menu opened, before focus moved.
      const edit = pasteAtCaret(field.value, start, end, content.text)
      // React owns the field value, so a direct write is ignored: the prototype
      // setter plus an input event is what reaches the component state.
      const prototype = field instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype
      Object.getOwnPropertyDescriptor(prototype, 'value')?.set?.call(field, edit.value)
      field.dispatchEvent(new Event('input', { bubbles: true }))
      field.focus()
      field.setSelectionRange(edit.caret, edit.caret)
    } catch (e) { setError(String(e)) }
  }
  const fieldMenu = (field: HTMLInputElement | HTMLTextAreaElement): DatabaseMenuItem[] => {
    const start = field.selectionStart ?? field.value.length
    const end = field.selectionEnd ?? start
    const selected = field.value.slice(Math.min(start, end), Math.max(start, end))
    const items: DatabaseMenuItem[] = []
    if (field === editorRef.current) items.push({ label: t('database.runSelection'), icon: <Play size={14} />, disabled: !runtimeId || !selected, action: () => { void runAction(execute, true) } })
    items.push({ label: t('common.copy'), icon: <Copy size={14} />, disabled: !selected, action: () => { void window.api.system.writeClipboard(selected).catch((e) => setError(String(e))) } })
    items.push({ label: t('common.paste'), icon: <ClipboardPaste size={14} />, action: () => { void pasteIntoField(field, start, end) } })
    items.push({ label: t('common.selectAll'), icon: <TextSelect size={14} />, action: () => { field.focus(); field.select() } })
    return items
  }

  const formId = 'database-form-' + mountId
  const feedback = (error || notice) && <div className={error ? 'database-feedback error' : 'database-feedback'} role={error ? 'alert' : 'status'}><span>{error || notice}</span><button type="button" className="icon-button" aria-label={t('common.close')} onClick={() => { setError(''); showNotice('') }}><X size={13} /></button></div>
  return <div ref={rootRef} className={'database-pane' + (managementOnly ? ' database-manager' : '')} aria-label={t('database.title')} aria-busy={busy} onContextMenu={(event) => {
    const field = (event.target as HTMLElement).closest<HTMLInputElement | HTMLTextAreaElement>('input, textarea')
    // Number, date, and checkbox inputs report no selection, so they get no menu.
    if (!field || field.selectionStart === null) return
    event.preventDefault(); event.stopPropagation()
    openMenu(field, fieldMenu(field), event.clientX, event.clientY)
  }}>
    {!managementOnly && <header className="database-toolbar">
      <div className="database-toolbar-title"><Database size={15} /><span title={activeProfile?.host}>{activeProfile ? driverLabels[activeProfile.driver] + ' · ' + activeProfile.host : t('database.title')}</span></div>
      <div className="database-toolbar-actions database-connection-actions">
        <button className="icon-button database-manage-connections" aria-label={t('database.manageConnections')} title={t('database.manageConnections')} disabled={busy} onClick={onManageConnections}><Settings2 size={15} /></button>
        <span className="database-toolbar-divider" aria-hidden="true" />
        <div ref={permissionsRef} className="database-grants">
          <button type="button" className={'database-permissions-trigger' + (readOnly || writeGrants.dml || writeGrants.ddl ? ' active' : '')} aria-haspopup="menu" aria-expanded={permissionsOpen} aria-label={t('database.permissions')} title={permissionSummary} disabled={busy} onClick={() => setPermissionsOpen((open) => !open)}><ShieldCheck size={14} /><span>{t('database.permissions')}</span><ChevronDown size={13} /></button>
          {permissionsOpen && <div className="database-permissions-menu" role="menu" aria-label={t('database.permissions')}>
            <div className="database-permissions-summary" role="status">{permissionSummary}</div>
            <button type="button" role="menuitemcheckbox" aria-checked={readOnly} className={readOnly ? 'database-permission-option active' : 'database-permission-option'} aria-label={t(readOnly ? 'database.readOnlyOn' : 'database.readOnlyOff')} title={t(readOnly ? 'database.readOnlyOn' : 'database.readOnlyOff')} disabled={busy} onClick={() => void runAction(() => applyReadOnly(!readOnly))}><Lock size={14} /><span>{t('database.readOnlyShort')}</span>{readOnly && <Check size={14} />}</button>
            <button type="button" role="menuitemcheckbox" aria-checked={writeGrants.dml} className={writeGrants.dml ? 'database-permission-option active' : 'database-permission-option'} aria-label={t(writeGrants.dml ? 'database.grantDmlActive' : 'database.grantDml')} title={t(writeGrants.dml ? 'database.grantDmlActive' : 'database.grantDml')} disabled={busy} onClick={() => void runAction(() => toggleWriteGrant('dml'))}><ShieldCheck size={14} /><span>{t('database.grantDmlShort')}</span>{writeGrants.dml && <Check size={14} />}</button>
            <button type="button" role="menuitemcheckbox" aria-checked={writeGrants.ddl} className={writeGrants.ddl ? 'database-permission-option active' : 'database-permission-option'} aria-label={t(writeGrants.ddl ? 'database.grantDdlActive' : 'database.grantDdl')} title={t(writeGrants.ddl ? 'database.grantDdlActive' : 'database.grantDdl')} disabled={busy} onClick={() => void runAction(() => toggleWriteGrant('ddl'))}><ShieldCheck size={14} /><span>{t('database.grantDdlShort')}</span>{writeGrants.ddl && <Check size={14} />}</button>
          </div>}
        </div>
        {runtimeId && <button className="secondary-button database-labeled-action" title={t('database.disconnect')} disabled={busy} onClick={() => void runAction(disconnect)}><Power size={14} /><span>{t('database.disconnect')}</span></button>}
      </div>
    </header>}
    <div className={'database-body' + (tableSidebarCollapsed && runtimeId && !managementOnly ? ' sidebar-collapsed' : '')}>
      <aside className="database-sidebar" aria-label={settingsVisible || !runtimeId ? t('database.connections') : t('database.tables')}>
        <header><strong>{settingsVisible || !runtimeId ? t('database.connections') : t('database.tables')}</strong><div className="database-sidebar-header-actions">
          {runtimeId && !settingsVisible && <>
            <button className="icon-button" title={readOnly ? t('database.importRequiresWrite') : t('database.importSql')} aria-label={t('database.importSql')} disabled={busy} onClick={importDatabase}><Upload size={14} /></button>
            <button className="icon-button" title={t('database.exportDatabase')} aria-label={t('database.exportDatabase')} disabled={busy} onClick={() => exportDatabase()}><Download size={14} /></button>
          </>}
          {settingsVisible ? <button className="icon-button" title={t('database.newConnection')} aria-label={t('database.newConnection')} disabled={busy} onClick={() => void startNew()}><Plus size={16} /></button>
            : runtimeId ? <button className="icon-button" title={t('database.refreshObjects')} aria-label={t('database.refreshObjects')} disabled={busy} onClick={() => void runAction(refreshTables)}><RefreshCw size={14} /></button> : null}
        </div></header>
        <label className="database-search"><Search size={13} /><input aria-label={t(settingsVisible || !runtimeId ? 'database.searchConnections' : 'database.filterTables')} placeholder={t(settingsVisible || !runtimeId ? 'database.searchConnections' : 'database.filterTables')}
          value={settingsVisible || !runtimeId ? connectionFilter : filter} onChange={(e) => settingsVisible || !runtimeId ? setConnectionFilter(e.target.value) : setFilter(e.target.value)} /></label>
        {settingsVisible || !runtimeId ? <div ref={listRef} className="database-list database-profile-list">
          {groups.map((group) => {
            const items = filteredProfiles.filter((profile) => profile.groupName === group)
            if (!items.length) return null
            const collapsed = !connectionFilter && collapsedGroups.has(group)
            return <section key={group} className="database-profile-group">
              {(groups.length > 1 || group) && <button className="database-group-heading" aria-expanded={!collapsed} onClick={() => setCollapsedGroups((current) => { const next = new Set(current); if (next.has(group)) next.delete(group); else next.add(group); return next })}><ChevronRight size={12} className={collapsed ? '' : 'expanded'} /><Folder size={13} /><span>{group || t('common.ungrouped')}</span><small>{items.length}</small></button>}
              {!collapsed && items.map((profile) => <div key={profile.id} data-database-profile={profile.id} className={'database-profile-row' + (profileId === profile.id ? ' active' : '') + (draggedProfile === profile.id ? ' dragging' : '') + (profileDrop?.id === profile.id ? ' drop-' + profileDrop.position : '')}
                onContextMenu={(e) => { e.preventDefault(); openMenu(e.currentTarget.querySelector<HTMLElement>('.database-profile-select')!, profileMenu(profile), e.clientX, e.clientY) }}>
                {managementOnly && <button className="mux-drag-handle" title={t('database.dragConnection')} aria-label={t('database.dragConnection')} disabled={busy || Boolean(connectionFilter)} onPointerDown={(e) => startProfileDrag(e, profile.id)} onKeyDown={(e) => {
                  if (!e.altKey || !['ArrowUp', 'ArrowDown'].includes(e.key)) return
                  e.preventDefault(); const siblings = profiles.filter((p) => p.groupName === profile.groupName); const index = siblings.findIndex((p) => p.id === profile.id); const next = siblings[index + (e.key === 'ArrowUp' ? -1 : 1)]
                  if (next) void runAction(() => reorderProfile(profile.id, next.id, e.key === 'ArrowUp' ? 'before' : 'after'))
                }}><GripVertical size={13} /></button>}
                <button className="database-profile-select" aria-pressed={profileId === profile.id} disabled={busy} onClick={() => void pickProfile(profile.id)} title={profile.host}><strong>{profile.name}</strong><small>{driverLabels[profile.driver]}</small></button>
                {managementOnly && <button className="icon-button database-row-menu" aria-label={t('database.profileActions', { name: profile.name })} title={t('database.profileActions', { name: profile.name })} disabled={busy} onClick={(e) => openMenu(e.currentTarget, profileMenu(profile))}><MoreHorizontal size={15} /></button>}
              </div>)}
            </section>
          })}
          {!filteredProfiles.length && <p className="database-sidebar-empty">{t(profiles.length ? 'database.noMatches' : 'database.noConnections')}</p>}
        </div> : <div className="database-list">
          {tables.filter((table) => table.toLowerCase().includes(filter.toLowerCase())).map((table) => <button key={table} className={selectedTable === table ? 'database-list-item active' : 'database-list-item'} aria-pressed={selectedTable === table} disabled={busy} onClick={() => void runAction(() => browseTable(table))} onContextMenu={(e) => { e.preventDefault(); openMenu(e.currentTarget, tableMenu(table), e.clientX, e.clientY) }}><Table2 size={14} /><span>{table}</span></button>)}
          {!tables.filter((table) => table.toLowerCase().includes(filter.toLowerCase())).length && <p className="database-sidebar-empty">{t(tables.length ? 'database.noMatches' : 'database.noTables')}</p>}
        </div>}
      </aside>
      <div className="database-main">
        {!managementOnly && runtimeId && <nav className="database-tabs" aria-label={t('database.views')}>
          <button className="database-sidebar-toggle" title={t(tableSidebarCollapsed ? 'database.expandTables' : 'database.collapseTables')} aria-label={t(tableSidebarCollapsed ? 'database.expandTables' : 'database.collapseTables')} aria-expanded={!tableSidebarCollapsed} onClick={() => setTableSidebarCollapsed((collapsed) => !collapsed)}>
            {tableSidebarCollapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
          </button>
          <button className={view === 'query' ? 'active' : ''} aria-current={view === 'query' ? 'page' : undefined} onClick={() => setView('query')}><FileCode2 size={14} />{t('database.query')}</button>
          <button disabled={!selectedTable} className={view === 'overview' ? 'active' : ''} aria-current={view === 'overview' ? 'page' : undefined} onClick={() => { setView('overview'); if (!tableResult) pendingViewLoad.current = 'overview' }}><Table2 size={14} />{t('database.data')}</button>
          <button disabled={!selectedTable} className={view === 'structure' ? 'active' : ''} aria-current={view === 'structure' ? 'page' : undefined} onClick={() => { setView('structure'); setStructureEdit(null); if (structureLoaded !== selectedTable) pendingViewLoad.current = 'structure' }}><KeyRound size={14} />{t('database.structure')}</button>
          <div className="database-tabs-tail">
            {selectedTable && <span className="database-selected-table" title={selectedTable}><Table2 size={13} /><span>{selectedTable}</span></span>}
          </div>
        </nav>}
        <main className="database-content">
          {settingsVisible ? <section className="database-connection-editor">
            <form id={formId} className="form-grid database-form" onChangeCapture={() => { setError(''); showNotice('') }} onSubmit={(event) => { event.preventDefault(); void runAction(saveProfile) }}>
              <div className="database-form-scroll">
                <div className="database-section-heading"><h2>{profileId ? name || t('database.settings') : t('database.newConnection')}</h2><span>{hasUnsavedChanges ? t('database.unsaved') : profileId ? t('database.settings') : t('database.connectionHint')}</span></div>
              <fieldset disabled={Boolean(runtimeId) || busy}>
                <legend>{t('database.driver')}</legend>
                <div className="database-driver-options">
                  {(['sqlite', 'mysql', 'postgresql'] as const).map((value) => <label key={value} className={driver === value ? 'database-driver-option selected' : 'database-driver-option'}>
                    <input type="radio" name={`database-driver-${paneId}`} value={value} checked={driver === value} onChange={() => { setPassword(''); setPasswordEdited(false); setPasswordVisible(false); setDriver(value); setPort(value === 'postgresql' ? 5432 : 3306) }} />
                    {value === 'sqlite' ? <HardDrive size={17} /> : <Server size={17} />}
                    <span><strong>{driverLabels[value]}</strong><small>{t(value === 'sqlite' ? 'database.localFile' : 'database.server')}</small></span>
                  </label>)}
                </div>
                <div className="database-form-columns"><label>{t('database.name')}<input value={name} onChange={(e) => setName(e.target.value)} placeholder={t('database.namePlaceholder')} autoComplete="off" /></label><label>{t('app.group')}<input value={groupName} onChange={(e) => setGroupName(e.target.value)} placeholder={t('common.ungrouped')} /></label></div>
                {driver === 'sqlite' ? <label>{t('database.path')}<div className="input-button"><input value={path} onChange={(e) => setPath(e.target.value)} placeholder={t('database.pathPlaceholder')} autoComplete="off" /><button type="button" className="secondary-button" onClick={() => void chooseSqlitePath()}><Folder size={14} />{t('common.choose')}</button></div><span className="field-hint">{t('database.pathHint')}</span></label> : <>
                  <div className="form-row"><label>{t('database.host')}<input value={host} onChange={(e) => setHost(e.target.value)} autoComplete="off" /></label><label>{t('database.port')}<input type="number" min={1} max={65535} value={port} onChange={(e) => setPort(Number(e.target.value))} /></label></div>
                  <div className="database-form-columns"><label>{t('database.username')}<input value={username} onChange={(e) => setUsername(e.target.value)} autoComplete="off" /></label><label>{t('database.database')}<input value={databaseName} onChange={(e) => setDatabaseName(e.target.value)} /></label></div>
                  <label>{t('app.password')}<div className="password-input">
                    <input type={passwordVisible ? 'text' : 'password'} autoComplete="new-password" value={password}
                      placeholder={canReuseCredential && !passwordEdited ? t('app.savedLeaveBlankToKeepUsingIt') : undefined}
                      onChange={(e) => { setPassword(e.target.value); setPasswordEdited(true) }} />
                    <button className="icon-button" type="button" aria-label={passwordVisible ? t('app.hidePassword') : t('app.showPassword')} onClick={() => setPasswordVisible(!passwordVisible)}>{passwordVisible ? <EyeOff size={16} /> : <Eye size={16} />}</button>
                  </div><span className="field-hint">{t('database.passwordHint')}</span></label>
                  {activeProfile?.hasSavedCredential && <button type="button" className="text-button" onClick={() => { setPassword(''); setPasswordEdited(true) }}>{t('database.clearPassword')}</button>}
                </>}
                {/* Read-only moved next to the write grants in the pane header, so the form keeps TLS only. */}
                {driver !== 'sqlite' && <div className="database-options">
                  <label className="check-label"><input type="checkbox" checked={sslEnabled} onChange={(e) => setSslEnabled(e.target.checked)} />{t('database.tls')}</label>
                </div>}
              </fieldset>
              </div>
              <footer className="database-form-actions">
                <button type="button" className="secondary-button" disabled={busy || !validEndpoint} onClick={() => void runAction(testConnection)}>{busy && <LoaderCircle size={13} className="database-spinner" />}{t('database.test')}</button>
                <div><button type="submit" className="secondary-button" disabled={busy || !name.trim() || !validEndpoint || !isDirty}>{t('common.save')}</button>
                  <button type="button" className="primary-button" disabled={busy || !name.trim() || !validEndpoint} onClick={() => void runAction(saveAndConnect)}><Play size={13} />{t(isDirty ? 'database.saveAndConnect' : 'database.connect')}</button></div>
              </footer>
            </form>
          </section> : !runtimeId ? <section className="database-welcome">
            <Database size={28} /><h2>{activeProfile?.name || t('database.title')}</h2><p>{busy ? t('database.working') : activeProfile ? t('database.readyToConnect') : t('database.chooseConnection')}</p>
            <button className="primary-button" disabled={busy || !activeProfile} onClick={() => void runAction(connect)}>{busy ? <LoaderCircle size={14} className="database-spinner" /> : <Play size={14} />}{t('database.connect')}</button>
            {!profiles.length && <button className="text-button" onClick={onManageConnections}>{t('database.manageConnections')}</button>}
          </section> : <>
            {view === 'overview' && <section className="database-data-view">
              <div className="database-query-toolbar"><strong><Table2 size={14} />{selectedTable}</strong><div className="database-toolbar-actions"><button className="icon-button" disabled={busy} title={t('database.refreshData')} aria-label={t('database.refreshData')} onClick={() => void runAction(() => loadTablePage(selectedTable, tablePage))}><RefreshCw size={14} /></button></div></div>
              {busy && !tableResult ? <div className="database-result-empty"><LoaderCircle size={16} className="database-spinner" />{t('database.working')}</div> : <DatabaseResultGrid result={tableResult} rowOffset={tablePage * tablePageSize} onCopy={copyCell} onRowMenu={(event, selection) => openMenu(event.currentTarget, resultMenu(selection), event.clientX, event.clientY)} emptyMessage={t('database.dataRetryHint')} />}
              <footer className="database-pagination"><div className="database-page-size"><span>{t('database.rowsPerPage')}</span><DatabasePageSizeSelect disabled={busy} value={tablePageSize} label={t('database.rowsPerPage')} onChange={changeTablePageSize} /></div><div className="database-page-nav"><button className="icon-button" disabled={busy || tablePage === 0} aria-label={t('database.previousPage')} onClick={() => void runAction(() => loadTablePage(selectedTable, tablePage - 1, tablePageSize))}><ChevronLeft size={14} /></button><span>{t('database.pageNumber', { page: tablePage + 1 })}</span><button className="icon-button" disabled={busy || !tableResult?.hasMore} aria-label={t('database.nextPage')} onClick={() => void runAction(() => loadTablePage(selectedTable, tablePage + 1, tablePageSize))}><ChevronRight size={14} /></button></div></footer>
            </section>}
            {view === 'query' && <section className="database-query-view">
              <div className="database-editor-wrap">
                <textarea ref={editorRef} aria-label={t('database.query')} className="database-editor" value={sql} spellCheck={false}
                  onFocus={(e) => { setEditorFocused(true); setSqlCursor(e.currentTarget.selectionStart); setCompletionDismissed(true) }}
                  onBlur={() => setEditorFocused(false)}
                  onClick={(e) => { setSqlCursor(e.currentTarget.selectionStart); setCompletionDismissed(true) }}
                  onKeyUp={(e) => { if (!['ArrowDown', 'ArrowUp', 'Enter', 'Tab', 'Escape'].includes(e.key)) setSqlCursor(e.currentTarget.selectionStart) }}
                  onChange={(e) => { setSql(e.target.value); setSqlCursor(e.target.selectionStart); setCompletionIndex(0); setCompletionDismissed(false) }}
                  onKeyDown={(e) => {
                    if ((e.ctrlKey || e.metaKey) && e.key === 'Enter') { e.preventDefault(); void runAction(execute); return }
                    if (completions.length && !e.ctrlKey && !e.metaKey) {
                      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); setCompletionIndex((current) => (current + (e.key === 'ArrowDown' ? 1 : -1) + completions.length) % completions.length); return }
                      if (e.key === 'Enter' || (e.key === 'Tab' && !e.shiftKey)) { e.preventDefault(); applyCompletion(completions[completionIndex % completions.length]!); return }
                      if (e.key === 'Escape') { e.preventDefault(); setCompletionDismissed(true); return }
                    }
                    if (e.key === 'Tab' && !e.shiftKey && !e.ctrlKey && !e.metaKey) { e.preventDefault(); const start = e.currentTarget.selectionStart, end = e.currentTarget.selectionEnd; setSql(sql.slice(0, start) + '  ' + sql.slice(end)); setSqlCursor(start + 2); requestAnimationFrame(() => editorRef.current?.setSelectionRange(start + 2, start + 2)) }
                  }} />
                {completions.length > 0 && <div className="database-completions" role="listbox" aria-label={t('database.suggestions')}>
                  {completions.map((item, index) => <button type="button" role="option" aria-selected={index === completionIndex} className={index === completionIndex ? 'active' : ''} key={`${item.kind}:${item.label}`} onMouseDown={(e) => { e.preventDefault(); applyCompletion(item) }}><span>{item.label}</span><small>{t(`database.suggestion.${item.kind}`)}</small></button>)}
                </div>}
              </div>
              <div className="database-query-actionbar"><div className="database-query-primary-actions"><button className="primary-button" disabled={busy || !sql.trim()} onClick={() => void runAction(execute)}>{busy ? <LoaderCircle size={13} className="database-spinner" /> : <Play size={13} />}{t('database.run')}</button><button className="secondary-button database-labeled-action" disabled={busy || !result?.columns.length || !result.rows.length} title={t('database.exportData')} onClick={exportResultSql}><Download size={14} /><span>{t('database.exportData')}</span></button><span className="database-query-shortcut">{t('database.runHint')}</span></div><div className="database-query-summary"><strong>{t('database.results')}</strong>{result && <span>{t('database.rowCount', { count: result.rows.length })}</span>}{queryTime !== null && <span>{queryTime} ms</span>}{result?.truncated && result.hasMore === undefined && <span>{t('database.truncated')}</span>}</div></div>
              <DatabaseResultGrid result={result} rowOffset={queryPage * queryPageSize} onCopy={copyCell} onRowMenu={(event, selection) => openMenu(event.currentTarget, resultMenu(selection), event.clientX, event.clientY)} emptyMessage={busy ? t('database.working') : error ? t('database.queryRetryHint') : undefined} />
              <footer className="database-pagination"><div className="database-page-size"><span>{t('database.rowsPerPage')}</span><DatabasePageSizeSelect disabled={busy || executedStatementWrites} value={queryPageSize} label={t('database.rowsPerPage')} onChange={changeQueryPageSize} /></div><div className="database-page-nav"><button className="icon-button" disabled={busy || executedStatementWrites || queryPage === 0} aria-label={t('database.previousPage')} onClick={() => void runAction(() => rerunQueryPage(queryPage - 1, queryPageSize))}><ChevronLeft size={14} /></button><span>{t('database.pageNumber', { page: queryPage + 1 })}</span><button className="icon-button" disabled={busy || executedStatementWrites || !result?.hasMore} aria-label={t('database.nextPage')} onClick={() => void runAction(() => rerunQueryPage(queryPage + 1, queryPageSize))}><ChevronRight size={14} /></button></div></footer>
            </section>}
            {view === 'structure' && <section className="database-structure-view">
              <div className="database-structure-heading"><div className="database-structure-title"><Table2 size={15} /><h2 title={selectedTable}>{selectedTable}</h2></div><button className="icon-button" title={t('database.refreshStructure')} aria-label={t('database.refreshStructure')} disabled={busy} onClick={() => void runAction(() => inspectTable(selectedTable))}><RefreshCw size={14} /></button></div>
              {busy && !structureLoaded ? <p className="database-result-empty">{t('database.working')}</p> : structureLoaded === selectedTable && <>
                <section className="database-structure-section"><h3>{t('database.columns')}<small>{columns.length}</small>{driver === 'sqlite' && <button className="icon-button" title={t('database.addColumn')} aria-label={t('database.addColumn')} disabled={readOnly || busy} onClick={() => { setNewColumnName(''); setNewColumnType('TEXT'); setStructureEdit('column') }}><Plus size={14} /></button>}</h3>
                  {structureEdit === 'column' && <form className="form-grid database-inline-form" onSubmit={(e) => { e.preventDefault(); void runAction(addColumn) }}><label>{t('database.columnName')}<input autoFocus value={newColumnName} onChange={(e) => setNewColumnName(e.target.value)} /></label><label>{t('database.type')}<input value={newColumnType} onChange={(e) => setNewColumnType(e.target.value)} /></label><div className="database-actions"><button type="submit" className="primary-button" disabled={busy || !newColumnName.trim() || !newColumnType.trim()}>{t('database.add')}</button><button type="button" className="secondary-button" onClick={() => setStructureEdit(null)}>{t('common.cancel')}</button></div></form>}
                  <div className="database-result"><table><thead><tr><th>{t('database.columnName')}</th><th>{t('database.type')}</th><th>{t('database.constraints')}</th><th>{t('database.defaultValue')}</th><th>{t('database.comment')}</th>{driver === 'sqlite' && <th />}</tr></thead><tbody>{columns.map((column) => <tr key={column.name}><td>{column.name}</td><td>{column.dataType}</td><td>{[column.primaryKey ? 'PK' : '', column.notNull ? 'NOT NULL' : ''].filter(Boolean).join(' · ') || '—'}</td><td>{column.defaultValue ?? '—'}</td><td className="database-column-comment" title={column.comment ?? undefined}>{column.comment || '—'}</td>{driver === 'sqlite' && <td><button className="icon-button danger database-row-action" title={t('database.delete')} aria-label={t('database.deleteColumn', { name: column.name })} disabled={readOnly || busy} onClick={() => void runAction(() => dropColumn(column.name))}><Trash2 size={13} /></button></td>}</tr>)}</tbody></table></div>
                </section>
                {driver !== 'postgresql' && <section className="database-structure-section"><h3>{t('database.indexes')}<small>{indexes.length}</small>{driver === 'sqlite' && <button className="icon-button" title={t('database.addIndex')} aria-label={t('database.addIndex')} disabled={readOnly || busy} onClick={() => { setNewIndexName(''); setNewIndexColumn(''); setUniqueIndex(false); setStructureEdit('index') }}><Plus size={14} /></button>}</h3>
                  {structureEdit === 'index' && <form className="form-grid database-inline-form" onSubmit={(e) => { e.preventDefault(); void runAction(addIndex) }}><label>{t('database.indexName')}<input autoFocus value={newIndexName} onChange={(e) => setNewIndexName(e.target.value)} /></label><label>{t('database.columnName')}<input value={newIndexColumn} onChange={(e) => setNewIndexColumn(e.target.value)} /></label><label className="check-label"><input type="checkbox" checked={uniqueIndex} onChange={(e) => setUniqueIndex(e.target.checked)} />UNIQUE</label><div className="database-actions"><button type="submit" className="primary-button" disabled={busy || !newIndexName.trim() || !newIndexColumn.trim()}>{t('database.add')}</button><button type="button" className="secondary-button" onClick={() => setStructureEdit(null)}>{t('common.cancel')}</button></div></form>}
                  {indexes.map((index) => <div className="database-meta-row" key={index.name}><KeyRound size={13} /><span>{index.name}</span><small>{index.unique ? 'UNIQUE' : 'INDEX'}</small>{driver === 'sqlite' && <button className="icon-button danger database-row-action" title={t('database.delete')} aria-label={t('database.deleteIndex', { name: index.name })} disabled={readOnly || busy} onClick={() => void runAction(async () => { if (readOnly) throw new Error(t('database.readOnly')); if (writeGranted('ddl') || await confirm(t('database.confirmDeleteIndex'))) { await window.api.databaseRuntimes.dropIndex(runtimeId, index.name); await inspectTable(selectedTable) } })}><Trash2 size={13} /></button>}</div>)}
                  {!indexes.length && <p className="field-hint">{t('database.noIndexes')}</p>}
                </section>}
                {driver === 'sqlite' && <section className="database-structure-section"><h3>{t('database.foreignKeys')}<small>{foreignKeys.length}</small></h3>{foreignKeys.map((key, i) => <div className="database-meta-row" key={i}><span>{key.column}</span><span>→ {key.referencesTable}.{key.referencesColumn}</span></div>)}{!foreignKeys.length && <p className="field-hint">{t('database.noForeignKeys')}</p>}</section>}
              </>}
            </section>}
          </>}
        </main>
      </div>
    </div>
    {feedback}
    {menu && <DatabaseMenu {...menu} onClose={() => setMenu(null)} />}
    {fileDialog && <DatabaseFileDialog {...fileDialog} />}
  </div>
}
