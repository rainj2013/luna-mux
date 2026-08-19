import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowRight, Bookmark as BookmarkIcon, Bot, Check, ChevronDown, ChevronRight, CircleHelp, CirclePlus, Columns2, Columns3, Copy, Database as DatabaseIcon, Download, Edit3, ExternalLink, Eye, EyeOff, FileInput, FileJson2, Folder, FolderOpen, FolderPlus, Globe2, Grid2x2, GripVertical, History as HistoryIcon, Image as ImageIcon, KeyRound, Languages, LayoutGrid, Maximize2, Minimize, Minus, Monitor, Moon, Network, Palette, PanelLeftClose, PanelLeftOpen, Play, Plus, Power, Rocket, RotateCcw, Rows2, Rows3, Search, Send, Server, Settings as SettingsIcon, ShieldAlert, Sparkles, Square, SquareTerminal, Star, Stethoscope, Sun, Trash2, Upload, WandSparkles, X } from 'lucide-react'
import { BUNDLED_TERMINAL_FONT, DEFAULT_AI_SETTINGS, DEFAULT_TERMINAL_SETTINGS, type AgentLaunchProfile, type AiCommandHistoryEntry, type AiCommandSuggestion, type AiProvider, type AiRawExchange, type AiRiskAssessment, type AiSettings, type AiSettingsInput, type AiShell, type AiThinkingMode, type AppEvent, type AppIconId, type AppIconSettings, type AppLanguage, type Bookmark, type BookmarkArchivePreview, type BookmarkArchiveSource, type BookmarkInput, type BrowserResource, type BrowserRuntime, type BrowserRuntimeStatus, type BrowserTunnel, type ChromeInstallation, type ConflictResolution, type ConnectInput, type DeploymentDiffEntry, type DeploymentProfile, type DoctorCheck, type DoctorCheckStatus, type DoctorManagedAgent, type DoctorReport, type HostKeyPrompt, type LunaRemoteImportPreview, type LunaRemoteImportResult, type LunaRemoteSource, type ManagedAgentEvent, type ManagedAgentStatus, type MuxPane, type MuxSession, type MuxSplitNode, type PortForwardProfile, type SessionStatus, type SshConfigPreview, type TerminalRuntime, type TerminalRuntimeEvent, type TerminalSettings, type TerminalTarget, type TransferTask, type TunnelSummary, type UiTheme } from './types'
import { discardTerminalSnapshot, TerminalPane, type TerminalPaneHandle } from './components/TerminalPane'
import { SftpPane } from './components/SftpPane'
import { HelpDialog } from './components/HelpDialog'
import { colorWithOpacity, terminalBackgroundStyle } from './terminal-style'
import { availableLanguages, getNativeMenuLabels, useI18n, type MessageKey } from './i18n'
import { PRODUCT_INFO } from './product-info'

interface WorkspaceTab extends MuxPane { key: string; sessionId?: string; runtimeId?: string; agentId?: string; status: SessionStatus; error?: string }
interface AiCommandTarget { name: string; detail: string; runtimeId?: string; connected: boolean; remote: boolean; initialShell: AiShell }
interface BrowserResourceState extends BrowserResource { runtime?: BrowserRuntime; tunnel?: BrowserTunnel; status: BrowserRuntimeStatus | 'stopped'; error?: string }
interface ManagedAgentSummary { agentId: string; paneId: string; runtimeId: string; status: ManagedAgentStatus | 'starting' | 'stopped'; waitingReason?: string; timestamp?: string; eventCount: number; unread: boolean; hasStructuredEvents: boolean; latest?: ManagedAgentEvent; latestAttention?: ManagedAgentEvent }
interface DiagnosticsRuntimeEnvironment { runtimeId: string; hook: string; mcp: string; tokens: string }
type AgentAttentionTone = 'info' | 'warning' | 'error'
interface ConnectionTarget { newSession?: boolean; tabKey?: string; launchAgentProfileId?: AgentLaunchProfile['id']; launchAgentLabel?: AgentLaunchProfile['label']; paneTitle?: string }
interface SidebarContextMenu { x: number; y: number; group?: string; bookmark?: Bookmark }
type MuxSidebarContextMenu = { x: number; y: number } & ({ session: MuxSession; pane?: never } | { pane: WorkspaceTab; session?: never })
interface GroupDialogState { mode: 'create' | 'rename'; group?: string }
interface MuxSessionDialogState { mode: 'create' | 'rename'; session?: MuxSession }
type SettingsSection = 'appearance' | 'terminal' | 'diagnostics' | 'ssh' | 'ai'
type SidebarPointerDrag = { pointerId: number; type: 'bookmark' | 'group'; value: string; startX: number; startY: number; active: boolean }
type SidebarPointerDrop = { type: 'bookmark'; id: string; group: string; position: 'before' | 'after' } | { type: 'group'; group: string; position: 'before' | 'after' | 'inside' }
type ConfirmationOptions = { title: string; message: string; detail?: string; kind: 'warning' | 'danger'; confirmLabel: string }
type ConfirmAction = (options: ConfirmationOptions) => Promise<boolean>
type PendingConfirmation = ConfirmationOptions & { resolve(value: boolean): void }
type LayoutPreset = 'horizontal' | 'vertical' | 'twoColumns'

const emptyBookmark: BookmarkInput = { name: '', host: '', port: 22, username: '', authType: 'password', privateKeyPath: '', jumpBookmarkId: '', groupName: '', favorite: false, keepaliveEnabled: true, keepaliveIntervalSeconds: 15, keepaliveCountMax: 3, note: '' }
type ConnectionCredentials = Omit<ConnectInput, 'bookmarkId' | 'newSession'>
const defaultSidebarWidth = 260
const minSidebarWidth = 200
const maxSidebarWidth = 480
const aiTerminalContextLines = 100
const aiTerminalContextChars = 16_000
const appIconMessageKeys: Record<AppIconId, MessageKey> = { luna: 'appIcon.luna', graphite: 'appIcon.graphite', signal: 'appIcon.signal', light: 'appIcon.light' }

