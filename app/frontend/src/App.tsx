import { useEffect, useMemo, useRef, useState } from 'react'
import { confirm as confirmDialog } from '@tauri-apps/plugin-dialog'
import { ArrowRight, Bookmark as BookmarkIcon, Bot, Check, ChevronDown, ChevronRight, CircleHelp, CirclePlus, Copy, Edit3, Eye, EyeOff, FileInput, Folder, FolderOpen, FolderPlus, GripVertical, Image as ImageIcon, KeyRound, Minus, Monitor, Moon, Network, Palette, PanelLeftClose, PanelLeftOpen, Play, Plug, Power, Rocket, RotateCcw, Search, Send, Server, Settings as SettingsIcon, ShieldAlert, Square, SquareTerminal, Star, Sun, Trash2, WandSparkles, X } from 'lucide-react'
import { DEFAULT_AI_SETTINGS, DEFAULT_TERMINAL_SETTINGS, type AiCommandSuggestion, type AiRiskAssessment, type AiSettings, type AiSettingsInput, type AiShell, type AppEvent, type AppIconId, type AppIconSettings, type Bookmark, type BookmarkInput, type ConflictResolution, type ConnectInput, type DeploymentDiffEntry, type DeploymentProfile, type HostKeyPrompt, type PortForwardProfile, type SessionStatus, type SshConfigPreview, type TerminalSettings, type TransferTask, type TunnelSummary, type UiTheme } from './types'
import { TerminalPane } from './components/TerminalPane'
import { SftpPane } from './components/SftpPane'
import { HelpDialog } from './components/HelpDialog'
import { colorWithOpacity, terminalBackgroundStyle } from './terminal-style'

interface WorkspaceTab { key: string; bookmarkId: string; sessionId?: string; status: SessionStatus; error?: string }
interface ConnectionTarget { newSession?: boolean; tabKey?: string }
interface SidebarContextMenu { x: number; y: number; group?: string; bookmark?: Bookmark }
interface GroupDialogState { mode: 'create' | 'rename'; group?: string }
type SettingsSection = 'appearance' | 'terminal' | 'application' | 'ai'
type SidebarPointerDrag = { pointerId: number; type: 'bookmark' | 'group'; value: string; startX: number; startY: number; active: boolean }
type SidebarPointerDrop = { type: 'bookmark'; id: string; group: string; position: 'before' | 'after' } | { type: 'group'; group: string; position: 'before' | 'after' | 'inside' }

const emptyBookmark: BookmarkInput = { name: '', host: '', port: 22, username: '', authType: 'password', privateKeyPath: '', jumpBookmarkId: '', groupName: '', favorite: false, keepaliveEnabled: true, keepaliveIntervalSeconds: 15, keepaliveCountMax: 3, note: '' }
type ConnectionCredentials = Omit<ConnectInput, 'bookmarkId' | 'newSession'>
const defaultSidebarWidth = 260
const minSidebarWidth = 200
const maxSidebarWidth = 480

function clampSidebarWidth(width: number): number {
  return Math.min(maxSidebarWidth, Math.max(minSidebarWidth, Math.round(width)))
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error)
}

