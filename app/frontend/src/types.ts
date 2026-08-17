import type {
  TerminalRuntime,
  TerminalRuntimeCreateRequest,
  TerminalRuntimeEvent,
  TerminalRuntimeOutputReadResult,
  TerminalTarget
} from './terminal-runtime-contract'

export interface MuxSession {
  id: string
  name: string
  rootPath: string
  layout?: MuxSplitNode
  sortOrder: number
  createdAt: string
  updatedAt: string
}

export interface MuxSessionInput {
  id?: string
  name: string
  rootPath?: string
  layout?: MuxSplitNode
}

export type MuxSplitNode =
  | { type: 'pane'; paneId: string }
  | { type: 'split'; direction: 'horizontal' | 'vertical'; ratio: number; first: MuxSplitNode; second: MuxSplitNode }

export type MuxPaneKind = 'terminal'

export interface ChromeInstallation {
  executablePath: string
  version: string
}

export type BrowserRuntimeStatus = 'starting' | 'running' | 'stopped' | 'error'

export interface BrowserRuntime {
  id: string
  muxSessionId: string
  browserResourceId: string
  url: string
  cdpPort: number
  profilePath: string
  processId: number
  status: BrowserRuntimeStatus
  error?: string
}

export interface BrowserRuntimeCreateRequest {
  muxSessionId: string
  browserResourceId: string
  url?: string
  temporaryProfile?: boolean
}

export type BrowserRuntimeEvent =
  | { type: 'started'; runtime: BrowserRuntime }
  | { type: 'status'; runtimeId: string; status: BrowserRuntimeStatus; error?: string }

export interface BrowserMouseEvent {
  eventType: 'mousePressed' | 'mouseReleased' | 'mouseMoved' | 'mouseWheel'
  x: number
  y: number
  button?: 'none' | 'left' | 'middle' | 'right'
  buttons?: number
  deltaX?: number
  deltaY?: number
  modifiers?: number
}

export interface BrowserKeyEvent {
  eventType: 'keyDown' | 'keyUp' | 'char'
  key: string
  code: string
  text?: string
  modifiers?: number
}

export interface MuxPane {
  id: string
  muxSessionId: string
  kind: MuxPaneKind
  title: string
  targetId: string
  bookmarkId: string
  cwd: string
  command: string
  launchProfileId: string
  sortOrder: number
  createdAt: string
  updatedAt: string
}

export interface MuxPaneInput {
  id?: string
  muxSessionId: string
  kind: MuxPaneKind
  title: string
  targetId: string
  bookmarkId?: string
  cwd?: string
  command?: string
  launchProfileId?: string
}

export interface BrowserResource {
  id: string
  muxSessionId: string
  name: string
  sourcePaneId: string
  bookmarkId: string
  url: string
  temporaryProfile: boolean
  sortOrder: number
  createdAt: string
  updatedAt: string
}

export interface BrowserResourceInput {
  id?: string
  muxSessionId: string
  name: string
  sourcePaneId?: string
  bookmarkId?: string
  url?: string
  temporaryProfile?: boolean
}

export type {
  TerminalRuntime,
  TerminalRuntimeCreateRequest,
  TerminalRuntimeEvent,
  TerminalRuntimeOutputReadResult,
  TerminalTarget
} from './terminal-runtime-contract'

export type AuthType = 'password' | 'privateKey' | 'agent'
export type Platform = 'darwin' | 'win32' | 'linux'

export type ManagedAgentStatus = 'working' | 'waiting' | 'completed' | 'error'
export type ManagedAgentWaitingReason = 'input' | 'permission' | 'external' | 'unknown'
export type ManagedAgentEvidence = 'structuredHook' | 'terminalHeuristic'

export interface AgentLaunchProfile {
  id: string
  label: string
  adapter: string
  command: string
  builtIn: boolean
}

export interface AgentProfileAvailability {
  profileId: string
  targetId: string
  available: boolean
  detail: string
}

export interface DoctorManagedAgent {
  agentId: string
  adapter: string
  runtimeId: string
  paneId: string
  paneTitle: string
  muxSessionId: string
  sessionName: string
  status: string
  lastActivity?: string | null
}