function primaryFontName(fontFamily: string): string {
  return fontFamily.split(',')[0]?.trim().replace(/^["']|["']$/g, '') || 'JetBrains Mono'
}

function terminalFontStack(fontName: string): string {
  const escaped = fontName.trim().replace(/\\/g, '\\\\').replace(/"/g, '\\"')
  return escaped ? `"${escaped}", "JetBrains Mono", monospace` : BUNDLED_TERMINAL_FONT
}

function clampSidebarWidth(width: number): number {
  return Math.min(maxSidebarWidth, Math.max(minSidebarWidth, Math.round(width)))
}

function terminalRuntimeSessionStatus(runtime: TerminalRuntime): SessionStatus {
  if (runtime.status === 'running') return 'connected'
  if (runtime.status === 'error') return 'error'
  if (runtime.status === 'exited') return 'disconnected'
  return 'connecting'
}

function applyTerminalRuntimeCreateResult(tab: WorkspaceTab, expectedRuntimeId: string, runtime: TerminalRuntime, agentId?: string, useAsSessionId = false): WorkspaceTab {
  if (tab.runtimeId !== expectedRuntimeId) return tab
  const status = terminalRuntimeSessionStatus(runtime)
  // Exit/error events may arrive before create() resolves; a stale running result must not revive the pane.
  const runtimeAlreadyStopped = ['disconnected', 'error'].includes(tab.status) && runtime.status === 'running'
  return {
    ...tab,
    ...(useAsSessionId ? { sessionId: runtime.runtimeId } : {}),
    agentId,
    ...(runtimeAlreadyStopped ? {} : { status, error: runtime.error })
  }
}

function selectCurrentPaneRuntimes(runtimes: TerminalRuntime[], events: ManagedAgentEvent[], panes: Array<Pick<WorkspaceTab, 'id' | 'runtimeId'>>): { byPane: Map<string, TerminalRuntime>; duplicateRuntimeIds: string[] } {
  const paneMap = new Map(panes.map((pane) => [pane.id, pane]))
  const latestSequence = new Map<string, number>()
  for (const event of events) latestSequence.set(event.context.runtimeId, Math.max(latestSequence.get(event.context.runtimeId) ?? -1, event.sequence))
  const candidatesByPane = new Map<string, TerminalRuntime[]>()
  for (const runtime of runtimes) {
    const context = runtime.managedAgent ?? runtime.context
    if (!context || !paneMap.has(context.paneId) || ['exited', 'error'].includes(runtime.status)) continue
    candidatesByPane.set(context.paneId, [...(candidatesByPane.get(context.paneId) ?? []), runtime])
  }
  const byPane = new Map<string, TerminalRuntime>()
  const duplicateRuntimeIds: string[] = []
  for (const [paneId, candidates] of candidatesByPane) {
    const boundRuntimeId = paneMap.get(paneId)?.runtimeId
    const selected = candidates.find((runtime) => runtime.runtimeId === boundRuntimeId) ?? [...candidates].sort((left, right) => {
      const leftSequence = latestSequence.get(left.runtimeId) ?? -1
      const rightSequence = latestSequence.get(right.runtimeId) ?? -1
      return rightSequence - leftSequence
    })[0]
    if (!selected) continue
    byPane.set(paneId, selected)
    duplicateRuntimeIds.push(...candidates.filter((runtime) => runtime.runtimeId !== selected.runtimeId).map((runtime) => runtime.runtimeId))
  }
  return { byPane, duplicateRuntimeIds }
}

function currentAgentIdsForRuntimes(events: ManagedAgentEvent[], runtimesByPane: Map<string, TerminalRuntime>): Set<string> {
  const latestByAgent = new Map<string, ManagedAgentEvent>()
  for (const event of events) {
    const runtime = runtimesByPane.get(event.context.paneId)
    if (runtime?.runtimeId !== event.context.runtimeId) continue
    const previous = latestByAgent.get(event.context.agentId)
    if (!previous || previous.sequence < event.sequence) latestByAgent.set(event.context.agentId, event)
  }
  return new Set([...latestByAgent.entries()].flatMap(([agentId, event]) => ['SessionEnd', 'RuntimeExit', 'AgentProcessExit'].includes(event.hookEventName) ? [] : [agentId]))
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message
  return String(error)
}

function isSshAuthenticationFailure(error?: string): boolean {
  const normalized = error?.toLowerCase() ?? ''
  return normalized.includes('认证失败') || normalized.includes('authentication failed') || normalized.includes('permission denied')
}

function agentAdapterLabel(adapterId: string): string {
  if (adapterId === 'codex') return 'Codex'
  if (adapterId === 'claude-code') return 'Claude Code'
  return adapterId
}

export function App(): React.JSX.Element {
  const { language, setLanguage, t } = useI18n()
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([])
  const [muxSessions, setMuxSessions] = useState<MuxSession[]>([])
  const [activeMuxSessionId, setActiveMuxSessionId] = useState('')
  const [collapsedMuxSessionIds, setCollapsedMuxSessionIds] = useState<Set<string>>(new Set())
  const [tabs, setTabs] = useState<WorkspaceTab[]>([])
  const [browserResources, setBrowserResources] = useState<BrowserResourceState[]>([])
  const [browserRuntimes, setBrowserRuntimes] = useState<BrowserRuntime[]>([])
  const [chromeInstallation, setChromeInstallation] = useState<ChromeInstallation | null | undefined>(undefined)
  const [activeKey, setActiveKey] = useState('')
  const [maximizedPaneId, setMaximizedPaneId] = useState('')
  const [layoutMenuOpen, setLayoutMenuOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [bookmarkDialog, setBookmarkDialog] = useState<Bookmark | 'new' | null>(null)
  const [authRequest, setAuthRequest] = useState<{ bookmark: Bookmark; target: ConnectionTarget } | null>(null)
  const [hostPrompts, setHostPrompts] = useState<HostKeyPrompt[]>([])
  const [conflict, setConflict] = useState<{ taskId: string; sourcePath: string; destinationPath: string } | null>(null)
  const [transfers, setTransfers] = useState<TransferTask[]>([])
  const [transferView, setTransferView] = useState<'queue' | 'history'>('queue')
  const [workspaceView, setWorkspaceView] = useState<'terminal' | 'agents' | 'browser' | 'files'>('terminal')
  const [managedAgentEvents, setManagedAgentEvents] = useState<ManagedAgentEvent[]>([])
  const [readAgentSequences, setReadAgentSequences] = useState<Set<number>>(new Set())
  const [dismissedAgentAttentionSequences, setDismissedAgentAttentionSequences] = useState<Set<number>>(new Set())
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
  const [uiTheme, setUiTheme] = useState<UiTheme>('dark')
  const [savedLanguage, setSavedLanguage] = useState<AppLanguage>(language)
  const [uiThemePreview, setUiThemePreview] = useState<UiTheme | null>(null)
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches)
  const [appIcons, setAppIcons] = useState<AppIconSettings>({ selected: 'luna', options: [] })
  const [aiSettings, setAiSettings] = useState<AiSettings>({ ...DEFAULT_AI_SETTINGS })
  const [remoteAgentIntegrationEnabled, setRemoteAgentIntegrationEnabled] = useState(false)
  const [aiDialog, setAiDialog] = useState(false)
  const [settingsDialog, setSettingsDialog] = useState(false)
  const [settingsInitialSection, setSettingsInitialSection] = useState<SettingsSection>('appearance')
  const [helpDialog, setHelpDialog] = useState(false)
  const [deploymentDialog, setDeploymentDialog] = useState(false)
  const [tunnelDialog, setTunnelDialog] = useState(false)
  const [importPreview, setImportPreview] = useState<SshConfigPreview | null>(null)
  const [archiveImportPreview, setArchiveImportPreview] = useState<BookmarkArchivePreview | null>(null)
  const [lunaRemoteSources, setLunaRemoteSources] = useState<LunaRemoteSource[] | null>(null)
  const [lunaRemoteImportPreview, setLunaRemoteImportPreview] = useState<LunaRemoteImportPreview | null>(null)
  const [sidebarContextMenu, setSidebarContextMenu] = useState<SidebarContextMenu | null>(null)
  const [muxSidebarContextMenu, setMuxSidebarContextMenu] = useState<MuxSidebarContextMenu | null>(null)
  const [groupDialog, setGroupDialog] = useState<GroupDialogState | null>(null)
  const [muxSessionDialog, setMuxSessionDialog] = useState<MuxSessionDialogState | null>(null)
  const [paneRenameDialog, setPaneRenameDialog] = useState<WorkspaceTab | null>(null)
  const [connectionLibraryDialog, setConnectionLibraryDialog] = useState(false)
  const [paneLauncher, setPaneLauncher] = useState<{ targets: TerminalTarget[]; loading: boolean } | null>(null)
  const [confirmation, setConfirmation] = useState<PendingConfirmation | null>(null)
  const [toast, setToast] = useState('')
  const [toastKind, setToastKind] = useState<'error' | 'success'>('error')
  const bootstrapStarted = useRef(false)
  const stateRestored = useRef(false)
  const sidebarWidthRef = useRef(defaultSidebarWidth)
  const sidebarPointerDragRef = useRef<SidebarPointerDrag | null>(null)
  const sidebarPointerDropRef = useRef<SidebarPointerDrop | null>(null)
  const suppressSidebarClickRef = useRef(false)
  const terminalPaneRefs = useRef(new Map<string, TerminalPaneHandle>())
  const browserResourceStartInFlightRef = useRef(new Set<string>())
  const terminalTargetsCacheRef = useRef<TerminalTarget[] | null>(null)

  const showToast = (message: string, kind: 'error' | 'success' = 'error'): void => { setToastKind(kind); setToast(message); setTimeout(() => setToast(''), 4500) }
  const showError = (message: string): void => showToast(message)
  const refreshChromeInstallation = (): void => {
    setChromeInstallation(undefined)
    void window.api.browserRuntimes.discoverChrome().then(setChromeInstallation).catch(() => setChromeInstallation(null))
  }
  const confirmAction: ConfirmAction = (options) => new Promise((resolve) => setConfirmation({ ...options, resolve }))
  const resolveConfirmation = (accepted: boolean): void => {
    if (!confirmation) return
    confirmation.resolve(accepted)
    setConfirmation(null)
  }
  const openSettings = (section: SettingsSection = 'appearance'): void => { setSettingsInitialSection(section); setSettingsDialog(true) }
  const openPaneLauncher = async (muxSessionId = activeMuxSessionId): Promise<void> => {
    if (!muxSessionId) { showError(t('app.createSessionFirst')); return }
    setActiveMuxSessionId(muxSessionId)
    setActiveKey(tabs.find((pane) => pane.muxSessionId === muxSessionId)?.key ?? '')
    setWorkspaceView('terminal')
    setMaximizedPaneId('')
    setCollapsedMuxSessionIds((current) => {
      if (!current.has(muxSessionId)) return current
      const next = new Set(current)
      next.delete(muxSessionId)
      return next
    })
    setPaneLauncher({ targets: terminalTargetsCacheRef.current ?? [], loading: true })
    try {
      const runtimeTargets = await window.api.terminalRuntimes.targets()
      const targets = runtimeTargets.filter((item) => item.transport === 'localPty' || item.transport === 'ssh')
      terminalTargetsCacheRef.current = targets
      setPaneLauncher((current) => current ? { targets, loading: false } : null)
    } catch (error) {
      setPaneLauncher((current) => current ? { ...current, loading: false } : null)
      showError(errorMessage(error))
    }
  }
  const persistSessionLayout = async (session: MuxSession, layout: MuxSplitNode | undefined): Promise<MuxSession> => {
    const saved = await window.api.muxSessions.save({ id: session.id, name: session.name, rootPath: session.rootPath, layout })
    setMuxSessions((current) => current.map((item) => item.id === saved.id ? saved : item))
    return saved
  }
  const openLocalTerminal = async (target?: TerminalTarget, paneTitle = ''): Promise<void> => {
    let createdPaneKey: string | undefined
    try {
      const muxSession = muxSessions.find((session) => session.id === activeMuxSessionId)
      if (!muxSession) { showError(t('app.createSessionFirst')); return }
      if (!target) { await openPaneLauncher(); return }
      if (target.transport !== 'localPty') { showError(t('app.localTerminalUnavailable')); return }
      setPaneLauncher(null)
      const title = paneTitle.trim() || target.label
      const savedPane = await window.api.muxPanes.save({ muxSessionId: muxSession.id, kind: 'terminal', title, targetId: target.id, cwd: muxSession.rootPath })
      const runtimeId = crypto.randomUUID()
      const pane: WorkspaceTab = { ...savedPane, key: savedPane.id, runtimeId, status: 'connecting' }
      createdPaneKey = pane.key
      setTabs((current) => [...current, pane])
      const layout = insertPaneInLayout(layoutFromPanes(muxSession.layout, sessionTabs), activeKey || undefined, pane.id, 'horizontal')
      await persistSessionLayout(muxSession, layout)
      setActiveKey(pane.key)
      setWorkspaceView('terminal')
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()))
      const runtime = await window.api.terminalRuntimes.create({
        runtimeId,
        context: { muxSessionId: muxSession.id, paneId: pane.id, runtimeId },
        targetId: target.id,
        title,
        cwd: muxSession.rootPath || undefined,
        cols: 100,
        rows: 30
      })
      setTabs((current) => current.map((tab) => tab.key === pane.key ? applyTerminalRuntimeCreateResult(tab, runtimeId, runtime) : tab))
    } catch (error) {
      const message = errorMessage(error)
      if (createdPaneKey) setTabs((current) => current.map((tab) => tab.key === createdPaneKey ? { ...tab, status: 'error', error: message } : tab))
      showError(message)
    }
  }
  const startBrowserResource = async (resource: BrowserResourceState): Promise<void> => {
    if (browserResourceStartInFlightRef.current.has(resource.id)) return
    const activeResource = browserResources.find((item) => item.id !== resource.id && item.muxSessionId === resource.muxSessionId && (item.status === 'running' || item.status === 'starting'))
    if (activeResource) {
      setBrowserResources((current) => current.map((item) => item.id === resource.id ? { ...item, status: 'stopped', error: undefined } : item))
      showError(t('app.anotherBrowserIsActive', { value0: activeResource.name }))
      return
    }
    browserResourceStartInFlightRef.current.add(resource.id)
    let tunnel: BrowserTunnel | undefined
    try {
      setBrowserResources((current) => current.map((item) => item.id === resource.id ? { ...item, status: 'starting', error: undefined } : item))
      let launchUrl = 'about:blank'
      if (resource.sourcePaneId) {
        const sourcePane = tabs.find((item) => item.id === resource.sourcePaneId && item.muxSessionId === resource.muxSessionId)
        if (!sourcePane?.sessionId || sourcePane.status !== 'connected') throw new Error(t('app.remoteBrowserSourceUnavailable'))
        tunnel = await window.api.tunnels.startBrowser(sourcePane.sessionId, resource.id, sourcePane.id, resource.url)
        launchUrl = tunnel.localUrl
      }
      const runtime = await window.api.browserRuntimes.create({ muxSessionId: resource.muxSessionId, browserResourceId: resource.id, url: launchUrl, temporaryProfile: false })
      setBrowserResources((current) => current.map((item) => item.id === resource.id ? { ...item, runtime, tunnel, status: runtime.status, error: runtime.error } : item))
      await window.api.browserRuntimes.focusExternal(runtime.id).catch((error) => showError(errorMessage(error)))
    } catch (error) {
      if (tunnel) await window.api.tunnels.stop(tunnel.tunnel.sessionId, tunnel.tunnel.id).catch(() => undefined)
      setBrowserResources((current) => current.map((item) => item.id === resource.id ? { ...item, status: 'error', error: errorMessage(error), runtime: undefined, tunnel: undefined } : item))
      showError(errorMessage(error))
    } finally {
      browserResourceStartInFlightRef.current.delete(resource.id)
    }
  }

  const stopBrowserResource = async (resource: BrowserResourceState): Promise<void> => {
    if (resource.runtime) await window.api.browserRuntimes.close(resource.runtime.id).catch(() => undefined)
    if (resource.tunnel) await window.api.tunnels.stop(resource.tunnel.tunnel.sessionId, resource.tunnel.tunnel.id).catch(() => undefined)
    setBrowserResources((current) => current.map((item) => item.id === resource.id ? { ...item, runtime: undefined, tunnel: undefined, status: 'stopped', error: undefined } : item))
  }
  const restartBrowserResource = async (resource: BrowserResourceState): Promise<void> => {
    await stopBrowserResource(resource)
    await startBrowserResource({ ...resource, runtime: undefined, tunnel: undefined, status: 'stopped', error: undefined })
  }
  const startLocalPane = async (pane: WorkspaceTab): Promise<void> => {
    const runtimeId = crypto.randomUUID()
    const agentId = pane.launchProfileId ? crypto.randomUUID() : undefined
    try {
      setActiveKey(pane.key)
      setWorkspaceView('terminal')
      setTabs((current) => current.map((item) => item.key === pane.key ? { ...item, runtimeId, agentId, status: 'connecting', error: undefined } : item))
      const runtime = await window.api.terminalRuntimes.create({
        runtimeId,
        context: { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId },
        targetId: pane.targetId,
        title: pane.title,
        cwd: pane.cwd || undefined,
        managedAgent: pane.launchProfileId && agentId ? { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId, agentId, launchProfileId: pane.launchProfileId } : undefined,
        cols: 100,
        rows: 30
      })
      setTabs((current) => current.map((item) => item.key === pane.key ? applyTerminalRuntimeCreateResult(item, runtimeId, runtime, agentId) : item))
    } catch (error) {
      setTabs((current) => current.map((item) => item.key === pane.key && item.runtimeId === runtimeId ? { ...item, status: 'error', error: errorMessage(error) } : item))
      showError(errorMessage(error))
    }
  }
  const reloadBookmarks = async (): Promise<void> => setBookmarks(await window.api.bookmarks.list())
  const reloadSidebarData = async (): Promise<void> => {
    const [items, groups] = await Promise.all([window.api.bookmarks.list(), window.api.bookmarkGroups.list()])
    setBookmarks(items)
    setBookmarkGroupNames(groups)
  }
  const reloadImportedSettings = async (): Promise<void> => {
    const [appearance, theme, importedLanguage, ai, remoteAgentIntegration, collapsed, collapsedGroups, width] = await Promise.all([window.api.settings.getTerminal(), window.api.settings.getUiTheme(), window.api.settings.getLanguage(), window.api.ai.getSettings(), window.api.settings.getRemoteAgentIntegrationEnabled(), window.api.state.getSidebarCollapsed(), window.api.state.getCollapsedBookmarkGroups(), window.api.state.getSidebarWidth()])
    setTerminalSettings(appearance)
    setUiTheme(theme)
    setUiThemePreview(null)
    setSavedLanguage(importedLanguage)
    setLanguage(importedLanguage)
    setAiSettings(ai)
    setRemoteAgentIntegrationEnabled(remoteAgentIntegration)
    setSidebarCollapsed(collapsed)
    setCollapsedBookmarkGroups(new Set(collapsedGroups))
    sidebarWidthRef.current = width
    setSidebarWidth(width)
    await window.api.settings.applyLanguage(getNativeMenuLabels(importedLanguage))
    if (appearance.backgroundImagePath) setTerminalBackground(await window.api.settings.loadTerminalBackground(appearance.backgroundImagePath).catch(() => ''))
    else setTerminalBackground('')
  }

  useEffect(() => {
    if (bootstrapStarted.current) return
    bootstrapStarted.current = true
    void Promise.all([window.api.bookmarks.list(), window.api.bookmarkGroups.list(), window.api.transfers.list(), window.api.state.getSidebarCollapsed(), window.api.state.getCollapsedBookmarkGroups(), window.api.state.getSidebarWidth(), window.api.settings.getTerminal(), window.api.settings.getAppIcons(), window.api.settings.getUiTheme(), window.api.ai.getSettings(), window.api.settings.getRemoteAgentIntegrationEnabled(), window.api.muxSessions.list(), window.api.muxPanes.list(), window.api.browserResources.list(), window.api.browserRuntimes.list(), window.api.terminalRuntimes.list(), window.api.managedAgents.events()]).then(async ([items, groups, transferItems, collapsed, collapsedGroups, width, appearance, icons, theme, ai, remoteAgentIntegration, storedSessions, storedPanes, storedBrowserResources, storedBrowserRuntimes, terminalRuntimes, agentEvents]) => {
      const uniqueSessions = [...new Map(storedSessions.map((session) => [session.id, session])).values()]
      const emptyDefaults = uniqueSessions.filter((session) => session.id === 'default-session' && !session.rootPath && !session.layout && !storedPanes.some((pane) => pane.muxSessionId === session.id) && ['Untitled session', '未命名会话'].includes(session.name))
      if (emptyDefaults.length) {
        const obsoleteIds = new Set(emptyDefaults.map((session) => session.id))
        await Promise.all([...obsoleteIds].map((id) => window.api.muxSessions.remove(id)))
        for (let index = uniqueSessions.length - 1; index >= 0; index -= 1) if (obsoleteIds.has(uniqueSessions[index]!.id)) uniqueSessions.splice(index, 1)
      }
      const sessionItems = uniqueSessions
      const managedRuntimes = selectCurrentPaneRuntimes(terminalRuntimes, agentEvents, storedPanes.map((pane) => ({ ...pane, key: pane.id, status: 'disconnected' as SessionStatus })))
      await Promise.all(managedRuntimes.duplicateRuntimeIds.map((runtimeId) => window.api.terminalRuntimes.close(runtimeId).catch(() => undefined)))
      const currentAgentIds = currentAgentIdsForRuntimes(agentEvents, managedRuntimes.byPane)
      setBookmarks(items); setTransfers(transferItems)
      setMuxSessions(sessionItems)
      setActiveMuxSessionId(sessionItems[0]?.id ?? '')
      setTabs(storedPanes.map((pane) => {
        const runtime = managedRuntimes.byPane.get(pane.id)
        return { ...pane, key: pane.id, runtimeId: runtime?.runtimeId, agentId: runtime?.managedAgent?.agentId, status: runtime ? terminalRuntimeSessionStatus(runtime) : 'disconnected', error: runtime?.error }
      }))
      setBrowserResources(storedBrowserResources.map((resource) => {
        const runtime = storedBrowserRuntimes.find((item) => item.browserResourceId === resource.id && item.muxSessionId === resource.muxSessionId && ['starting', 'running'].includes(item.status))
        return { ...resource, runtime, status: runtime?.status ?? 'stopped', error: runtime?.error }
      }))
      setBrowserRuntimes(storedBrowserRuntimes)
      setActiveKey(storedPanes.find((pane) => pane.muxSessionId === sessionItems[0]?.id)?.id ?? '')
      setBookmarkGroupNames(groups)
      setSidebarCollapsed(collapsed)
      setCollapsedBookmarkGroups(new Set(collapsedGroups))
      sidebarWidthRef.current = width
      setSidebarWidth(width)
      setTerminalSettings(appearance)
      setAppIcons(icons)
      setUiTheme(theme)
      setAiSettings(ai)
      setRemoteAgentIntegrationEnabled(remoteAgentIntegration)
      setManagedAgentEvents(agentEvents.filter((event) => currentAgentIds.has(event.context.agentId)))
      if (appearance.backgroundImagePath) void window.api.settings.loadTerminalBackground(appearance.backgroundImagePath).then(setTerminalBackground).catch(() => setTerminalBackground(''))
      stateRestored.current = true
    }).catch((error) => showError(errorMessage(error)))
  }, [])

  useEffect(() => { if (stateRestored.current) void window.api.state.setSidebarCollapsed(sidebarCollapsed).catch((error) => showError(errorMessage(error))) }, [sidebarCollapsed])

  useEffect(() => {
    refreshChromeInstallation()
  }, [])

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
    if (!sidebarContextMenu && !muxSidebarContextMenu) return
    const close = (): void => { setSidebarContextMenu(null); setMuxSidebarContextMenu(null) }
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
  }, [sidebarContextMenu, muxSidebarContextMenu])

  useEffect(() => {
    if (!layoutMenuOpen) return
    const close = (): void => setLayoutMenuOpen(false)
    const closeOnKey = (event: KeyboardEvent): void => { if (event.key === 'Escape') close() }
    document.addEventListener('pointerdown', close)
    window.addEventListener('blur', close)
    document.addEventListener('keydown', closeOnKey)
    return () => {
      document.removeEventListener('pointerdown', close)
      window.removeEventListener('blur', close)
      document.removeEventListener('keydown', closeOnKey)
    }
  }, [layoutMenuOpen])

  useEffect(() => window.api.onEvent((event: AppEvent) => {
    if (event.type === 'session') {
      setTabs((current) => current.map((tab) => tab.sessionId === event.payload.id ? { ...tab, status: event.payload.status, error: event.payload.error } : tab))
      if (event.payload.status === 'connected') void reloadBookmarks()
      if (['disconnected', 'error'].includes(event.payload.status)) {
        setHostPrompts((current) => current.filter((item) => item.sessionId !== event.payload.id))
        setBrowserResources((current) => current.map((resource) => {
          if (resource.tunnel?.tunnel.sessionId !== event.payload.id) return resource
          if (resource.runtime) void window.api.browserRuntimes.close(resource.runtime.id).catch(() => undefined)
          return { ...resource, runtime: undefined, tunnel: undefined, status: 'error', error: t('app.remoteBrowserConnectionLost') }
        }))
      }
    }
    if (event.type === 'host-key') setHostPrompts((current) => current.some((item) => item.sessionId === event.payload.sessionId && item.fingerprint === event.payload.fingerprint) ? current : [...current, event.payload])
    if (event.type === 'transfer') {
      setTransfers((current) => [event.payload, ...current.filter((item) => item.id !== event.payload.id)].sort((a, b) => b.createdAt.localeCompare(a.createdAt)))
      if (event.payload.status !== 'conflict') setConflict((current) => current?.taskId === event.payload.id ? null : current)
    }
    if (event.type === 'tunnel' && event.payload.status === 'error') {
      void window.api.tunnels.stop(event.payload.sessionId, event.payload.id).catch(() => undefined)
      setBrowserResources((current) => current.map((resource) => {
        if (resource.tunnel?.tunnel.id !== event.payload.id) return resource
        if (resource.runtime) void window.api.browserRuntimes.close(resource.runtime.id).catch(() => undefined)
        return { ...resource, runtime: undefined, tunnel: undefined, status: 'error', error: event.payload.error || t('app.remoteBrowserConnectionLost') }
      }))
    }
    if (event.type === 'transfer-conflict') setConflict(event.payload)
  }), [])

  useEffect(() => window.api.onTerminalRuntimeEvent((event: TerminalRuntimeEvent) => {
    const runtime = event.type === 'status' ? event.payload.runtime : undefined
    const runtimeId = event.type === 'status' ? event.payload.runtime.runtimeId : event.payload.runtimeId
    if (!runtimeId) return
    if (runtime?.status === 'error' && runtime.targetId.startsWith('ssh-bookmark:') && runtime.error) showError(runtime.error)
    setTabs((current) => current.map((tab) => {
      if (tab.runtimeId !== runtimeId) return tab
      if (event.type === 'status') return { ...tab, status: runtime!.status === 'running' ? 'connected' : runtime!.status === 'error' ? 'error' : runtime!.status === 'exited' ? 'disconnected' : 'connecting', error: runtime!.error }
      if (event.type === 'exit') return { ...tab, status: event.payload.reason === 'failed' ? 'error' : 'disconnected', error: event.payload.message }
      return tab
    }))
  }), [])

  useEffect(() => window.api.onUiThemeChanged((theme) => {
    setUiTheme(theme)
    setUiThemePreview(null)
  }), [])

  useEffect(() => window.api.onTerminalSettingsChanged((settings) => {
    setTerminalSettings(settings)
    if (settings.backgroundImagePath) {
      void window.api.settings.loadTerminalBackground(settings.backgroundImagePath).then(setTerminalBackground).catch(() => setTerminalBackground(''))
    } else setTerminalBackground('')
  }), [])

  useEffect(() => window.api.onControlStateChanged((event) => {
    if (event.type === 'muxSessionSaved') {
      setMuxSessions((current) => [...current.filter((item) => item.id !== event.payload.id), event.payload].sort((a, b) => a.sortOrder - b.sortOrder))
      return
    }
    if (event.type === 'muxPaneCreated') {
      const { pane, session, start } = event.payload
      setMuxSessions((current) => [...current.filter((item) => item.id !== session.id), session].sort((a, b) => a.sortOrder - b.sortOrder))
      setTabs((current) => current.some((item) => item.id === pane.id) ? current : [...current, { ...pane, key: pane.id, status: start ? 'connecting' : 'disconnected' }])
      setActiveMuxSessionId(session.id)
      setActiveKey(pane.id)
      setWorkspaceView('terminal')
      setMaximizedPaneId('')
      if (start) void (async () => {
        const runtimeId = crypto.randomUUID()
        const agentId = pane.launchProfileId ? crypto.randomUUID() : undefined
        setTabs((current) => current.map((item) => item.id === pane.id ? { ...item, runtimeId, agentId, status: 'connecting', error: undefined } : item))
        try {
          const runtime = await window.api.terminalRuntimes.create({
            runtimeId,
            context: { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId },
            targetId: pane.targetId,
            title: pane.title,
            cwd: pane.cwd || undefined,
            command: pane.command || undefined,
            managedAgent: pane.launchProfileId && agentId ? { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId, agentId, launchProfileId: pane.launchProfileId } : undefined,
            cols: 100,
            rows: 30
          })
          setTabs((current) => current.map((item) => item.id === pane.id ? applyTerminalRuntimeCreateResult(item, runtimeId, runtime, agentId, true) : item))
        } catch (error) {
          setTabs((current) => current.map((item) => item.id === pane.id && item.runtimeId === runtimeId ? { ...item, status: 'error', error: errorMessage(error) } : item))
          showError(errorMessage(error))
        }
      })()
      return
    }
    setTabs((current) => current.map((item) => item.id === event.payload.id ? { ...item, ...event.payload, key: item.key } : item))
  }), [])

  useEffect(() => window.api.onBrowserRuntimeEvent((event) => {
    if (event.type === 'started') {
      setBrowserRuntimes((current) => [...current.filter((runtime) => runtime.id !== event.runtime.id), event.runtime])
      setBrowserResources((current) => current.map((resource) => resource.id === event.runtime.browserResourceId
        ? { ...resource, runtime: event.runtime, status: event.runtime.status, error: event.runtime.error }
        : resource))
      return
    }
    setBrowserRuntimes((current) => current.map((runtime) => runtime.id === event.runtimeId ? { ...runtime, status: event.status, error: event.error } : runtime))
    setBrowserResources((current) => current.map((resource) => {
      if (resource.runtime?.id !== event.runtimeId) return resource
      const stopped = event.status === 'stopped' || event.status === 'error'
      if (stopped && resource.tunnel) void window.api.tunnels.stop(resource.tunnel.tunnel.sessionId, resource.tunnel.tunnel.id).catch(() => undefined)
      return {
        ...resource,
        status: event.status,
        error: event.error,
        ...(stopped ? { runtime: undefined, tunnel: undefined } : {})
      }
    }))
  }), [])

  useEffect(() => window.api.onManagedAgentEvent((event) => {
    const ended = ['SessionEnd', 'RuntimeExit', 'AgentProcessExit'].includes(event.hookEventName)
    setManagedAgentEvents((current) => ended
      ? current.filter((item) => item.context.agentId !== event.context.agentId)
      : [...current.filter((item) => item.sequence !== event.sequence), event].sort((a, b) => a.sequence - b.sequence))
  }), [])

  useEffect(() => window.api.onManagedAgentNotificationActivate((event) => {
    setActiveMuxSessionId(event.muxSessionId)
    setActiveKey(event.paneId)
    setWorkspaceView('terminal')
    setReadAgentSequences((current) => new Set(current).add(event.sequence))
  }), [])

  useEffect(() => {
    void window.api.managedAgents.setNotificationFocus(
      activeMuxSessionId || undefined,
      workspaceView === 'terminal' ? activeKey || undefined : undefined,
      workspaceView === 'terminal'
    ).catch((error) => console.warn('Failed to synchronize Agent notification focus', error))
  }, [activeMuxSessionId, activeKey, workspaceView])

  const bookmarkMap = useMemo(() => new Map(bookmarks.map((item) => [item.id, item])), [bookmarks])
  const activeMuxSession = muxSessions.find((session) => session.id === activeMuxSessionId)
  const sessionTabs = tabs.filter((tab) => tab.muxSessionId === activeMuxSessionId)
  const sessionBrowserResources = browserResources.filter((resource) => resource.muxSessionId === activeMuxSessionId)
  const activeTab = sessionTabs.find((tab) => tab.key === activeKey)
  const activeBookmark = activeTab?.kind === 'terminal' && activeTab.bookmarkId ? bookmarkMap.get(activeTab.bookmarkId) : undefined
  const activeSshRuntimeId = activeBookmark ? activeTab?.runtimeId ?? activeTab?.sessionId : undefined
  const activeAiCommandTarget: AiCommandTarget | undefined = activeTab ? {
    name: activeBookmark?.name ?? activeTab.title,
    detail: activeBookmark ? `${activeBookmark.username}@${activeBookmark.host}:${activeBookmark.port}` : activeTab.cwd || activeTab.targetId,
    runtimeId: activeTab.runtimeId,
    connected: activeTab.status === 'connected',
    remote: Boolean(activeBookmark),
    initialShell: activeBookmark ? aiSettings.defaultShell : activeTab.targetId === 'local:macos-shell' ? 'macos' : (activeTab.targetId === 'local:powershell' || activeTab.targetId === 'local:powershell5') ? 'powerShell' : activeTab.targetId.startsWith('local:wsl:') ? 'linux' : aiSettings.defaultShell,
  } : undefined
  const allAgents = useMemo(() => summarizeManagedAgents(tabs, managedAgentEvents, readAgentSequences, dismissedAgentAttentionSequences), [tabs, managedAgentEvents, readAgentSequences, dismissedAgentAttentionSequences])
  const agentAttentionByPane = useMemo(() => new Map(allAgents.flatMap((agent) => {
    const tone = agentAttentionTone(agent)
    return tone ? [[agent.paneId, tone] as const] : []
  })), [allAgents])
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

  useEffect(() => {
    if (workspaceView === 'files' && activeTab && !activeTab.bookmarkId) setWorkspaceView('terminal')
  }, [activeTab, workspaceView])

  useEffect(() => {
    void Promise.all([window.api.terminalRuntimes.list(), window.api.managedAgents.events(), window.api.browserRuntimes.list()]).then(async ([runtimes, events, activeBrowserRuntimes]) => {
      const managedRuntimes = selectCurrentPaneRuntimes(runtimes, events, tabs)
      await Promise.all(managedRuntimes.duplicateRuntimeIds.map((runtimeId) => window.api.terminalRuntimes.close(runtimeId).catch(() => undefined)))
      const currentAgentIds = currentAgentIdsForRuntimes(events, managedRuntimes.byPane)
      setTabs((current) => current.map((pane) => {
        const runtime = managedRuntimes.byPane.get(pane.id)
        if (!runtime) return pane
        return { ...pane, runtimeId: runtime.runtimeId, agentId: runtime.managedAgent?.agentId, status: terminalRuntimeSessionStatus(runtime), error: runtime.error }
      }))
      setManagedAgentEvents(events.filter((event) => currentAgentIds.has(event.context.agentId)))
      if (workspaceView === 'agents') {
        setBrowserRuntimes(activeBrowserRuntimes)
        setBrowserResources((current) => current.map((resource) => {
          const runtime = activeBrowserRuntimes.find((item) => item.browserResourceId === resource.id && item.muxSessionId === resource.muxSessionId && ['starting', 'running'].includes(item.status))
          return { ...resource, runtime, status: runtime?.status ?? 'stopped', error: runtime?.error }
        }))
      }
    }).catch((error) => showError(errorMessage(error)))
  }, [workspaceView])

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
    setSidebarContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 340)), ...options })
  }

  const openMuxSidebarContextMenu = (event: React.MouseEvent, target: { session: MuxSession } | { pane: WorkspaceTab }): void => {
    event.preventDefault()
    event.stopPropagation()
    setSidebarContextMenu(null)
    setMuxSidebarContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 90)), ...target })
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

  const saveMuxSession = async (name: string, rootPath: string): Promise<void> => {
    try {
      const creating = !muxSessionDialog?.session
      const saved = await window.api.muxSessions.save({
        id: muxSessionDialog?.session?.id,
        name,
        rootPath,
        layout: muxSessionDialog?.session?.layout
      })
      setMuxSessions((current) => [...current.filter((session) => session.id !== saved.id), saved].sort((a, b) => a.sortOrder - b.sortOrder))
      if (creating) {
        const resources = await window.api.browserResources.list(saved.id)
        setBrowserResources((current) => [...current.filter((resource) => resource.muxSessionId !== saved.id), ...resources.map((resource) => ({ ...resource, status: 'stopped' as const }))])
      }
      setActiveMuxSessionId(saved.id)
      setActiveKey(tabs.find((pane) => pane.muxSessionId === saved.id)?.key ?? '')
      setMuxSessionDialog(null)
    } catch (error) { showError(errorMessage(error)) }
  }

  const selectMuxSession = (id: string): void => {
    if (id === activeMuxSessionId) {
      setCollapsedMuxSessionIds((current) => {
        const next = new Set(current)
        if (next.has(id)) next.delete(id); else next.add(id)
        return next
      })
      return
    }
    setCollapsedMuxSessionIds((current) => {
      if (!current.has(id)) return current
      const next = new Set(current)
      next.delete(id)
      return next
    })
    setActiveMuxSessionId(id)
    setActiveKey(tabs.find((pane) => pane.muxSessionId === id)?.key ?? '')
    setWorkspaceView(tabs.some((pane) => pane.muxSessionId === id) || !browserResources.some((resource) => resource.muxSessionId === id) ? 'terminal' : 'browser')
    setMaximizedPaneId('')
  }

  const removeMuxSession = async (session: MuxSession): Promise<void> => {
    if (!await confirmAction({ title: t('app.deleteSession'), message: t('app.deleteValue', { value0: session.name }), detail: t('app.sessionPanesWillBeDeleted'), kind: 'danger', confirmLabel: t('app.deleteSession') })) return
    try {
      const panes = tabs.filter((pane) => pane.muxSessionId === session.id)
      for (const pane of panes) {
        if (pane.runtimeId) await window.api.terminalRuntimes.close(pane.runtimeId).catch(() => undefined)
      }
      for (const resource of browserResources.filter((item) => item.muxSessionId === session.id)) await stopBrowserResource(resource)
      await window.api.muxSessions.remove(session.id)
      for (const pane of panes) discardTerminalSnapshot(pane.id)
      const remaining = muxSessions.filter((item) => item.id !== session.id)
      setMuxSessions(remaining)
      setTabs((current) => current.filter((pane) => pane.muxSessionId !== session.id))
      setBrowserResources((current) => current.filter((resource) => resource.muxSessionId !== session.id))
      if (activeMuxSessionId === session.id) {
        setActiveMuxSessionId(remaining[0]?.id ?? '')
        setActiveKey(tabs.find((pane) => pane.muxSessionId === remaining[0]?.id)?.key ?? '')
      }
    } catch (error) { showError(errorMessage(error)) }
  }

  const connect = async (bookmark: Bookmark, credentials: ConnectionCredentials = {}, target: ConnectionTarget = {}): Promise<void> => {
    let key = target.tabKey
    let pane = key ? tabs.find((tab) => tab.key === key) : undefined
    try {
      const muxSession = muxSessions.find((session) => session.id === activeMuxSessionId)
      if (!muxSession) { showError(t('app.createSessionFirst')); return }
      const launchProfileId = target.launchAgentProfileId ?? pane?.launchProfileId ?? ''
      if (!key) {
        const title = target.paneTitle?.trim() || (launchProfileId ? (target.launchAgentLabel || 'Agent') + ' · ' + bookmark.name : bookmark.name)
        const savedPane = await window.api.muxPanes.save({ muxSessionId: muxSession.id, kind: 'terminal', title, targetId: 'ssh-bookmark:' + bookmark.id, bookmarkId: bookmark.id, launchProfileId })
        key = savedPane.id
        pane = { ...savedPane, key: savedPane.id, status: 'connecting' }
        setTabs((current) => [...current, pane!])
        const layout = insertPaneInLayout(layoutFromPanes(muxSession.layout, sessionTabs), activeKey || undefined, savedPane.id, 'horizontal')
        await persistSessionLayout(muxSession, layout)
      } else setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: undefined, runtimeId: undefined, status: 'connecting', error: undefined } : tab))
      setActiveKey(key); setAuthRequest(null)
      if (!pane) throw new Error(t('app.sshTargetUnavailable'))
      const runtimeId = crypto.randomUUID()
      const agentId = launchProfileId ? crypto.randomUUID() : undefined
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, runtimeId, agentId, status: 'connecting', error: undefined } : tab))
      const runtime = await window.api.terminalRuntimes.create({
        runtimeId,
        context: { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId },
        targetId: 'ssh-bookmark:' + bookmark.id,
        title: pane.title,
        cwd: pane.cwd || undefined,
        authentication: { type: 'ssh', options: { credential: credentials.credential, rememberCredential: Boolean(credentials.rememberCredential), jumpCredential: credentials.jumpCredential, rememberJumpCredential: Boolean(credentials.rememberJumpCredential) } },
        managedAgent: launchProfileId && agentId ? { muxSessionId: pane.muxSessionId, paneId: pane.id, runtimeId, agentId, launchProfileId } : undefined,
        cols: 100,
        rows: 30
      })
      setTabs((current) => current.map((tab) => tab.key === key ? applyTerminalRuntimeCreateResult(tab, runtimeId, runtime, agentId, true) : tab))
      if (credentials.rememberCredential || credentials.rememberJumpCredential) await reloadBookmarks()
    } catch (error) {
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, status: 'error', error: errorMessage(error) } : tab))
      showError(errorMessage(error))
    }
  }

  const openBookmark = (bookmark: Bookmark, target: ConnectionTarget = {}): void => {
    const requestedTab = target.tabKey ? tabs.find((tab) => tab.key === target.tabKey) : undefined
    if (requestedTab && ['connected', 'connecting'].includes(requestedTab.status)) { setActiveKey(requestedTab.key); return }
    const existing = target.newSession ? requestedTab : requestedTab ?? sessionTabs.find((tab) => tab.bookmarkId === bookmark.id && ['connected', 'connecting'].includes(tab.status)) ?? sessionTabs.find((tab) => tab.bookmarkId === bookmark.id)
    if (!target.tabKey && (existing?.status === 'connected' || existing?.status === 'connecting')) { setActiveKey(existing.key); return }
    const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
    const hasOtherActiveSession = Boolean(target.tabKey && sessionTabs.some((tab) => tab.key !== target.tabKey && tab.bookmarkId === bookmark.id && ['connected', 'connecting'].includes(tab.status)))
    const connectionTarget = target.tabKey ? { ...target, newSession: target.newSession ?? hasOtherActiveSession } : { ...target, tabKey: existing?.key }
    const needsPrompt = (item: Bookmark): boolean => item.authType === 'privateKey' || (item.authType === 'password' && !item.hasSavedCredential)
    if (!needsPrompt(bookmark) && (!jumpBookmark || !needsPrompt(jumpBookmark))) void connect(bookmark, {}, connectionTarget)
    else setAuthRequest({ bookmark, target: connectionTarget })
  }

  const openSshTargetAsPane = (bookmark: Bookmark): void => {
    openBookmark(bookmark, { newSession: true })
  }

  const closeTab = async (tab: WorkspaceTab, alreadyConfirmed = false): Promise<void> => {
    if (!alreadyConfirmed && tab.runtimeId && ['connected', 'connecting'].includes(tab.status)) {
      const accepted = await confirmAction({
        title: t('app.closeRunningPane'),
        message: t('app.closeRunningPaneMessage', { value0: tab.title }),
        detail: t('app.closeRunningPaneDetail'),
        kind: 'warning',
        confirmLabel: t('app.closePane')
      })
      if (!accepted) return
    }
    try {
      if (tab.runtimeId) await window.api.terminalRuntimes.close(tab.runtimeId)
      await window.api.muxPanes.remove(tab.id)
    } catch (error) {
      showError(errorMessage(error))
      return
    }
    const owner = muxSessions.find((session) => session.id === tab.muxSessionId)
    if (owner) await persistSessionLayout(owner, removePaneFromLayout(layoutFromPanes(owner.layout, tabs.filter((pane) => pane.muxSessionId === owner.id)), tab.id)).catch((error) => showError(errorMessage(error)))
    if (maximizedPaneId === tab.id) setMaximizedPaneId('')
    discardTerminalSnapshot(tab.id)
    setTabs((current) => {
      const siblings = current.filter((item) => item.muxSessionId === tab.muxSessionId)
      const index = siblings.findIndex((item) => item.key === tab.key)
      const nextSibling = siblings.filter((item) => item.key !== tab.key)[Math.min(index, siblings.length - 2)]
      const next = current.filter((item) => item.key !== tab.key)
      if (activeKey === tab.key) setActiveKey(nextSibling?.key ?? '')
      return next
    })
  }

  const removeBookmark = async (bookmark: Bookmark): Promise<void> => {
    if (!await confirmAction({ title: t('common.deleteConnection'), message: t('app.deleteValue', { value0: bookmark.name }), detail: t('app.relatedOpenSessionsWillAlsoCloseThisCannot'), kind: 'danger', confirmLabel: t('common.deleteConnection') })) return
    try {
      await window.api.bookmarks.remove(bookmark.id)
      for (const tab of tabs.filter((item) => item.bookmarkId === bookmark.id)) await closeTab(tab, true)
      setSelectedBookmarkId((current) => current === bookmark.id ? '' : current)
      await reloadBookmarks()
    } catch (error) { showError(errorMessage(error)) }
  }

  const deleteBookmarkGroup = async (group: string): Promise<void> => {
    const label = group || t('common.ungrouped')
    const count = bookmarks.filter((bookmark) => bookmark.groupName === group).length
    if (!await confirmAction({ title: t('app.deleteGroup'), message: t('app.deleteGroupValue', { value0: label }), detail: t('app.valueConnectionsInThisGroupWillAlsoBe', { value0: count }), kind: 'danger', confirmLabel: t('common.deleteGroupAndConnections') })) return
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
    if (!task.bookmarkId) { showError(t('app.thisLegacyTransferHasNoConnectionInformationAnd')); return }
    const tab = tabs.find((item) => item.bookmarkId === task.bookmarkId && item.status === 'connected' && item.sessionId)
    if (!tab?.sessionId) {
      const bookmark = bookmarkMap.get(task.bookmarkId)
      const existing = tabs.find((item) => item.bookmarkId === task.bookmarkId)
      if (existing) setActiveKey(existing.key)
      else if (bookmark) openBookmark(bookmark)
      showError(t('app.connectToTheHostForThisTransferThen'))
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
        event.stopPropagation()
        setHelpDialog(true)
        return
      }
      if (document.querySelector('.modal-backdrop, .help-backdrop')) return
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.shiftKey && event.key.toLowerCase() === 't' && activeMuxSession) {
        event.preventDefault()
        event.stopPropagation()
        void openPaneLauncher()
        return
      }
      if (commandKey && event.shiftKey && event.key.toLowerCase() === 'd' && activeTab) {
        event.preventDefault()
        event.stopPropagation()
        void splitPane(activeTab, event.altKey ? 'vertical' : 'horizontal')
        return
      }
      if (commandKey && event.shiftKey && event.key.toLowerCase() === 'r' && activeTab) {
        event.preventDefault()
        event.stopPropagation()
        void restartPane(activeTab)
        return
      }
      if (commandKey && !event.shiftKey && event.key.toLowerCase() === 'w' && activeTab) {
        event.preventDefault()
        event.stopPropagation()
        closeTab(activeTab)
        return
      }
      if (event.ctrlKey && event.key === 'Tab' && sessionTabs.length > 1) {
        event.preventDefault()
        event.stopPropagation()
        const index = sessionTabs.findIndex((tab) => tab.key === activeKey)
        const offset = event.shiftKey ? -1 : 1
        setActiveKey(sessionTabs[(index + offset + sessionTabs.length) % sessionTabs.length]!.key)
      }
    }
    window.addEventListener('keydown', onKeyDown, true)
    return () => window.removeEventListener('keydown', onKeyDown, true)
  }, [activeBookmark, activeTab, activeKey, sessionTabs, activeMuxSession])


  const saveSettings = async (settings: TerminalSettings, previewImage: string, appIcon: AppIconId, theme: UiTheme, appLanguage: AppLanguage, ai: AiSettingsInput, remoteAgentIntegration: boolean): Promise<void> => {
    try {
      const terminalChanged = JSON.stringify(settings) !== JSON.stringify(terminalSettings)
      const themeChanged = theme !== uiTheme
      const languageChanged = appLanguage !== savedLanguage
      const iconChanged = appIcon !== appIcons.selected
      const aiChanged = ai.baseUrl !== aiSettings.baseUrl || ai.model !== aiSettings.model || ai.defaultShell !== aiSettings.defaultShell || ai.provider !== aiSettings.provider || ai.thinkingMode !== aiSettings.thinkingMode || Boolean(ai.apiKey)
      const remoteAgentIntegrationChanged = remoteAgentIntegration !== remoteAgentIntegrationEnabled
      const [saved, savedTheme, savedAppLanguage, selected, savedAi, savedRemoteAgentIntegration] = await Promise.all([
        terminalChanged ? window.api.settings.saveTerminal(settings) : Promise.resolve(terminalSettings),
        themeChanged ? window.api.settings.saveUiTheme(theme) : Promise.resolve(uiTheme),
        languageChanged ? window.api.settings.saveLanguage(appLanguage, getNativeMenuLabels(appLanguage)) : Promise.resolve(savedLanguage),
        iconChanged ? window.api.settings.setAppIcon(appIcon) : Promise.resolve(appIcons.selected),
        aiChanged ? window.api.ai.saveSettings(ai) : Promise.resolve(aiSettings),
        remoteAgentIntegrationChanged ? window.api.settings.saveRemoteAgentIntegrationEnabled(remoteAgentIntegration) : Promise.resolve(remoteAgentIntegrationEnabled),
      ])
      let image = saved.backgroundImagePath === settings.backgroundImagePath ? previewImage : ''
      if (saved.backgroundImagePath && !image) image = await window.api.settings.loadTerminalBackground(saved.backgroundImagePath)
      setTerminalSettings(saved)
      setUiTheme(savedTheme)
      setSavedLanguage(savedAppLanguage)
      setLanguage(savedAppLanguage)
      setUiThemePreview(null)
      setTerminalBackground(image)
      setAppIcons((current) => ({ ...current, selected }))
      setAiSettings(savedAi)
      setRemoteAgentIntegrationEnabled(savedRemoteAgentIntegration)
      setSettingsDialog(false)
    } catch (error) { setLanguage(savedLanguage); showError(errorMessage(error)) }
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
  const exportConnectionArchive = async (): Promise<void> => {
    try {
      const path = await window.api.bookmarks.exportArchive()
      if (path) showToast(t('app.connectionBackupExportedValue', { value0: path }), 'success')
    } catch (error) { showError(errorMessage(error)) }
  }
  const importConnectionArchive = async (): Promise<void> => {
    try {
      const preview = await window.api.bookmarks.previewArchive()
      if (preview) setArchiveImportPreview(preview)
    } catch (error) { showError(errorMessage(error)) }
  }

  const splitPane = async (pane: WorkspaceTab, direction: 'horizontal' | 'vertical'): Promise<void> => {
    const owner = muxSessions.find((session) => session.id === pane.muxSessionId)
    if (!owner) return
    try {
      const savedPane = await window.api.muxPanes.save({ muxSessionId: owner.id, kind: pane.kind, title: pane.title, targetId: pane.targetId, bookmarkId: pane.bookmarkId, cwd: pane.cwd })
      const nextPane: WorkspaceTab = { ...savedPane, key: savedPane.id, status: 'disconnected' }
      setTabs((current) => [...current, nextPane])
      await persistSessionLayout(owner, insertPaneInLayout(layoutFromPanes(owner.layout, sessionTabs), pane.id, savedPane.id, direction))
      setActiveKey(savedPane.id)
    } catch (error) { showError(errorMessage(error)) }
  }

  const reconnectPane = (pane: WorkspaceTab): void => {
    const bookmark = pane.bookmarkId ? bookmarkMap.get(pane.bookmarkId) : undefined
    if (bookmark) openBookmark(bookmark, { tabKey: pane.key, newSession: true, launchAgentProfileId: pane.launchProfileId || undefined })
    else void startLocalPane(pane)
  }

  const reauthenticatePane = (pane: WorkspaceTab): void => {
    const bookmark = pane.bookmarkId ? bookmarkMap.get(pane.bookmarkId) : undefined
    if (!bookmark) { showError(t('app.sshTargetUnavailable')); return }
    setActiveMuxSessionId(pane.muxSessionId)
    setActiveKey(pane.key)
    setAuthRequest({ bookmark, target: { tabKey: pane.key, newSession: true, launchAgentProfileId: pane.launchProfileId || undefined } })
  }

  const restartPane = async (pane: WorkspaceTab): Promise<void> => {
    if (pane.runtimeId) await window.api.terminalRuntimes.close(pane.runtimeId).catch((error) => showError(errorMessage(error)))
    const stopped = { ...pane, runtimeId: undefined, sessionId: undefined, status: 'disconnected' as SessionStatus, error: undefined }
    setTabs((current) => current.map((item) => item.id === pane.id ? stopped : item))
    reconnectPane(stopped)
  }

  const dismissAgentWaitingAttention = (agent: ManagedAgentSummary): void => {
    const event = agent.latestAttention
    if (!event || event.status !== 'waiting') return
    setDismissedAgentAttentionSequences((current) => {
      if (current.has(event.sequence)) return current
      const next = new Set(current)
      next.add(event.sequence)
      return next
    })
    markAgentRead(agent.agentId)
  }

  const markAgentRead = (agentId: string): void => {
    setReadAgentSequences((current) => {
      const next = new Set(current)
      for (const event of managedAgentEvents) if (event.context.agentId === agentId) next.add(event.sequence)
      return next
    })
  }

  const resizeLayout = (path: string, ratio: number, persist: boolean): void => {
    if (!activeMuxSession) return
    const currentLayout = layoutFromPanes(activeMuxSession.layout, sessionTabs)
    if (!currentLayout) return
    const layout = setSplitRatio(currentLayout, path, ratio)
    setMuxSessions((current) => current.map((session) => session.id === activeMuxSession.id ? { ...session, layout } : session))
    if (persist) void persistSessionLayout(activeMuxSession, layout).catch((error) => showError(errorMessage(error)))
  }

  const applyLayoutPreset = async (preset: LayoutPreset): Promise<void> => {
    if (!activeMuxSession) return
    const currentLayout = layoutFromPanes(activeMuxSession.layout, sessionTabs)
    if (!currentLayout) return
    const layout = layoutForPreset(paneIdsInLayout(currentLayout), preset)
    setLayoutMenuOpen(false)
    setMaximizedPaneId('')
    try { await persistSessionLayout(activeMuxSession, layout) }
    catch (error) { showError(errorMessage(error)) }
  }

  const renamePane = async (pane: WorkspaceTab, title: string): Promise<void> => {
    try {
      const saved = await window.api.muxPanes.save({
        id: pane.id,
        muxSessionId: pane.muxSessionId,
        kind: pane.kind,
        title,
        targetId: pane.targetId,
        bookmarkId: pane.bookmarkId,
        cwd: pane.cwd,
        command: pane.command,
        launchProfileId: pane.launchProfileId
      })
      setTabs((current) => current.map((item) => item.id === pane.id ? { ...item, ...saved, key: item.key } : item))
      setPaneRenameDialog(null)
    } catch (error) { showError(errorMessage(error)) }
  }
  const importLunaRemoteDatabase = async (): Promise<void> => {
    try {
      const sources = await window.api.bookmarks.discoverLunaRemoteSources()
      setLunaRemoteSources(sources)
    } catch (error) { showError(errorMessage(error)) }
  }
  const previewLunaRemoteSource = async (path: string): Promise<void> => {
    try {
      const preview = await window.api.bookmarks.previewLunaRemote(path)
      setLunaRemoteSources(null)
      setLunaRemoteImportPreview(preview)
    } catch (error) { showError(errorMessage(error)) }
  }
  const chooseLunaRemoteDatabase = async (): Promise<void> => {
    try {
      const preview = await window.api.bookmarks.chooseLunaRemoteDatabase()
      if (preview) { setLunaRemoteSources(null); setLunaRemoteImportPreview(preview) }
    } catch (error) { showError(errorMessage(error)) }
  }

  const contextBookmark = sidebarContextMenu?.bookmark ?? selectedBookmark

  return <div className={`app-shell platform-${window.api.platform}`} data-theme={resolvedUiTheme} data-theme-mode={activeThemeMode}>
    <header className="titlebar" data-tauri-drag-region="deep">
      <div className="app-brand">{selectedAppIcon ? <img className="app-logo" src={selectedAppIcon.dataUrl} alt="" /> : <span className="app-logo app-logo-fallback">&gt;_</span>}<span>{PRODUCT_INFO.displayName}</span></div>
      <span className="titlebar-spacer" />
      <button className="icon-button titlebar-help" title={t('app.helpF1')} aria-label={t('app.help')} onClick={() => setHelpDialog(true)}><CircleHelp size={18} /></button>
      <button className="icon-button titlebar-settings" title={t('common.settings')} aria-label={t('common.settings')} onClick={() => openSettings()}><SettingsIcon size={18} /></button>
      <div className="window-controls">
        <button className="window-control" title={t('common.minimize')} aria-label={t('common.minimize')} onClick={() => void window.api.system.minimizeWindow().catch((error) => showError(errorMessage(error)))}><Minus size={16} /></button>
        <button className="window-control" title={t('common.maximizeOrRestore')} aria-label={t('common.maximizeOrRestore')} onClick={() => void window.api.system.toggleMaximizeWindow().catch((error) => showError(errorMessage(error)))}><Square size={13} /></button>
        <button className="window-control close" title={t('common.close')} aria-label={t('common.close')} onClick={() => void window.api.system.closeWindow().catch((error) => showError(errorMessage(error)))}><X size={17} /></button>
      </div>
    </header>
    <div className={`main-layout ${sidebarCollapsed ? 'sidebar-collapsed' : ''}`} style={{ '--sidebar-width': `${sidebarWidth}px` } as React.CSSProperties}>
      <aside className="sidebar">
        <div className="sidebar-heading mux-heading"><div className="session-nav-title"><strong>{t('app.sessions')}</strong><small>{muxSessions.length}</small></div><div className="sidebar-actions">
          <button className="icon-button" title={t('app.newSession')} aria-label={t('app.newSession')} onClick={() => setMuxSessionDialog({ mode: 'create' })}><CirclePlus size={18} /></button>
        </div></div>
        <div className="mux-session-list">
          {muxSessions.map((session) => {
            const active = session.id === activeMuxSessionId
            const expanded = !collapsedMuxSessionIds.has(session.id)
            const panes = tabs.filter((pane) => pane.muxSessionId === session.id)
            return <div className={`mux-session-item ${active ? 'active' : ''}`} key={session.id}>
              <div className="mux-session-row" onContextMenu={(event) => openMuxSidebarContextMenu(event, { session })}>
                <button className="mux-session-select" aria-expanded={expanded} title={session.rootPath || session.name} onClick={() => selectMuxSession(session.id)}>
                  {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
                  <span><strong>{session.name}</strong>{session.rootPath && <small>{session.rootPath}</small>}</span>
                </button>
                <div className="mux-session-actions">
                  <button className="icon-button" title={t('app.addPane')} aria-label={t('app.addPane')} onClick={() => void openPaneLauncher(session.id)}><Plus size={15} /></button>
                </div>
              </div>
              {expanded && <div className="mux-pane-list">
                {panes.map((pane) => {
                  const agent = allAgents.find((item) => item.paneId === pane.id)
                  const tone = agentAttentionByPane.get(pane.id)
                  const attentionLabel = tone === 'error' ? t('app.agentError') : tone === 'warning' ? t('app.agentWaiting') : t('app.unreadAgentEvents')
                  const paneLabel = pane.error ? `${pane.title} - ${t('app.connectionFailed')}: ${pane.error}` : tone ? `${pane.title} - ${attentionLabel}` : pane.title
                  return <div className={`mux-pane-item ${pane.key === activeKey ? 'active' : ''} ${agent?.unread ? 'unread' : ''} ${tone ? `attention-${tone}` : ''} ${pane.error ? 'has-error' : ''}`} key={pane.key} onContextMenu={(event) => openMuxSidebarContextMenu(event, { pane })}>
                    <button className="mux-pane-select" title={paneLabel} onClick={() => { setActiveMuxSessionId(pane.muxSessionId); setActiveKey(pane.key); setWorkspaceView('terminal'); if (agent) markAgentRead(agent.agentId) }} onDoubleClick={() => { if (pane.status !== 'connected') reconnectPane(pane) }}>
                      <span className={`status-dot ${pane.status}`} />
                      {agent ? <Sparkles size={14} /> : pane.bookmarkId ? <Server size={14} /> : <SquareTerminal size={14} />}
                      <span className="mux-pane-copy">
                        <span className="mux-pane-title">{pane.title}</span>
                        {pane.error && <small className="mux-pane-error">{pane.error}</small>}
                      </span>
                    </button>
                    <div className="mux-pane-actions">
                      <button title={t('app.closePane')} aria-label={t('app.closePane')} onClick={() => closeTab(pane)}><X size={13} /></button>
                    </div>
                  </div>
                })}
                {panes.length === 0 && <div className="mux-pane-empty">{t('app.noPanes')}</div>}
              </div>}
            </div>
          })}
        </div>
      </aside>
      <div className="sidebar-resizer" role="separator" aria-label={t('app.resizeSidebar')} aria-orientation="vertical" aria-valuemin={minSidebarWidth} aria-valuemax={maxSidebarWidth} aria-valuenow={sidebarWidth} tabIndex={0} title={t('app.dragToResizeDoubleClickToReset')} onPointerDown={startSidebarResize} onDoubleClick={() => saveSidebarWidth(defaultSidebarWidth)} onKeyDown={resizeSidebarWithKeyboard} />
      <main className={`workspace ${sessionTabs.length && workspaceView === 'files' ? 'file-mode' : ''}`}>
        <div className="session-toolbar">
          <div className="session-toolbar-leading"><button className="icon-button" title={sidebarCollapsed ? t('app.showWorkspace') : t('app.hideWorkspace')} aria-label={sidebarCollapsed ? t('app.showWorkspace') : t('app.hideWorkspace')} aria-expanded={!sidebarCollapsed} onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}>
            {sidebarCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
          </button></div>
          <div className="view-switcher" role="tablist" aria-label={t('app.sessionView')}>
            <button role="tab" disabled={!activeMuxSession} aria-selected={workspaceView === 'terminal'} className={workspaceView === 'terminal' ? 'active' : ''} onClick={() => setWorkspaceView('terminal')}><Columns2 size={15} />{t('app.panes')}</button>
            <button role="tab" disabled={!activeMuxSession} aria-selected={workspaceView === 'agents'} className={workspaceView === 'agents' ? 'active' : ''} onClick={() => setWorkspaceView('agents')}><Sparkles size={15} />{t('app.agents')}</button>
            <button role="tab" disabled={!activeMuxSession} aria-selected={workspaceView === 'browser'} className={workspaceView === 'browser' ? 'active' : ''} onClick={() => setWorkspaceView('browser')}><Globe2 size={15} />{t('app.browserResources')}</button>
            {activeBookmark && <button role="tab" aria-selected={workspaceView === 'files'} className={workspaceView === 'files' ? 'active' : ''} onClick={() => setWorkspaceView('files')}><FolderOpen size={15} />{t('app.files')}</button>}
          </div>
          <div className="session-actions">
            {workspaceView === 'files' && activeBookmark && activeSshRuntimeId && <button className="secondary-button" onClick={() => setDeploymentDialog(true)}><Rocket size={15} />{t('app.deploy')}</button>}
            {workspaceView === 'terminal' && activeAiCommandTarget && <button className="secondary-button" onClick={() => setAiDialog(true)}><WandSparkles size={15} />{t('app.aiCommand')}</button>}
            {workspaceView !== 'browser' && activeBookmark && activeTab?.status === 'connected' && activeSshRuntimeId && <button className="secondary-button" onClick={() => setTunnelDialog(true)}><Network size={15} />{t('app.portForwarding')}</button>}
            {workspaceView === 'terminal' && sessionTabs.length > 1 && <div className="layout-menu-anchor" onPointerDown={(event) => event.stopPropagation()}>
              <button className="icon-button" title={t('app.arrangePanes')} aria-label={t('app.arrangePanes')} aria-haspopup="menu" aria-expanded={layoutMenuOpen} onClick={() => setLayoutMenuOpen((open) => !open)}><LayoutGrid size={17} /></button>
              {layoutMenuOpen && <div className="layout-menu" role="menu" aria-label={t('app.arrangePanes')}>
                <button role="menuitem" onClick={() => void applyLayoutPreset('horizontal')}><Columns3 size={16} />{t('app.horizontalLayout')}</button>
                <button role="menuitem" onClick={() => void applyLayoutPreset('vertical')}><Rows3 size={16} />{t('app.verticalLayout')}</button>
                <button role="menuitem" onClick={() => void applyLayoutPreset('twoColumns')}><Grid2x2 size={16} />{t('app.twoColumnLayout')}</button>
              </div>}
            </div>}
          </div>
        </div>
        <div className="session-stack">
          {muxSessions.map((session) => {
            const panes = tabs.filter((pane) => pane.muxSessionId === session.id)
            const layout = layoutFromPanes(session.layout, panes)
            if (!layout) return null
            const shown = session.id === activeMuxSessionId && workspaceView === 'terminal'
            const hasBackgroundImage = Boolean(terminalSettings.backgroundImagePath && terminalBackground)
            return <div key={session.id} hidden={!shown} className={`terminal-workspace ${hasBackgroundImage ? 'has-background-image' : ''}`} style={terminalBackgroundStyle(terminalSettings, terminalBackground)}><MuxLayout node={layout} panes={panes} bookmarks={bookmarkMap} activePaneId={shown ? activeKey : ''} settings={terminalSettings} backgroundImage={terminalBackground} agentAttentionByPane={agentAttentionByPane} terminalPaneRefs={terminalPaneRefs} onFocus={(pane) => { setActiveKey(pane.key); const agent = allAgents.find((item) => item.paneId === pane.id); if (agent) markAgentRead(agent.agentId) }} onTerminalAgentAction={(pane) => { const agent = allAgents.find((item) => item.paneId === pane.id); if (agent) dismissAgentWaitingAttention(agent) }} onRuntimeError={(pane, runtimeId, message) => { setTabs((current) => current.map((item) => item.key === pane.key && item.runtimeId === runtimeId ? { ...item, status: 'error', error: message } : item)); showError(message) }} onReconnect={reconnectPane} onReauthenticate={reauthenticatePane} onSplit={(pane, direction) => void splitPane(pane, direction)} onClose={closeTab} onResize={resizeLayout} onToggleMaximize={(paneId) => setMaximizedPaneId((current) => current ? '' : paneId)} onOpenSettings={() => openSettings('terminal')} maximizedPaneId={shown ? maximizedPaneId : ''} /></div>
          })}
          {!activeMuxSession ? <div className="welcome-state"><div className="welcome-icon"><SquareTerminal size={30} /></div><h2>{t('app.createYourFirstSession')}</h2><div className="welcome-actions"><button className="primary-button" onClick={() => setMuxSessionDialog({ mode: 'create' })}><CirclePlus size={16} />{t('app.newSession')}</button></div></div>
            : sessionTabs.length === 0 && sessionBrowserResources.length === 0 ? <div className="welcome-state"><div className="welcome-icon"><SquareTerminal size={30} /></div><h2>{activeMuxSession.name}</h2>{activeMuxSession.rootPath && <p>{activeMuxSession.rootPath}</p>}<div className="welcome-actions"><button className="primary-button" onClick={() => void openPaneLauncher()}><CirclePlus size={16} />{t('app.addFirstPane')}</button></div></div>
              : workspaceView === 'terminal' && sessionTabs.length === 0 ? <div className="welcome-state pane-empty-state"><div className="welcome-icon"><SquareTerminal size={30} /></div><h2>{t('app.noPanes')}</h2><p>{t('app.addPaneDescription')}</p><div className="welcome-actions"><button className="primary-button" onClick={() => void openPaneLauncher()}><CirclePlus size={16} />{t('app.addFirstPane')}</button></div></div>
                : workspaceView === 'agents' ? <AgentEnvironmentPanel agents={allAgents} session={activeMuxSession} panes={sessionTabs} browserResources={sessionBrowserResources} browserRuntimes={browserRuntimes.filter((runtime) => runtime.muxSessionId === activeMuxSession.id)} chromeInstallation={chromeInstallation} bookmarks={bookmarks} />
                  : workspaceView === 'browser' ? <BrowserResourceManager resources={sessionBrowserResources} panes={sessionTabs} chromeInstallation={chromeInstallation} onRefreshChrome={refreshChromeInstallation} onStart={(resource) => void startBrowserResource(resource)} onFocus={(resource) => { if (resource.runtime) void window.api.browserRuntimes.focusExternal(resource.runtime.id).catch((error) => showError(errorMessage(error))) }} onRestart={(resource) => void restartBrowserResource(resource)} onStop={(resource) => void stopBrowserResource(resource)} />
                    : workspaceView === 'files' && activeTab && activeBookmark ? <div className="session-view active files"><div className="sftp-region"><SftpPane sessionId={activeSshRuntimeId} bookmarkId={activeTab.bookmarkId} connected={activeTab.status === 'connected'} visible onError={showError} onConnect={() => reconnectPane(activeTab)} /></div></div>
                      : null}
        </div>
        {activeBookmark && sessionTabs.length > 0 && workspaceView === 'files' && <TransferPanel transfers={transfers} view={transferView} setView={setTransferView} onRetry={(task) => void retryTransfer(task)} onCancel={(task) => void cancelTransfer(task)} onClear={() => void clearCompletedTransfers()} />}
      </main>
    </div>
    {muxSidebarContextMenu && <div className="sidebar-context-menu" role="menu" style={{ left: muxSidebarContextMenu.x, top: muxSidebarContextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      {muxSidebarContextMenu.session ? <>
        <button role="menuitem" onClick={() => { const session = muxSidebarContextMenu.session; setMuxSidebarContextMenu(null); if (session) setMuxSessionDialog({ mode: 'rename', session }) }}><Edit3 size={15} />{t('app.renameSession')}</button>
        <button role="menuitem" className="danger" onClick={() => { const session = muxSidebarContextMenu.session; setMuxSidebarContextMenu(null); if (session) void removeMuxSession(session) }}><Trash2 size={15} />{t('app.deleteSession')}</button>
      </> : muxSidebarContextMenu.pane ? <button role="menuitem" onClick={() => { const pane = muxSidebarContextMenu.pane; setMuxSidebarContextMenu(null); if (pane) setPaneRenameDialog(pane) }}><Edit3 size={15} />{t('app.renamePane')}</button> : null}
    </div>}
    {sidebarContextMenu && <div className="sidebar-context-menu" role="menu" style={{ left: sidebarContextMenu.x, top: sidebarContextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      {sidebarContextMenu.group !== undefined ? <>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setGroupDialog({ mode: 'create' }) }}><FolderPlus size={15} />{t('common.newGroup')}</button>
        <button role="menuitem" onClick={() => { const group = sidebarContextMenu.group ?? ''; setSidebarContextMenu(null); setGroupDialog({ mode: 'rename', group }) }}><Edit3 size={15} />{t('common.renameGroup')}</button>
        <button role="menuitem" className="danger" onClick={() => { const group = sidebarContextMenu.group ?? ''; setSidebarContextMenu(null); void deleteBookmarkGroup(group) }}><Trash2 size={15} />{t('common.deleteGroupAndConnections')}</button>
      </> : <>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); void chooseSshConfig() }}><FileInput size={15} />{t('common.importOpensshConfig')}</button>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); void importConnectionArchive() }}><Upload size={15} />{t('app.importConnectionBackup')}</button>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); void importLunaRemoteDatabase() }}><DatabaseIcon size={15} />{t('app.importFromLunaRemote')}</button>
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); void exportConnectionArchive() }}><Download size={15} />{t('app.exportConnectionBackup')}</button>
        <div className="context-menu-separator" />
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setBookmarkDialog('new') }}><CirclePlus size={15} />{t('common.newConnection')}</button>
        <button role="menuitem" disabled={!contextBookmark} onClick={() => { setSidebarContextMenu(null); if (contextBookmark) setBookmarkDialog(contextBookmark) }}><Edit3 size={15} />{t('common.editConnection')}</button>
        <button role="menuitem" disabled={!contextBookmark} onClick={() => { setSidebarContextMenu(null); if (contextBookmark) void duplicateBookmark(contextBookmark) }}><Copy size={15} />{t('app.duplicate')}</button>
        <button role="menuitem" disabled={!contextBookmark} className="danger" onClick={() => { setSidebarContextMenu(null); if (contextBookmark) void removeBookmark(contextBookmark) }}><Trash2 size={15} />{t('common.deleteConnection')}</button>
        <div className="context-menu-separator" />
        <button role="menuitem" onClick={() => { setSidebarContextMenu(null); setGroupDialog({ mode: 'create' }) }}><FolderPlus size={15} />{t('common.newGroup')}</button>
      </>}
    </div>}
    {connectionLibraryDialog && <Modal title={t('app.sshTargetLibrary')} onClose={() => setConnectionLibraryDialog(false)} wide className="connection-library-dialog"><div className="resource-library">
      <div className="resource-library-toolbar">
        <label className="search-box"><Search size={16} /><input placeholder={t('app.searchSshTargets')} value={query} onChange={(event) => setQuery(event.target.value)} /></label>
        <div className="resource-library-actions">
          <button className="secondary-button" title={t('common.importOpensshConfig')} onClick={() => void chooseSshConfig()}><FileInput size={15} />{t('common.import')}</button>
          <button className="primary-button" onClick={() => setBookmarkDialog('new')}><CirclePlus size={15} />{t('common.newConnection')}</button>
        </div>
      </div>
      <div className="resource-library-meta"><span>{t('app.sshTargetCount', { value0: bookmarks.length })}</span><span>{t('app.sshTargetsArePaneResources')}</span></div>
      <div className="bookmark-list resource-bookmark-list" onContextMenu={(event) => openSidebarContextMenu(event)}>
        {filteredBookmarks.length === 0 && <div className="sidebar-empty"><BookmarkIcon size={24} /><span>{query ? t('app.noMatchingConnections') : t('app.noSshTargets')}</span></div>}
        {bookmarkGroups.flatMap(([group, items]) => {
          const collapsed = !query && collapsedBookmarkGroups.has(group)
          const groupDropClass = groupDrop?.group === group ? `drop-${groupDrop.position}` : ''
          return [<button type="button" data-group-name={group} className={`bookmark-group ${draggedGroupName === group ? 'dragging' : ''} ${groupDropClass}`} key={`group:${group}`} aria-expanded={!collapsed} onPointerDown={(event) => startSidebarPointerDrag(event, 'group', group)} onClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } toggleBookmarkGroup(group) }} onContextMenu={(event) => openSidebarContextMenu(event, { group })}>
            <span className="group-drag-handle" title={query ? t('common.clearTheSearchToReorder') : t('app.dragToReorderGroup')}><GripVertical size={13} /></span>{collapsed ? <ChevronRight size={13} /> : <ChevronDown size={13} />}{collapsed ? <Folder size={14} /> : <FolderOpen size={14} />}<span>{group || t('common.ungrouped')}</span><small>{items.length}</small>
          </button>, ...(collapsed ? [] : items.map((bookmark) => {
            const tab = sessionTabs.find((item) => item.bookmarkId === bookmark.id)
            const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
            const dropClass = bookmarkDrop?.id === bookmark.id ? `drop-${bookmarkDrop.position}` : ''
            const launch = (): void => { setConnectionLibraryDialog(false); openSshTargetAsPane(bookmark) }
            return <button key={bookmark.id} data-bookmark-id={bookmark.id} className={`bookmark-item ${selectedBookmark?.id === bookmark.id ? 'active' : ''} ${draggedBookmarkId === bookmark.id ? 'dragging' : ''} ${dropClass}`} onPointerDown={(event) => startSidebarPointerDrag(event, 'bookmark', bookmark.id)} onClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } setSelectedBookmarkId(bookmark.id) }} onDoubleClick={(event) => { if (suppressSidebarClickRef.current) { event.preventDefault(); return } launch() }} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); setSelectedBookmarkId(bookmark.id); launch() } }} onContextMenu={(event) => openSidebarContextMenu(event, { bookmark })}>
              <span className="bookmark-drag-handle" title={query ? t('common.clearTheSearchToReorder') : t('app.dragToReorder')}><GripVertical size={14} /></span>
              <span className={`server-icon ${tab?.status ?? 'disconnected'}`}><Server size={17} /></span>
              <span className="bookmark-copy"><strong>{bookmark.name}</strong><small title={jumpBookmark ? t('app.viaValue', { value0: jumpBookmark.name }) : undefined}>{bookmark.username}@{bookmark.host}:{bookmark.port}{jumpBookmark ? t('common.viaValue', { value0: jumpBookmark.name }) : ''}</small></span>
              <span className="bookmark-flags">{bookmark.favorite && <Star size={12} className="favorite-star" fill="currentColor" />}{bookmark.hasSavedCredential && <KeyRound size={12} className="saved-key" />}</span>
            </button>
          }))]
        })}
      </div>
      <div className="resource-library-footer"><span>{t('app.doubleClickTargetToAddPane')}</span><div><button className="text-button" onClick={() => void importConnectionArchive()}>{t('app.importConnectionBackup')}</button><button className="text-button" onClick={() => void importLunaRemoteDatabase()}>{t('app.importFromLunaRemote')}</button><button className="text-button" onClick={() => void exportConnectionArchive()}>{t('app.exportConnectionBackup')}</button></div></div>
    </div></Modal>}
    {groupDialog && <GroupNameDialog mode={groupDialog.mode} initialName={groupDialog.group ?? ''} onClose={() => setGroupDialog(null)} onSave={(name) => void saveGroup(name)} />}
    {paneLauncher && <PaneLauncherDialog targets={paneLauncher.targets} loading={paneLauncher.loading} onClose={() => setPaneLauncher(null)} onManageConnections={() => { setPaneLauncher(null); setConnectionLibraryDialog(true) }} onSelect={(target, paneTitle) => {
      if (target.transport === 'ssh') {
        const bookmark = bookmarkMap.get(target.id.replace(/^ssh-bookmark:/, ''))
        if (!bookmark) { showError(t('app.sshTargetUnavailable')); return }
        setPaneLauncher(null)
        openBookmark(bookmark, { newSession: true, paneTitle })
      } else void openLocalTerminal(target, paneTitle)
    }} />}
    {muxSessionDialog && <MuxSessionDialog mode={muxSessionDialog.mode} session={muxSessionDialog.session} onClose={() => setMuxSessionDialog(null)} onSave={(name, rootPath) => void saveMuxSession(name, rootPath)} />}
    {paneRenameDialog && <PaneNameDialog pane={paneRenameDialog} onClose={() => setPaneRenameDialog(null)} onSave={(title) => void renamePane(paneRenameDialog, title)} />}
    {bookmarkDialog && <BookmarkDialog bookmark={bookmarkDialog === 'new' ? undefined : bookmarkDialog} connections={bookmarks} groups={bookmarkGroupNames} onClose={() => setBookmarkDialog(null)} onSaved={async () => { setBookmarkDialog(null); await reloadSidebarData() }} onError={showError} />}
    {authRequest && <AuthDialog bookmark={authRequest.bookmark} jumpBookmark={authRequest.bookmark.jumpBookmarkId ? bookmarkMap.get(authRequest.bookmark.jumpBookmarkId) : undefined} onClose={() => setAuthRequest(null)} onConnect={(credentials) => void connect(authRequest.bookmark, credentials, authRequest.target)} />}
    {hostPrompts[0] && <HostKeyDialog prompt={hostPrompts[0]} onDecision={(accept) => { const prompt = hostPrompts[0]; if (!prompt) return; window.api.sessions.hostKeyDecision(prompt.sessionId, accept); setHostPrompts((current) => current.filter((item) => item !== prompt)) }} />}
    {conflict && <ConflictDialog conflict={conflict} onDecision={(resolution, apply) => { window.api.transfers.resolveConflict(conflict.taskId, resolution, apply); setConflict(null) }} />}
    {settingsDialog && <SettingsDialog initialSection={settingsInitialSection} settings={terminalSettings} backgroundImage={terminalBackground} appIcons={appIcons} uiTheme={uiTheme} appLanguage={savedLanguage} aiSettings={aiSettings} remoteAgentIntegrationEnabled={remoteAgentIntegrationEnabled} onAiSettingsChange={setAiSettings} onThemePreview={setUiThemePreview} onLanguagePreview={setLanguage} onClose={() => { setUiThemePreview(null); setLanguage(savedLanguage); setSettingsDialog(false) }} onSave={(settings, image, icon, theme, appLanguage, ai, remoteAgentIntegration) => void saveSettings(settings, image, icon, theme, appLanguage, ai, remoteAgentIntegration)} onConfirm={confirmAction} onError={showError} />}
    {aiDialog && activeAiCommandTarget && activeTab && <AiCommandDialog target={activeAiCommandTarget} settings={aiSettings} getTerminalContext={() => terminalPaneRefs.current.get(activeTab.key)?.getRecentLines(aiTerminalContextLines, aiTerminalContextChars) ?? ''} onSettingsChange={setAiSettings} onSettings={() => { setAiDialog(false); openSettings('ai') }} onClose={() => setAiDialog(false)} onError={showError} />}
    {helpDialog && <HelpDialog onClose={() => setHelpDialog(false)} />}
    {deploymentDialog && activeBookmark && activeSshRuntimeId && <DeploymentDialog bookmark={activeBookmark} sessionId={activeSshRuntimeId} onClose={() => setDeploymentDialog(false)} onConfirm={confirmAction} onError={showError} />}
    {tunnelDialog && activeBookmark && activeSshRuntimeId && <TunnelDialog bookmark={activeBookmark} sessionId={activeSshRuntimeId} onClose={() => setTunnelDialog(false)} onError={showError} />}
    {importPreview && <SshConfigImportDialog preview={importPreview} onClose={() => setImportPreview(null)} onImported={async () => { setImportPreview(null); await reloadSidebarData() }} onError={showError} />}
    {archiveImportPreview && <BookmarkArchiveImportDialog preview={archiveImportPreview} onClose={() => setArchiveImportPreview(null)} onImported={async (result) => { setArchiveImportPreview(null); await reloadSidebarData(); showToast(t('app.importedValueConnectionsAndValueNewGroupsCredentials', { value0: result.importedConnections, value1: result.importedGroups }), 'success') }} onError={showError} />}
    {lunaRemoteSources && <LunaRemoteSourceDialog sources={lunaRemoteSources} onClose={() => setLunaRemoteSources(null)} onChoose={() => void chooseLunaRemoteDatabase()} onSelect={(path) => void previewLunaRemoteSource(path)} />}
    {lunaRemoteImportPreview && <LunaRemoteImportDialog preview={lunaRemoteImportPreview} onClose={() => setLunaRemoteImportPreview(null)} onImported={async (result) => { setLunaRemoteImportPreview(null); await Promise.all([reloadSidebarData(), reloadImportedSettings()]); const credentialText = result.unavailableCredentials ? t('app.importedLunaRemoteWithUnavailableCredentials', { value0: result.unavailableCredentials }) : ''; showToast(`${t('app.importedLunaRemoteData', { value0: result.importedConnections, value1: result.importedGroups, value2: result.importedHostKeys, value3: result.importedSettings, value4: result.importedForwardingProfiles, value5: result.importedCredentials })}${credentialText}`, 'success') }} onConfirm={confirmAction} onError={showError} />}
    {confirmation && <ConfirmationDialog confirmation={confirmation} onDecision={resolveConfirmation} />}
    {toast && <div className={`toast ${toastKind}`}>{toast}</div>}
  </div>
}