export function App(): React.JSX.Element {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([])
  const [tabs, setTabs] = useState<WorkspaceTab[]>([])
  const [activeKey, setActiveKey] = useState('')
  const [query, setQuery] = useState('')
  const [bookmarkDialog, setBookmarkDialog] = useState<Bookmark | 'new' | null>(null)
  const [authRequest, setAuthRequest] = useState<{ bookmark: Bookmark; target: ConnectionTarget } | null>(null)
  const [hostPrompts, setHostPrompts] = useState<HostKeyPrompt[]>([])
  const [conflict, setConflict] = useState<{ taskId: string; sourcePath: string; destinationPath: string } | null>(null)
  const [transfers, setTransfers] = useState<TransferTask[]>([])
  const [transferView, setTransferView] = useState<'queue' | 'history'>('queue')
  const [workspaceView, setWorkspaceView] = useState<'terminal' | 'files'>('terminal')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [collapsedBookmarkGroups, setCollapsedBookmarkGroups] = useState<Set<string>>(new Set())
  const [bookmarkGroupNames, setBookmarkGroupNames] = useState<string[]>([])
  const [selectedBookmarkId, setSelectedBookmarkId] = useState('')
  const [draggedBookmarkId, setDraggedBookmarkId] = useState('')
  const [bookmarkDrop, setBookmarkDrop] = useState<{ id: string; position: 'before' | 'after' } | null>(null)
  const [draggedGroupName, setDraggedGroupName] = useState<string | null>(null)
  const [groupDrop, setGroupDrop] = useState<{ group: string; position: 'before' | 'after' | 'inside' } | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(defaultSidebarWidth)
  const [terminalSettings, setTerminalSettings] = useState<TerminalSettings>(DEFAULT_TERMINAL_SETTINGS)
  const [terminalBackground, setTerminalBackground] = useState('')
  const [uiTheme, setUiTheme] = useState<UiTheme>('system')
  const [uiThemePreview, setUiThemePreview] = useState<UiTheme | null>(null)
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches)
  const [appIcons, setAppIcons] = useState<AppIconSettings>({ selected: 'ssh', options: [] })
  const [aiSettings, setAiSettings] = useState<AiSettings>({ ...DEFAULT_AI_SETTINGS })
  const [aiDialog, setAiDialog] = useState(false)
  const [settingsDialog, setSettingsDialog] = useState(false)
  const [settingsInitialSection, setSettingsInitialSection] = useState<SettingsSection>('appearance')
  const [helpDialog, setHelpDialog] = useState(false)
  const [deploymentDialog, setDeploymentDialog] = useState(false)
  const [tunnelDialog, setTunnelDialog] = useState(false)
  const [importPreview, setImportPreview] = useState<SshConfigPreview | null>(null)
  const [sidebarContextMenu, setSidebarContextMenu] = useState<SidebarContextMenu | null>(null)
  const [groupDialog, setGroupDialog] = useState<GroupDialogState | null>(null)
  const [toast, setToast] = useState('')
  const stateRestored = useRef(false)
  const sidebarWidthRef = useRef(defaultSidebarWidth)
  const sidebarPointerDragRef = useRef<SidebarPointerDrag | null>(null)
  const sidebarPointerDropRef = useRef<SidebarPointerDrop | null>(null)
  const suppressSidebarClickRef = useRef(false)

  const showError = (message: string): void => { setToast(message); setTimeout(() => setToast(''), 4500) }
  const openSettings = (section: SettingsSection = 'appearance'): void => { setSettingsInitialSection(section); setSettingsDialog(true) }
  const reloadBookmarks = async (): Promise<void> => setBookmarks(await window.api.bookmarks.list())
  const reloadSidebarData = async (): Promise<void> => {
    const [items, groups] = await Promise.all([window.api.bookmarks.list(), window.api.bookmarkGroups.list()])
    setBookmarks(items)
    setBookmarkGroupNames(groups)
  }

  useEffect(() => {
    void Promise.all([window.api.bookmarks.list(), window.api.bookmarkGroups.list(), window.api.transfers.list(), window.api.state.getSidebarCollapsed(), window.api.state.getCollapsedBookmarkGroups(), window.api.state.getSidebarWidth(), window.api.settings.getTerminal(), window.api.settings.getAppIcons(), window.api.settings.getUiTheme(), window.api.ai.getSettings()]).then(([items, groups, transferItems, collapsed, collapsedGroups, width, appearance, icons, theme, ai]) => {
      setBookmarks(items); setTransfers(transferItems)
      setBookmarkGroupNames(groups)
      setSidebarCollapsed(collapsed)
      setCollapsedBookmarkGroups(new Set(collapsedGroups))
      sidebarWidthRef.current = width
      setSidebarWidth(width)
      setTerminalSettings(appearance)
      setAppIcons(icons)
      setUiTheme(theme)
      setAiSettings(ai)
      if (appearance.backgroundImagePath) void window.api.settings.loadTerminalBackground(appearance.backgroundImagePath).then(setTerminalBackground).catch(() => setTerminalBackground(''))
      stateRestored.current = true
    }).catch((error) => showError(errorMessage(error)))
  }, [])

  useEffect(() => { if (stateRestored.current) void window.api.state.setSidebarCollapsed(sidebarCollapsed).catch((error) => showError(errorMessage(error))) }, [sidebarCollapsed])

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const update = (event: MediaQueryListEvent): void => setSystemDark(event.matches)
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])

  useEffect(() => {
    if (window.api.platform !== 'win32') return
    const preventBrowserMenu = (event: MouseEvent): void => event.preventDefault()
    document.addEventListener('contextmenu', preventBrowserMenu)
    return () => document.removeEventListener('contextmenu', preventBrowserMenu)
  }, [])

  useEffect(() => {
    if (!sidebarContextMenu) return
    const close = (): void => setSidebarContextMenu(null)
    const closeOnKey = (event: KeyboardEvent): void => { if (event.key === 'Escape') close() }
    document.addEventListener('pointerdown', close)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    document.addEventListener('keydown', closeOnKey)
    return () => {
      document.removeEventListener('pointerdown', close)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
      document.removeEventListener('keydown', closeOnKey)
    }
  }, [sidebarContextMenu])

  useEffect(() => window.api.onEvent((event: AppEvent) => {
    if (event.type === 'session') {
      setTabs((current) => current.map((tab) => tab.sessionId === event.payload.id ? { ...tab, status: event.payload.status, error: event.payload.error } : tab))
      if (event.payload.status === 'connected') void reloadBookmarks()
      if (['disconnected', 'error'].includes(event.payload.status)) setHostPrompts((current) => current.filter((item) => item.sessionId !== event.payload.id))
    }
    if (event.type === 'host-key') setHostPrompts((current) => current.some((item) => item.sessionId === event.payload.sessionId && item.fingerprint === event.payload.fingerprint) ? current : [...current, event.payload])
    if (event.type === 'transfer') {
      setTransfers((current) => [event.payload, ...current.filter((item) => item.id !== event.payload.id)].sort((a, b) => b.createdAt.localeCompare(a.createdAt)))
      if (event.payload.status !== 'conflict') setConflict((current) => current?.taskId === event.payload.id ? null : current)
    }
    if (event.type === 'transfer-conflict') setConflict(event.payload)
  }), [])

  const bookmarkMap = useMemo(() => new Map(bookmarks.map((item) => [item.id, item])), [bookmarks])
  const activeTab = tabs.find((tab) => tab.key === activeKey)
  const activeBookmark = activeTab ? bookmarkMap.get(activeTab.bookmarkId) : undefined
  const selectedBookmark = bookmarkMap.get(selectedBookmarkId) ?? activeBookmark
  const filteredBookmarks = bookmarks.filter((item) => `${item.name} ${item.host} ${item.username} ${item.groupName} ${item.note}`.toLowerCase().includes(query.toLowerCase()))
  const bookmarkGroups = useMemo(() => {
    const groups = new Map(bookmarkGroupNames.map((group) => [group, [] as Bookmark[]]))
    for (const bookmark of filteredBookmarks) {
      const group = bookmark.groupName
      groups.set(group, [...(groups.get(group) ?? []), bookmark])
    }
    return [...groups].filter(([, items]) => !query || items.length > 0)
  }, [bookmarkGroupNames, filteredBookmarks, query])

  const toggleBookmarkGroup = (group: string): void => {
    setCollapsedBookmarkGroups((current) => {
      const next = new Set(current)
      if (next.has(group)) next.delete(group); else next.add(group)
      void window.api.state.setCollapsedBookmarkGroups([...next]).catch((error) => showError(errorMessage(error)))
      return next
    })
  }

  const moveBookmark = async (sourceId: string, targetGroup: string, targetId?: string, position: 'before' | 'after' = 'after'): Promise<void> => {
    if (!sourceId || sourceId === targetId || query) return
    const source = bookmarks.find((bookmark) => bookmark.id === sourceId)
    if (!source || !bookmarkGroupNames.includes(targetGroup)) return
    const next = bookmarks.filter((bookmark) => bookmark.id !== sourceId)
    let targetIndex = targetId ? next.findIndex((bookmark) => bookmark.id === targetId) : -1
    if (targetIndex < 0) {
      targetIndex = next.reduce((last, bookmark, index) => bookmark.groupName === targetGroup ? index : last, -1)
      position = 'after'
    }
    next.splice(targetIndex < 0 ? next.length : targetIndex + (position === 'after' ? 1 : 0), 0, { ...source, groupName: targetGroup })
    setBookmarks(next)
    try {
      if (source.groupName !== targetGroup) await window.api.bookmarks.moveToGroup(sourceId, targetGroup)
      setBookmarks(await window.api.bookmarks.reorder(next.map((bookmark) => bookmark.id)))
    }
    catch (error) { showError(errorMessage(error)); await reloadBookmarks() }
  }

  const reorderBookmarkGroup = async (sourceGroup: string, targetGroup: string, position: 'before' | 'after'): Promise<void> => {
    if (sourceGroup === targetGroup || query) return
    const previous = bookmarkGroupNames
    const next = bookmarkGroupNames.filter((group) => group !== sourceGroup)
    const targetIndex = next.indexOf(targetGroup)
    if (targetIndex < 0) return
    next.splice(targetIndex + (position === 'after' ? 1 : 0), 0, sourceGroup)
    setBookmarkGroupNames(next)
    try { setBookmarkGroupNames(await window.api.bookmarkGroups.reorder(next)) }
    catch (error) { setBookmarkGroupNames(previous); showError(errorMessage(error)) }
  }

  const finishSidebarDrag = (): void => {
    setDraggedBookmarkId('')
    setDraggedGroupName(null)
    setBookmarkDrop(null)
    setGroupDrop(null)
  }

  const startSidebarPointerDrag = (event: React.PointerEvent, type: 'bookmark' | 'group', value: string): void => {
    if (event.button !== 0 || query || sidebarPointerDragRef.current) return
    const drag: SidebarPointerDrag = { pointerId: event.pointerId, type, value, startX: event.clientX, startY: event.clientY, active: false }
    sidebarPointerDragRef.current = drag
    sidebarPointerDropRef.current = null

    const move = (moveEvent: PointerEvent): void => {
      const current = sidebarPointerDragRef.current
      if (!current || moveEvent.pointerId !== current.pointerId) return
      if (!current.active && Math.hypot(moveEvent.clientX - current.startX, moveEvent.clientY - current.startY) < 5) return
      if (!current.active) {
        current.active = true
        document.body.classList.add('sidebar-item-dragging')
        if (current.type === 'bookmark') { setSelectedBookmarkId(current.value); setDraggedBookmarkId(current.value) }
        else setDraggedGroupName(current.value)
      }
      moveEvent.preventDefault()
      const target = document.elementFromPoint(moveEvent.clientX, moveEvent.clientY) as HTMLElement | null
      if (current.type === 'bookmark') {
        const bookmarkElement = target?.closest<HTMLElement>('[data-bookmark-id]')
        if (bookmarkElement?.dataset.bookmarkId && bookmarkElement.dataset.bookmarkId !== current.value) {
          const targetBookmark = bookmarks.find((bookmark) => bookmark.id === bookmarkElement.dataset.bookmarkId)
          if (targetBookmark) {
            const bounds = bookmarkElement.getBoundingClientRect()
            const position = moveEvent.clientY < bounds.top + bounds.height / 2 ? 'before' : 'after'
            sidebarPointerDropRef.current = { type: 'bookmark', id: targetBookmark.id, group: targetBookmark.groupName, position }
            setBookmarkDrop({ id: targetBookmark.id, position })
            setGroupDrop(null)
            return
          }
        }
        const groupElement = target?.closest<HTMLElement>('[data-group-name]')
        if (groupElement?.dataset.groupName !== undefined) {
          const group = groupElement.dataset.groupName
          sidebarPointerDropRef.current = { type: 'group', group, position: 'inside' }
          setBookmarkDrop(null)
          setGroupDrop({ group, position: 'inside' })
          return
        }
      } else {
        const groupElement = target?.closest<HTMLElement>('[data-group-name]')
        if (groupElement?.dataset.groupName !== undefined && groupElement.dataset.groupName !== current.value) {
          const group = groupElement.dataset.groupName
          const bounds = groupElement.getBoundingClientRect()
          const position = moveEvent.clientY < bounds.top + bounds.height / 2 ? 'before' : 'after'
          sidebarPointerDropRef.current = { type: 'group', group, position }
          setGroupDrop({ group, position })
          return
        }
      }
      sidebarPointerDropRef.current = null
      setBookmarkDrop(null)
      setGroupDrop(null)
    }

    const finish = (finishEvent: PointerEvent, commit: boolean): void => {
      const current = sidebarPointerDragRef.current
      if (!current || finishEvent.pointerId !== current.pointerId) return
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', cancel)
      document.body.classList.remove('sidebar-item-dragging')
      const drop = sidebarPointerDropRef.current
      sidebarPointerDragRef.current = null
      sidebarPointerDropRef.current = null
      finishSidebarDrag()
      if (!current.active) return
      suppressSidebarClickRef.current = true
      window.setTimeout(() => { suppressSidebarClickRef.current = false }, 0)
      if (!commit || !drop) return
      if (current.type === 'bookmark') {
        if (drop.type === 'bookmark') void moveBookmark(current.value, drop.group, drop.id, drop.position)
        else void moveBookmark(current.value, drop.group)
      } else if (drop.type === 'group') void reorderBookmarkGroup(current.value, drop.group, drop.position === 'inside' ? 'after' : drop.position)
    }
    const up = (upEvent: PointerEvent): void => finish(upEvent, true)
    const cancel = (cancelEvent: PointerEvent): void => finish(cancelEvent, false)
    window.addEventListener('pointermove', move, { passive: false })
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', cancel)
  }

  const openSidebarContextMenu = (event: React.MouseEvent, options: Omit<SidebarContextMenu, 'x' | 'y'> = {}): void => {
    event.preventDefault()
    event.stopPropagation()
    if (options.bookmark) setSelectedBookmarkId(options.bookmark.id)
    setSidebarContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 230)), ...options })
  }

  const saveGroup = async (name: string): Promise<void> => {
    if (!groupDialog) return
    try {
      if (groupDialog.mode === 'create') setBookmarkGroupNames(await window.api.bookmarkGroups.create(name))
      else {
        const oldGroup = groupDialog.group ?? ''
        setBookmarkGroupNames(await window.api.bookmarkGroups.rename(oldGroup, name))
        setCollapsedBookmarkGroups((current) => {
          if (!current.has(oldGroup)) return current
          const next = new Set(current)
          next.delete(oldGroup)
          next.add(name.trim())
          void window.api.state.setCollapsedBookmarkGroups([...next]).catch((error) => showError(errorMessage(error)))
          return next
        })
        await reloadBookmarks()
      }
      setGroupDialog(null)
    } catch (error) { showError(errorMessage(error)) }
  }

  const duplicateBookmark = async (bookmark: Bookmark): Promise<void> => {
    try {
      const duplicate = await window.api.bookmarks.duplicate(bookmark.id)
      await reloadBookmarks()
      setBookmarkDialog(duplicate)
    } catch (error) { showError(errorMessage(error)) }
  }

  const connect = async (bookmark: Bookmark, credentials: ConnectionCredentials = {}, target: ConnectionTarget = {}): Promise<void> => {
    let key = target.tabKey ?? (target.newSession ? undefined : tabs.find((tab) => tab.bookmarkId === bookmark.id)?.key)
    if (!key) { key = crypto.randomUUID(); setTabs((current) => [...current, { key: key!, bookmarkId: bookmark.id, status: 'connecting' }]) }
    else setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: undefined, status: 'connecting', error: undefined } : tab))
    setActiveKey(key); setAuthRequest(null)
    try {
      const summary = await window.api.sessions.connect({ bookmarkId: bookmark.id, newSession: target.newSession, ...credentials })
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: summary.id, status: summary.status, error: undefined } : tab))
      if (credentials.rememberCredential || credentials.rememberJumpCredential) await reloadBookmarks()
    } catch (error) {
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, status: 'error', error: errorMessage(error) } : tab))
      showError(errorMessage(error))
    }
  }

  const openBookmark = (bookmark: Bookmark, target: ConnectionTarget = {}): void => {
    const requestedTab = target.tabKey ? tabs.find((tab) => tab.key === target.tabKey) : undefined
    if (requestedTab && ['connected', 'connecting'].includes(requestedTab.status)) { setActiveKey(requestedTab.key); return }
    const existing = target.newSession ? undefined : requestedTab ?? tabs.find((tab) => tab.bookmarkId === bookmark.id && ['connected', 'connecting'].includes(tab.status)) ?? tabs.find((tab) => tab.bookmarkId === bookmark.id)
    if (!target.tabKey && (existing?.status === 'connected' || existing?.status === 'connecting')) { setActiveKey(existing.key); return }
    const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
    const hasOtherActiveSession = Boolean(target.tabKey && tabs.some((tab) => tab.key !== target.tabKey && tab.bookmarkId === bookmark.id && ['connected', 'connecting'].includes(tab.status)))
    const connectionTarget = target.tabKey ? { ...target, newSession: target.newSession ?? hasOtherActiveSession } : { ...target, tabKey: existing?.key }
    const needsPrompt = (item: Bookmark): boolean => item.authType === 'privateKey' || (item.authType === 'password' && !item.hasSavedCredential)
    if (!needsPrompt(bookmark) && (!jumpBookmark || !needsPrompt(jumpBookmark))) void connect(bookmark, {}, connectionTarget)
    else setAuthRequest({ bookmark, target: connectionTarget })
  }

  const openBookmarkFromSidebar = (bookmark: Bookmark): void => {
    const hasActiveSession = tabs.some((tab) => tab.bookmarkId === bookmark.id && ['connected', 'connecting'].includes(tab.status))
    openBookmark(bookmark, hasActiveSession ? { newSession: true } : {})
  }

  const tabLabel = (tab: WorkspaceTab, bookmark?: Bookmark): string => {
    const matchingTabs = tabs.filter((item) => item.bookmarkId === tab.bookmarkId)
    if (matchingTabs.length < 2) return bookmark?.name ?? '已删除连接'
    return `${bookmark?.name ?? '已删除连接'} ${matchingTabs.findIndex((item) => item.key === tab.key) + 1}`
  }

  const closeTab = (tab: WorkspaceTab): void => {
    if (tab.sessionId) void window.api.sessions.disconnect(tab.sessionId).catch((error) => showError(errorMessage(error)))
    setTabs((current) => {
      const index = current.findIndex((item) => item.key === tab.key)
      const next = current.filter((item) => item.key !== tab.key)
      if (activeKey === tab.key) setActiveKey(next[Math.min(index, next.length - 1)]?.key ?? '')
      return next
    })
  }

  const removeBookmark = async (bookmark: Bookmark): Promise<void> => {
    if (!await confirmDialog(`确定删除连接“${bookmark.name}”吗？`, { title: '删除连接', kind: 'warning' })) return
    try {
      await window.api.bookmarks.remove(bookmark.id)
      for (const tab of tabs.filter((item) => item.bookmarkId === bookmark.id)) closeTab(tab)
      setSelectedBookmarkId((current) => current === bookmark.id ? '' : current)
      await reloadBookmarks()
    } catch (error) { showError(errorMessage(error)) }
  }

  const deleteBookmarkGroup = async (group: string): Promise<void> => {
    const label = group || '未分组'
    const count = bookmarks.filter((bookmark) => bookmark.groupName === group).length
    if (!await confirmDialog(`确定删除分组“${label}”及其中的 ${count} 个连接吗？此操作无法撤销。`, { title: '删除分组', kind: 'warning' })) return
    try {
      const result = await window.api.bookmarkGroups.delete(group)
      const deleted = new Set(result.deletedBookmarkIds)
      for (const tab of tabs.filter((item) => deleted.has(item.bookmarkId))) closeTab(tab)
      setBookmarkGroupNames(result.groups)
      setBookmarks((current) => current.filter((bookmark) => !deleted.has(bookmark.id)))
      setSelectedBookmarkId((current) => deleted.has(current) ? '' : current)
      setCollapsedBookmarkGroups((current) => {
        if (!current.has(group)) return current
        const next = new Set(current)
        next.delete(group)
        void window.api.state.setCollapsedBookmarkGroups([...next]).catch((error) => showError(errorMessage(error)))
        return next
      })
    } catch (error) { showError(errorMessage(error)) }
  }

  const retryTransfer = async (task: TransferTask): Promise<void> => {
    if (!task.bookmarkId) { showError('此旧传输记录缺少连接信息，无法自动重试'); return }
    const tab = tabs.find((item) => item.bookmarkId === task.bookmarkId && item.status === 'connected' && item.sessionId)
    if (!tab?.sessionId) {
      const bookmark = bookmarkMap.get(task.bookmarkId)
      const existing = tabs.find((item) => item.bookmarkId === task.bookmarkId)
      if (existing) setActiveKey(existing.key)
      else if (bookmark) openBookmark(bookmark)
      showError('请先连接此传输任务对应的连接，再点击重试')
      return
    }
    try { await window.api.transfers.retry(task.id, tab.sessionId) }
    catch (error) { showError(errorMessage(error)) }
  }

  const cancelTransfer = async (task: TransferTask): Promise<void> => {
    try { await window.api.transfers.cancel(task.id) }
    catch (error) { showError(errorMessage(error)) }
  }

  const clearCompletedTransfers = async (): Promise<void> => {
    try {
      await window.api.transfers.clearCompleted()
      setTransfers((current) => current.filter((item) => !['completed', 'cancelled'].includes(item.status)))
    } catch (error) { showError(errorMessage(error)) }
  }

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => {
      if (event.key === 'F1' && !document.querySelector('.modal-backdrop, .help-backdrop')) {
        event.preventDefault()
        setHelpDialog(true)
        return
      }
      if (document.querySelector('.modal-backdrop, .help-backdrop')) return
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.shiftKey && event.key.toLowerCase() === 't' && activeBookmark) {
        event.preventDefault()
        openBookmark(activeBookmark, { newSession: true })
        return
      }
      if (commandKey && !event.shiftKey && event.key.toLowerCase() === 'w' && activeTab) {
        event.preventDefault()
        closeTab(activeTab)
        return
      }
      if (event.ctrlKey && event.key === 'Tab' && tabs.length > 1) {
        event.preventDefault()
        const index = tabs.findIndex((tab) => tab.key === activeKey)
        const offset = event.shiftKey ? -1 : 1
        setActiveKey(tabs[(index + offset + tabs.length) % tabs.length]!.key)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [activeBookmark, activeTab, activeKey, tabs])

  const statusText: Record<SessionStatus, string> = { connecting: '连接中', connected: '已连接', disconnected: '未连接', error: '连接失败' }

  const saveSettings = async (settings: TerminalSettings, previewImage: string, appIcon: AppIconId, theme: UiTheme, ai: AiSettingsInput): Promise<void> => {
    try {
      const [saved, savedTheme, selected, savedAi] = await Promise.all([window.api.settings.saveTerminal(settings), window.api.settings.saveUiTheme(theme), window.api.settings.setAppIcon(appIcon), window.api.ai.saveSettings(ai)])
      let image = saved.backgroundImagePath === settings.backgroundImagePath ? previewImage : ''
      if (saved.backgroundImagePath && !image) image = await window.api.settings.loadTerminalBackground(saved.backgroundImagePath)
      setTerminalSettings(saved)
      setUiTheme(savedTheme)
      setUiThemePreview(null)
      setTerminalBackground(image)
      setAppIcons((current) => ({ ...current, selected }))
      setAiSettings(savedAi)
      setSettingsDialog(false)
    } catch (error) { showError(errorMessage(error)) }
  }

  const selectedAppIcon = appIcons.options.find((item) => item.id === appIcons.selected)
  const activeThemeMode = uiThemePreview ?? uiTheme
  const resolvedUiTheme = activeThemeMode === 'system' ? systemDark ? 'dark' : 'light' : activeThemeMode

  const chooseSshConfig = async (): Promise<void> => {
    try {
      const preview = await window.api.bookmarks.previewSshConfig()
      if (preview) setImportPreview(preview)
    } catch (error) { showError(errorMessage(error)) }
  }

  useEffect(() => window.api.onEvent((event) => {
    if (event.type !== 'menu-command') return
    const modalOpen = Boolean(document.querySelector('.modal-backdrop, .help-backdrop'))
    if (modalOpen) return
    if (event.payload === 'new-connection') setBookmarkDialog('new')
    else if (event.payload === 'import-connections') void chooseSshConfig()
    else if (event.payload === 'new-session' && activeBookmark) openBookmark(activeBookmark, { newSession: true })
    else if (event.payload === 'close-tab' && activeTab) closeTab(activeTab)
    else if (event.payload === 'settings') openSettings()
    else if (event.payload === 'help') setHelpDialog(true)
    else if (event.payload === 'show-terminal' && tabs.length) setWorkspaceView('terminal')
    else if (event.payload === 'show-files' && tabs.length) setWorkspaceView('files')
    else if (event.payload === 'toggle-sidebar') setSidebarCollapsed((collapsed) => !collapsed)
  }), [activeBookmark, activeTab, tabs, bookmarks])

  const saveSidebarWidth = (width: number): void => {
    const next = clampSidebarWidth(width)
    sidebarWidthRef.current = next
    setSidebarWidth(next)
    void window.api.state.setSidebarWidth(next).catch((error) => showError(errorMessage(error)))
  }

  const startSidebarResize = (event: React.PointerEvent<HTMLDivElement>): void => {
    if (event.button !== 0) return
    event.preventDefault()
    const startX = event.clientX
    const startWidth = sidebarWidthRef.current
    document.body.classList.add('sidebar-resizing')
    const move = (moveEvent: PointerEvent): void => {
      const next = clampSidebarWidth(startWidth + moveEvent.clientX - startX)
      sidebarWidthRef.current = next
      setSidebarWidth(next)
    }
    const finish = (): void => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', finish)
      window.removeEventListener('pointercancel', finish)
      document.body.classList.remove('sidebar-resizing')
      void window.api.state.setSidebarWidth(sidebarWidthRef.current).catch((error) => showError(errorMessage(error)))
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', finish)
    window.addEventListener('pointercancel', finish)
  }

  const resizeSidebarWithKeyboard = (event: React.KeyboardEvent<HTMLDivElement>): void => {
    if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return
    event.preventDefault()
    if (event.key === 'Home') saveSidebarWidth(minSidebarWidth)
    else if (event.key === 'End') saveSidebarWidth(maxSidebarWidth)
    else saveSidebarWidth(sidebarWidth + (event.key === 'ArrowLeft' ? -10 : 10))
  }

  const contextBookmark = sidebarContextMenu?.bookmark ?? selectedBookmark

  return <div className={`app-shell platform-${window.api.platform}`} data-theme={resolvedUiTheme} data-theme-mode={activeThemeMode}>
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="app-brand">{selectedAppIcon ? <img className="app-logo" src={selectedAppIcon.dataUrl} alt="" /> : <span className="app-logo app-logo-fallback">&gt;_</span>}<span>SSH Client</span></div>
      <span className="titlebar-spacer" />
      <button className="icon-button titlebar-help" title="帮助 (F1)" aria-label="帮助" onClick={() => setHelpDialog(true)}><CircleHelp size={18} /></button>
      <button className="icon-button titlebar-settings" title="设置" aria-label="设置" onClick={() => openSettings()}><SettingsIcon size={18} /></button>
      <div className="window-controls">
        <button className="window-control" title="最小化" aria-label="最小化" onClick={() => void window.api.system.minimizeWindow().catch((error) => showError(errorMessage(error)))}><Minus size={16} /></button>
        <button className="window-control" title="最大化或还原" aria-label="最大化或还原" onClick={() => void window.api.system.toggleMaximizeWindow().catch((error) => showError(errorMessage(error)))}><Square size={13} /></button>
        <button className="window-control close" title="关闭" aria-label="关闭" onClick={() => void window.api.system.closeWindow().catch((error) => showError(errorMessage(error)))}><X size={17} /></button>
      </div>
    </header>
    <div className={`main-layout ${sidebarCollapsed ? 'sidebar-collapsed' : ''}`} style={{ '--sidebar-width': `${sidebarWidth}px` } as React.CSSProperties}>
      <aside className="sidebar" onContextMenu={(event) => openSidebarContextMenu(event)}>
        <div className="sidebar-heading"><strong>连接管理</strong><div className="sidebar-actions">
          <button className="icon-button" title="新增连接" onClick={() => setBookmarkDialog('new')}><CirclePlus size={18} /></button>
          <button className="icon-button" title="删除选中连接" disabled={!selectedBookmark} onClick={() => selectedBookmark && void removeBookmark(selectedBookmark)}><Trash2 size={17} /></button>
        </div></div>
        <label className="search-box"><Search size={16} /><input placeholder="搜索连接" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
        <div className="bookmark-list">
          {query && filteredBookmarks.length === 0 && <div className="sidebar-empty"><BookmarkIcon size={24} /><span>没有匹配的连接</span></div>}
          {bookmarkGroups.flatMap(([group, items]) => {
            const collapsed = !query && collapsedBookmarkGroups.has(group)
            const groupDropClass = groupDrop?.group === group ? `drop-${groupDrop.position}` : ''
            return [<button type="button" data-group-name={group} className={`bookmark-group ${draggedGroupName === group ? 'dragging' : ''} ${groupDropClass}`} key={`group:${group}`} aria-expanded={!collapsed} onPointerDown={(event) => startSidebarPointerDrag(event, 'group', group)} onClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } toggleBookmarkGroup(group) }} onContextMenu={(event) => openSidebarContextMenu(event, { group })}>
              <span className="group-drag-handle" title={query ? '清除搜索后可拖动排序' : '拖动分组排序'}><GripVertical size={13} /></span>{collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}{collapsed ? <Folder size={14} /> : <FolderOpen size={14} />}<span>{group || '未分组'}</span><small>{items.length}</small>
            </button>, ...(collapsed ? [] : items.map((bookmark) => {
            const tab = tabs.find((item) => item.bookmarkId === bookmark.id)
            const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
            const dropClass = bookmarkDrop?.id === bookmark.id ? `drop-${bookmarkDrop.position}` : ''
            return <button key={bookmark.id} data-bookmark-id={bookmark.id} className={`bookmark-item ${selectedBookmark?.id === bookmark.id ? 'active' : ''} ${draggedBookmarkId === bookmark.id ? 'dragging' : ''} ${dropClass}`} onPointerDown={(event) => startSidebarPointerDrag(event, 'bookmark', bookmark.id)} onClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } setSelectedBookmarkId(bookmark.id) }} onDoubleClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } openBookmarkFromSidebar(bookmark) }} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); setSelectedBookmarkId(bookmark.id); openBookmarkFromSidebar(bookmark) } }} onContextMenu={(event) => openSidebarContextMenu(event, { bookmark })}>
              <span className="bookmark-drag-handle" title={query ? '清除搜索后可拖动排序' : '拖动排序'}><GripVertical size={14} /></span>
              <span className={`server-icon ${tab?.status ?? 'disconnected'}`}><Server size={17} /></span>
              <span className="bookmark-copy"><strong>{bookmark.name}</strong><small title={jumpBookmark ? `通过 ${jumpBookmark.name}` : undefined}>{bookmark.username}@{bookmark.host}:{bookmark.port}{jumpBookmark ? ` · 经 ${jumpBookmark.name}` : ''}</small></span>
              <span className="bookmark-flags">{bookmark.favorite && <Star size={12} className="favorite-star" fill="currentColor" />}{bookmark.hasSavedCredential && <KeyRound size={12} className="saved-key" />}</span>
            </button>
          }))]})}
        </div>
      </aside>
      <div className="sidebar-resizer" role="separator" aria-label="调整侧边栏宽度" aria-orientation="vertical" aria-valuemin={minSidebarWidth} aria-valuemax={maxSidebarWidth} aria-valuenow={sidebarWidth} tabIndex={0} title="拖动调整侧边栏宽度，双击恢复默认" onPointerDown={startSidebarResize} onDoubleClick={() => saveSidebarWidth(defaultSidebarWidth)} onKeyDown={resizeSidebarWithKeyboard} />
      <main className={`workspace ${tabs.length && workspaceView === 'files' ? 'file-mode' : ''}`}>
        <div className="tabbar">
          <button className="icon-button tab-sidebar-toggle" title={sidebarCollapsed ? '展开连接管理' : '折叠连接管理'} aria-label={sidebarCollapsed ? '展开连接管理' : '折叠连接管理'} aria-expanded={!sidebarCollapsed} onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}>
            {sidebarCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
          </button>
          {tabs.map((tab) => { const bookmark = bookmarkMap.get(tab.bookmarkId); return <div key={tab.key} className={`session-tab ${tab.key === activeKey ? 'active' : ''}`}>
            <button className="session-tab-select" title={bookmark ? `${tabLabel(tab, bookmark)} · ${bookmark.username}@${bookmark.host}:${bookmark.port}` : undefined} onClick={() => setActiveKey(tab.key)} onAuxClick={(event) => { if (event.button === 1) closeTab(tab) }}><span className={`status-dot ${tab.status}`} /><span>{tabLabel(tab, bookmark)}</span></button>
            <button className="session-tab-close" title="关闭会话" aria-label={`关闭 ${bookmark?.name ?? '会话'}`} onClick={() => closeTab(tab)}><X size={14} /></button>
          </div> })}
          <button className="icon-button tab-add" title="新增连接" onClick={() => setBookmarkDialog('new')}><CirclePlus size={17} /></button>
        </div>
        {tabs.length === 0 ? <div className="welcome-state"><div className="welcome-icon"><Plug size={30} /></div><h2>选择一个 SSH 连接</h2><p>连接后可在同一窗口使用终端和 SFTP 文件传输。</p><button className="primary-button" onClick={() => setBookmarkDialog('new')}><CirclePlus size={16} />新增连接</button></div> : <>
          <div className="session-toolbar"><div className="session-info"><span className={`status-dot ${activeTab?.status}`} /><span className="session-status">{activeTab ? statusText[activeTab.status] : ''}</span>{activeBookmark && <span className="session-endpoint" title={`${activeBookmark.username}@${activeBookmark.host}:${activeBookmark.port}`}>{activeBookmark.username}@{activeBookmark.host}:{activeBookmark.port}</span>}{activeTab?.error && <span className="error-text" title={activeTab.error}>{activeTab.error}</span>}</div>
            <div className="view-switcher" role="tablist" aria-label="会话视图">
              <button role="tab" aria-selected={workspaceView === 'terminal'} className={workspaceView === 'terminal' ? 'active' : ''} onClick={() => setWorkspaceView('terminal')}><SquareTerminal size={15} />终端</button>
              <button role="tab" aria-selected={workspaceView === 'files'} className={workspaceView === 'files' ? 'active' : ''} onClick={() => setWorkspaceView('files')}><FolderOpen size={15} />文件传输</button>
            </div><div className="session-actions">
            {workspaceView === 'files' && activeBookmark && activeTab?.sessionId && <button className="secondary-button" onClick={() => setDeploymentDialog(true)}><Rocket size={15} />部署</button>}
            {workspaceView === 'terminal' && activeBookmark && <button className="secondary-button" onClick={() => setAiDialog(true)}><WandSparkles size={15} />AI 命令</button>}
            {activeTab?.status === 'connected' && activeTab.sessionId && <button className="secondary-button" onClick={() => setTunnelDialog(true)}><Network size={15} />端口转发</button>}
            {activeTab?.status === 'connected' && <button className="secondary-button" onClick={() => activeTab.sessionId && void window.api.sessions.disconnect(activeTab.sessionId).catch((error) => showError(errorMessage(error)))}><Power size={15} />断开</button>}
          </div></div>
          <div className="session-stack">
            {tabs.map((tab) => { const bookmark = bookmarkMap.get(tab.bookmarkId); const reconnect = bookmark ? () => openBookmark(bookmark, { tabKey: tab.key }) : undefined; return <div key={tab.key} className={`session-view ${tab.key === activeKey ? 'active' : ''} ${workspaceView}`}>
              <div className="terminal-region"><TerminalPane sessionId={tab.sessionId} visible={tab.key === activeKey && workspaceView === 'terminal'} settings={terminalSettings} backgroundImage={terminalBackground} onConnect={reconnect} onOpenSettings={() => openSettings('terminal')} /></div>
              <div className="sftp-region"><SftpPane sessionId={tab.sessionId} bookmarkId={tab.bookmarkId} connected={tab.status === 'connected'} visible={tab.key === activeKey && workspaceView === 'files'} onError={showError} onConnect={reconnect} /></div>
            </div> })}
          </div>
        </>}
        {tabs.length > 0 && workspaceView === 'files' && <TransferPanel transfers={transfers} view={transferView} setView={setTransferView} onRetry={(task) => void retryTransfer(task)} onCancel={(task) => void cancelTransfer(task)} onClear={() => void clearCompletedTransfers()} />}
      </main>
    </div>
    {sidebarContextMenu && <div className="sidebar-context-menu" role="menu" style={{ left: sidebarContextMenu.x, top: sidebarContextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      {sidebarContextMenu.group !== undefined ? <>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setGroupDialog({ mode: 'create' }) }}><FolderPlus size={15} />新建分组</button>
        <button role="menuitem" onClick={() => { const group = sidebarContextMenu.group ?? ''; setSidebarContextMenu(null); setGroupDialog({ mode: 'rename', group }) }}><Edit3 size={15} />重命名分组</button>
        <button role="menuitem" className="danger" onClick={() => { const group = sidebarContextMenu.group ?? ''; setSidebarContextMenu(null); void deleteBookmarkGroup(group) }}><Trash2 size={15} />删除分组及连接</button>
      </> : <>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); void chooseSshConfig() }}><FileInput size={15} />导入连接</button>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setBookmarkDialog('new') }}><CirclePlus size={15} />新增连接</button>
        <button role="menuitem" disabled={!contextBookmark} onClick={() => { setSidebarContextMenu(null); if (contextBookmark) setBookmarkDialog(contextBookmark) }}><Edit3 size={15} />编辑连接</button>
        <button role="menuitem" disabled={!contextBookmark} onClick={() => { setSidebarContextMenu(null); if (contextBookmark) void duplicateBookmark(contextBookmark) }}><Copy size={15} />复制配置</button>
        <button role="menuitem" disabled={!contextBookmark} className="danger" onClick={() => { setSidebarContextMenu(null); if (contextBookmark) void removeBookmark(contextBookmark) }}><Trash2 size={15} />删除连接</button>
        <div className="context-menu-separator" />
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setGroupDialog({ mode: 'create' }) }}><FolderPlus size={15} />新建分组</button>
      </>}
    </div>}
    {groupDialog && <GroupNameDialog mode={groupDialog.mode} initialName={groupDialog.group ?? ''} onClose={() => setGroupDialog(null)} onSave={(name) => void saveGroup(name)} />}
    {bookmarkDialog && <BookmarkDialog bookmark={bookmarkDialog === 'new' ? undefined : bookmarkDialog} connections={bookmarks} groups={bookmarkGroupNames} onClose={() => setBookmarkDialog(null)} onSaved={async () => { setBookmarkDialog(null); await reloadSidebarData() }} onError={showError} />}
    {authRequest && <AuthDialog bookmark={authRequest.bookmark} jumpBookmark={authRequest.bookmark.jumpBookmarkId ? bookmarkMap.get(authRequest.bookmark.jumpBookmarkId) : undefined} onClose={() => setAuthRequest(null)} onConnect={(credentials) => void connect(authRequest.bookmark, credentials, authRequest.target)} />}
    {hostPrompts[0] && <HostKeyDialog prompt={hostPrompts[0]} onDecision={(accept) => { const prompt = hostPrompts[0]; if (!prompt) return; window.api.sessions.hostKeyDecision(prompt.sessionId, accept); setHostPrompts((current) => current.filter((item) => item !== prompt)) }} />}
    {conflict && <ConflictDialog conflict={conflict} onDecision={(resolution, apply) => { window.api.transfers.resolveConflict(conflict.taskId, resolution, apply); setConflict(null) }} />}
    {settingsDialog && <SettingsDialog initialSection={settingsInitialSection} settings={terminalSettings} backgroundImage={terminalBackground} appIcons={appIcons} uiTheme={uiTheme} aiSettings={aiSettings} onAiSettingsChange={setAiSettings} onThemePreview={setUiThemePreview} onClose={() => { setUiThemePreview(null); setSettingsDialog(false) }} onSave={(settings, image, icon, theme, ai) => void saveSettings(settings, image, icon, theme, ai)} onError={showError} />}
    {aiDialog && activeBookmark && activeTab && <AiCommandDialog bookmark={activeBookmark} sessionId={activeTab.sessionId} connected={activeTab.status === 'connected'} settings={aiSettings} onSettingsChange={setAiSettings} onSettings={() => { setAiDialog(false); openSettings('ai') }} onClose={() => setAiDialog(false)} onError={showError} />}
    {helpDialog && <HelpDialog onClose={() => setHelpDialog(false)} />}
    {deploymentDialog && activeBookmark && activeTab?.sessionId && <DeploymentDialog bookmark={activeBookmark} sessionId={activeTab.sessionId} onClose={() => setDeploymentDialog(false)} onError={showError} />}
    {tunnelDialog && activeBookmark && activeTab?.sessionId && <TunnelDialog bookmark={activeBookmark} sessionId={activeTab.sessionId} onClose={() => setTunnelDialog(false)} onError={showError} />}
    {importPreview && <SshConfigImportDialog preview={importPreview} onClose={() => setImportPreview(null)} onImported={async () => { setImportPreview(null); await reloadSidebarData() }} onError={showError} />}
    {toast && <div className="toast">{toast}</div>}
  </div>
}