export interface ManagedAgentEvent {
  sequence: number
  timestamp: string
  context: {
    muxSessionId: string
    paneId: string
    runtimeId: string
    agentId: string
    launchProfileId: string
  }
  adapterId: string
  agentSessionId?: string
  agentTurnId?: string
  hookEventName: string
  status: ManagedAgentStatus
  waitingReason?: ManagedAgentWaitingReason
  evidence: ManagedAgentEvidence
}

export interface ManagedAgentNotificationActivation {
  muxSessionId: string
  paneId: string
  sequence: number
}

export interface ManagedAgentDesktopNotification extends ManagedAgentNotificationActivation {
  title: string
  body: string
  status: ManagedAgentStatus
}

export interface ControlAuditRecord {
  id: string
  timestamp: string
  callerId: string
  callerKind: string
  operation: string
  resourceKind: string
  resourceId: string
  arguments: Record<string, unknown>
  result: string
  errorCode: string
}

export interface Bookmark {
  id: string
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  privateKeyPath: string
  jumpBookmarkId: string
  groupName: string
  favorite: boolean
  lastConnectedAt: string
  keepaliveEnabled: boolean
  keepaliveIntervalSeconds: number
  keepaliveCountMax: number
  note: string
  hasSavedCredential: boolean
  sortOrder: number
  createdAt: string
  updatedAt: string
}

export type BookmarkInput = Omit<Bookmark, 'id' | 'hasSavedCredential' | 'lastConnectedAt' | 'sortOrder' | 'createdAt' | 'updatedAt'>

export interface BookmarkGroupDeleteResult {
  groups: string[]
  deletedBookmarkIds: string[]
}

export interface SshConfigImportEntry {
  alias: string
  name: string
  host: string
  port: number
  username: string
  privateKeyPath: string
  proxyJumpAlias: string
}

export interface SshConfigPreview {
  path: string
  entries: SshConfigImportEntry[]
}

export interface BookmarkArchiveEntry {
  id: string
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  privateKeyPath: string
  jumpBookmarkId: string
  groupName: string
  favorite: boolean
  keepaliveEnabled: boolean
  keepaliveIntervalSeconds: number
  keepaliveCountMax: number
  note: string
}

export type BookmarkArchiveSource = 'lunaMux' | 'lunaRemote' | 'legacy'

export interface BookmarkArchivePreview {
  previewId: string
  path: string
  source: BookmarkArchiveSource
  exportedAt: string
  groups: string[]
  connections: BookmarkArchiveEntry[]
  credentialsIncluded: false
}

export interface KnownHostImportEntry {
  host: string
  port: number
  fingerprint: string
}

export interface LunaRemoteSource {
  path: string
  sourceModifiedAt: string
}

export interface LunaRemoteImportPreview {
  previewId: string
  path: string
  sourceModifiedAt: string
  groups: string[]
  connections: BookmarkArchiveEntry[]
  knownHosts: KnownHostImportEntry[]
  settingKeys: string[]
  forwardingProfiles: PortForwardProfile[]
  credentialConnectionIds: string[]
}

export interface LunaRemoteImportSelection {
  previewId: string
  connectionIds: string[]
  groups: string[]
  importHostKeys: boolean
  importSettings: boolean
  importForwardingProfiles: boolean
  importCredentials: boolean
}

export interface LunaRemoteImportResult {
  importedConnections: number
  importedGroups: number
  importedHostKeys: number
  importedSettings: number
  importedForwardingProfiles: number
  importedCredentials: number
  unavailableCredentials: number
}

export interface ConnectInput {
  bookmarkId: string
  newSession?: boolean
  credential?: string
  rememberCredential?: boolean
  jumpCredential?: string
  rememberJumpCredential?: boolean
}

export type SessionStatus = 'connecting' | 'connected' | 'disconnected' | 'error'

export interface SessionSummary {
  id: string
  bookmarkId: string
  title: string
  status: SessionStatus
  error?: string
}

export type PortForwardType = 'local' | 'remote' | 'dynamic'
export type TunnelStatus = 'starting' | 'running' | 'error'

export interface PortForwardProfile {
  id: string
  bookmarkId: string
  name: string
  type: PortForwardType
  bindAddress: string
  bindPort: number
  targetHost: string
  targetPort: number
}