function AiCommandDialog({ target, settings, getTerminalContext, onSettingsChange, onSettings, onClose, onError }: { target: AiCommandTarget; settings: AiSettings; getTerminalContext(): string; onSettingsChange(settings: AiSettings): void; onSettings(): void; onClose(): void; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [requirement, setRequirement] = useState('')
  const [shell, setShell] = useState<AiShell>(target.initialShell)
  const [suggestion, setSuggestion] = useState<AiCommandSuggestion | null>(null)
  const [command, setCommand] = useState('')
  const [edited, setEdited] = useState(false)
  const [loading, setLoading] = useState(false)
  const [executing, setExecuting] = useState(false)
  const [error, setError] = useState('')
  const [confirming, setConfirming] = useState<AiRiskAssessment | null>(null)
  const [confirmationText, setConfirmationText] = useState('')
  const [includeTerminalContext, setIncludeTerminalContext] = useState(false)
  const [redactTerminalContext, setRedactTerminalContext] = useState(true)
  const [rawExchange, setRawExchange] = useState<AiRawExchange | null>(null)
  const [history, setHistory] = useState<AiCommandHistoryEntry[] | null>(null)

  const effectiveAssessment = (local: AiRiskAssessment): AiRiskAssessment => {
    if (!suggestion || edited) return local
    const rank = { low: 0, medium: 1, high: 2 } as const
    const riskLevel = rank[suggestion.riskLevel] > rank[local.riskLevel] ? suggestion.riskLevel : local.riskLevel
    return { riskLevel, warnings: [...new Set([...suggestion.warnings, ...local.warnings])] }
  }

  const selectShell = async (value: AiShell): Promise<void> => {
    setShell(value)
    try {
      const saved = await window.api.ai.saveSettings({ baseUrl: settings.baseUrl, model: settings.model, defaultShell: value, provider: settings.provider, thinkingMode: settings.thinkingMode })
      onSettingsChange(saved)
    } catch (saveError) { onError(errorMessage(saveError)) }
  }
  const generate = async (): Promise<void> => {
    if (!requirement.trim()) return
    const terminalContext = includeTerminalContext ? getTerminalContext() : undefined
    if (includeTerminalContext && !terminalContext?.trim()) { setError(t('app.theCurrentTerminalHasNoTextToInclude')); return }
    setLoading(true); setError(''); setSuggestion(null); setCommand(''); setEdited(false)
    try {
      const result = await window.api.ai.generate(requirement.trim(), shell, terminalContext, includeTerminalContext && redactTerminalContext)
      setSuggestion(result); setCommand(result.command)
    } catch (generateError) { setError(errorMessage(generateError)) }
    finally { setLoading(false) }
  }
  const requireConnectedRuntime = (): string | null => {
    if (!target.connected || !target.runtimeId) { setError(t('app.theTerminalIsDisconnectedAndCannotReceive')); return null }
    return target.runtimeId
  }
  const fillCommand = async (): Promise<void> => {
    const runtimeId = requireConnectedRuntime()
    if (!runtimeId) return
    try {
      const assessment = effectiveAssessment(await window.api.ai.analyze(command))
      setSuggestion((current) => current ? { ...current, riskLevel: assessment.riskLevel, warnings: assessment.warnings } : current)
      await window.api.terminalRuntimes.write(runtimeId, command.trim())
      onClose()
    } catch (fillError) { setError(errorMessage(fillError)) }
  }
  const executeConfirmed = async (): Promise<void> => {
    const runtimeId = requireConnectedRuntime()
    if (!runtimeId) return
    setExecuting(true)
    try {
      await window.api.ai.analyze(command)
      await window.api.terminalRuntimes.write(runtimeId, `${command.trim()}\r`)
      onClose()
    } catch (executeError) { setError(errorMessage(executeError)); setConfirming(null); setExecuting(false) }
  }
  const requestExecution = async (): Promise<void> => {
    const runtimeId = requireConnectedRuntime()
    if (!runtimeId) return
    try {
      const assessment = effectiveAssessment(await window.api.ai.analyze(command))
      setSuggestion((current) => current ? { ...current, riskLevel: assessment.riskLevel, warnings: assessment.warnings } : current)
      setConfirmationText('')
      setConfirming(assessment)
    } catch (executeError) { setError(errorMessage(executeError)) }
  }
  const riskText = suggestion ? ({ low: t('common.lowRisk'), medium: t('common.caution'), high: t('common.highRisk') } as const)[suggestion.riskLevel] : ''

  const openRawExchange = async (): Promise<void> => {
    try {
      const exchange = await window.api.ai.getLastExchange()
      if (!exchange) { setError(t('app.noAiRequestDiagnosticsAreAvailable')); return }
      setRawExchange(exchange)
    } catch (diagnosticError) { setError(errorMessage(diagnosticError)) }
  }

  const openHistory = async (): Promise<void> => {
    try { setHistory(await window.api.ai.listHistory()) }
    catch (historyError) { setError(errorMessage(historyError)) }
  }

  const useHistoryEntry = (entry: AiCommandHistoryEntry): void => {
    setRequirement(entry.requirement)
    setShell(entry.shell)
    setSuggestion(entry)
    setCommand(entry.command)
    setEdited(false)
    setError('')
    setHistory(null)
  }

  if (rawExchange) return <AiDiagnosticsDialog exchange={rawExchange} onClose={() => setRawExchange(null)} onClear={async () => { try { await window.api.ai.clearLastExchange(); setRawExchange(null) } catch (clearError) { onError(errorMessage(clearError)) } }} />
  if (history) return <AiCommandHistoryDialog entries={history} onUse={useHistoryEntry} onClose={() => setHistory(null)} onClear={async () => { try { await window.api.ai.clearHistory(); setHistory([]) } catch (clearError) { onError(errorMessage(clearError)) } }} />

  const executionConfirmation = confirming ? (() => {
    const confirmationCopy = {
      low: { title: t('app.confirmCommandExecution'), heading: t('app.confirmRunningThisCommandInTheCurrentSession') },
      medium: { title: t('app.confirmCommandExecution2'), heading: t('app.thisCommandMayChangeTheSystemOrFiles') },
      high: { title: t('app.confirmHighRiskCommand'), heading: t('app.thisCommandMayCauseIrreversibleChanges') },
    } as const
    const copy = confirmationCopy[confirming.riskLevel]
    const requiresTextConfirmation = confirming.riskLevel === 'high'
    return <Modal title={copy.title} onClose={() => setConfirming(null)} raised className="secondary-confirm-dialog"><div className={`ai-execution-confirm ${confirming.riskLevel}`}>
      <div className="ai-confirm-heading">{confirming.riskLevel === 'low' ? <SquareTerminal size={22} /> : <ShieldAlert size={22} />}<div><strong>{copy.heading}</strong><span>{target.detail}</span></div></div>
      <pre>{command.trim()}</pre>
      {confirming.warnings.length > 0 && <ul>{confirming.warnings.map((warning) => <li key={warning}>{warning}</li>)}</ul>}
      {requiresTextConfirmation && <label>{t('app.typeExecuteToContinue')}<input autoFocus value={confirmationText} onChange={(event) => setConfirmationText(event.target.value)} /></label>}
      <div className="dialog-actions"><button className="secondary-button" disabled={executing} onClick={() => setConfirming(null)}>{t('common.cancel')}</button><button className={requiresTextConfirmation ? 'danger-button' : 'primary-button'} disabled={(requiresTextConfirmation && confirmationText !== t('app.execute')) || executing} onClick={() => void executeConfirmed()}><Play size={15} />{executing ? t('app.executing') : t('app.executeCommand')}</button></div>
    </div></Modal>
  })() : null

  return <><Modal title={t('app.aiCommandAssistant')} onClose={confirming ? () => undefined : onClose} wide className="workflow-dialog"><div className="ai-command-dialog">
    <div className="ai-command-content"><div className="ai-target">{target.remote ? <Server size={16} /> : <SquareTerminal size={16} />}<strong>{target.name}</strong><span>{target.detail}</span><button type="button" className="icon-button" title={t('app.recentCommands')} onClick={() => void openHistory()}><HistoryIcon size={15} /></button><button type="button" className="icon-button" title={t('app.viewRawAiRequestAndResponse')} onClick={() => void openRawExchange()}><FileJson2 size={15} /></button><span className={`status-dot ${target.connected ? 'connected' : 'disconnected'}`} /></div>
    {!settings.model.trim() ? <div className="ai-unconfigured"><Bot size={28} /><strong>{t('app.noAiModelConfigured')}</strong><button className="primary-button" onClick={onSettings}><SettingsIcon size={15} />{t('app.openAiSettings')}</button></div> : <>
      <section className="ai-compose">
        <div className="ai-prompt-row"><label>{t('app.targetShell')}<select value={shell} onChange={(event) => void selectShell(event.target.value as AiShell)}><option value="linux">Linux Shell</option><option value="powerShell">PowerShell</option><option value="cmd">Windows cmd</option><option value="macos">macOS Shell</option></select></label><label className="ai-requirement">{t('app.whatDoYouNeedToDo')}<textarea autoFocus rows={4} maxLength={8000} value={requirement} onChange={(event) => setRequirement(event.target.value)} placeholder={t('app.forExampleFindAppLogEntriesWhereThe')} onKeyDown={(event) => { if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') { event.preventDefault(); void generate() } }} /></label></div>
        <div className="ai-compose-actions"><div className="ai-context-options"><label className="check-label ai-context-option" title={t('app.includeUpToValueCharactersInTheAi', { value0: aiTerminalContextChars })}><input type="checkbox" checked={includeTerminalContext} onChange={(event) => setIncludeTerminalContext(event.target.checked)} />{t('app.includeTheLastValueTerminalLines', { value0: aiTerminalContextLines })}</label><label className="check-label ai-context-option" title={t('app.maskCommonPhoneNumbersEmailAddressesAndIdentity')}><input type="checkbox" disabled={!includeTerminalContext} checked={redactTerminalContext} onChange={(event) => setRedactTerminalContext(event.target.checked)} />{t('app.redactSensitiveInformation')}</label></div><button className="primary-button" disabled={loading || !requirement.trim()} onClick={() => void generate()}><WandSparkles size={15} />{loading ? t('app.generating') : suggestion ? t('app.regenerate') : t('app.generateCommand')}</button></div>
      </section>
      {error && <div className="ai-error">{error}</div>}
      {suggestion && <div className="ai-result">
        <div className="ai-result-heading"><div><strong>{t('app.generatedResult')}</strong><span className={`ai-risk ${suggestion.riskLevel}`}>{riskText}</span></div>{edited && <span className="ai-edited">{t('app.theCommandWasEditedTheOriginalExplanationMay')}</span>}</div>
        <textarea className="ai-command-editor" rows={4} spellCheck={false} value={command} onChange={(event) => { setCommand(event.target.value); setEdited(event.target.value !== suggestion.command) }} />
        {suggestion.explanation && <p>{suggestion.explanation}</p>}
        {suggestion.assumptions.length > 0 && <details className="ai-notes"><summary>{t('app.assumptions')} <span>{suggestion.assumptions.length}</span></summary><ul>{suggestion.assumptions.map((item) => <li key={item}>{item}</li>)}</ul></details>}
        {suggestion.warnings.length > 0 && <details className="ai-notes warning"><summary>{t('app.warnings')} <span>{suggestion.warnings.length}</span></summary><ul>{suggestion.warnings.map((item) => <li key={item}>{item}</li>)}</ul></details>}
      </div>}
    </>}
    </div><div className="ai-command-footer"><button className="secondary-button" disabled={!command.trim()} onClick={() => void window.api.system.writeClipboard(command).catch((copyError) => setError(errorMessage(copyError)))}><Copy size={15} />{t('common.copy')}</button><div><button className="secondary-button" disabled={!target.connected || !command.trim() || executing} onClick={() => void fillCommand()}><Send size={15} />{t('app.insertIntoTerminal')}</button><button className="primary-button" disabled={!target.connected || !command.trim() || executing} onClick={() => void requestExecution()}><Play size={15} />{t('app.execute2')}</button></div></div>
  </div></Modal>{executionConfirmation}</>
}

function prettyRawJson(value: string): string {
  try { return JSON.stringify(JSON.parse(value), null, 2) }
  catch { return value }
}

function AiCommandHistoryDialog({ entries, onUse, onClose, onClear }: { entries: AiCommandHistoryEntry[]; onUse(entry: AiCommandHistoryEntry): void; onClose(): void; onClear(): Promise<void> }): React.JSX.Element {
  const { t } = useI18n()
  const [confirmingClear, setConfirmingClear] = useState(false)
  const shellNames: Record<AiShell, string> = { linux: 'Linux Shell', powerShell: 'PowerShell', cmd: 'Windows cmd', macos: 'macOS Shell' }
  const riskNames = { low: t('common.lowRisk'), medium: t('common.caution'), high: t('common.highRisk') } as const
  return <><Modal title={t('app.aiCommandHistory')} onClose={confirmingClear ? () => undefined : onClose} wide className="workflow-dialog"><div className="ai-history">
    <div className="ai-history-list">{entries.length === 0 ? <div className="ai-history-empty"><HistoryIcon size={26} /><strong>{t('app.noCommandHistory')}</strong><span>{t('app.the10MostRecentSuccessfulCommandsAppearHere')}</span></div> : entries.map((entry) => <article key={entry.id} className="ai-history-entry">
      <header><div><span>{shellNames[entry.shell]}</span><time>{new Date(entry.createdAt).toLocaleString()}</time></div><span className={`ai-risk ${entry.riskLevel}`}>{riskNames[entry.riskLevel]}</span></header>
      <p title={entry.requirement}>{entry.requirement}</p>
      <pre>{entry.command}</pre>
      <div><button className="secondary-button" onClick={() => void window.api.system.writeClipboard(entry.command)}><Copy size={15} />{t('common.copy')}</button><button className="primary-button" onClick={() => onUse(entry)}><Check size={15} />{t('app.use')}</button></div>
    </article>)}</div>
    <div className="dialog-actions spread"><button className="danger-button" disabled={entries.length === 0} onClick={() => setConfirmingClear(true)}><Trash2 size={15} />{t('common.clearHistory')}</button><button className="primary-button" onClick={onClose}>{t('common.back')}</button></div>
  </div></Modal>{confirmingClear && <Modal title={t('app.clearAiCommandHistory')} onClose={() => setConfirmingClear(false)} raised className="secondary-confirm-dialog"><div className="confirmation-dialog danger">
    <div className="confirmation-heading"><Trash2 size={22} /><div><strong>{t('app.clearAllAiCommandHistory')}</strong><span>{t('app.the10MostRecentlySavedCommandsWillBe')}</span></div></div>
    <div className="dialog-actions"><button className="secondary-button" onClick={() => setConfirmingClear(false)}>{t('common.cancel')}</button><button className="danger-button" onClick={async () => { await onClear(); setConfirmingClear(false) }}><Trash2 size={15} />{t('common.clearHistory')}</button></div>
  </div></Modal>}</>
}

function AiDiagnosticsDialog({ exchange, onClose, onClear }: { exchange: AiRawExchange; onClose(): void; onClear(): void }): React.JSX.Element {
  const { t } = useI18n()
  const [view, setView] = useState<'request' | 'response'>('response')
  const headers = view === 'request' ? exchange.requestHeaders : exchange.responseHeaders
  const body = prettyRawJson(view === 'request' ? exchange.requestBody : exchange.responseBody)
  const copy = [view === 'request' ? `POST ${exchange.endpoint}` : `HTTP ${exchange.responseStatus ?? t('common.noResponse')}`, headers, body, exchange.error && `Error: ${exchange.error}`].filter(Boolean).join('\n\n')
  return <Modal title={t('app.rawAiRequestAndResponse')} onClose={onClose} wide className="workflow-dialog"><div className="ai-diagnostics">
    <div className="ai-diagnostics-meta"><span title={exchange.endpoint}>POST {exchange.endpoint}</span><time>{new Date(exchange.occurredAt).toLocaleString()}</time>{exchange.responseStatus ? <strong className={exchange.responseStatus >= 400 ? 'error' : ''}>HTTP {exchange.responseStatus}</strong> : <strong className="error">{t('common.noResponse')}</strong>}</div>
    {exchange.error && <div className="ai-error">{exchange.error}</div>}
    <div className="ai-diagnostics-tabs" role="tablist" aria-label={t('app.rawDataType')}><button role="tab" aria-selected={view === 'request'} className={view === 'request' ? 'active' : ''} onClick={() => setView('request')}>{t('app.request')}</button><button role="tab" aria-selected={view === 'response'} className={view === 'response' ? 'active' : ''} onClick={() => setView('response')}>{t('app.response')}</button></div>
    <div className="ai-diagnostics-content">{headers && <section><strong>Headers</strong><pre>{headers}</pre></section>}<section><strong>Body</strong><pre>{body || t('app.empty')}</pre></section></div>
    <div className="dialog-actions spread"><button className="danger-button" onClick={onClear}><Trash2 size={15} />{t('app.clear')}</button><div><button className="secondary-button" onClick={() => void window.api.system.writeClipboard(copy)}><Copy size={15} />{t('common.copy')}</button><button className="primary-button" onClick={onClose}>{t('common.back')}</button></div></div>
  </div></Modal>
}

function AgentEnvironmentPanel({ agents, session, panes, browserResources, browserRuntimes, chromeInstallation, bookmarks }: { agents: ManagedAgentSummary[]; session: MuxSession; panes: WorkspaceTab[]; browserResources: BrowserResourceState[]; browserRuntimes: BrowserRuntime[]; chromeInstallation: ChromeInstallation | null | undefined; bookmarks: Bookmark[] }): React.JSX.Element {
  const { t } = useI18n()
  const paneMap = new Map(panes.map((pane) => [pane.id, pane]))
  const bookmarkMap = new Map(bookmarks.map((bookmark) => [bookmark.id, bookmark]))
  const sessionAgents = agents.filter((agent) => paneMap.has(agent.paneId))
  const runningBrowserRuntimes = browserRuntimes.filter((runtime) => runtime.status === 'running')
  const localBrowserResources = browserResources.filter((resource) => !resource.sourcePaneId)
  const browserEnvironment = (() => {
    if (runningBrowserRuntimes.length === 1) {
      const runtime = runningBrowserRuntimes[0]!
      const resource = browserResources.find((item) => item.id === runtime.browserResourceId)
      return { tone: 'ready', label: t('app.browserMcpReady'), detail: `${resource?.name ?? runtime.browserResourceId} · CDP :${runtime.cdpPort}` }
    }
    if (runningBrowserRuntimes.length > 1) return { tone: 'conflict', label: t('app.browserMcpConflict'), detail: t('app.browserMcpConflictDetail') }
    if (chromeInstallation === undefined) return { tone: 'passive', label: t('app.checking'), detail: t('app.checkingBrowserAvailability') }
    if (chromeInstallation === null) return { tone: 'unavailable', label: t('app.browserMcpUnavailable'), detail: t('app.installChromeFirst') }
    if (localBrowserResources.length === 1) return { tone: 'on-demand', label: t('app.browserMcpOnDemand'), detail: localBrowserResources[0]!.name }
    if (localBrowserResources.length > 1) return { tone: 'selection', label: t('app.browserMcpNeedsSelection'), detail: t('app.browserMcpNeedsSelectionDetail') }
    return { tone: 'unavailable', label: t('app.browserMcpUnavailable'), detail: t('app.browserMcpUnavailableDetail') }
  })()

  return <div className="agent-environment">
    <section className="agent-environment-summary" aria-label={t('app.sessionEnvironment')}>
      <div className="agent-environment-summary-item">
        <FolderOpen size={17} />
        <span><small>{t('app.projectRoot')}</small><code title={session.rootPath || t('app.notConfigured')}>{session.rootPath || t('app.notConfigured')}</code></span>
      </div>
      <div className="agent-environment-summary-item">
        <Globe2 size={17} />
        <span><small>{t('app.browserMcp')}</small><strong className={`agent-environment-state ${browserEnvironment.tone}`}><i />{browserEnvironment.label}</strong><em title={browserEnvironment.detail}>{browserEnvironment.detail}</em></span>
      </div>
    </section>
    <section className="agent-environment-agents">
      <header><Sparkles size={15} /><strong>{t('app.activeAgentEnvironment')}</strong></header>
      {sessionAgents.length === 0 ? <div className="agent-environment-empty"><Bot size={24} /><span>{t('app.noActiveAgents')}</span></div> : <div className="agent-environment-table-wrap"><table className="agent-environment-table">
        <thead><tr><th>{t('app.agentAdapter')}</th><th>{t('app.owningPane')}</th><th>{t('app.terminalTarget')}</th><th>{t('app.launchMode')}</th><th>{t('app.hookStatus')}</th><th>{t('app.lunaMcp')}</th></tr></thead>
        <tbody>{sessionAgents.map((agent) => {
          const pane = paneMap.get(agent.paneId)
          if (!pane) return null
          const bookmark = pane.bookmarkId ? bookmarkMap.get(pane.bookmarkId) : undefined
          const targetLabel = bookmark ? bookmark.name : t('app.localEnvironment')
          const targetDetail = bookmark ? `${bookmark.username}@${bookmark.host}:${bookmark.port}` : pane.targetId
          return <tr key={agent.agentId}>
            <td><span className="agent-environment-primary"><Sparkles size={14} />{agentAdapterLabel(agent.latest?.adapterId ?? t('app.agentGeneric'))}</span></td>
            <td><span className="agent-environment-primary"><SquareTerminal size={14} /><span title={pane.title}>{pane.title}</span></span></td>
            <td><span className="agent-environment-target">{bookmark ? <Server size={14} /> : <Monitor size={14} />}<span><strong title={targetLabel}>{targetLabel}</strong><code title={targetDetail}>{targetDetail}</code></span></span></td>
            <td>{pane.launchProfileId ? t('app.managedLaunch') : t('app.manualLaunch')}</td>
            <td><span className={`agent-environment-state ${agent.hasStructuredEvents ? 'ready' : 'passive'}`}><i />{agent.hasStructuredEvents ? t('app.hookConnected') : t('app.terminalDetection')}</span></td>
            <td><span className="agent-environment-state ready"><i />{t('app.mcpConfigured')}</span></td>
          </tr>
        })}</tbody>
      </table></div>}
    </section>
  </div>
}

function summarizeManagedAgents(panes: WorkspaceTab[], events: ManagedAgentEvent[], readSequences: Set<number>, dismissedAttentionSequences: Set<number>): ManagedAgentSummary[] {
  const byAgent = new Map<string, ManagedAgentEvent[]>()
  for (const event of events) byAgent.set(event.context.agentId, [...(byAgent.get(event.context.agentId) ?? []), event])
  const summaries: ManagedAgentSummary[] = []
  const activeAgentIds = [...byAgent.entries()]
    .filter(([, matching]) => !['SessionEnd', 'RuntimeExit', 'AgentProcessExit'].includes(matching.at(-1)?.hookEventName ?? ''))
    .map(([agentId]) => agentId)
  for (const agentId of activeAgentIds) {
    const matching = byAgent.get(agentId) ?? []
    const latest = matching.at(-1)
    const pane = panes.find((item) => item.id === latest?.context.paneId)
    if (!pane || !latest || !pane.runtimeId || pane.runtimeId !== latest.context.runtimeId) continue
    const latestWorkingSequence = matching.filter((event) => event.status === 'working').at(-1)?.sequence ?? -1
    const attentionEvents = matching.filter((event) => isAgentAttentionEvent(event) && event.sequence > latestWorkingSequence && !dismissedAttentionSequences.has(event.sequence))
    const latestAttention = attentionEvents.at(-1)
    summaries.push({
      agentId,
      paneId: pane.id,
      runtimeId: latest.context.runtimeId,
      status: latest.status,
      waitingReason: latest?.waitingReason,
      timestamp: latest?.timestamp,
      eventCount: matching.length,
      unread: attentionEvents.some((event) => !readSequences.has(event.sequence)),
      hasStructuredEvents: matching.some((event) => event.evidence === 'structuredHook'),
      latest,
      latestAttention
    })
  }
  return summaries.sort((a, b) => (b.unread ? 1 : 0) - (a.unread ? 1 : 0) || (b.timestamp ?? '').localeCompare(a.timestamp ?? ''))
}

function isAgentAttentionEvent(event: ManagedAgentEvent): boolean {
  return event.evidence === 'structuredHook' && ['waiting', 'completed', 'error'].includes(event.status)
}

function agentAttentionTone(agent?: ManagedAgentSummary): AgentAttentionTone | undefined {
  if (!agent?.latestAttention) return undefined
  if (agent.latestAttention.status === 'error') return 'error'
  if (agent.latestAttention.status === 'waiting') return 'warning'
  return agent.unread ? 'info' : undefined
}

function SshConfigImportDialog({ preview, onClose, onImported, onError }: { preview: SshConfigPreview; onClose(): void; onImported(): void; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
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
  return <Modal title={t('common.importOpensshConfig')} onClose={onClose} wide><div className="ssh-import-dialog">
    <div className="import-source" title={preview.path}>{preview.path}</div>
    <div className="import-list">{preview.entries.length === 0 ? <div className="import-empty">{t('app.noConcreteHostEntriesWereFound')}</div> : preview.entries.map((entry) => <label key={entry.alias}>
      <input type="checkbox" checked={selected.has(entry.alias)} onChange={() => toggle(entry.alias)} />
      <span><strong>{entry.name}</strong><small>{entry.username}@{entry.host}:{entry.port}{entry.proxyJumpAlias ? t('common.viaValue', { value0: entry.proxyJumpAlias }) : ''}</small></span>
      <em>{entry.privateKeyPath ? t('common.privateKey') : 'Agent'}</em>
    </label>)}</div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" disabled={!selected.size || loading} onClick={() => void importSelected()}>{loading ? t('app.importing') : t('app.importValueConnections', { value0: selected.size })}</button></div>
  </div></Modal>
}

function layoutFromPanes(layout: MuxSplitNode | undefined, panes: WorkspaceTab[]): MuxSplitNode | undefined {
  const paneIds = new Set(panes.map((pane) => pane.id))
  const prune = (node: MuxSplitNode | undefined): MuxSplitNode | undefined => {
    if (!node) return undefined
    if (node.type === 'pane') return paneIds.has(node.paneId) ? node : undefined
    const first = prune(node.first)
    const second = prune(node.second)
    if (!first) return second
    if (!second) return first
    return { ...node, first, second }
  }
  let normalized = prune(layout)
  const placed = new Set(normalized ? paneIdsInLayout(normalized) : [])
  for (const pane of panes) {
    if (placed.has(pane.id)) continue
    const leaf: MuxSplitNode = { type: 'pane', paneId: pane.id }
    normalized = normalized ? { type: 'split', direction: 'horizontal', ratio: 0.5, first: normalized, second: leaf } : leaf
  }
  return normalized
}

function insertPaneInLayout(layout: MuxSplitNode | undefined, anchorPaneId: string | undefined, paneId: string, direction: 'horizontal' | 'vertical'): MuxSplitNode {
  const leaf: MuxSplitNode = { type: 'pane', paneId }
  if (!layout) return leaf
  if (!anchorPaneId) return { type: 'split', direction, ratio: 0.5, first: layout, second: leaf }
  const visit = (node: MuxSplitNode): [MuxSplitNode, boolean] => {
    if (node.type === 'pane') return node.paneId === anchorPaneId
      ? [{ type: 'split', direction, ratio: 0.5, first: node, second: leaf }, true]
      : [node, false]
    const [first, insertedFirst] = visit(node.first)
    if (insertedFirst) return [{ ...node, first }, true]
    const [second, insertedSecond] = visit(node.second)
    return insertedSecond ? [{ ...node, second }, true] : [node, false]
  }
  const [next, inserted] = visit(layout)
  return inserted ? next : { type: 'split', direction, ratio: 0.5, first: layout, second: leaf }
}

function removePaneFromLayout(layout: MuxSplitNode | undefined, paneId: string): MuxSplitNode | undefined {
  if (!layout) return undefined
  if (layout.type === 'pane') return layout.paneId === paneId ? undefined : layout
  const first = removePaneFromLayout(layout.first, paneId)
  const second = removePaneFromLayout(layout.second, paneId)
  if (!first) return second
  if (!second) return first
  return { ...layout, first, second }
}

function setSplitRatio(layout: MuxSplitNode, path: string, ratio: number, currentPath = ''): MuxSplitNode {
  if (layout.type === 'pane') return layout
  if (path === currentPath) return { ...layout, ratio }
  return {
    ...layout,
    first: setSplitRatio(layout.first, path, ratio, `${currentPath}0`),
    second: setSplitRatio(layout.second, path, ratio, `${currentPath}1`)
  }
}

function paneIdsInLayout(layout: MuxSplitNode): string[] {
  return layout.type === 'pane' ? [layout.paneId] : [...paneIdsInLayout(layout.first), ...paneIdsInLayout(layout.second)]
}

function balancedLayout(nodes: MuxSplitNode[], direction: 'horizontal' | 'vertical'): MuxSplitNode {
  if (nodes.length === 1) return nodes[0]!
  const midpoint = Math.ceil(nodes.length / 2)
  return {
    type: 'split',
    direction,
    ratio: midpoint / nodes.length,
    first: balancedLayout(nodes.slice(0, midpoint), direction),
    second: balancedLayout(nodes.slice(midpoint), direction)
  }
}

function layoutForPreset(paneIds: string[], preset: LayoutPreset): MuxSplitNode {
  const leaves = paneIds.map((paneId): MuxSplitNode => ({ type: 'pane', paneId }))
  if (preset !== 'twoColumns') return balancedLayout(leaves, preset)
  const rows: MuxSplitNode[] = []
  for (let index = 0; index < leaves.length; index += 2) {
    const row = leaves.slice(index, index + 2)
    rows.push(row.length === 1 ? row[0]! : balancedLayout(row, 'horizontal'))
  }
  return balancedLayout(rows, 'vertical')
}

interface MuxLayoutProps {
  node: MuxSplitNode
  panes: WorkspaceTab[]
  bookmarks: Map<string, Bookmark>
  activePaneId: string
  settings: TerminalSettings
  backgroundImage: string
  agentAttentionByPane: Map<string, AgentAttentionTone>
  terminalPaneRefs: React.MutableRefObject<Map<string, TerminalPaneHandle>>
  onFocus(pane: WorkspaceTab): void
  onTerminalAgentAction(pane: WorkspaceTab): void
  onRuntimeError(pane: WorkspaceTab, runtimeId: string, message: string): void
  onReconnect(pane: WorkspaceTab): void
  onReauthenticate(pane: WorkspaceTab): void
  onSplit(pane: WorkspaceTab, direction: 'horizontal' | 'vertical'): void
  onClose(pane: WorkspaceTab): void
  onResize(path: string, ratio: number, persist: boolean): void
  onToggleMaximize(paneId: string): void
  onOpenSettings(): void
  maximizedPaneId: string
  path?: string
}

function MuxLayout({ node, panes, bookmarks, activePaneId, settings, backgroundImage, agentAttentionByPane, terminalPaneRefs, onFocus, onTerminalAgentAction, onRuntimeError, onReconnect, onReauthenticate, onSplit, onClose, onResize, onToggleMaximize, onOpenSettings, maximizedPaneId, path = '' }: MuxLayoutProps): React.JSX.Element | null {
  const { t } = useI18n()
  if (node.type === 'pane') {
    const pane = panes.find((item) => item.id === node.paneId)
    if (!pane) return null
    const active = pane.id === activePaneId
    const attentionTone = agentAttentionByPane.get(pane.id)
    const bookmark = pane.bookmarkId ? bookmarks.get(pane.bookmarkId) : undefined
    const jumpBookmark = bookmark?.jumpBookmarkId ? bookmarks.get(bookmark.jumpBookmarkId) : undefined
    const canEnterCredentials = Boolean(bookmark && (bookmark.authType !== 'agent' || (jumpBookmark && jumpBookmark.authType !== 'agent')))
    const authenticationFailure = canEnterCredentials && isSshAuthenticationFailure(pane.error)
    const stoppedState = pane.bookmarkId
      ? pane.error
        ? { title: t('app.connectionFailed'), description: pane.error, actionLabel: authenticationFailure ? t('terminal.enterCredentialsAgain') : t('terminal.reconnectSshTerminal'), actionIcon: authenticationFailure ? 'credentials' as const : 'retry' as const, tone: 'error' as const }
        : { title: t('terminal.sshTerminalDisconnected'), description: t('terminal.reconnectSshTerminalToContinue'), actionLabel: t('terminal.reconnectSshTerminal'), actionIcon: 'retry' as const }
      : { title: t('terminal.localTerminalStopped'), description: t('terminal.startLocalTerminalToContinue'), actionLabel: t('terminal.startLocalTerminal'), actionIcon: 'start' as const }
    const maximizeClass = maximizedPaneId ? pane.id === maximizedPaneId ? 'pane-maximized' : 'maximize-hidden' : ''
    return <section className={['mux-leaf', active ? 'active' : '', attentionTone ? `attention-${attentionTone}` : '', maximizeClass].filter(Boolean).join(' ')} onPointerDown={() => onFocus(pane)}>
      <header className="mux-leaf-header">
        <span className={`status-dot ${pane.status}`} />
        {pane.bookmarkId ? <Server size={13} /> : <SquareTerminal size={13} />}
        <strong title={pane.title}>{pane.title}</strong>
        <div className="mux-leaf-actions">
          <button className="icon-button" title={t('app.splitRight')} aria-label={t('app.splitRight')} onClick={() => onSplit(pane, 'horizontal')}><Columns2 size={14} /></button>
          <button className="icon-button" title={t('app.splitDown')} aria-label={t('app.splitDown')} onClick={() => onSplit(pane, 'vertical')}><Rows2 size={14} /></button>
          <button className="icon-button" title={maximizedPaneId ? t('app.restorePane') : t('app.maximizePane')} aria-label={maximizedPaneId ? t('app.restorePane') : t('app.maximizePane')} onClick={() => onToggleMaximize(pane.id)}>{maximizedPaneId ? <Minimize size={14} /> : <Maximize2 size={14} />}</button>
          <button className="icon-button danger" title={t('app.closePane')} aria-label={t('app.closePane')} onClick={() => onClose(pane)}><X size={14} /></button>
        </div>
      </header>
      <div className="terminal-region"><TerminalPane key={pane.key} paneId={pane.id} targetId={pane.targetId} ref={(terminalPane) => { if (terminalPane) terminalPaneRefs.current.set(pane.key, terminalPane); else terminalPaneRefs.current.delete(pane.key) }} runtimeId={pane.runtimeId} connected={pane.status === 'connected'} connecting={pane.status === 'connecting'} visible={maximizedPaneId ? pane.id === maximizedPaneId : active} settings={settings} backgroundImage={backgroundImage} stoppedState={stoppedState} onAgentAction={() => onTerminalAgentAction(pane)} onRuntimeError={(runtimeId, message) => { onRuntimeError(pane, runtimeId, message) }} onStart={() => authenticationFailure ? onReauthenticate(pane) : onReconnect(pane)} onClose={() => onClose(pane)} onOpenSettings={onOpenSettings} /></div>
    </section>
  }
  const direction = node.direction
  const startResize = (event: React.PointerEvent<HTMLDivElement>): void => {
    event.preventDefault()
    event.stopPropagation()
    const container = event.currentTarget.parentElement
    if (!container) return
    const bounds = container.getBoundingClientRect()
    const ratioAt = (pointer: PointerEvent): number => {
      const raw = direction === 'horizontal' ? (pointer.clientX - bounds.left) / bounds.width : (pointer.clientY - bounds.top) / bounds.height
      return Math.min(0.85, Math.max(0.15, raw))
    }
    const move = (pointer: PointerEvent): void => onResize(path, ratioAt(pointer), false)
    const up = (pointer: PointerEvent): void => {
      document.removeEventListener('pointermove', move)
      document.removeEventListener('pointerup', up)
      document.body.classList.remove('mux-split-resizing')
      onResize(path, ratioAt(pointer), true)
    }
    document.body.classList.add('mux-split-resizing')
    document.addEventListener('pointermove', move)
    document.addEventListener('pointerup', up)
  }
  const style = direction === 'horizontal'
    ? { gridTemplateColumns: `minmax(0, ${node.ratio}fr) 5px minmax(0, ${1 - node.ratio}fr)` }
    : { gridTemplateRows: `minmax(0, ${node.ratio}fr) 5px minmax(0, ${1 - node.ratio}fr)` }
  const maximizeClass = maximizedPaneId ? paneIdsInLayout(node).includes(maximizedPaneId) ? 'maximized-branch' : 'maximize-hidden' : ''
  return <div className={['mux-split', direction, maximizeClass].filter(Boolean).join(' ')} style={style}>
    <MuxLayout node={node.first} panes={panes} bookmarks={bookmarks} activePaneId={activePaneId} settings={settings} backgroundImage={backgroundImage} agentAttentionByPane={agentAttentionByPane} terminalPaneRefs={terminalPaneRefs} onFocus={onFocus} onTerminalAgentAction={onTerminalAgentAction} onRuntimeError={onRuntimeError} onReconnect={onReconnect} onReauthenticate={onReauthenticate} onSplit={onSplit} onClose={onClose} onResize={onResize} onToggleMaximize={onToggleMaximize} onOpenSettings={onOpenSettings} maximizedPaneId={maximizedPaneId} path={`${path}0`} />
    <div className="mux-split-divider" role="separator" aria-orientation={direction === 'horizontal' ? 'vertical' : 'horizontal'} onPointerDown={startResize} />
    <MuxLayout node={node.second} panes={panes} bookmarks={bookmarks} activePaneId={activePaneId} settings={settings} backgroundImage={backgroundImage} agentAttentionByPane={agentAttentionByPane} terminalPaneRefs={terminalPaneRefs} onFocus={onFocus} onTerminalAgentAction={onTerminalAgentAction} onRuntimeError={onRuntimeError} onReconnect={onReconnect} onReauthenticate={onReauthenticate} onSplit={onSplit} onClose={onClose} onResize={onResize} onToggleMaximize={onToggleMaximize} onOpenSettings={onOpenSettings} maximizedPaneId={maximizedPaneId} path={`${path}1`} />
  </div>
}

function BrowserResourceManager({ resources, panes, chromeInstallation, onRefreshChrome, onStart, onFocus, onRestart, onStop }: { resources: BrowserResourceState[]; panes: WorkspaceTab[]; chromeInstallation: ChromeInstallation | null | undefined; onRefreshChrome(): void; onStart(resource: BrowserResourceState): void; onFocus(resource: BrowserResourceState): void; onRestart(resource: BrowserResourceState): void; onStop(resource: BrowserResourceState): void }): React.JSX.Element {
  const { t } = useI18n()
  const paneMap = new Map(panes.map((pane) => [pane.id, pane]))
  if (!resources.length) return <div className="browser-resource-manager empty"><Globe2 size={30} /><strong>{t('app.noBrowserResources')}</strong></div>
  return <div className="browser-resource-manager">
    <div className="browser-resource-list">{resources.map((resource) => {
      const live = resource.status === 'running' && Boolean(resource.runtime)
      const sourcePane = resource.sourcePaneId ? paneMap.get(resource.sourcePaneId) : undefined
      const statusLabel = resource.status === 'starting' ? t('app.connecting') : live ? t('app.browserRunning') : resource.status === 'error' ? t('app.browserRuntimeFailed') : t('app.browserRuntimeStopped')
      return <section className={`browser-resource-detail ${live ? 'running' : resource.status}`} key={resource.id}>
        <header>
          <div className="browser-resource-identity"><span className={`status-dot ${live ? 'connected' : resource.status === 'error' ? 'error' : resource.status === 'starting' ? 'connecting' : 'disconnected'}`} /><Globe2 size={18} /><span><strong>{resource.name}</strong><small>{sourcePane ? `${t('app.viaSshPane')} ${sourcePane.title}` : t('app.localService')}</small></span></div>
          <div className="browser-resource-actions">
            {live ? <button className="secondary-button" onClick={() => onFocus(resource)}><ExternalLink size={15} />{t('app.openManagedChrome')}</button> : <button className="primary-button" disabled={!chromeInstallation || resource.status === 'starting'} onClick={() => onStart(resource)}><Play size={15} />{t('app.startBrowser')}</button>}
            <button className="icon-button" title={t('app.restartBrowser')} aria-label={t('app.restartBrowser')} disabled={!resource.runtime || resource.status === 'starting'} onClick={() => onRestart(resource)}><RotateCcw size={15} /></button>
            <button className="icon-button" title={t('app.stopBrowser')} aria-label={t('app.stopBrowser')} disabled={!resource.runtime} onClick={() => onStop(resource)}><Power size={15} /></button>
          </div>
        </header>
        {chromeInstallation === null && <div className="browser-resource-warning"><ShieldAlert size={16} /><span><strong>{t('app.chromeNotFound')}</strong><small>{t('app.installChromeFirst')}</small></span><button className="icon-button" title={t('app.recheckBrowser')} aria-label={t('app.recheckBrowser')} onClick={onRefreshChrome}><RotateCcw size={15} /></button></div>}
        {resource.error && <div className="browser-resource-error" title={resource.error}><ShieldAlert size={15} />{resource.error}</div>}
        <dl className="browser-resource-diagnostics">
          <div><dt>{t('app.runtimeStatus')}</dt><dd><span className={`status-dot ${live ? 'connected' : resource.status === 'error' ? 'error' : resource.status === 'starting' ? 'connecting' : 'disconnected'}`} />{statusLabel}</dd></div>
          <div><dt>{t('app.chromeProcess')}</dt><dd><code>{resource.runtime ? `PID ${resource.runtime.processId}` : '—'}</code></dd></div>
          <div><dt>CDP</dt><dd><code>{resource.runtime ? `127.0.0.1:${resource.runtime.cdpPort}` : '—'}</code></dd></div>
          <div><dt>{t('app.browserProfile')}</dt><dd><code title={resource.runtime?.profilePath}>{resource.runtime?.profilePath || '—'}</code></dd></div>
        </dl>
      </section>
    })}</div>
  </div>
}

function BookmarkArchiveImportDialog({ preview, onClose, onImported, onError }: { preview: BookmarkArchivePreview; onClose(): void; onImported(result: { importedConnections: number; importedGroups: number }): void; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const connectionGroups = useMemo(() => new Set(preview.connections.map((item) => item.groupName).filter(Boolean)), [preview])
  const emptyGroups = useMemo(() => preview.groups.filter((group) => group && !connectionGroups.has(group)), [preview, connectionGroups])
  const [selectedConnections, setSelectedConnections] = useState(() => new Set(preview.connections.map((item) => item.id)))
  const [selectedGroups, setSelectedGroups] = useState(() => new Set(emptyGroups))
  const [loading, setLoading] = useState(false)
  const byId = useMemo(() => new Map(preview.connections.map((item) => [item.id, item])), [preview])
  const sourceKeys: Record<BookmarkArchiveSource, MessageKey> = { lunaMux: 'app.archiveSourceLunaMux', lunaRemote: 'app.archiveSourceLunaRemote', legacy: 'app.archiveSourceLegacy' }
  const toggleConnection = (id: string): void => setSelectedConnections((current) => {
    const next = new Set(current)
    if (next.has(id)) {
      next.delete(id)
      let changed = true
      while (changed) {
        changed = false
        for (const entry of preview.connections) {
          if (next.has(entry.id) && entry.jumpBookmarkId && !next.has(entry.jumpBookmarkId)) {
            next.delete(entry.id)
            changed = true
          }
        }
      }
    } else {
      let currentId: string | undefined = id
      while (currentId && !next.has(currentId)) {
        next.add(currentId)
        currentId = byId.get(currentId)?.jumpBookmarkId || undefined
      }
    }
    return next
  })
  const toggleGroup = (group: string): void => setSelectedGroups((current) => {
    const next = new Set(current)
    if (next.has(group)) next.delete(group); else next.add(group)
    return next
  })
  const importSelected = async (): Promise<void> => {
    setLoading(true)
    try {
      const result = await window.api.bookmarks.importArchive(preview.previewId, [...selectedConnections], [...selectedGroups])
      onImported(result)
    } catch (error) { onError(errorMessage(error)) }
    finally { setLoading(false) }
  }
  return <Modal title={t('app.importConnectionBackup')} onClose={onClose} wide><div className="ssh-import-dialog archive-import-dialog">
    <div className="import-source" title={preview.path}>{preview.path}</div>
    <div className="archive-import-meta"><span>{t('app.backupSourceValue', { value0: t(sourceKeys[preview.source]) })}</span><span>{t('app.exportedAtValue', { value0: new Date(preview.exportedAt).toLocaleString() })}</span></div>
    <div className="archive-import-notice"><KeyRound size={15} /><span>{t('app.archiveCredentialsNotImported')}</span></div>
    {emptyGroups.length > 0 && <fieldset className="archive-import-groups"><legend>{t('app.emptyGroups')}</legend><div>{emptyGroups.map((group) => <label key={group}><input type="checkbox" checked={selectedGroups.has(group)} onChange={() => toggleGroup(group)} /><span>{group}</span></label>)}</div></fieldset>}
    <div className="import-list">{preview.connections.length === 0 ? <div className="import-empty">{t('app.noConnectionsInBackup')}</div> : preview.connections.map((entry) => <label key={entry.id}>
      <input type="checkbox" checked={selectedConnections.has(entry.id)} onChange={() => toggleConnection(entry.id)} />
      <span><strong>{entry.name}</strong><small>{entry.username}@{entry.host}:{entry.port}{entry.jumpBookmarkId ? t('common.viaValue', { value0: byId.get(entry.jumpBookmarkId)?.name ?? entry.jumpBookmarkId }) : ''}</small></span>
      <em>{entry.groupName || t('common.ungrouped')}</em>
    </label>)}</div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" disabled={(!selectedConnections.size && !selectedGroups.size) || loading} onClick={() => void importSelected()}>{loading ? t('app.importing') : t('app.importSelectedBackupData', { value0: selectedConnections.size, value1: selectedGroups.size })}</button></div>
  </div></Modal>
}

function LunaRemoteSourceDialog({ sources, onClose, onChoose, onSelect }: { sources: LunaRemoteSource[]; onClose(): void; onChoose(): void; onSelect(path: string): void }): React.JSX.Element {
  const { t } = useI18n()
  return <Modal title={t('app.importFromLunaRemote')} onClose={onClose} wide><div className="ssh-import-dialog luna-remote-source-dialog">
    <div className="dialog-copy"><p>{t('app.lunaRemoteSourceDescription')}</p></div>
    {sources.length > 0 ? <div className="import-list">{sources.map((source) => <button className="source-choice" key={source.path} onClick={() => onSelect(source.path)}><DatabaseIcon size={16} /><span><strong>{t('app.lunaRemoteDetectedDatabase')}</strong><small title={source.path}>{source.path}</small><em>{new Date(source.sourceModifiedAt).toLocaleString()}</em></span><ChevronRight size={16} /></button>)}</div> : <div className="import-empty">{t('app.noLunaRemoteDatabaseDetected')}</div>}
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" onClick={onChoose}><FolderOpen size={15} />{t('app.chooseLunaRemoteDatabase')}</button></div>
  </div></Modal>
}

function LunaRemoteImportDialog({ preview, onClose, onImported, onConfirm, onError }: { preview: LunaRemoteImportPreview; onClose(): void; onImported(result: LunaRemoteImportResult): void; onConfirm: ConfirmAction; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const connectionGroups = useMemo(() => new Set(preview.connections.map((item) => item.groupName).filter(Boolean)), [preview])
  const emptyGroups = useMemo(() => preview.groups.filter((group) => group && !connectionGroups.has(group)), [preview, connectionGroups])
  const [selectedConnections, setSelectedConnections] = useState(() => new Set(preview.connections.map((item) => item.id)))
  const [selectedGroups, setSelectedGroups] = useState(() => new Set(emptyGroups))
  const [importHostKeys, setImportHostKeys] = useState(preview.knownHosts.length > 0)
  const [importSettings, setImportSettings] = useState(false)
  const [importForwardingProfiles, setImportForwardingProfiles] = useState(preview.forwardingProfiles.length > 0)
  const [importCredentials, setImportCredentials] = useState(false)
  const [loading, setLoading] = useState(false)
  const byId = useMemo(() => new Map(preview.connections.map((item) => [item.id, item])), [preview])
  const credentialCount = preview.connections.filter((item) => selectedConnections.has(item.id) && preview.credentialConnectionIds.includes(item.id)).length
  useEffect(() => { if (!selectedConnections.size) setImportForwardingProfiles(false) }, [selectedConnections])
  useEffect(() => { if (!credentialCount) setImportCredentials(false) }, [credentialCount])
  const toggleConnection = (id: string): void => setSelectedConnections((current) => {
    const next = new Set(current)
    if (next.has(id)) {
      next.delete(id)
      let changed = true
      while (changed) {
        changed = false
        for (const entry of preview.connections) {
          if (next.has(entry.id) && entry.jumpBookmarkId && !next.has(entry.jumpBookmarkId)) { next.delete(entry.id); changed = true }
        }
      }
    } else {
      let currentId: string | undefined = id
      while (currentId && !next.has(currentId)) { next.add(currentId); currentId = byId.get(currentId)?.jumpBookmarkId || undefined }
    }
    return next
  })
  const toggleGroup = (group: string): void => setSelectedGroups((current) => { const next = new Set(current); if (next.has(group)) next.delete(group); else next.add(group); return next })
  const importSelected = async (): Promise<void> => {
    if (importCredentials && credentialCount > 0) {
      const confirmed = await onConfirm({ title: t('app.importCredentialsTitle'), message: t('app.importCredentialsMessage', { value0: credentialCount }), detail: t('app.importCredentialsDetail'), kind: 'warning', confirmLabel: t('app.importCredentialsConfirm') })
      if (!confirmed) return
    }
    setLoading(true)
    try {
      const result = await window.api.bookmarks.importLunaRemote({ previewId: preview.previewId, connectionIds: [...selectedConnections], groups: [...selectedGroups], importHostKeys, importSettings, importForwardingProfiles: importForwardingProfiles && selectedConnections.size > 0, importCredentials: importCredentials && credentialCount > 0 })
      onImported(result)
    } catch (error) { onError(errorMessage(error)) }
    finally { setLoading(false) }
  }
  return <Modal title={t('app.importFromLunaRemote')} onClose={onClose} wide><div className="ssh-import-dialog luna-remote-import-dialog">
    <div className="import-source" title={preview.path}>{preview.path}</div>
    <div className="archive-import-meta"><span>{t('app.lunaRemoteDatabaseModifiedValue', { value0: new Date(preview.sourceModifiedAt).toLocaleString() })}</span><span>{t('app.lunaRemoteCounts', { value0: preview.connections.length, value1: preview.knownHosts.length, value2: preview.forwardingProfiles.length })}</span></div>
    <div className="archive-import-notice"><KeyRound size={15} /><span>{t('app.lunaRemoteCredentialsDefaultOff')}</span></div>
    {emptyGroups.length > 0 && <fieldset className="archive-import-groups"><legend>{t('app.emptyGroups')}</legend><div>{emptyGroups.map((group) => <label key={group}><input type="checkbox" checked={selectedGroups.has(group)} onChange={() => toggleGroup(group)} /><span>{group}</span></label>)}</div></fieldset>}
    <fieldset className="archive-import-groups"><legend>{t('app.lunaRemoteDataClasses')}</legend><div className="import-options"><label><input type="checkbox" checked={importHostKeys} disabled={!preview.knownHosts.length} onChange={(event) => setImportHostKeys(event.target.checked)} /><span>{t('app.lunaRemoteHostKeys', { value0: preview.knownHosts.length })}</span></label><label><input type="checkbox" checked={importSettings} disabled={!preview.settingKeys.length} onChange={(event) => setImportSettings(event.target.checked)} /><span>{t('app.lunaRemoteSettings', { value0: preview.settingKeys.length })}</span></label><label><input type="checkbox" checked={importForwardingProfiles} disabled={!preview.forwardingProfiles.length || !selectedConnections.size} onChange={(event) => setImportForwardingProfiles(event.target.checked)} /><span>{t('app.lunaRemoteForwardingProfiles', { value0: preview.forwardingProfiles.length })}</span></label><label className="sensitive-option"><input type="checkbox" checked={importCredentials} disabled={!credentialCount || !selectedConnections.size} onChange={(event) => setImportCredentials(event.target.checked)} /><span>{t('app.lunaRemoteCredentials', { value0: credentialCount })}</span></label></div></fieldset>
    <div className="import-list">{preview.connections.length === 0 ? <div className="import-empty">{t('app.noConnectionsInLunaRemote')}</div> : preview.connections.map((entry) => <label key={entry.id}><input type="checkbox" checked={selectedConnections.has(entry.id)} onChange={() => toggleConnection(entry.id)} /><span><strong>{entry.name}</strong><small>{entry.username}@{entry.host}:{entry.port}{entry.jumpBookmarkId ? t('common.viaValue', { value0: byId.get(entry.jumpBookmarkId)?.name ?? entry.jumpBookmarkId }) : ''}</small></span><em>{entry.groupName || t('common.ungrouped')}</em></label>)}</div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" disabled={(!selectedConnections.size && !selectedGroups.size && !importHostKeys && !importSettings && !importForwardingProfiles) || loading} onClick={() => void importSelected()}>{loading ? t('app.importing') : t('app.importLunaRemoteSelected')}</button></div>
  </div></Modal>
}

function TunnelDialog({ bookmark, sessionId, onClose, onError }: { bookmark: Bookmark; sessionId: string; onClose(): void; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
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
  const currentTarget = ['127.0.0.1', 'localhost', '::1'].includes(profile.targetHost.trim().toLowerCase())
  const bindPort = active?.bindPort ?? profile.bindPort
  const bindEndpoint = `${profile.bindAddress || t('common.notSet')}:${bindPort || t('app.auto')}`
  const targetEndpoint = profile.type === 'dynamic' ? t('app.selectedByClient') : `${profile.targetHost || t('common.notSet')}:${profile.targetPort || t('common.notSet')}`
  const route = profile.type === 'local'
    ? [{ label: t('app.localApp'), value: bindEndpoint, icon: Monitor }, { label: t('common.sshTunnel'), value: `${bookmark.host}:${bookmark.port}`, icon: Server }, { label: currentTarget ? t('common.sshServer') : t('app.hostReachableByServer'), value: targetEndpoint, icon: Network }]
    : profile.type === 'remote'
      ? [{ label: t('app.sshServerListener'), value: bindEndpoint, icon: Server }, { label: t('common.sshTunnel'), value: bookmark.name, icon: Network }, { label: currentTarget ? t('common.thisComputer') : t('app.hostReachableLocally'), value: targetEndpoint, icon: Monitor }]
      : [{ label: t('app.localSocks5'), value: bindEndpoint, icon: Monitor }, { label: t('common.sshTunnel'), value: `${bookmark.host}:${bookmark.port}`, icon: Server }, { label: t('app.requestedTarget'), value: targetEndpoint, icon: Network }]
  const selectTarget = (useCurrent: boolean): void => set('targetHost', useCurrent ? '127.0.0.1' : currentTarget ? '' : profile.targetHost)
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
  return <Modal title={t('app.portForwardingValue', { value0: bookmark.name })} onClose={onClose} wide><div className="tunnel-dialog">
    <div className="deployment-profile-bar"><select value={profile.id} onChange={(event) => { const selected = profiles.find((item) => item.id === event.target.value); if (selected) setProfile(selected) }}><option value={profile.id}>{profiles.some((item) => item.id === profile.id) ? profile.name || t('common.unnamedProfile') : t('app.newForwardingProfile')}</option>{profiles.filter((item) => item.id !== profile.id).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select><button className="secondary-button" onClick={() => setProfile(empty())}><CirclePlus size={15} />{t('common.new')}</button>{profiles.some((item) => item.id === profile.id) && <button className="icon-button danger" title={t('app.deleteForwardingProfile')} onClick={async () => { if (active) await stop(); await window.api.tunnels.removeProfile(profile.id); const next = profiles.filter((item) => item.id !== profile.id); setProfiles(next); setProfile(next[0] ?? empty()) }}><Trash2 size={15} /></button>}</div>
    <div className="form-grid">
      <label>{t('app.profileNameOptional')}<input value={profile.name} onChange={(event) => set('name', event.target.value)} placeholder={t('app.leaveBlankToGenerateFromTheRoute')} /></label>
      <label>{t('app.forwardingType')}<select value={profile.type} disabled={Boolean(active)} onChange={(event) => set('type', event.target.value as PortForwardProfile['type'])}><option value="local">{t('app.localForwarding')}</option><option value="remote">{t('app.remoteForwarding')}</option><option value="dynamic">{t('app.dynamicSocks5')}</option></select></label>
      <section className="tunnel-route-preview"><span>{t('app.trafficRoute')}</span><div className="tunnel-route-flow">{route.map((step, index) => { const Icon = step.icon; return <div className="tunnel-route-part" key={step.label}>{index > 0 && <ArrowRight size={15} />}<div><Icon size={16} /><span><small>{step.label}</small><strong title={step.value}>{step.value}</strong></span></div></div> })}</div></section>
      <div className="form-row"><label>{profile.type === 'remote' ? t('app.sshServerBindAddress') : t('app.localBindAddress')}<input value={profile.bindAddress} disabled={Boolean(active)} onChange={(event) => set('bindAddress', event.target.value)} /></label><label>{t('app.bindPort')}<input type="number" min="0" max="65535" disabled={Boolean(active)} value={profile.bindPort} onChange={(event) => set('bindPort', Number(event.target.value))} /></label></div>
      {profile.bindPort === 0 && <div className="tunnel-hint">{t('app.port0IsAssignedAutomaticallyWhenStartingThe')}</div>}
      {window.api.platform === 'darwin' && profile.type !== 'remote' && profile.bindPort > 0 && profile.bindPort < 1024 && <div className="tunnel-hint warning">{t('app.regularMacosAppsCannotListenOnPrivilegedPorts')}</div>}
      {profile.type === 'local' && profile.bindPort === 6000 && <div className="tunnel-hint warning">{t('app.chromeBlocksLocalPort6000UseBindPort')}</div>}
      {profile.type !== 'dynamic' && <div className="tunnel-target-setting"><span>{t('app.targetLocation')}</span><div className="tunnel-target-options"><button type="button" className={currentTarget ? 'active' : ''} disabled={Boolean(active)} onClick={() => selectTarget(true)}>{profile.type === 'local' ? t('common.sshServer') : t('common.thisComputer')}</button><button type="button" className={!currentTarget ? 'active' : ''} disabled={Boolean(active)} onClick={() => selectTarget(false)}>{t('app.anotherHost')}</button></div></div>}
      {profile.type !== 'dynamic' && <div className="form-row"><label>{currentTarget ? profile.type === 'local' ? t('app.targetAddressSshServerItself') : t('app.targetAddressThisComputer') : profile.type === 'local' ? t('app.targetAddressFromSshServer') : t('app.targetAddressFromThisComputer')}<input value={profile.targetHost} readOnly={currentTarget} disabled={Boolean(active)} placeholder={currentTarget ? undefined : t('app.hostnameOrPrivateIp')} onChange={(event) => set('targetHost', event.target.value)} /></label><label>{t('app.targetPort')}<input type="number" min="1" max="65535" disabled={Boolean(active)} value={profile.targetPort} onChange={(event) => set('targetPort', Number(event.target.value))} /></label></div>}
      {active && <div className={`tunnel-status ${active.status}`}><span className={`status-dot ${active.status === 'running' ? 'connected' : active.status === 'error' ? 'error' : 'connecting'}`} /><strong>{active.status === 'running' ? t('app.runningValueValue', { value0: active.bindAddress, value1: active.bindPort }) : active.status === 'starting' ? t('app.starting') : t('app.forwardingError')}</strong>{active.error && <small>{active.error}</small>}</div>}
    </div>
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.close')}</button><button className="secondary-button" disabled={Boolean(active) || loading} onClick={() => void save()}>{t('common.save')}</button>{active ? <button className="danger-button" disabled={loading} onClick={() => void stop()}>{t('app.stop')}</button> : <button className="primary-button" disabled={loading} onClick={() => void start()}><Power size={15} />{t('app.start')}</button>}</div>
  </div></Modal>
}

function DeploymentDialog({ bookmark, sessionId, onClose, onConfirm, onError }: { bookmark: Bookmark; sessionId: string; onClose(): void; onConfirm: ConfirmAction; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
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
    if (!await onConfirm({ title: t('app.confirmDeployment'), message: removals ? t('app.deployUploadAndDelete', { changed, removals }) : t('app.deployUploadOnly', { changed }), detail: removals ? t('app.remoteFilesAreDeletedOnlyAfterEveryUpload') : t('app.verifyTheDeploymentTargetAndDifferences'), kind: 'warning', confirmLabel: t('common.startDeployment') })) return
    try { await window.api.deployments.execute(profile.id, sessionId); onClose() }
    catch (error) { onError(errorMessage(error)) }
  }
  const counts = diff ? Object.fromEntries(['new', 'changed', 'same', 'remote-only'].map((status) => [status, diff.filter((item) => item.status === status).length])) : {}
  return <Modal title={t('app.deployToValue', { value0: bookmark.name })} onClose={onClose} wide><div className="deployment-dialog">
    <div className="deployment-profile-bar"><select value={profile.id} onChange={(event) => { const selected = profiles.find((item) => item.id === event.target.value); if (selected) { setProfile(selected); setDiff(null) } }}><option value={profile.id}>{profiles.some((item) => item.id === profile.id) ? profile.name || t('common.unnamedProfile') : t('app.newDeploymentProfile')}</option>{profiles.filter((item) => item.id !== profile.id).map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select><button className="secondary-button" onClick={() => { setProfile(empty()); setDiff(null) }}><CirclePlus size={15} />{t('common.new')}</button>{profiles.some((item) => item.id === profile.id) && <button className="icon-button danger" title={t('app.deleteDeploymentProfile')} onClick={async () => { await window.api.deployments.remove(profile.id); const next = profiles.filter((item) => item.id !== profile.id); setProfiles(next); setProfile(next[0] ?? empty()); setDiff(null) }}><Trash2 size={15} /></button>}</div>
    <div className="form-grid"><label>{t('app.profileName')}<input value={profile.name} onChange={(event) => set('name', event.target.value)} /></label><label>{t('app.localDirectory')}<div className="input-button"><input readOnly value={profile.localDirectory} /><button className="secondary-button" onClick={async () => { const path = await window.api.files.chooseLocalDirectory(); if (path) set('localDirectory', path) }}>{t('common.choose')}</button></div></label><label>{t('app.remoteDirectory')}<input value={profile.remoteDirectory} onChange={(event) => set('remoteDirectory', event.target.value)} /></label><label className="check-label"><input type="checkbox" checked={profile.deleteExtraneous} onChange={(event) => set('deleteExtraneous', event.target.checked)} />{t('app.deleteExtraRemoteFilesAfterAllUploadsSucceed')}</label></div>
    {diff && <><div className="deployment-summary"><span>{t('common.new2')} {counts['new'] ?? 0}</span><span>{t('app.changed')} {counts['changed'] ?? 0}</span><span>{t('common.same')} {counts['same'] ?? 0}</span><span>{t('app.remoteOnly')} {counts['remote-only'] ?? 0}</span></div><div className="deployment-diff">{diff.filter((item) => item.status !== 'same').slice(0, 300).map((item) => <div key={`${item.status}:${item.relativePath}`}><span className={item.status}>{({ new: t('common.new2'), changed: t('app.update'), 'remote-only': t('common.remote'), same: t('common.same') } as const)[item.status]}</span><code>{item.relativePath}</code></div>)}</div></>}
    <div className="dialog-actions"><button className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="secondary-button" disabled={loading} onClick={() => void preview()}>{loading ? t('app.comparing') : t('app.saveAndPreview')}</button><button className="primary-button" disabled={!diff || loading} onClick={() => void execute()}><Rocket size={15} />{t('common.startDeployment')}</button></div>
  </div></Modal>
}

function SettingsDialog({ initialSection, settings, backgroundImage, appIcons, uiTheme, appLanguage, aiSettings, remoteAgentIntegrationEnabled, onAiSettingsChange, onThemePreview, onLanguagePreview, onClose, onSave, onConfirm, onError }: { initialSection: SettingsSection; settings: TerminalSettings; backgroundImage: string; appIcons: AppIconSettings; uiTheme: UiTheme; appLanguage: AppLanguage; aiSettings: AiSettings; remoteAgentIntegrationEnabled: boolean; onAiSettingsChange(settings: AiSettings): void; onThemePreview(theme: UiTheme): void; onLanguagePreview(language: AppLanguage): void; onClose(): void; onSave(settings: TerminalSettings, backgroundImage: string, appIcon: AppIconId, uiTheme: UiTheme, appLanguage: AppLanguage, ai: AiSettingsInput, remoteAgentIntegrationEnabled: boolean): void; onConfirm: ConfirmAction; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [form, setForm] = useState<TerminalSettings>({ ...settings })
  const [previewImage, setPreviewImage] = useState(backgroundImage)
  const [appIcon, setAppIcon] = useState(appIcons.selected)
  const [theme, setTheme] = useState(uiTheme)
  const [language, setDialogLanguage] = useState(appLanguage)
  const [aiForm, setAiForm] = useState<AiSettings>({ ...aiSettings })
  const [remoteAgentIntegration, setRemoteAgentIntegration] = useState(remoteAgentIntegrationEnabled)
  const [apiKey, setApiKey] = useState('')
  const [testingAi, setTestingAi] = useState(false)
  const [aiTested, setAiTested] = useState(false)
  const [diagnosticPath, setDiagnosticPath] = useState('')
  const [section, setSection] = useState<SettingsSection>(initialSection)
  const [systemFonts, setSystemFonts] = useState<string[] | null>(null)
  const [manualFontMode, setManualFontMode] = useState(false)
  const [manualFontName, setManualFontName] = useState(() => primaryFontName(settings.fontFamily))
  const set = <K extends keyof TerminalSettings>(key: K, value: TerminalSettings[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const setAi = <K extends keyof AiSettings>(key: K, value: AiSettings[K]): void => { setAiTested(false); setAiForm((current) => ({ ...current, [key]: value })) }
  const aiInput = (): AiSettingsInput => ({ baseUrl: aiForm.baseUrl, model: aiForm.model, defaultShell: aiForm.defaultShell, provider: aiForm.provider, thinkingMode: aiForm.thinkingMode, ...(apiKey.trim() ? { apiKey: apiKey.trim() } : {}) })
  const selectTheme = (value: UiTheme): void => { setTheme(value); onThemePreview(value) }
  useEffect(() => {
    if (section !== 'terminal' || systemFonts !== null) return
    let active = true
    void window.api.settings.listSystemFonts()
      .then((fonts) => { if (active) setSystemFonts(fonts) })
      .catch(() => { if (active) setSystemFonts([]) })
    return () => { active = false }
  }, [section, systemFonts])
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
    if (section === 'ssh') { setRemoteAgentIntegration(false); return }
    setForm({ ...DEFAULT_TERMINAL_SETTINGS }); setManualFontMode(false); setManualFontName('JetBrains Mono'); setPreviewImage(''); setAppIcon('luna'); selectTheme('system'); setDialogLanguage('zh-CN'); onLanguagePreview('zh-CN')
  }
  const changeRemoteAgentIntegration = async (enabled: boolean): Promise<void> => {
    if (!enabled) { setRemoteAgentIntegration(false); return }
    const confirmed = await onConfirm({
      title: t('app.enableRemoteAgentIntegration'),
      message: t('app.remoteAgentIntegrationWarningTitle'),
      detail: t('app.remoteAgentIntegrationWarningDetail'),
      kind: 'warning',
      confirmLabel: t('app.understandAndEnable')
    })
    if (confirmed) setRemoteAgentIntegration(true)
  }
  const testAi = async (): Promise<void> => {
    setTestingAi(true); setAiTested(false)
    try { await window.api.ai.testSettings(aiInput()); setAiTested(true) }
    catch (error) { onError(errorMessage(error)) }
    finally { setTestingAi(false) }
  }
  const deleteAiKey = async (): Promise<void> => {
    if (!await onConfirm({ title: t('common.deleteApiKey'), message: t('app.deleteTheSavedAiApiKey'), detail: t('app.youWillNeedToEnterTheKeyAgain'), kind: 'danger', confirmLabel: t('common.deleteApiKey') })) return
    try { const saved = await window.api.ai.deleteApiKey(); setApiKey(''); setAiForm((current) => ({ ...current, apiKeyConfigured: saved.apiKeyConfigured })); onAiSettingsChange(saved) }
    catch (error) { onError(errorMessage(error)) }
  }
  const themeOptions: { value: UiTheme; label: string; icon: typeof Monitor }[] = [{ value: 'system', label: t('app.system'), icon: Monitor }, { value: 'light', label: t('app.light'), icon: Sun }, { value: 'dark', label: t('app.dark'), icon: Moon }]
  const providerOptions: { value: AiProvider; label: string }[] = [{ value: 'auto', label: t('app.autoDetect') }, { value: 'openAi', label: 'OpenAI' }, { value: 'anthropic', label: 'Anthropic' }, { value: 'qwen', label: 'Qwen' }, { value: 'deepSeek', label: 'DeepSeek' }, { value: 'kimi', label: 'Kimi' }, { value: 'glm', label: 'GLM' }, { value: 'miniMax', label: 'MiniMax' }, { value: 'grok', label: 'Grok' }, { value: 'gemini', label: 'Gemini' }]
  const providerClue = `${aiForm.provider} ${aiForm.baseUrl} ${aiForm.model}`.toLowerCase()
  const thinkingCannotDisable = providerClue.includes('grok') || providerClue.includes('api.x.ai') || providerClue.includes('kimi-k3') || providerClue.includes('k2.7') || providerClue.includes('gemini-3') || (providerClue.includes('gemini-2.5') && providerClue.includes('pro'))
  const thinkingHint = aiForm.thinkingMode === 'default' ? t('app.noThinkingControlsAreSentTheServiceDecides') : thinkingCannotDisable && aiForm.thinkingMode === 'disabled' ? t('app.thisModelCannotFullyDisableThinkingTheLowest') : t('app.theSelectedControlIsSentWithConnectionTests')
  const currentFontName = primaryFontName(form.fontFamily)
  const installedFonts = (systemFonts ?? []).filter((font) => font.toLocaleLowerCase() !== 'jetbrains mono')
  const installedFont = installedFonts.find((font) => font.toLocaleLowerCase() === currentFontName.toLocaleLowerCase())
  const fontChoice = manualFontMode ? '__manual__' : currentFontName === 'JetBrains Mono' ? '__bundled__' : currentFontName === 'monospace' ? '__system__' : systemFonts === null ? '__current__' : installedFont ?? '__manual__'
  return <Modal title={t('common.settings')} onClose={onClose} wide className="settings-dialog"><form className="form-grid terminal-settings-form" onSubmit={(event) => { event.preventDefault(); onSave(form, previewImage, appIcon, theme, language, aiInput(), remoteAgentIntegration) }}>
    <div className="settings-tabs" role="tablist" aria-label={t('app.settingsCategories')}>
      <div className="settings-nav-group" role="presentation"><span>{t('app.generalSettings')}</span>
        <button type="button" role="tab" aria-selected={section === 'appearance'} className={section === 'appearance' ? 'active' : ''} onClick={() => setSection('appearance')}><Palette size={15} />{t('app.appearance')}</button>
        <button type="button" role="tab" aria-selected={section === 'terminal'} className={section === 'terminal' ? 'active' : ''} onClick={() => setSection('terminal')}><SquareTerminal size={15} />{t('common.terminal')}</button>
      </div>
      <div className="settings-nav-group" role="presentation"><span>{t('app.sshSettings')}</span>
        <button type="button" role="tab" aria-selected={section === 'ssh'} className={section === 'ssh' ? 'active' : ''} onClick={() => setSection('ssh')}><Network size={15} />{t('app.remoteAgentIntegration')}</button>
      </div>
      <div className="settings-nav-group" role="presentation"><span>{t('app.toolSettings')}</span>
        <button type="button" role="tab" aria-selected={section === 'ai'} className={section === 'ai' ? 'active' : ''} onClick={() => setSection('ai')}><Bot size={15} />{t('app.aiCommandAssistant')}</button>
      </div>
      <div className="settings-nav-group" role="presentation"><span>{t('app.advanced')}</span>
        <button type="button" role="tab" aria-selected={section === 'diagnostics'} className={section === 'diagnostics' ? 'active' : ''} onClick={() => setSection('diagnostics')}><Stethoscope size={15} />{t('app.diagnostics')}</button>
      </div>
    </div>
    <div className={`settings-content ${section}`}>
    {section === 'appearance' ? <><fieldset className="ui-theme-settings"><legend>{t('common.theme')}</legend><div className="theme-options" role="radiogroup" aria-label={t('common.theme')}>{themeOptions.map((option) => { const Icon = option.icon; return <button key={option.value} type="button" role="radio" aria-checked={theme === option.value} className={theme === option.value ? 'active' : ''} onClick={() => selectTheme(option.value)}><Icon size={17} /><span>{option.label}</span></button> })}</div></fieldset><fieldset className="ui-theme-settings"><legend>{t('common.language')}</legend><div className="theme-options" role="radiogroup" aria-label={t('common.language')}>{availableLanguages.map((option) => <button key={option.code} type="button" role="radio" aria-checked={language === option.code} className={language === option.code ? 'active' : ''} onClick={() => { setDialogLanguage(option.code); onLanguagePreview(option.code) }}><Languages size={17} /><span>{option.label}</span></button>)}</div></fieldset><fieldset className="app-icon-settings"><legend>{t('app.appIcon')}</legend><div className="app-icon-options">{appIcons.options.map((icon) => <label key={icon.id} className={appIcon === icon.id ? 'selected' : ''}><input type="radio" name="app-icon" checked={appIcon === icon.id} onChange={() => setAppIcon(icon.id)} /><img src={icon.dataUrl} alt="" /><span>{t(appIconMessageKeys[icon.id])}</span></label>)}</div></fieldset><section className="diagnostic-section"><strong>{t('app.diagnostics')}</strong><div className="diagnostic-export"><button type="button" className="secondary-button" onClick={async () => { try { const path = await window.api.diagnostics.export(); if (path) setDiagnosticPath(path) } catch (error) { onError(errorMessage(error)) } }}><FileInput size={15} />{t('app.exportDiagnostics')}</button>{diagnosticPath && <small title={diagnosticPath}>{diagnosticPath}</small>}</div></section></> : section === 'diagnostics' ? <DiagnosticsPanel onError={onError} /> : section === 'ssh' ? <div className="remote-agent-settings">
      <div className="settings-option-row"><div><strong>{t('app.remoteAgentIntegration')}</strong><span>{t('app.remoteAgentIntegrationDescription')}</span></div><label className="switch-control"><input type="checkbox" checked={remoteAgentIntegration} onChange={(event) => void changeRemoteAgentIntegration(event.target.checked)} /><span aria-hidden="true" /></label></div>
      <div className="settings-information"><ShieldAlert size={17} /><div><strong>{remoteAgentIntegration ? t('app.remoteAgentIntegrationOn') : t('app.remoteAgentIntegrationOff')}</strong><span>{remoteAgentIntegration ? t('app.remoteAgentIntegrationOnDescription') : t('app.remoteAgentIntegrationOffDescription')}</span></div></div>
    </div> : section === 'ai' ? <div className="ai-settings">
      <label>API Base URL<input required type="url" value={aiForm.baseUrl} onChange={(event) => setAi('baseUrl', event.target.value)} placeholder="https://api.openai.com/v1" /></label>
      <label>{t('app.model')}<input value={aiForm.model} onChange={(event) => setAi('model', event.target.value)} placeholder="gpt-5" /></label>
      <label>API Key<div className="input-button"><input type="password" autoComplete="off" value={apiKey} onChange={(event) => { setApiKey(event.target.value); setAiTested(false) }} placeholder={aiForm.apiKeyConfigured ? t('app.savedSecurelyLeaveBlankToKeepIt') : t('common.optional')} /><button type="button" className="icon-button danger" title={t('app.deleteSavedApiKey')} disabled={!aiForm.apiKeyConfigured} onClick={() => void deleteAiKey()}><Trash2 size={15} /></button></div></label>
      <label>{t('app.defaultTargetShell')}<select value={aiForm.defaultShell} onChange={(event) => setAi('defaultShell', event.target.value as AiShell)}><option value="linux">Linux Shell</option><option value="powerShell">PowerShell</option><option value="cmd">Windows cmd</option><option value="macos">macOS Shell</option></select></label>
      <div className="ai-provider-row"><label>{t('app.provider')}<select value={aiForm.provider} onChange={(event) => setAi('provider', event.target.value as AiProvider)}>{providerOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label><label>{t('app.thinkingMode')}<select value={aiForm.thinkingMode} onChange={(event) => setAi('thinkingMode', event.target.value as AiThinkingMode)}><option value="default">{t('app.modelDefault')}</option><option value="disabled">{t('app.minimize')}</option><option value="enabled">{t('app.enabled')}</option></select></label></div>
      <small className="ai-thinking-hint">{thinkingHint}</small>
      <div className="ai-settings-test"><button type="button" className="secondary-button" disabled={testingAi || !aiForm.model.trim()} onClick={() => void testAi()}>{testingAi ? t('app.testing') : t('app.testConnection')}</button>{aiTested && <span><Check size={14} />{t('app.connected2')}</span>}</div>
    </div> : <>
      <div className="terminal-preview" style={terminalBackgroundStyle(form, previewImage)}><div style={{ color: form.foregroundColor, background: colorWithOpacity(form.backgroundColor, form.backgroundOpacity), fontFamily: form.fontFamily, fontSize: form.fontSize }}><span>user@host:~$</span> ls -la<br />drwxr-xr-x&nbsp;&nbsp;projects</div></div>
      <label>{t('app.font')}<div className={`terminal-font-picker ${fontChoice === '__manual__' ? 'manual' : ''}`}><select value={fontChoice} onChange={(event) => { const value = event.target.value; if (value === '__manual__') { setManualFontMode(true); setManualFontName(currentFontName); return } setManualFontMode(false); if (value === '__bundled__') { set('fontFamily', BUNDLED_TERMINAL_FONT); setManualFontName('JetBrains Mono'); return } if (value === '__system__') { set('fontFamily', 'monospace'); setManualFontName('monospace'); return } set('fontFamily', terminalFontStack(value)); setManualFontName(value) }}><option value="__bundled__">JetBrains Mono {t('app.bundled')}</option><option value="__system__">{t('app.systemMonospace')}</option>{systemFonts === null && !['JetBrains Mono', 'monospace'].includes(currentFontName) && <option value="__current__">{currentFontName} {t('app.current')}</option>}{installedFonts.map((font) => <option key={font} value={font}>{font}</option>)}<option value="__manual__">{t('app.enterAnotherFont')}</option></select>{fontChoice === '__manual__' && <input autoFocus value={manualFontName} placeholder={t('app.fontName')} onChange={(event) => { const value = event.target.value; setManualFontName(value); if (value.trim()) set('fontFamily', terminalFontStack(value)) }} />}</div></label>
      <label>{t('app.fontSize')}<div className="range-control"><input type="range" min="10" max="30" step="1" value={form.fontSize} style={{ '--range-progress': `${((form.fontSize - 10) / 20) * 100}%` } as React.CSSProperties} onChange={(event) => set('fontSize', Number(event.target.value))} /><output>{form.fontSize} px</output></div></label>
      <div className="color-settings"><label>{t('app.textColor')}<div className="color-control"><input type="color" value={form.foregroundColor} onChange={(event) => set('foregroundColor', event.target.value)} /><span>{form.foregroundColor}</span></div></label><label>{t('app.backgroundColor')}<div className="color-control"><input type="color" value={form.backgroundColor} onChange={(event) => set('backgroundColor', event.target.value)} /><span>{form.backgroundColor}</span></div></label></div>
      <label>{t('app.backgroundOpacity')}<div className="range-control"><input type="range" min="0" max="1" step="0.05" value={form.backgroundOpacity} style={{ '--range-progress': `${form.backgroundOpacity * 100}%` } as React.CSSProperties} onChange={(event) => set('backgroundOpacity', Number(event.target.value))} /><output>{Math.round(form.backgroundOpacity * 100)}%</output></div></label>
      <label>{t('app.backgroundImage')}<div className="input-button background-image-input"><input readOnly value={form.backgroundImagePath} placeholder={t('app.noneSelected')} /><button type="button" className="secondary-button" onClick={() => void chooseBackground()}><ImageIcon size={15} />{t('common.choose')}</button><button type="button" className="icon-button background-clear" title={t('app.clearBackgroundImage')} disabled={!form.backgroundImagePath} onClick={() => { set('backgroundImagePath', ''); setPreviewImage('') }}><Trash2 size={15} /></button></div></label>
      <label>{t('app.imageFit')}<select value={form.backgroundImageFit} onChange={(event) => set('backgroundImageFit', event.target.value as TerminalSettings['backgroundImageFit'])}><option value="cover">{t('app.cover')}</option><option value="contain">{t('app.contain')}</option><option value="stretch">{t('app.stretch')}</option><option value="tile">{t('app.tile')}</option></select></label>
    </>}
    </div>
    <div className="dialog-actions spread"><button type="button" className="secondary-button" onClick={reset}><RotateCcw size={15} />{t('app.restoreDefaults')}</button><div><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" type="submit">{t('common.save')}</button></div></div>
  </form></Modal>
}

function parseRuntimeEnvironmentDetail(detail: string): DiagnosticsRuntimeEnvironment[] {
  if (!detail || detail.includes('no luna-mux temp directory') || detail.includes('no persistent environment files') || detail.includes('active managed agents')) return []
  return detail.split(' | ').map((entry) => {
    const colon = entry.indexOf(':')
    if (colon < 0) return null
    const runtimeId = entry.slice(0, colon).trim()
    const fields: Record<string, string> = {}
    for (const part of entry.slice(colon + 1).split(',')) {
      const equals = part.indexOf('=')
      if (equals > 0) fields[part.slice(0, equals).trim()] = part.slice(equals + 1).trim()
    }
    return { runtimeId, hook: fields.hook_endpoint ?? '', mcp: fields.mcp_endpoint ?? '', tokens: fields.tokens ?? '' }
  }).filter((entry): entry is DiagnosticsRuntimeEnvironment => Boolean(entry && entry.runtimeId))
}

function agentTypeLabel(adapter: string): string {
  if (adapter === 'claude-code') return 'Claude Code'
  if (adapter === 'codex') return 'Codex'
  return adapter || 'Agent'
}

function formatActivity(value?: string | null): string {
  if (!value) return '-'
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  const datePart = [
    date.getFullYear(),
    String(date.getMonth() + 1).padStart(2, '0'),
    String(date.getDate()).padStart(2, '0'),
  ].join('-')
  const timePart = [
    String(date.getHours()).padStart(2, '0'),
    String(date.getMinutes()).padStart(2, '0'),
    String(date.getSeconds()).padStart(2, '0'),
  ].join(':')
  return datePart + ' ' + timePart
}

function readableDiagnosticDetail(name: string, detail: string): string {
  if (name === 'local_agents') return detail.replace(/;\s*/g, '\n')
  if (name === 'wsl_distributions') return detail.replace(/,\s*/g, '\n')
  if (name === 'wsl_interop_exe') return detail.replace(/;\s*/g, '\n')
  return detail
}

function DiagnosticsPanel({ onError }: { onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [report, setReport] = useState<DoctorReport | null>(null)
  const [running, setRunning] = useState(false)
  const [copied, setCopied] = useState(false)
  const [diagnosticPath, setDiagnosticPath] = useState('')
  const [expandedAgents, setExpandedAgents] = useState<Record<string, boolean>>({})
  const runDiagnostics = async (): Promise<void> => {
    setRunning(true)
    setCopied(false)
    setExpandedAgents({})
    try {
      const next = await window.api.diagnostics.run()
      setReport(next)
    } catch (error) {
      setReport(null)
      onError(errorMessage(error))
    } finally {
      setRunning(false)
    }
  }
  useEffect(() => {
    void runDiagnostics()
  }, [])
  const copyReport = async (): Promise<void> => {
    if (!report) return
    try {
      await window.api.system.writeClipboard(JSON.stringify(report, null, 2))
      setCopied(true)
      setTimeout(() => setCopied(false), 1800)
    } catch (error) { onError(errorMessage(error)) }
  }
  const exportDiagnostics = async (): Promise<void> => {
    try {
      const path = await window.api.diagnostics.export()
      if (path) setDiagnosticPath(path)
    } catch (error) { onError(errorMessage(error)) }
  }
  const statusIcon = (status: DoctorCheckStatus): React.JSX.Element => status === 'ok' ? <Check size={15} /> : status === 'warn' ? <ShieldAlert size={15} /> : <X size={15} />
  const statusLabel = (status: DoctorCheckStatus): string => status === 'ok' ? t('app.diagnosticsStatusOk') : status === 'warn' ? t('app.diagnosticsStatusWarn') : t('app.diagnosticsStatusError')
  const checkLabel = (name: string): string => {
    if (name === 'executable') return t('app.diagnosticsCheckExecutable')
    if (name === 'local_agents') return t('app.diagnosticsCheckLocalAgents')
    if (name === 'runtime_env_files') return t('app.diagnosticsCheckRuntimeEnv')
    if (name === 'managed_agents') return t('app.diagnosticsCheckManagedAgents')
    if (name === 'wsl_distributions') return t('app.diagnosticsCheckWsl')
    if (name === 'wsl_interop_exe') return t('app.diagnosticsCheckWslInterop')
    return name
  }
  const toggleAgent = (agentId: string): void => setExpandedAgents((current) => ({ ...current, [agentId]: !current[agentId] }))
  const agentStatusLabel = (status: string): string => {
    const value = status.toLowerCase()
    if (value === 'working') return t('app.agentWorking')
    if (value === 'waiting') return t('app.agentWaiting')
    if (value === 'completed') return t('app.agentCompleted')
    if (value === 'error') return t('app.agentError')
    return status
  }
  const agentStatusTone = (status: string): string => {
    const value = status.toLowerCase()
    if (value === 'error') return 'error'
    if (value === 'waiting') return 'warning'
    if (value === 'completed') return 'ok'
    return 'working'
  }
  const runtimeStatus = (value: string): DoctorCheckStatus => {
    if (value === 'reachable') return 'ok'
    if (!value || value === 'missing') return 'warn'
    return 'error'
  }
  const runtimeStatusLabel = (value: string): string => {
    if (value === 'reachable') return t('app.diagnosticsReachable')
    if (!value || value === 'missing') return t('app.diagnosticsMissing')
    return value
  }
  const renderCheckDetail = (check: DoctorCheck): React.JSX.Element => {
    if (check.name === 'managed_agents') {
      const agents = report?.managedAgents ?? []
      if (agents.length === 0) return <code className="diagnostics-check-detail">{check.detail || t('app.diagnosticsNoDetail')}</code>
      return <div className="diagnostics-agent-list">
        {agents.map((agent) => {
          const expanded = Boolean(expandedAgents[agent.agentId])
          const paneLabel = agent.paneTitle || agent.sessionName || agent.paneId || '-'
          return <div className="diagnostics-agent" key={agent.agentId}>
            <button type="button" className="diagnostics-agent-summary" aria-expanded={expanded} onClick={() => toggleAgent(agent.agentId)}>
              <span className="diagnostics-agent-primary">
                <span className="diagnostics-agent-kind">{agentTypeLabel(agent.adapter)}</span>
                <span className="diagnostics-agent-location" title={agent.paneId}>{paneLabel}</span>
              </span>
              <span className={'diagnostics-agent-status ' + agentStatusTone(agent.status)}>{agentStatusLabel(agent.status)}</span>
              {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
            </button>
            {expanded && <dl className="diagnostics-agent-details">
              <div><dt>{t('app.diagnosticsAgentType')}</dt><dd>{agentTypeLabel(agent.adapter)}</dd></div>
              <div><dt>{t('app.diagnosticsAgentPane')}</dt><dd title={agent.paneId}>{agent.paneTitle || agent.paneId || '-'}</dd></div>
              <div><dt>{t('app.diagnosticsAgentSession')}</dt><dd title={agent.muxSessionId}>{agent.sessionName || agent.muxSessionId || '-'}</dd></div>
              <div><dt>{t('app.diagnosticsAgentLastActivity')}</dt><dd title={agent.lastActivity ?? ''}>{formatActivity(agent.lastActivity)}</dd></div>
              <div><dt>{t('app.diagnosticsAgentId')}</dt><dd title={agent.agentId}>{agent.agentId}</dd></div>
              <div><dt>{t('app.diagnosticsAgentRuntimeId')}</dt><dd title={agent.runtimeId}>{agent.runtimeId}</dd></div>
            </dl>}
          </div>
        })}
      </div>
    }
    if (check.name === 'runtime_env_files') {
      const runtimes = parseRuntimeEnvironmentDetail(check.detail)
      if (runtimes.length === 0) {
        const fallback = check.detail.includes('active managed agents')
          ? t('app.diagnosticsRuntimeEnvManagedAgents')
          : check.detail.includes('no persistent runtime environment files')
            ? t('app.diagnosticsRuntimeEnvNone')
            : check.detail.includes('no luna-mux temp directory')
              ? t('app.diagnosticsRuntimeEnvNoTemp')
              : readableDiagnosticDetail(check.name, check.detail) || t('app.diagnosticsNoDetail')
        return <span className="diagnostics-runtime-fallback">{fallback}</span>
      }
      return <div className="diagnostics-runtime-list">
        {runtimes.map((runtime) => {
          const tokenMissing = runtime.tokens === 'missing'
          return <div className="diagnostics-runtime" key={runtime.runtimeId}>
            <div className="diagnostics-runtime-heading"><strong>{t('app.diagnosticsRuntimeInstance')}</strong><code title={runtime.runtimeId}>{runtime.runtimeId}</code></div>
            <dl className="diagnostics-runtime-details">
              <div><dt>{t('app.diagnosticsHookEndpoint')}</dt><dd className={runtimeStatus(runtime.hook)}>{runtimeStatusLabel(runtime.hook)}</dd></div>
              <div><dt>{t('app.diagnosticsMcpEndpoint')}</dt><dd className={runtimeStatus(runtime.mcp)}>{runtimeStatusLabel(runtime.mcp)}</dd></div>
              <div><dt>{t('app.diagnosticsAuthorization')}</dt><dd className={tokenMissing ? 'warn' : 'ok'}>{tokenMissing ? t('app.diagnosticsMissing') : t('app.diagnosticsConfigured')}</dd></div>
            </dl>
          </div>
        })}
      </div>
    }
    if (check.name === 'wsl_interop_exe') {
      const detail = readableDiagnosticDetail(check.name, check.detail) || t('app.diagnosticsNoDetail')
      const repair = check.status === 'error' ? t('app.diagnosticsWslInteropRepair') : ''
      return <code className="diagnostics-check-detail">{repair ? `${detail}\n${repair}` : detail}</code>
    }
    return <code className="diagnostics-check-detail">{readableDiagnosticDetail(check.name, check.detail) || t('app.diagnosticsNoDetail')}</code>
  }
  const overallStatus: DoctorCheckStatus = report ? report.ok ? report.checks.some((check) => check.status === 'warn') ? 'warn' : 'ok' : 'error' : 'ok'
  const overallLabel = report ? overallStatus === 'ok' ? t('app.diagnosticsAllGood') : overallStatus === 'warn' ? t('app.diagnosticsNeedsAttention') : t('app.diagnosticsHasErrors') : t('app.diagnosticsReady')
  return <div className="diagnostics-panel">
    <section className={report ? 'diagnostics-summary ' + overallStatus : 'diagnostics-summary idle'}>
      <div className="diagnostics-summary-icon">{report ? statusIcon(overallStatus) : <Stethoscope size={15} />}</div>
      <div className="diagnostics-summary-copy">
        <strong>{overallLabel}</strong>
        <span>{t('app.diagnosticsDescription')}</span>
      </div>
      <div className="diagnostics-actions">
        <button type="button" className="secondary-button" disabled={running} onClick={() => void runDiagnostics()}><RotateCcw size={15} />{running ? t('app.diagnosticsRunning') : t('app.diagnosticsRerun')}</button>
        <button type="button" className="secondary-button" disabled={!report} onClick={() => void copyReport()}>{copied ? <Check size={15} /> : <Copy size={15} />}{copied ? t('app.diagnosticsCopied') : t('app.diagnosticsCopyReport')}</button>
        <button type="button" className="secondary-button" onClick={() => void exportDiagnostics()}><FileInput size={15} />{t('app.exportDiagnostics')}</button>
      </div>
    </section>
    {diagnosticPath && <small className="diagnostics-path" title={diagnosticPath}>{diagnosticPath}</small>}
    {running && !report && <div className="diagnostics-empty"><RotateCcw size={22} className="spin" /><span>{t('app.diagnosticsRunning')}</span></div>}
    {!running && !report && <div className="diagnostics-empty"><button type="button" className="primary-button" onClick={() => void runDiagnostics()}><Stethoscope size={15} />{t('app.diagnosticsRun')}</button></div>}
    {report && <section className="diagnostics-checks">
      {report.checks.map((check) => <article key={check.name} className={'diagnostics-check ' + check.status}>
        <div className="diagnostics-check-icon">{statusIcon(check.status)}</div>
        <div className="diagnostics-check-main">
          <div className="diagnostics-check-heading"><strong>{checkLabel(check.name)}</strong><span className={'diagnostics-status ' + check.status}>{statusLabel(check.status)}</span></div>
          {check.name === 'runtime_env_files' && <span className="diagnostics-check-description">{t('app.diagnosticsRuntimeEnvDescription')}</span>}
          {renderCheckDetail(check)}
        </div>
      </article>)}
    </section>}
  </div>
}

function GroupNameDialog({ mode, initialName, onClose, onSave }: { mode: 'create' | 'rename'; initialName: string; onClose(): void; onSave(name: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [name, setName] = useState(initialName)
  return <Modal title={mode === 'create' ? t('common.newGroup') : t('common.renameGroup')} onClose={onClose}><form className="form-grid compact-form" onSubmit={(event) => { event.preventDefault(); onSave(name.trim()) }}>
    <label>{t('app.groupName')}<input autoFocus required value={name} onChange={(event) => setName(event.target.value)} placeholder={t('app.forExampleProduction')} /></label>
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button type="submit" className="primary-button">{t('common.save')}</button></div>
  </form></Modal>
}

function MuxSessionDialog({ mode, session, onClose, onSave }: { mode: 'create' | 'rename'; session?: MuxSession; onClose(): void; onSave(name: string, rootPath: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [name, setName] = useState(session?.name ?? '')
  const [rootPath, setRootPath] = useState(session?.rootPath ?? '')
  const chooseRoot = async (): Promise<void> => {
    const path = await window.api.files.chooseLocalDirectory()
    if (path) { setRootPath(path); if (!name.trim()) setName(path.split(/[\\/]/).filter(Boolean).at(-1) ?? '') }
  }
  return <Modal title={mode === 'create' ? t('app.newSession') : t('app.renameSession')} onClose={onClose}><form className="form-grid compact-form" onSubmit={(event) => { event.preventDefault(); onSave(name.trim(), rootPath.trim()) }}>
    <label>{t('app.sessionName')}<input autoFocus required value={name} onChange={(event) => setName(event.target.value)} placeholder={t('app.untitledSession')} /></label>
    <label>{t('app.projectRoot')}<div className="input-button"><input value={rootPath} onChange={(event) => setRootPath(event.target.value)} placeholder={t('app.optionalProjectRoot')} /><button type="button" className="secondary-button" onClick={() => void chooseRoot()}>{t('common.choose')}</button></div></label>
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button type="submit" className="primary-button">{t('common.save')}</button></div>
  </form></Modal>
}

function PaneLauncherDialog({ targets, loading, onClose, onManageConnections, onSelect }: { targets: TerminalTarget[]; loading: boolean; onClose(): void; onManageConnections(): void; onSelect(target: TerminalTarget, paneTitle: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [paneTitle, setPaneTitle] = useState('')
  const localTargets = targets.filter((target) => target.transport === 'localPty')
  const sshTargets = targets.filter((target) => target.transport === 'ssh')
  const renderTarget = (target: TerminalTarget): React.JSX.Element => <button key={target.id} type="button" onClick={() => onSelect(target, paneTitle.trim())}>{target.transport === 'ssh' ? <Server size={18} /> : <SquareTerminal size={18} />}<span><strong>{target.label}</strong><small>{target.transport === 'ssh' ? t('app.sshEnvironment') : target.kind === 'wsl' ? t('app.wslEnvironment') : t('app.localEnvironment')}</small></span><ChevronRight size={16} /></button>
  return <Modal title={t('app.addPane')} onClose={onClose} className="pane-launcher-dialog"><div className="pane-launcher">
    <label className="pane-name-field">{t('app.paneName')}<input autoFocus value={paneTitle} onChange={(event) => setPaneTitle(event.target.value)} placeholder={t('app.paneNamePlaceholder')} /></label>
    {loading && <div className="pane-target-loading"><span className="status-dot connecting" />{t('app.loadingTerminalTargets')}</div>}
    <section><header><span>{t('app.onThisComputer')}</span><small>{localTargets.length}</small></header><div className="local-target-list">{localTargets.map(renderTarget)}</div></section>
    <section><header><span>{t('app.overSsh')}</span><button className="text-button" onClick={onManageConnections}>{t('app.manageSshTargets')}</button></header><div className="local-target-list">{sshTargets.map(renderTarget)}{sshTargets.length === 0 && !loading && <button type="button" className="empty-target" onClick={onManageConnections}><Server size={18} /><span><strong>{t('app.noSshTargets')}</strong><small>{t('app.addSshTargetToUse')}</small></span><ChevronRight size={16} /></button>}</div></section>
  </div></Modal>
}

function PaneNameDialog({ pane, onClose, onSave }: { pane: WorkspaceTab; onClose(): void; onSave(title: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [title, setTitle] = useState(pane.title)
  return <Modal title={t('app.renamePane')} onClose={onClose}><form className="form-grid compact-form" onSubmit={(event) => { event.preventDefault(); onSave(title.trim()) }}>
    <label>{t('app.paneNameRequired')}<input autoFocus required value={title} onChange={(event) => setTitle(event.target.value)} /></label>
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button type="submit" className="primary-button">{t('common.save')}</button></div>
  </form></Modal>
}

function BookmarkDialog({ bookmark, connections, groups, onClose, onSaved, onError }: { bookmark?: Bookmark; connections: Bookmark[]; groups: string[]; onClose(): void; onSaved(): void; onError(message: string): void }): React.JSX.Element {
  const { t } = useI18n()
  const [form, setForm] = useState<BookmarkInput>(bookmark ? { name: bookmark.name, host: bookmark.host, port: bookmark.port, username: bookmark.username, authType: bookmark.authType, privateKeyPath: bookmark.privateKeyPath, jumpBookmarkId: bookmark.jumpBookmarkId, groupName: bookmark.groupName, favorite: bookmark.favorite, keepaliveEnabled: bookmark.keepaliveEnabled, keepaliveIntervalSeconds: bookmark.keepaliveIntervalSeconds, keepaliveCountMax: bookmark.keepaliveCountMax, note: bookmark.note } : emptyBookmark)
  const set = <K extends keyof BookmarkInput>(key: K, value: BookmarkInput[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const jumpCandidates = connections.filter((item) => item.id !== bookmark?.id && !item.jumpBookmarkId)
  return <Modal title={bookmark ? t('common.editConnection') : t('common.newConnection')} onClose={onClose}><form className="form-grid" onSubmit={async (event) => { event.preventDefault(); try { await window.api.bookmarks.save({ ...form, id: bookmark?.id }); onSaved() } catch (error) { onError(errorMessage(error)) } }}>
    <div className="form-row"><label>{t('app.name')}<input required value={form.name} onChange={(event) => set('name', event.target.value)} placeholder={t('app.productionServer')} /></label><label>{t('app.group')}<select value={form.groupName} onChange={(event) => set('groupName', event.target.value)}><option value="">{t('common.ungrouped')}</option>{groups.filter(Boolean).map((group) => <option key={group} value={group}>{group}</option>)}</select></label></div>
    <label className="check-label"><input type="checkbox" checked={form.favorite} onChange={(event) => set('favorite', event.target.checked)} />{t('app.favoriteThisConnection')}</label>
    <div className="form-row"><label>{t('app.hostIpOrDomain')}<input required value={form.host} onChange={(event) => set('host', event.target.value)} placeholder="192.168.1.10" /></label><label className="port-field">{t('app.port')}<input type="number" min="1" max="65535" value={form.port} onChange={(event) => set('port', Number(event.target.value))} /></label></div>
    <label>{t('app.username')}<input required value={form.username} onChange={(event) => set('username', event.target.value)} placeholder="root" /></label>
    <label>{t('app.authentication')}<select value={form.authType} onChange={(event) => set('authType', event.target.value as BookmarkInput['authType'])}><option value="password">{t('app.password')}</option><option value="privateKey">{t('common.privateKey')}</option><option value="agent">SSH Agent</option></select></label>
    {form.authType === 'privateKey' && <label>{t('app.privateKeyFile')}<div className="input-button"><input readOnly value={form.privateKeyPath} placeholder={t('app.chooseAnOpensshPrivateKey')} /><button type="button" className="secondary-button" onClick={async () => { const path = await window.api.files.choosePrivateKey(); if (path) set('privateKeyPath', path) }}>{t('common.choose')}</button></div></label>}
    <label>{t('app.route')}<select value={form.jumpBookmarkId} onChange={(event) => set('jumpBookmarkId', event.target.value)}><option value="">{t('app.direct')}</option>{jumpCandidates.map((item) => <option key={item.id} value={item.id}>{t('app.viaValueValueValue', { value0: item.name, value1: item.username, value2: item.host })}</option>)}</select></label>
    <label className="check-label"><input type="checkbox" checked={form.keepaliveEnabled} onChange={(event) => set('keepaliveEnabled', event.target.checked)} />{t('app.keepTheSshSessionAlive')}</label>
    {form.keepaliveEnabled && <div className="form-row"><label>{t('app.keepaliveIntervalSeconds')}<input type="number" min="5" max="300" value={form.keepaliveIntervalSeconds} onChange={(event) => set('keepaliveIntervalSeconds', Number(event.target.value))} /></label><label>{t('app.maximumMissedResponses')}<input type="number" min="1" max="10" value={form.keepaliveCountMax} onChange={(event) => set('keepaliveCountMax', Number(event.target.value))} /></label></div>}
    <label>{t('app.notes')}<textarea rows={3} value={form.note} onChange={(event) => set('note', event.target.value)} placeholder={t('common.optional')} /></label>
    {bookmark?.hasSavedCredential && <button type="button" className="text-button danger" onClick={async () => { await window.api.bookmarks.forgetCredential(bookmark.id); onSaved() }}>{t('app.clearSavedCredentials')}</button>}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button className="primary-button" type="submit">{t('common.save')}</button></div>
  </form></Modal>
}

function AuthDialog({ bookmark, jumpBookmark, onClose, onConnect }: { bookmark: Bookmark; jumpBookmark?: Bookmark; onClose(): void; onConnect(credentials: ConnectionCredentials): void }): React.JSX.Element {
  const { t } = useI18n()
  const [secret, setSecret] = useState(''); const [remember, setRemember] = useState(false); const [visible, setVisible] = useState(false)
  const [jumpSecret, setJumpSecret] = useState(''); const [rememberJump, setRememberJump] = useState(false); const [jumpVisible, setJumpVisible] = useState(false)
  const required = bookmark.authType === 'password' && !bookmark.hasSavedCredential
  const jumpRequired = Boolean(jumpBookmark?.authType === 'password' && !jumpBookmark.hasSavedCredential)
  return <Modal title={t('app.connectToValue', { value0: bookmark.name })} onClose={onClose}><form className="form-grid" onSubmit={(event) => { event.preventDefault(); if ((required && !secret) || (jumpRequired && !jumpSecret)) return; onConnect({ credential: secret || undefined, rememberCredential: remember, jumpCredential: jumpSecret || undefined, rememberJumpCredential: rememberJump }) }}>
    {jumpBookmark ? <div className="connection-route"><span><small>{t('common.jumpHost')}</small><strong>{jumpBookmark.name}</strong></span><ArrowRight size={16} /><span><small>{t('common.target')}</small><strong>{bookmark.name}</strong></span></div> : <div className="connection-target"><Server size={18} /><strong>{bookmark.username}@{bookmark.host}:{bookmark.port}</strong></div>}
    {jumpBookmark && jumpBookmark.authType !== 'agent' && <CredentialField bookmark={jumpBookmark} role={t('common.jumpHost')} secret={jumpSecret} setSecret={setJumpSecret} remember={rememberJump} setRemember={setRememberJump} visible={jumpVisible} setVisible={setJumpVisible} autoFocus />}
    {bookmark.authType !== 'agent' && <CredentialField bookmark={bookmark} role={jumpBookmark ? t('common.target') : ''} secret={secret} setSecret={setSecret} remember={remember} setRemember={setRemember} visible={visible} setVisible={setVisible} autoFocus={!jumpBookmark || jumpBookmark.authType === 'agent'} />}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>{t('common.cancel')}</button><button type="submit" className="primary-button">{t('common.connect')}</button></div>
  </form></Modal>
}

function CredentialField({ bookmark, role, secret, setSecret, remember, setRemember, visible, setVisible, autoFocus }: { bookmark: Bookmark; role: string; secret: string; setSecret(value: string): void; remember: boolean; setRemember(value: boolean): void; visible: boolean; setVisible(value: boolean): void; autoFocus: boolean }): React.JSX.Element {
  const { t } = useI18n()
  const credentialName = bookmark.authType === 'password' ? t('app.password2') : t('app.privateKeyPassphraseOptional')
  const roleName = role || t('app.connection')
  return <div className="credential-block"><div className="credential-heading"><strong>{t('app.valueCredentials', { value0: roleName })}</strong><span>{bookmark.username}@{bookmark.host}:{bookmark.port}</span></div>
    <label>{role}{credentialName}<div className="password-input"><input autoFocus={autoFocus} type={visible ? 'text' : 'password'} required={bookmark.authType === 'password' && !bookmark.hasSavedCredential} value={secret} placeholder={bookmark.hasSavedCredential ? t('app.savedLeaveBlankToKeepUsingIt') : undefined} onChange={(event) => setSecret(event.target.value)} /><button className="icon-button" type="button" title={visible ? t('app.hidePassword') : t('app.showPassword')} onClick={() => setVisible(!visible)}>{visible ? <EyeOff size={16} /> : <Eye size={16} />}</button></div></label>
    <label className="check-label"><input type="checkbox" checked={remember} disabled={!secret} onChange={(event) => setRemember(event.target.checked)} />{t('app.rememberCredentials', { role: role || t('common.connection') })}</label>
  </div>
}

function HostKeyDialog({ prompt, onDecision }: { prompt: HostKeyPrompt; onDecision(accept: boolean): void }): React.JSX.Element {
  const { t } = useI18n()
  const changed = prompt.status === 'changed'
  return <Modal title={changed ? t('app.hostKeyChanged') : t('app.verifyHostKey')} onClose={() => onDecision(false)}><div className="dialog-copy">
    <p>{changed ? t('app.theHostKeyDoesNotMatchThePreviously') : t('app.thisIsTheFirstConnectionToThisServer')}</p>
    <dl><dt>{t('app.server')}</dt><dd>{prompt.host}:{prompt.port}</dd>{prompt.previousFingerprint && <><dt>{t('app.previousFingerprint')}</dt><dd>{prompt.previousFingerprint}</dd></>}<dt>{t('app.currentFingerprint')}</dt><dd>{prompt.fingerprint}</dd></dl>
    <div className="dialog-actions"><button className="secondary-button" onClick={() => onDecision(false)}>{t('app.cancel')}</button><button className={changed ? 'danger-button' : 'primary-button'} onClick={() => onDecision(true)}>{t('app.trustAndContinue')}</button></div>
  </div></Modal>
}

function ConflictDialog({ conflict, onDecision }: { conflict: { sourcePath: string; destinationPath: string }; onDecision(value: ConflictResolution, apply: boolean): void }): React.JSX.Element {
  const { t } = useI18n()
  const [apply, setApply] = useState(false)
  return <Modal title={t('app.aFileWithThisNameAlreadyExists')} onClose={() => onDecision('skip', false)}><div className="dialog-copy"><p className="path-line">{conflict.destinationPath}</p><label className="check-label"><input type="checkbox" checked={apply} onChange={(event) => setApply(event.target.checked)} />{t('app.applyToAllConflictsInThisBatch')}</label><div className="dialog-actions spread"><button className="secondary-button" onClick={() => onDecision('skip', apply)}>{t('app.skip')}</button><button className="secondary-button" onClick={() => onDecision('rename', apply)}>{t('app.rename')}</button><button className="danger-button" onClick={() => onDecision('overwrite', apply)}>{t('app.overwrite')}</button></div></div></Modal>
}

function Modal({ title, onClose, children, wide = false, raised = false, className = '' }: { title: string; onClose(): void; children: React.ReactNode; wide?: boolean; raised?: boolean; className?: string }): React.JSX.Element {
  const { t } = useI18n()
  return <div className={`modal-backdrop ${raised ? 'raised' : ''}`} onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}><section className={`modal ${wide ? 'wide' : ''} ${className}`} role="dialog" aria-modal="true"><header><strong>{title}</strong><button className="icon-button" title={t('common.close')} aria-label={t('common.close')} onClick={onClose}><X size={18} /></button></header>{children}</section></div>
}

function ConfirmationDialog({ confirmation, onDecision }: { confirmation: ConfirmationOptions; onDecision(accepted: boolean): void }): React.JSX.Element {
  const { t } = useI18n()
  return <Modal title={confirmation.title} onClose={() => onDecision(false)} raised className="secondary-confirm-dialog"><div className={`confirmation-dialog ${confirmation.kind}`}>
    <div className="confirmation-heading">{confirmation.kind === 'danger' ? <Trash2 size={22} /> : <ShieldAlert size={22} />}<div><strong>{confirmation.message}</strong>{confirmation.detail && <span>{confirmation.detail}</span>}</div></div>
    <div className="dialog-actions"><button type="button" autoFocus className="secondary-button" onClick={() => onDecision(false)}>{t('common.cancel')}</button><button type="button" className={confirmation.kind === 'danger' ? 'danger-button' : 'primary-button'} onClick={() => onDecision(true)}>{confirmation.kind === 'danger' ? <Trash2 size={15} /> : <Rocket size={15} />}{confirmation.confirmLabel}</button></div>
  </div></Modal>
}

function TransferPanel({ transfers, view, setView, onRetry, onCancel, onClear }: { transfers: TransferTask[]; view: 'queue' | 'history'; setView(value: 'queue' | 'history'): void; onRetry(task: TransferTask): void; onCancel(task: TransferTask): void; onClear(): void }): React.JSX.Element {
  const { t } = useI18n()
  const activeStatuses = ['queued', 'scanning', 'running', 'conflict']
  const shown = transfers.filter((item) => view === 'queue' ? activeStatuses.includes(item.status) || item.status === 'failed' : !activeStatuses.includes(item.status))
  const active = transfers.filter((item) => activeStatuses.includes(item.status))
  const totalBytes = active.reduce((sum, item) => sum + item.bytesTotal, 0)
  const transferredBytes = active.reduce((sum, item) => sum + item.bytesTransferred, 0)
  const speed = active.reduce((sum, item) => sum + item.speed, 0)
  const eta = speed > 0 && totalBytes > transferredBytes ? Math.ceil((totalBytes - transferredBytes) / speed) : 0
  const speedText = speed >= 1024 * 1024 ? `${(speed / 1024 / 1024).toFixed(1)} MB/s` : `${Math.round(speed / 1024)} KB/s`
  return <section className="transfer-panel"><header><div className="transfer-tabs"><button className={view === 'queue' ? 'active' : ''} onClick={() => setView('queue')}>{t('app.queue')} <span>{active.length}</span></button><button className={view === 'history' ? 'active' : ''} onClick={() => setView('history')}>{t('app.history')}</button></div>{active.length > 0 && <div className="transfer-summary"><strong>{totalBytes ? Math.round(transferredBytes / totalBytes * 100) : 0}%</strong><span>{speedText}</span>{eta > 0 && <span>{eta >= 60 ? t('app.transferEtaMinutes', { value: Math.ceil(eta / 60) }) : t('app.transferEtaSeconds', { value: eta })}</span>}</div>}<button className="text-button" onClick={onClear}>{t('app.clearCompleted')}</button></header>
    <div className="transfer-list">{shown.length === 0 ? <div className="transfer-empty">{view === 'queue' ? t('app.noActiveTransfers') : t('app.noTransferHistory')}</div> : shown.slice(0, 40).map((task) => {
      const percent = task.bytesTotal ? Math.min(100, Math.round(task.bytesTransferred / task.bytesTotal * 100)) : 0
      return <div className="transfer-row" key={task.id}><span className={`transfer-direction ${task.direction}`}>{task.direction === 'upload' ? '↑' : '↓'}</span><div className="transfer-info"><strong>{task.displayName}</strong><small>{task.error ?? `${task.sourcePath} → ${task.destinationPath}`}</small>{['running', 'scanning'].includes(task.status) && <div className="progress"><span style={{ width: `${percent}%` }} /></div>}</div><span className={`transfer-status ${task.status}`}>{task.status === 'running' ? `${percent}%` : ({ queued: t('app.queued'), scanning: t('app.scanning'), conflict: t('app.waiting'), completed: t('app.completed'), failed: t('app.failed'), cancelled: t('app.cancelled'), interrupted: t('app.interrupted') } as Record<string, string>)[task.status]}</span><div className="transfer-actions">{activeStatuses.includes(task.status) && <button className="icon-button" title={t('common.cancel')} onClick={() => onCancel(task)}><X size={15} /></button>}{['failed', 'cancelled', 'interrupted'].includes(task.status) && <button className="text-button" onClick={() => onRetry(task)}>{t('app.retry')}</button>}</div></div>
    })}</div>
  </section>
}