function AiCommandDialog({ bookmark, sessionId, connected, settings, onSettingsChange, onSettings, onClose, onError }: { bookmark: Bookmark; sessionId?: string; connected: boolean; settings: AiSettings; onSettingsChange(settings: AiSettings): void; onSettings(): void; onClose(): void; onError(message: string): void }): React.JSX.Element {
  const [requirement, setRequirement] = useState('')
  const [shell, setShell] = useState<AiShell>(settings.defaultShell)
  const [suggestion, setSuggestion] = useState<AiCommandSuggestion | null>(null)
  const [command, setCommand] = useState('')
  const [edited, setEdited] = useState(false)
  const [loading, setLoading] = useState(false)
  const [executing, setExecuting] = useState(false)
  const [error, setError] = useState('')
  const [confirming, setConfirming] = useState<AiRiskAssessment | null>(null)
  const [confirmationText, setConfirmationText] = useState('')

  const effectiveAssessment = (local: AiRiskAssessment): AiRiskAssessment => {
    if (!suggestion || edited) return local
    const rank = { low: 0, medium: 1, high: 2 } as const
    const riskLevel = rank[suggestion.riskLevel] > rank[local.riskLevel] ? suggestion.riskLevel : local.riskLevel
    return { riskLevel, warnings: [...new Set([...suggestion.warnings, ...local.warnings])] }
  }

  const selectShell = async (value: AiShell): Promise<void> => {
    setShell(value)
    try {
      const saved = await window.api.ai.saveSettings({ baseUrl: settings.baseUrl, model: settings.model, defaultShell: value })
      onSettingsChange(saved)
    } catch (saveError) { onError(errorMessage(saveError)) }
  }
  const generate = async (): Promise<void> => {
    if (!requirement.trim()) return
    setLoading(true); setError(''); setSuggestion(null); setCommand(''); setEdited(false)
    try {
      const result = await window.api.ai.generate(requirement.trim(), shell)
      setSuggestion(result); setCommand(result.command)
    } catch (generateError) { setError(errorMessage(generateError)) }
    finally { setLoading(false) }
  }
  const requireConnectedSession = (): string | null => {
    if (!connected || !sessionId) { setError('当前 SSH 会话已断开，无法发送命令'); return null }
    return sessionId
  }
  const fillCommand = async (): Promise<void> => {
    const targetSession = requireConnectedSession()
    if (!targetSession) return
    try {
      const assessment = effectiveAssessment(await window.api.ai.analyze(command))
      setSuggestion((current) => current ? { ...current, riskLevel: assessment.riskLevel, warnings: assessment.warnings } : current)
      await window.api.sessions.writeCommand(targetSession, command.trim())
      onClose()
    } catch (fillError) { setError(errorMessage(fillError)) }
  }
  const executeConfirmed = async (): Promise<void> => {
    const targetSession = requireConnectedSession()
    if (!targetSession) return
    setExecuting(true)
    try {
      await window.api.ai.analyze(command)
      await window.api.sessions.writeCommand(targetSession, `${command.trim()}\r`)
      onClose()
    } catch (executeError) { setError(errorMessage(executeError)); setConfirming(null); setExecuting(false) }
  }
  const requestExecution = async (): Promise<void> => {
    const targetSession = requireConnectedSession()
    if (!targetSession) return
    try {
      const assessment = effectiveAssessment(await window.api.ai.analyze(command))
      setSuggestion((current) => current ? { ...current, riskLevel: assessment.riskLevel, warnings: assessment.warnings } : current)
      if (assessment.riskLevel === 'high') { setConfirmationText(''); setConfirming(assessment); return }
      const accepted = await confirmDialog(`目标：${bookmark.username}@${bookmark.host}:${bookmark.port}\n\n${command.trim()}`, { title: assessment.riskLevel === 'medium' ? '确认执行有风险的命令' : '确认执行命令', kind: assessment.riskLevel === 'medium' ? 'warning' : 'info' })
      if (accepted) await executeConfirmed()
    } catch (executeError) { setError(errorMessage(executeError)) }
  }
  const riskText = suggestion ? ({ low: '低风险', medium: '需注意', high: '高风险' } as const)[suggestion.riskLevel] : ''

  if (confirming) return <Modal title="确认执行高风险命令" onClose={() => setConfirming(null)} wide><div className="ai-danger-confirm">
    <div className="ai-danger-heading"><ShieldAlert size={22} /><div><strong>该命令可能造成不可逆修改</strong><span>{bookmark.username}@{bookmark.host}:{bookmark.port}</span></div></div>
    <pre>{command.trim()}</pre>
    {confirming.warnings.length > 0 && <ul>{confirming.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
    <label>输入“执行”以继续<input autoFocus value={confirmationText} onChange={(event) => setConfirmationText(event.target.value)} /></label>
    <div className="dialog-actions"><button className="secondary-button" disabled={executing} onClick={() => setConfirming(null)}>取消</button><button className="danger-button" disabled={confirmationText !== '执行' || executing} onClick={() => void executeConfirmed()}><Play size={15} />{executing ? '正在执行…' : '执行命令'}</button></div>
  </div></Modal>

  return <Modal title="AI 命令助手" onClose={onClose} wide><div className="ai-command-dialog">
    <div className="ai-target"><Server size={16} /><strong>{bookmark.name}</strong><span>{bookmark.username}@{bookmark.host}:{bookmark.port}</span><span className={`status-dot ${connected ? 'connected' : 'disconnected'}`} /></div>
    {!settings.model.trim() ? <div className="ai-unconfigured"><Bot size={28} /><strong>尚未配置 AI 模型</strong><button className="primary-button" onClick={onSettings}><SettingsIcon size={15} />打开 AI 设置</button></div> : <>
      <div className="ai-prompt-row"><label>目标 Shell<select value={shell} onChange={(event) => void selectShell(event.target.value as AiShell)}><option value="linux">Linux Shell</option><option value="powerShell">PowerShell</option><option value="cmd">Windows cmd</option><option value="macos">macOS Shell</option></select></label><label className="ai-requirement">需要完成什么<textarea autoFocus rows={4} maxLength={8000} value={requirement} onChange={(event) => setRequirement(event.target.value)} placeholder="例如：从 app.log 中找出更新条数大于 0 的日志" onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); void generate() } }} /></label></div>
      <div className="ai-generate-actions"><button className="primary-button" disabled={loading || !requirement.trim()} onClick={() => void generate()}><WandSparkles size={15} />{loading ? '正在生成…' : suggestion ? '重新生成' : '生成命令'}</button></div>
      {error && <div className="ai-error">{error}</div>}
      {suggestion && <div className="ai-result">
        <div className="ai-result-heading"><strong>生成结果</strong><span className={`ai-risk ${suggestion.riskLevel}`}>{riskText}</span>{edited && <span className="ai-edited">命令已修改，原说明可能不再适用</span>}</div>
        <textarea className="ai-command-editor" rows={3} spellCheck={false} value={command} onChange={(event) => { setCommand(event.target.value); setEdited(event.target.value !== suggestion.command) }} />
        {suggestion.explanation && <p>{suggestion.explanation}</p>}
        {suggestion.assumptions.length > 0 && <div className="ai-notes"><strong>假设</strong><ul>{suggestion.assumptions.map((item) => <li key={item}>{item}</li>)}</ul></div>}
        {suggestion.warnings.length > 0 && <div className="ai-notes warning"><strong>注意</strong><ul>{suggestion.warnings.map((item) => <li key={item}>{item}</li>)}</ul></div>}
        <div className="dialog-actions spread"><button className="secondary-button" onClick={() => void window.api.system.writeClipboard(command).catch((copyError) => setError(errorMessage(copyError)))}><Copy size={15} />复制</button><div><button className="secondary-button" disabled={!connected || !command.trim() || executing} onClick={() => void fillCommand()}><Send size={15} />填入终端</button><button className="primary-button" disabled={!connected || !command.trim() || executing} onClick={() => void requestExecution()}><Play size={15} />执行</button></div></div>
      </div>}
    </>}
  </div></Modal>
}