export interface TunnelSummary {
  id: string
  profileId: string
  sessionId: string
  name: string
  type: PortForwardType
  bindAddress: string
  bindPort: number
  targetHost: string
  targetPort: number
  status: TunnelStatus
  error?: string
}

export interface BrowserTunnel {
  tunnel: TunnelSummary
  localUrl: string
}

export interface HostKeyPrompt {
  sessionId: string
  host: string
  port: number
  fingerprint: string
  status: 'unknown' | 'changed'
  previousFingerprint?: string
}

export interface DirectoryEntry {
  name: string
  path: string
  kind: 'file' | 'directory' | 'symlink' | 'other'
  size?: number
  modifiedAt?: number
}

export interface FilePreview {
  content: string
  size: number
  truncated: boolean
  position: 'start' | 'end'
  binary: boolean
}

export interface DeploymentProfile {
  id: string
  bookmarkId: string
  name: string
  localDirectory: string
  remoteDirectory: string
  deleteExtraneous: boolean
}

export interface DeploymentDiffEntry {
  relativePath: string
  localPath?: string
  remotePath: string
  status: 'new' | 'changed' | 'same' | 'remote-only'
  size?: number
}

export type TerminalBackgroundFit = 'cover' | 'contain' | 'stretch' | 'tile'
export type AppIconId = 'luna' | 'graphite' | 'signal' | 'light'
export type UiTheme = 'system' | 'light' | 'dark'
export type AppLanguage = string

export interface NativeMenuLabels {
  settings: string
  newConnection: string
  importOpenSshConfig: string
  newSession: string
  closeTab: string
  terminal: string
  files: string
  toggleSidebar: string
  helpItem: string
  about: string
  services: string
  hide: string
  hideOthers: string
  showAll: string
  quit: string
  connectionMenu: string
  editMenu: string
  undo: string
  redo: string
  cut: string
  copy: string
  paste: string
  selectAll: string
  viewMenu: string
  fullscreen: string
  windowMenu: string
  minimize: string
  zoom: string
  bringAllToFront: string
  helpMenu: string
}
export type AiShell = 'linux' | 'powerShell' | 'cmd' | 'macos'
export type AiRiskLevel = 'low' | 'medium' | 'high'
export type AiProvider = 'auto' | 'openAi' | 'anthropic' | 'qwen' | 'deepSeek' | 'kimi' | 'glm' | 'miniMax' | 'grok' | 'gemini'
export type AiThinkingMode = 'default' | 'disabled' | 'enabled'

export interface AiSettings {
  baseUrl: string
  model: string
  defaultShell: AiShell
  provider: AiProvider
  thinkingMode: AiThinkingMode
  apiKeyConfigured: boolean
}

export interface AiSettingsInput {
  baseUrl: string
  model: string
  defaultShell: AiShell
  provider: AiProvider
  thinkingMode: AiThinkingMode
  apiKey?: string
}

export interface AiCommandSuggestion {
  command: string
  explanation: string
  assumptions: string[]
  warnings: string[]
  riskLevel: AiRiskLevel
}

export interface AiCommandHistoryEntry extends AiCommandSuggestion {
  id: string
  createdAt: string
  requirement: string
  shell: AiShell
}

export interface AiRiskAssessment {
  riskLevel: AiRiskLevel
  warnings: string[]
}

export interface AiRawExchange {
  occurredAt: string
  endpoint: string
  requestHeaders: string
  requestBody: string
  responseStatus?: number
  responseHeaders: string
  responseBody: string
  error: string
}

export const DEFAULT_AI_SETTINGS: AiSettings = {
  baseUrl: 'https://api.openai.com/v1',
  model: '',
  defaultShell: 'linux',
  provider: 'auto',
  thinkingMode: 'default',
  apiKeyConfigured: false
}

export interface AppIconOption {
  id: AppIconId
  name: string
  dataUrl: string
}

export interface AppIconSettings {
  selected: AppIconId
  options: AppIconOption[]
}

export interface TerminalSettings {
  fontFamily: string
  fontSize: number
  foregroundColor: string
  backgroundColor: string
  backgroundOpacity: number
  backgroundImagePath: string
  backgroundImageFit: TerminalBackgroundFit
}

