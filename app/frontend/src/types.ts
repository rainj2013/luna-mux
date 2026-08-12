export type AuthType = 'password' | 'privateKey' | 'agent'
export type Platform = 'darwin' | 'win32' | 'linux'

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
export type AppIconId = 'ssh' | 'classic' | 'neon' | 'light'
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

export interface AppApi {
  platform: Platform
  system: {
    openExternal(url: string): Promise<void>
    readClipboard(): Promise<string>
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
    stop(sessionId: string, tunnelId: string): Promise<void>
  }
  diagnostics: {
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
    getTerminal(): Promise<TerminalSettings>
    saveTerminal(settings: TerminalSettings): Promise<TerminalSettings>
    listSystemFonts(): Promise<string[]>
    chooseTerminalBackground(): Promise<string | null>
    loadTerminalBackground(path: string): Promise<string>
    getAppIcons(): Promise<AppIconSettings>
    setAppIcon(icon: AppIconId): Promise<AppIconId>
  }
  onEvent(listener: (event: AppEvent) => void): () => void
}