function SshConfigImportDialog({ preview, onClose, onImported, onError }: { preview: SshConfigPreview; onClose(): void; onImported(): void; onError(message: string): void }): React.JSX.Element {
  const [selected, setSelected] = useState(() => new Set(preview.entries.map((item) => item.alias)))
  const [loading, setLoading] = useState(false)
  const toggle = (alias: string): void => setSelected((current) => {
    const next = new Set(current)
    if (next.has(alias)) next.delete(alias); else next.add(alias)
    return next
  })
  const importSelected = async (): Promise<void> => {
    setLoading(true)
    try { await window.api.bookmarks.importSshConfig(preview.path, [...selected]); onImported() }
    catch (error) { onError(errorMessage(error)) }
    finally { setLoading(false) }
  }
  return <Modal title="导入 OpenSSH Config" onClose={onClose} wide><div className="ssh-import-dialog">
    <div className="import-source" title={preview.path}>{preview.path}</div>
    <div className="import-list">{preview.entries.length === 0 ? <div className="import-empty">没有找到可导入的具体 Host 配置</div> : preview.entries.map((entry) => <label key={entry.alias}>
      <input type="checkbox" checked={selected.has(entry.alias)} onChange={() => toggle(entry.alias)} />
      <span><strong>{entry.name}</strong><small>{entry.username}@{entry.host}:{entry.port}{entry.proxyJumpAlias ? ` · 经 ${entry.proxyJumpAlias}` : ''}</small></span>
      <em>{entry.privateKeyPath ? '私钥' : 'Agent'}</em>
    </label>)}</div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" disabled={!selected.size || loading} onClick={() => void importSelected()}>{loading ? '正在导入…' : `导入 ${selected.size} 个连接`}</button></div>
  </div></Modal>
}