export const BUNDLED_TERMINAL_FONT = '"JetBrains Mono", "SFMono-Regular", Consolas, monospace'

export const DEFAULT_TERMINAL_SETTINGS: TerminalSettings = {
  fontFamily: BUNDLED_TERMINAL_FONT,
  fontSize: 14,
  foregroundColor: '#e8eaed',
  backgroundColor: '#111315',
  backgroundOpacity: 1,
  backgroundImagePath: '',
  backgroundImageFit: 'cover'
}

export type TransferDirection = 'upload' | 'download'
export type TransferStatus = 'queued' | 'scanning' | 'running' | 'conflict' | 'completed' | 'failed' | 'cancelled' | 'interrupted'
export type ConflictResolution = 'overwrite' | 'skip' | 'rename'

export interface TransferTask {
  id: string
  sessionId: string
  bookmarkId: string
  direction: TransferDirection
  sourcePath: string
  destinationPath: string
  displayName: string
  status: TransferStatus
  bytesTotal: number
  bytesTransferred: number
  speed: number
  error?: string
  createdAt: string
  updatedAt: string
}

export type MenuCommand =
  | 'new-connection'
  | 'import-connections'
  | 'new-session'
  | 'close-tab'
  | 'settings'
  | 'help'
  | 'show-terminal'
  | 'show-files'
  | 'toggle-sidebar'

export type AppEvent =
  | { type: 'session'; payload: SessionSummary }
  | { type: 'terminal-data'; payload: { sessionId: string; data: string } }
  | { type: 'host-key'; payload: HostKeyPrompt }
  | { type: 'transfer'; payload: TransferTask }
  | { type: 'tunnel'; payload: TunnelSummary & { removed?: boolean } }
  | { type: 'transfer-conflict'; payload: { taskId: string; sourcePath: string; destinationPath: string } }
  | { type: 'menu-command'; payload: MenuCommand }

export interface TransferRequest {
  sessionId: string
  direction: TransferDirection
  sourcePaths: string[]
  destinationDirectory: string
}

export type ControlStateChangedEvent =
  | { type: 'muxSessionSaved'; payload: MuxSession }
  | { type: 'muxPaneSaved'; payload: MuxPane }
  | { type: 'muxPaneCreated'; payload: { pane: MuxPane; session: MuxSession; start: boolean } }

export type DoctorCheckStatus = 'ok' | 'warn' | 'error'

export interface DoctorCheck {
  name: string
  status: DoctorCheckStatus
  detail: string
}

export interface DoctorReport {
  ok: boolean
  checks: DoctorCheck[]
  managedAgents?: DoctorManagedAgent[]
}

export type ClipboardContent =
  | { type: 'text'; text: string }
  | { type: 'image' }
  | { type: 'empty' }

export interface AppApi {
  platform: Platform
  system: {
    openExternal(url: string): Promise<void>
    readClipboard(): Promise<ClipboardContent>
    writeClipboard(text: string): Promise<void>
    minimizeWindow(): Promise<void>
    toggleMaximizeWindow(): Promise<void>
      closeWindow(): Promise<void>
  }
  bookmarks: {
    list(): Promise<Bookmark[]>
    save(input: BookmarkInput & { id?: string }): Promise<Bookmark>
    reorder(ids: string[]): Promise<Bookmark[]>
    moveToGroup(id: string, groupName: string): Promise<Bookmark[]>
    duplicate(id: string): Promise<Bookmark>
    remove(id: string): Promise<void>
    forgetCredential(id: string): Promise<void>
    previewSshConfig(): Promise<SshConfigPreview | null>
    importSshConfig(path: string, aliases: string[]): Promise<Bookmark[]>
    exportArchive(): Promise<string | null>
    previewArchive(): Promise<BookmarkArchivePreview | null>
    importArchive(previewId: string, connectionIds: string[], groups: string[]): Promise<{ importedConnections: number; importedGroups: number }>
    discoverLunaRemoteSources(): Promise<LunaRemoteSource[]>
    previewLunaRemote(path: string): Promise<LunaRemoteImportPreview>
    chooseLunaRemoteDatabase(): Promise<LunaRemoteImportPreview | null>
    importLunaRemote(selection: LunaRemoteImportSelection): Promise<LunaRemoteImportResult>
  }
  bookmarkGroups: {
    list(): Promise<string[]>
    create(name: string): Promise<string[]>
    rename(oldName: string, newName: string): Promise<string[]>
    delete(name: string): Promise<BookmarkGroupDeleteResult>
    reorder(groups: string[]): Promise<string[]>
  }
  sessions: {
    connect(input: ConnectInput): Promise<SessionSummary>
    disconnect(id: string): Promise<void>
    write(id: string, data: string): void
    writeCommand(id: string, data: string): Promise<void>
    resize(id: string, cols: number, rows: number): void
    flow(id: string, paused: boolean): void
    hostKeyDecision(id: string, accept: boolean): void
  }
  muxSessions: {
    list(): Promise<MuxSession[]>
    save(input: MuxSessionInput): Promise<MuxSession>
    remove(id: string): Promise<void>
  }
  muxPanes: {
    list(muxSessionId?: string): Promise<MuxPane[]>
    save(input: MuxPaneInput): Promise<MuxPane>
    remove(id: string): Promise<void>
  }
  browserResources: {
    list(muxSessionId?: string): Promise<BrowserResource[]>
    save(input: BrowserResourceInput): Promise<BrowserResource>
    remove(id: string): Promise<void>
  }
  browserRuntimes: {
    discoverChrome(): Promise<ChromeInstallation | null>
    create(request: BrowserRuntimeCreateRequest): Promise<BrowserRuntime>
    list(): Promise<BrowserRuntime[]>
    close(runtimeId: string): Promise<void>
    navigate(runtimeId: string, url: string): Promise<void>
    focusExternal(runtimeId: string): Promise<void>
    resize(runtimeId: string, width: number, height: number): Promise<void>
    mouse(runtimeId: string, event: BrowserMouseEvent): Promise<void>
    key(runtimeId: string, event: BrowserKeyEvent): Promise<void>
  }
  terminalRuntimes: {
    targets(): Promise<TerminalTarget[]>
    list(): Promise<TerminalRuntime[]>
    create(request: TerminalRuntimeCreateRequest): Promise<TerminalRuntime>
    readOutput(runtimeId: string, fromCursor: number, maxBytes: number): Promise<TerminalRuntimeOutputReadResult>
    write(runtimeId: string, data: string): Promise<void>
    resize(runtimeId: string, cols: number, rows: number): Promise<void>
    flow(runtimeId: string, paused: boolean): Promise<void>
    interrupt(runtimeId: string): Promise<void>
    close(runtimeId: string): Promise<void>
  }
  managedAgents: {
    profiles(): Promise<AgentLaunchProfile[]>
    availability(profileId: string, targetId: string): Promise<AgentProfileAvailability>
    events(): Promise<ManagedAgentEvent[]>
    setNotificationFocus(muxSessionId: string | undefined, paneId: string | undefined, terminalVisible: boolean): Promise<void>
  }
  controlAudit: {
    list(limit: number): Promise<ControlAuditRecord[]>
    clear(): Promise<number>
  }
  files: {
    home(): Promise<string>
    parentLocal(path: string): Promise<string>
    remoteHome(sessionId: string): Promise<string>
    listLocal(path: string): Promise<DirectoryEntry[]>
    listRemote(sessionId: string, path: string): Promise<DirectoryEntry[]>
    createDirectory(remote: boolean, sessionId: string | undefined, path: string): Promise<void>
    rename(remote: boolean, sessionId: string | undefined, from: string, to: string): Promise<void>
    remove(remote: boolean, sessionId: string | undefined, paths: string[]): Promise<void>
    preview(remote: boolean, sessionId: string | undefined, path: string, position: 'start' | 'end'): Promise<FilePreview>
    getFavorites(bookmarkId: string): Promise<{ local: string[]; remote: string[] }>
    setFavorites(bookmarkId: string, value: { local: string[]; remote: string[] }): Promise<void>
    chooseLocalDirectory(): Promise<string | null>
    choosePrivateKey(): Promise<string | null>
  }
  transfers: {
    list(): Promise<TransferTask[]>
    enqueue(request: TransferRequest): Promise<TransferTask[]>
    cancel(id: string): Promise<void>
    retry(id: string, sessionId: string): Promise<void>
    clearCompleted(): Promise<void>
    resolveConflict(id: string, resolution: ConflictResolution, applyToBatch: boolean): void
  }
  deployments: {
    list(bookmarkId: string): Promise<DeploymentProfile[]>
    save(profile: DeploymentProfile): Promise<DeploymentProfile>
    remove(id: string): Promise<void>
    preview(id: string, sessionId: string): Promise<DeploymentDiffEntry[]>
    execute(id: string, sessionId: string): Promise<TransferTask[]>
  }
  tunnels: {
    listProfiles(bookmarkId: string): Promise<PortForwardProfile[]>
    saveProfile(profile: PortForwardProfile): Promise<PortForwardProfile>
    removeProfile(id: string): Promise<void>
    list(sessionId: string): Promise<TunnelSummary[]>
    start(sessionId: string, profileId: string): Promise<TunnelSummary>
    startBrowser(sessionId: string, browserResourceId: string, sourcePaneId: string, remoteUrl: string): Promise<BrowserTunnel>
    stop(sessionId: string, tunnelId: string): Promise<void>
  }
  diagnostics: {
    run(filter?: string): Promise<DoctorReport>
    export(): Promise<string | null>
  }
  ai: {
    getSettings(): Promise<AiSettings>
    saveSettings(settings: AiSettingsInput): Promise<AiSettings>
    deleteApiKey(): Promise<AiSettings>
    testSettings(settings: AiSettingsInput): Promise<void>
    generate(requirement: string, shell: AiShell, terminalContext?: string, redactTerminalContext?: boolean): Promise<AiCommandSuggestion>
    analyze(command: string): Promise<AiRiskAssessment>
    listHistory(): Promise<AiCommandHistoryEntry[]>
    clearHistory(): Promise<void>
    getLastExchange(): Promise<AiRawExchange | null>
    clearLastExchange(): Promise<void>
  }
  state: {
    getSidebarCollapsed(): Promise<boolean>
    setSidebarCollapsed(collapsed: boolean): Promise<void>
    getCollapsedBookmarkGroups(): Promise<string[]>
    setCollapsedBookmarkGroups(groups: string[]): Promise<void>
    getSidebarWidth(): Promise<number>
    setSidebarWidth(width: number): Promise<void>
  }
  settings: {
    getLanguage(): Promise<AppLanguage>
    applyLanguage(menu: NativeMenuLabels): Promise<void>
    saveLanguage(language: AppLanguage, menu: NativeMenuLabels): Promise<AppLanguage>
    getUiTheme(): Promise<UiTheme>
    saveUiTheme(theme: UiTheme): Promise<UiTheme>
    getRemoteAgentIntegrationEnabled(): Promise<boolean>
    saveRemoteAgentIntegrationEnabled(enabled: boolean): Promise<boolean>
    getTerminal(): Promise<TerminalSettings>
    saveTerminal(settings: TerminalSettings): Promise<TerminalSettings>
    listSystemFonts(): Promise<string[]>
    chooseTerminalBackground(): Promise<string | null>
    loadTerminalBackground(path: string): Promise<string>
    getAppIcons(): Promise<AppIconSettings>
    setAppIcon(icon: AppIconId): Promise<AppIconId>
  }
  onEvent(listener: (event: AppEvent) => void): () => void
  onTerminalRuntimeEvent(listener: (event: TerminalRuntimeEvent) => void): () => void
  onManagedAgentEvent(listener: (event: ManagedAgentEvent) => void): () => void
  onManagedAgentNotificationActivate(listener: (event: ManagedAgentNotificationActivation) => void): () => void
  onBrowserRuntimeEvent(listener: (event: BrowserRuntimeEvent) => void): () => void
  onUiThemeChanged(listener: (theme: UiTheme) => void): () => void
  onTerminalSettingsChanged(listener: (settings: TerminalSettings) => void): () => void
  onControlStateChanged(listener: (event: ControlStateChangedEvent) => void): () => void
}