function TunnelDialog({ bookmark, sessionId, onClose, onError }: { bookmark: Bookmark; sessionId: string; onClose(): void; onError(message: string): void }): React.JSX.Element {
  const empty = (): PortForwardProfile => ({ id: crypto.randomUUID(), bookmarkId: bookmark.id, name: '', type: 'local', bindAddress: '127.0.0.1', bindPort: 0, targetHost: '127.0.0.1', targetPort: 80 })
  const [profiles, setProfiles] = useState<PortForwardProfile[]>([])
  const [profile, setProfile] = useState<PortForwardProfile>(empty)
  const [tunnels, setTunnels] = useState<TunnelSummary[]>([])
  const [loading, setLoading] = useState(false)
  useEffect(() => {
    void Promise.all([window.api.tunnels.listProfiles(bookmark.id), window.api.tunnels.list(sessionId)]).then(([items, active]) => {
      setProfiles(items); setTunnels(active); if (items[0]) setProfile(items[0])
    }).catch((error) => onError(errorMessage(error)))
    return window.api.onEvent((event) => {
      if (event.type !== 'tunnel' || event.payload.sessionId !== sessionId) return
      setTunnels((current) => event.payload.removed ? current.filter((item) => item.id !== event.payload.id) : [event.payload, ...current.filter((item) => item.id !== event.payload.id)])
    })
  }, [bookmark.id, sessionId])
  const set = <K extends keyof PortForwardProfile>(key: K, value: PortForwardProfile[K]): void => setProfile((current) => ({ ...current, [key]: value }))
  const active = tunnels.find((item) => item.profileId === profile.id)
  const save = async (): Promise<PortForwardProfile | undefined> => {
    try {
      const saved = await window.api.tunnels.saveProfile(profile)
      setProfile(saved); setProfiles((current) => [saved, ...current.filter((item) => item.id !== saved.id)])
      return saved
    } catch (error) { onError(errorMessage(error)); return undefined }
  }
  const start = async (): Promise<void> => {
    const saved = await save(); if (!saved) return
    setLoading(true)
    try {
      const tunnel = await window.api.tunnels.start(sessionId, saved.id)
      setTunnels((current) => [tunnel, ...current.filter((item) => item.id !== tunnel.id)])
    } catch (error) { onError(errorMessage(error)); setTunnels(await window.api.tunnels.list(sessionId)) }
    finally { setLoading(false) }
  }
  const stop = async (): Promise<void> => {
    if (!active) return
    setLoading(true)
    try { await window.api.tunnels.stop(sessionId, active.id); setTunnels((current) => current.filter((item) => item.id !== active.id)) }
    catch (error) { onError(errorMessage(error)) }
    finally { setLoading(false) }
  }
  return <Modal title={`端口转发 · ${bookmark.name}`} onClose={onClose} wide><div className="tunnel-dialog">
    <div className="deployment-profile-bar"><select value={profile.id} onChange={(event) => { const selected = profiles.find((item) => item.id === event.target.value); if (selected) setProfile(selected) }}><option value={profile.id}>{profiles.some((item) => item.id === profile.id) ? profile.name || '未命名配置' : '新转发配置'}</option>{profiles.filter((item) => item.id !== profile.id).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select><button className="secondary-button" onClick={() => setProfile(empty())}><CirclePlus size={15} />新建</button>{profiles.some((item) => item.id === profile.id) && <button className="icon-button danger" title="删除转发配置" onClick={async () => { if (active) await stop(); await window.api.tunnels.removeProfile(profile.id); const next = profiles.filter((item) => item.id !== profile.id); setProfiles(next); setProfile(next[0] ?? empty()) }}><Trash2 size={15} /></button>}</div>
    <div className="form-grid">
      <label>配置名称<input value={profile.name} onChange={(event) => set('name', event.target.value)} placeholder="Web 服务" /></label>
      <label>转发类型<select value={profile.type} disabled={Boolean(active)} onChange={(event) => set('type', event.target.value as PortForwardProfile['type'])}><option value="local">本地转发</option><option value="remote">远端转发</option><option value="dynamic">SOCKS5 动态转发</option></select></label>
      <div className="form-row"><label>监听地址<input value={profile.bindAddress} disabled={Boolean(active)} onChange={(event) => set('bindAddress', event.target.value)} /></label><label>监听端口<input type="number" min="0" max="65535" disabled={Boolean(active)} value={profile.bindPort} onChange={(event) => set('bindPort', Number(event.target.value))} /></label></div>
      {profile.type !== 'dynamic' && <div className="form-row"><label>目标地址<input value={profile.targetHost} disabled={Boolean(active)} onChange={(event) => set('targetHost', event.target.value)} /></label><label>目标端口<input type="number" min="1" max="65535" disabled={Boolean(active)} value={profile.targetPort} onChange={(event) => set('targetPort', Number(event.target.value))} /></label></div>}
      {active && <div className={`tunnel-status ${active.status}`}><span className={`status-dot ${active.status === 'running' ? 'connected' : active.status === 'error' ? 'error' : 'connecting'}`} /><strong>{active.status === 'running' ? `${active.bindAddress}:${active.bindPort}` : active.status === 'starting' ? '正在启动' : '启动失败'}</strong>{active.error && <small>{active.error}</small>}</div>}
    </div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>关闭</button><button className="secondary-button" disabled={Boolean(active) || loading} onClick={() => void save()}>保存</button>{active ? <button className="danger-button" disabled={loading} onClick={() => void stop()}>停止</button> : <button className="primary-button" disabled={loading} onClick={() => void start()}><Power size={15} />启动</button>}</div>
  </div></Modal>
}

function DeploymentDialog({ bookmark, sessionId, onClose, onError }: { bookmark: Bookmark; sessionId: string; onClose(): void; onError(message: string): void }): React.JSX.Element {
  const empty = (): DeploymentProfile => ({ id: crypto.randomUUID(), bookmarkId: bookmark.id, name: '', localDirectory: '', remoteDirectory: '/', deleteExtraneous: false })
  const [profiles, setProfiles] = useState<DeploymentProfile[]>([])
  const [profile, setProfile] = useState<DeploymentProfile>(empty)
  const [diff, setDiff] = useState<DeploymentDiffEntry[] | null>(null)
  const [loading, setLoading] = useState(false)
  useEffect(() => { void window.api.deployments.list(bookmark.id).then((items) => { setProfiles(items); if (items[0]) setProfile(items[0]) }).catch((error) => onError(errorMessage(error))) }, [bookmark.id])
  const set = <K extends keyof DeploymentProfile>(key: K, value: DeploymentProfile[K]): void => { setProfile((current) => ({ ...current, [key]: value })); setDiff(null) }
  const save = async (): Promise<DeploymentProfile | undefined> => {
    try {
      const saved = await window.api.deployments.save(profile)
      setProfile(saved); setProfiles((current) => [saved, ...current.filter((item) => item.id !== saved.id)])
      return saved
    } catch (error) { onError(errorMessage(error)); return undefined }
  }
  const preview = async (): Promise<void> => {
    const saved = await save(); if (!saved) return
    setLoading(true)
    try { setDiff(await window.api.deployments.preview(saved.id, sessionId)) }
    catch (error) { onError(errorMessage(error)) }
    finally { setLoading(false) }
  }
  const execute = async (): Promise<void> => {
    if (!diff) return
    const changed = diff.filter((item) => ['new', 'changed'].includes(item.status)).length
    const removals = profile.deleteExtraneous ? diff.filter((item) => item.status === 'remote-only').length : 0
    if (!await confirmDialog(`将上传 ${changed} 个文件${removals ? `，并在上传全部成功后删除 ${removals} 个远端文件` : ''}。确定继续吗？`, { title: '确认部署', kind: 'warning' })) return
    try { await window.api.deployments.execute(profile.id, sessionId); onClose() }
    catch (error) { onError(errorMessage(error)) }
  }
  const counts = diff ? Object.fromEntries(['new', 'changed', 'same', 'remote-only'].map((status) => [status, diff.filter((item) => item.status === status).length])) : {}
  return <Modal title={`部署到 ${bookmark.name}`} onClose={onClose} wide><div className="deployment-dialog">
    <div className="deployment-profile-bar"><select value={profile.id} onChange={(event) => { const selected = profiles.find((item) => item.id === event.target.value); if (selected) { setProfile(selected); setDiff(null) } }}><option value={profile.id}>{profiles.some((item) => item.id === profile.id) ? profile.name || '未命名配置' : '新部署配置'}</option>{profiles.filter((item) => item.id !== profile.id).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select><button className="secondary-button" onClick={() => { setProfile(empty()); setDiff(null) }}><CirclePlus size={15} />新建</button>{profiles.some((item) => item.id === profile.id) && <button className="icon-button danger" title="删除部署配置" onClick={async () => { await window.api.deployments.remove(profile.id); const next = profiles.filter((item) => item.id !== profile.id); setProfiles(next); setProfile(next[0] ?? empty()); setDiff(null) }}><Trash2 size={15} /></button>}</div>
    <div className="form-grid"><label>配置名称<input value={profile.name} onChange={(event) => set('name', event.target.value)} /></label><label>本地目录<div className="input-button"><input readOnly value={profile.localDirectory} /><button className="secondary-button" onClick={async () => { const path = await window.api.files.chooseLocalDirectory(); if (path) set('localDirectory', path) }}>选择</button></div></label><label>远端目录<input value={profile.remoteDirectory} onChange={(event) => set('remoteDirectory', event.target.value)} /></label><label className="check-label"><input type="checkbox" checked={profile.deleteExtraneous} onChange={(event) => set('deleteExtraneous', event.target.checked)} />上传全部成功后删除远端多余文件</label></div>
    {diff && <><div className="deployment-summary"><span>新增 {counts['new'] ?? 0}</span><span>变化 {counts['changed'] ?? 0}</span><span>相同 {counts['same'] ?? 0}</span><span>仅远端 {counts['remote-only'] ?? 0}</span></div><div className="deployment-diff">{diff.filter((item) => item.status !== 'same').slice(0, 300).map((item) => <div key={`${item.status}:${item.relativePath}`}><span className={item.status}>{({ new: '新增', changed: '更新', 'remote-only': '远端', same: '相同' } as const)[item.status]}</span><code>{item.relativePath}</code></div>)}</div></>}
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>取消</button><button className="secondary-button" disabled={loading} onClick={() => void preview()}>{loading ? '正在比较…' : '保存并预览'}</button><button className="primary-button" disabled={!diff || loading} onClick={() => void execute()}><Rocket size={15} />开始部署</button></div>
  </div></Modal>
}

function SettingsDialog({ initialSection, settings, backgroundImage, appIcons, uiTheme, aiSettings, onAiSettingsChange, onThemePreview, onClose, onSave, onError }: { initialSection: SettingsSection; settings: TerminalSettings; backgroundImage: string; appIcons: AppIconSettings; uiTheme: UiTheme; aiSettings: AiSettings; onAiSettingsChange(settings: AiSettings): void; onThemePreview(theme: UiTheme): void; onClose(): void; onSave(settings: TerminalSettings, backgroundImage: string, appIcon: AppIconId, uiTheme: UiTheme, ai: AiSettingsInput): void; onError(message: string): void }): React.JSX.Element {
  const [form, setForm] = useState<TerminalSettings>({ ...settings })
  const [previewImage, setPreviewImage] = useState(backgroundImage)
  const [appIcon, setAppIcon] = useState(appIcons.selected)
  const [theme, setTheme] = useState(uiTheme)
  const [aiForm, setAiForm] = useState<AiSettings>({ ...aiSettings })
  const [apiKey, setApiKey] = useState('')
  const [testingAi, setTestingAi] = useState(false)
  const [aiTested, setAiTested] = useState(false)
  const [diagnosticPath, setDiagnosticPath] = useState('')
  const [section, setSection] = useState<SettingsSection>(initialSection)
  const set = <K extends keyof TerminalSettings>(key: K, value: TerminalSettings[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const setAi = <K extends keyof AiSettings>(key: K, value: AiSettings[K]): void => { setAiTested(false); setAiForm((current) => ({ ...current, [key]: value })) }
  const aiInput = (): AiSettingsInput => ({ baseUrl: aiForm.baseUrl, model: aiForm.model, defaultShell: aiForm.defaultShell, ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}) })
  const selectTheme = (value: UiTheme): void => { setTheme(value); onThemePreview(value) }
  const chooseBackground = async (): Promise<void> => {
    try {
      const path = await window.api.settings.chooseTerminalBackground()
      if (!path) return
      const image = await window.api.settings.loadTerminalBackground(path)
      set('backgroundImagePath', path)
      setPreviewImage(image)
    } catch (error) { onError(errorMessage(error)) }
  }
  const reset = (): void => {
    if (section === 'ai') { setAiForm({ ...DEFAULT_AI_SETTINGS, apiKeyConfigured: aiForm.apiKeyConfigured }); setApiKey(''); setAiTested(false); return }
    setForm({ ...DEFAULT_TERMINAL_SETTINGS }); setPreviewImage(''); setAppIcon('ssh'); selectTheme('system')
  }
  const testAi = async (): Promise<void> => {
    setTestingAi(true); setAiTested(false)
    try { await window.api.ai.testSettings(aiInput()); setAiTested(true) }
    catch (error) { onError(errorMessage(error)) }
    finally { setTestingAi(false) }
  }
  const deleteAiKey = async (): Promise<void> => {
    if (!await confirmDialog('确定删除已保存的 AI API Key 吗？', { title: '删除 API Key', kind: 'warning' })) return
    try { const saved = await window.api.ai.deleteApiKey(); setApiKey(''); setAiForm((current) => ({ ...current, apiKeyConfigured: saved.apiKeyConfigured })); onAiSettingsChange(saved) }
    catch (error) { onError(errorMessage(error)) }
  }
  const themeOptions: { value: UiTheme; label: string; icon: typeof Monitor }[] = [{ value: 'system', label: '跟随系统', icon: Monitor }, { value: 'light', label: '浅色', icon: Sun }, { value: 'dark', label: '深色', icon: Moon }]
  return <Modal title="设置" onClose={onClose}><form className="form-grid terminal-settings-form" onSubmit={(event) => { event.preventDefault(); onSave(form, previewImage, appIcon, theme, aiInput()) }}>
    <div className="settings-tabs" role="tablist" aria-label="设置类别"><button type="button" role="tab" aria-selected={section === 'appearance'} className={section === 'appearance' ? 'active' : ''} onClick={() => setSection('appearance')}><Palette size={15} />界面</button><button type="button" role="tab" aria-selected={section === 'terminal'} className={section === 'terminal' ? 'active' : ''} onClick={() => setSection('terminal')}><SquareTerminal size={15} />终端</button><button type="button" role="tab" aria-selected={section === 'application'} className={section === 'application' ? 'active' : ''} onClick={() => setSection('application')}><ImageIcon size={15} />应用图标</button><button type="button" role="tab" aria-selected={section === 'ai'} className={section === 'ai' ? 'active' : ''} onClick={() => setSection('ai')}><Bot size={15} />AI</button></div>
    {section === 'appearance' ? <fieldset className="ui-theme-settings"><legend>界面主题</legend><div className="theme-options" role="radiogroup" aria-label="界面主题">{themeOptions.map((option) => { const Icon = option.icon; return <button key={option.value} type="button" role="radio" aria-checked={theme === option.value} className={theme === option.value ? 'active' : ''} onClick={() => selectTheme(option.value)}><Icon size={17} /><span>{option.label}</span></button> })}</div></fieldset> : section === 'application' ? <><fieldset className="app-icon-settings"><legend>选择应用图标</legend><div className="app-icon-options">{appIcons.options.map((icon) => <label key={icon.id} className={appIcon === icon.id ? 'selected' : ''}><input type="radio" name="app-icon" checked={appIcon === icon.id} onChange={() => setAppIcon(icon.id)} /><img src={icon.dataUrl} alt="" /><span>{icon.name}</span></label>)}</div></fieldset><div className="diagnostic-export"><button type="button" className="secondary-button" onClick={async () => { try { const path = await window.api.diagnostics.export(); if (path) setDiagnosticPath(path) } catch (error) { onError(errorMessage(error)) } }}><FileInput size={15} />导出诊断信息</button>{diagnosticPath && <small title={diagnosticPath}>{diagnosticPath}</small>}</div></> : section === 'ai' ? <div className="ai-settings">
      <label>API Base URL<input required type="url" value={aiForm.baseUrl} onChange={(event) => setAi('baseUrl', event.target.value)} placeholder="https://api.openai.com/v1" /></label>
      <label>模型名称<input value={aiForm.model} onChange={(event) => setAi('model', event.target.value)} placeholder="gpt-5" /></label>
      <label>API Key<div className="input-button"><input type="password" autoComplete="off" value={apiKey} onChange={(event) => { setApiKey(event.target.value); setAiTested(false) }} placeholder={aiForm.apiKeyConfigured ? '已安全保存，留空则保持不变' : '可选'} /><button type="button" className="icon-button danger" title="删除已保存的 API Key" disabled={!aiForm.apiKeyConfigured} onClick={() => void deleteAiKey()}><Trash2 size={15} /></button></div></label>
      <label>默认目标 Shell<select value={aiForm.defaultShell} onChange={(event) => setAi('defaultShell', event.target.value as AiShell)}><option value="linux">Linux Shell</option><option value="powerShell">PowerShell</option><option value="cmd">Windows cmd</option><option value="macos">macOS Shell</option></select></label>
      <div className="ai-settings-test"><button type="button" className="secondary-button" disabled={testingAi || !aiForm.model.trim()} onClick={() => void testAi()}>{testingAi ? '正在测试…' : '测试连接'}</button>{aiTested && <span><Check size={14} />连接成功</span>}</div>
    </div> : <>
      <div className="terminal-preview" style={terminalBackgroundStyle(form, previewImage)}><div style={{ color: form.foregroundColor, background: colorWithOpacity(form.backgroundColor, form.backgroundOpacity), fontFamily: form.fontFamily, fontSize: form.fontSize }}><span>user@host:~$</span> ls -la<br />drwxr-xr-x&nbsp;&nbsp;projects</div></div>
      <label>字体<select value={form.fontFamily} onChange={(event) => set('fontFamily', event.target.value)}><option value="Cascadia Mono, SFMono-Regular, Consolas, monospace">默认等宽字体</option><option value="Cascadia Mono, Consolas, monospace">Cascadia Mono</option><option value="SFMono-Regular, Menlo, Monaco, monospace">SF Mono</option><option value="JetBrains Mono, monospace">JetBrains Mono</option><option value="Fira Code, monospace">Fira Code</option><option value="Consolas, monospace">Consolas</option><option value="Menlo, monospace">Menlo</option><option value="monospace">系统等宽字体</option></select></label>
      <label>字号<div className="range-control"><input type="range" min="10" max="30" step="1" value={form.fontSize} onChange={(event) => set('fontSize', Number(event.target.value))} /><output>{form.fontSize} px</output></div></label>
      <div className="color-settings"><label>文字颜色<div className="color-control"><input type="color" value={form.foregroundColor} onChange={(event) => set('foregroundColor', event.target.value)} /><span>{form.foregroundColor}</span></div></label><label>背景颜色<div className="color-control"><input type="color" value={form.backgroundColor} onChange={(event) => set('backgroundColor', event.target.value)} /><span>{form.backgroundColor}</span></div></label></div>
      <label>背景颜色不透明度<div className="range-control"><input type="range" min="0" max="1" step="0.05" value={form.backgroundOpacity} onChange={(event) => set('backgroundOpacity', Number(event.target.value))} /><output>{Math.round(form.backgroundOpacity * 100)}%</output></div></label>
      <label>背景图片<div className="input-button background-image-input"><input readOnly value={form.backgroundImagePath} placeholder="未选择" /><button type="button" className="secondary-button" onClick={() => void chooseBackground()}><ImageIcon size={15} />选择</button><button type="button" className="icon-button background-clear" title="清除背景图片" disabled={!form.backgroundImagePath} onClick={() => { set('backgroundImagePath', ''); setPreviewImage('') }}><Trash2 size={15} /></button></div></label>
      <label>图片显示方式<select value={form.backgroundImageFit} onChange={(event) => set('backgroundImageFit', event.target.value as TerminalSettings['backgroundImageFit'])}><option value="cover">覆盖窗口</option><option value="contain">完整显示</option><option value="stretch">拉伸</option><option value="tile">平铺</option></select></label>
    </>}
    <div className="dialog-actions spread"><button type="button" className="secondary-button" onClick={reset}><RotateCcw size={15} />恢复默认</button><div><button type="button" className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" type="submit">保存</button></div></div>
  </form></Modal>
}

function GroupNameDialog({ mode, initialName, onClose, onSave }: { mode: 'create' | 'rename'; initialName: string; onClose(): void; onSave(name: string): void }): React.JSX.Element {
  const [name, setName] = useState(initialName)
  return <Modal title={mode === 'create' ? '新建分组' : '重命名分组'} onClose={onClose}><form className="form-grid compact-form" onSubmit={(event) => { event.preventDefault(); onSave(name.trim()) }}>
    <label>分组名称<input autoFocus required value={name} onChange={(event) => setName(event.target.value)} placeholder="例如：生产环境" /></label>
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button type="submit" className="primary-button">保存</button></div>
  </form></Modal>
}

function BookmarkDialog({ bookmark, connections, groups, onClose, onSaved, onError }: { bookmark?: Bookmark; connections: Bookmark[]; groups: string[]; onClose(): void; onSaved(): void; onError(message: string): void }): React.JSX.Element {
  const [form, setForm] = useState<BookmarkInput>(bookmark ? { name: bookmark.name, host: bookmark.host, port: bookmark.port, username: bookmark.username, authType: bookmark.authType, privateKeyPath: bookmark.privateKeyPath, jumpBookmarkId: bookmark.jumpBookmarkId, groupName: bookmark.groupName, favorite: bookmark.favorite, keepaliveEnabled: bookmark.keepaliveEnabled, keepaliveIntervalSeconds: bookmark.keepaliveIntervalSeconds, keepaliveCountMax: bookmark.keepaliveCountMax, note: bookmark.note } : emptyBookmark)
  const set = <K extends keyof BookmarkInput>(key: K, value: BookmarkInput[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const jumpCandidates = connections.filter((item) => item.id !== bookmark?.id && !item.jumpBookmarkId)
  return <Modal title={bookmark ? '编辑连接' : '新增连接'} onClose={onClose}><form className="form-grid" onSubmit={async (event) => { event.preventDefault(); try { await window.api.bookmarks.save({ ...form, id: bookmark?.id }); onSaved() } catch (error) { onError(errorMessage(error)) } }}>
    <div className="form-row"><label>连接名称<input required value={form.name} onChange={(event) => set('name', event.target.value)} placeholder="Production 服务器" /></label><label>分组<select value={form.groupName} onChange={(event) => set('groupName', event.target.value)}><option value="">未分组</option>{groups.filter(Boolean).map((group) => <option key={group} value={group}>{group}</option>)}</select></label></div>
    <label className="check-label"><input type="checkbox" checked={form.favorite} onChange={(event) => set('favorite', event.target.checked)} />收藏此连接</label>
    <div className="form-row"><label>Host（IP 或域名）<input required value={form.host} onChange={(event) => set('host', event.target.value)} placeholder="192.168.1.10" /></label><label className="port-field">端口<input type="number" min="1" max="65535" value={form.port} onChange={(event) => set('port', Number(event.target.value))} /></label></div>
    <label>用户名<input required value={form.username} onChange={(event) => set('username', event.target.value)} placeholder="root" /></label>
    <label>认证方式<select value={form.authType} onChange={(event) => set('authType', event.target.value as BookmarkInput['authType'])}><option value="password">密码</option><option value="privateKey">私钥</option><option value="agent">SSH Agent</option></select></label>
    {form.authType === 'privateKey' && <label>私钥文件<div className="input-button"><input readOnly value={form.privateKeyPath} placeholder="选择 OpenSSH 私钥" /><button type="button" className="secondary-button" onClick={async () => { const path = await window.api.files.choosePrivateKey(); if (path) set('privateKeyPath', path) }}>选择</button></div></label>}
    <label>连接方式<select value={form.jumpBookmarkId} onChange={(event) => set('jumpBookmarkId', event.target.value)}><option value="">直接连接</option>{jumpCandidates.map((item) => <option key={item.id} value={item.id}>通过 {item.name}（{item.username}@{item.host}）</option>)}</select></label>
    <label className="check-label"><input type="checkbox" checked={form.keepaliveEnabled} onChange={(event) => set('keepaliveEnabled', event.target.checked)} />保持 SSH session 活跃</label>
    {form.keepaliveEnabled && <div className="form-row"><label>保活间隔（秒）<input type="number" min="5" max="300" value={form.keepaliveIntervalSeconds} onChange={(event) => set('keepaliveIntervalSeconds', Number(event.target.value))} /></label><label>最大无响应次数<input type="number" min="1" max="10" value={form.keepaliveCountMax} onChange={(event) => set('keepaliveCountMax', Number(event.target.value))} /></label></div>}
    <label>备注<textarea rows={3} value={form.note} onChange={(event) => set('note', event.target.value)} placeholder="可选" /></label>
    {bookmark?.hasSavedCredential && <button type="button" className="text-button danger" onClick={async () => { await window.api.bookmarks.forgetCredential(bookmark.id); onSaved() }}>清除已保存的凭据</button>}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" type="submit">保存</button></div>
  </form></Modal>
}

function AuthDialog({ bookmark, jumpBookmark, onClose, onConnect }: { bookmark: Bookmark; jumpBookmark?: Bookmark; onClose(): void; onConnect(credentials: ConnectionCredentials): void }): React.JSX.Element {
  const [secret, setSecret] = useState(''); const [remember, setRemember] = useState(false); const [visible, setVisible] = useState(false)
  const [jumpSecret, setJumpSecret] = useState(''); const [rememberJump, setRememberJump] = useState(false); const [jumpVisible, setJumpVisible] = useState(false)
  const required = bookmark.authType === 'password' && !bookmark.hasSavedCredential
  const jumpRequired = Boolean(jumpBookmark?.authType === 'password' && !jumpBookmark.hasSavedCredential)
  return <Modal title={`连接到 ${bookmark.name}`} onClose={onClose}><form className="form-grid" onSubmit={(event) => { event.preventDefault(); if ((required && !secret) || (jumpRequired && !jumpSecret)) return; onConnect({ credential: secret || undefined, rememberCredential: remember, jumpCredential: jumpSecret || undefined, rememberJumpCredential: rememberJump }) }}>
    {jumpBookmark ? <div className="connection-route"><span><small>跳板机</small><strong>{jumpBookmark.name}</strong></span><ArrowRight size={16} /><span><small>目标机器</small><strong>{bookmark.name}</strong></span></div> : <div className="connection-target"><Server size={18} /><strong>{bookmark.username}@{bookmark.host}:{bookmark.port}</strong></div>}
    {jumpBookmark && jumpBookmark.authType !== 'agent' && <CredentialField bookmark={jumpBookmark} role="跳板机" secret={jumpSecret} setSecret={setJumpSecret} remember={rememberJump} setRemember={setRememberJump} visible={jumpVisible} setVisible={setJumpVisible} autoFocus />}
    {bookmark.authType !== 'agent' && <CredentialField bookmark={bookmark} role={jumpBookmark ? '目标机器' : ''} secret={secret} setSecret={setSecret} remember={remember} setRemember={setRemember} visible={visible} setVisible={setVisible} autoFocus={!jumpBookmark || jumpBookmark.authType === 'agent'} />}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button type="submit" className="primary-button">连接</button></div>
  </form></Modal>
}

function CredentialField({ bookmark, role, secret, setSecret, remember, setRemember, visible, setVisible, autoFocus }: { bookmark: Bookmark; role: string; secret: string; setSecret(value: string): void; remember: boolean; setRemember(value: boolean): void; visible: boolean; setVisible(value: boolean): void; autoFocus: boolean }): React.JSX.Element {
  const credentialName = bookmark.authType === 'password' ? '密码' : '私钥口令（可选）'
  return <div className="credential-block"><div className="credential-heading"><strong>{role || '连接'}凭据</strong><span>{bookmark.username}@{bookmark.host}:{bookmark.port}</span></div>
    <label>{role}{credentialName}<div className="password-input"><input autoFocus={autoFocus} type={visible ? 'text' : 'password'} required={bookmark.authType === 'password' && !bookmark.hasSavedCredential} value={secret} placeholder={bookmark.hasSavedCredential ? '已保存，不填写则继续使用' : undefined} onChange={(event) => setSecret(event.target.value)} /><button className="icon-button" type="button" title={visible ? '隐藏密码' : '显示密码'} onClick={() => setVisible(!visible)}>{visible ? <EyeOff size={16} /> : <Eye size={16} />}</button></div></label>
    <label className="check-label"><input type="checkbox" checked={remember} disabled={!secret} onChange={(event) => setRemember(event.target.checked)} />记住{role || '连接'}凭据（使用系统安全存储）</label>
  </div>
}

function HostKeyDialog({ prompt, onDecision }: { prompt: HostKeyPrompt; onDecision(accept: boolean): void }): React.JSX.Element {
  const changed = prompt.status === 'changed'
  return <Modal title={changed ? '主机指纹已变化' : '确认主机指纹'} onClose={() => onDecision(false)}><div className="dialog-copy">
    <p>{changed ? '服务器返回的主机指纹与此前记录不一致。除非你确认服务器密钥已更换，否则不要继续。' : '这是首次连接该服务器。确认指纹与服务器管理员提供的信息一致后再继续。'}</p>
    <dl><dt>服务器</dt><dd>{prompt.host}:{prompt.port}</dd>{prompt.previousFingerprint && <><dt>原指纹</dt><dd>{prompt.previousFingerprint}</dd></>}<dt>当前指纹</dt><dd>{prompt.fingerprint}</dd></dl>
    <div className="dialog-actions"><button className="secondary-button" onClick={() => onDecision(false)}>取消连接</button><button className={changed ? 'danger-button' : 'primary-button'} onClick={() => onDecision(true)}>信任并继续</button></div>
  </div></Modal>
}

function ConflictDialog({ conflict, onDecision }: { conflict: { sourcePath: string; destinationPath: string }; onDecision(value: ConflictResolution, apply: boolean): void }): React.JSX.Element {
  const [apply, setApply] = useState(false)
  return <Modal title="目标位置已有同名文件" onClose={() => onDecision('skip', false)}><div className="dialog-copy"><p className="path-line">{conflict.destinationPath}</p><label className="check-label"><input type="checkbox" checked={apply} onChange={(event) => setApply(event.target.checked)} />应用到当前批次中的所有冲突</label><div className="dialog-actions spread"><button className="secondary-button" onClick={() => onDecision('skip', apply)}>跳过</button><button className="secondary-button" onClick={() => onDecision('rename', apply)}>自动重命名</button><button className="danger-button" onClick={() => onDecision('overwrite', apply)}>覆盖</button></div></div></Modal>
}

function Modal({ title, onClose, children, wide = false }: { title: string; onClose(): void; children: React.ReactNode; wide?: boolean }): React.JSX.Element {
  return <div className="modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}><section className={`modal ${wide ? 'wide' : ''}`} role="dialog" aria-modal="true"><header><strong>{title}</strong><button className="icon-button" onClick={onClose}><X size={18} /></button></header>{children}</section></div>
}

function TransferPanel({ transfers, view, setView, onRetry, onCancel, onClear }: { transfers: TransferTask[]; view: 'queue' | 'history'; setView(value: 'queue' | 'history'): void; onRetry(task: TransferTask): void; onCancel(task: TransferTask): void; onClear(): void }): React.JSX.Element {
  const activeStatuses = ['queued', 'scanning', 'running', 'conflict']
  const shown = transfers.filter((item) => view === 'queue' ? activeStatuses.includes(item.status) || item.status === 'failed' : !activeStatuses.includes(item.status))
  const active = transfers.filter((item) => activeStatuses.includes(item.status))
  const totalBytes = active.reduce((sum, item) => sum + item.bytesTotal, 0)
  const transferredBytes = active.reduce((sum, item) => sum + item.bytesTransferred, 0)
  const speed = active.reduce((sum, item) => sum + item.speed, 0)
  const eta = speed > 0 && totalBytes > transferredBytes ? Math.ceil((totalBytes - transferredBytes) / speed) : 0
  const speedText = speed >= 1024 * 1024 ? `${(speed / 1024 / 1024).toFixed(1)} MB/s` : `${Math.round(speed / 1024)} KB/s`
  return <section className="transfer-panel"><header><div className="transfer-tabs"><button className={view === 'queue' ? 'active' : ''} onClick={() => setView('queue')}>传输队列 <span>{active.length}</span></button><button className={view === 'history' ? 'active' : ''} onClick={() => setView('history')}>传输记录</button></div>{active.length > 0 && <div className="transfer-summary"><strong>{totalBytes ? Math.round(transferredBytes / totalBytes * 100) : 0}%</strong><span>{speedText}</span>{eta > 0 && <span>约 {eta >= 60 ? `${Math.ceil(eta / 60)} 分钟` : `${eta} 秒`}</span>}</div>}<button className="text-button" onClick={onClear}>清除已完成</button></header>
    <div className="transfer-list">{shown.length === 0 ? <div className="transfer-empty">{view === 'queue' ? '当前没有传输任务' : '暂无传输记录'}</div> : shown.slice(0, 40).map((task) => {
      const percent = task.bytesTotal ? Math.min(100, Math.round(task.bytesTransferred / task.bytesTotal * 100)) : 0
      return <div className="transfer-row" key={task.id}><span className={`transfer-direction ${task.direction}`}>{task.direction === 'upload' ? '↑' : '↓'}</span><div className="transfer-info"><strong>{task.displayName}</strong><small>{task.error ?? `${task.sourcePath} → ${task.destinationPath}`}</small>{['running', 'scanning'].includes(task.status) && <div className="progress"><span style={{ width: `${percent}%` }} /></div>}</div><span className={`transfer-status ${task.status}`}>{task.status === 'running' ? `${percent}%` : ({ queued: '等待中', scanning: '扫描中', conflict: '等待确认', completed: '已完成', failed: '失败', cancelled: '已取消', interrupted: '已中断' } as Record<string, string>)[task.status]}</span><div className="transfer-actions">{activeStatuses.includes(task.status) && <button className="icon-button" title="取消" onClick={() => onCancel(task)}><X size={15} /></button>}{['failed', 'cancelled', 'interrupted'].includes(task.status) && <button className="text-button" onClick={() => onRetry(task)}>重试</button>}</div></div>
    })}</div>
  </section>
}
