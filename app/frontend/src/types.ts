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
  createdAt: string
  updatedAt: string
}

export type BookmarkInput = Omit<Bookmark, 'id' | 'hasSavedCredential' | 'lastConnectedAt' | 'createdAt' | 'updatedAt'>

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

export const DEFAULT_TERMINAL_SETTINGS: TerminalSettings = {
  fontFamily: 'Cascadia Mono, SFMono-Regular, Consolas, monospace',
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
    remove(id: string): Promise<void>
    forgetCredential(id: string): Promise<void>
    previewSshConfig(): Promise<SshConfigPreview | null>
    importSshConfig(path: string, aliases: string[]): Promise<Bookmark[]>
  }
  sessions: {
    list(): Promise<SessionSummary[]>
    connect(input: ConnectInput): Promise<SessionSummary>
    disconnect(id: string): Promise<void>
    write(id: string, data: string): void
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
  state: {
    getOpenBookmarks(): Promise<string[]>
    setOpenBookmarks(ids: string[]): Promise<void>
    getSidebarCollapsed(): Promise<boolean>
    setSidebarCollapsed(collapsed: boolean): Promise<void>
    getSidebarWidth(): Promise<number>
    setSidebarWidth(width: number): Promise<void>
  }
  settings: {
    getUiTheme(): Promise<UiTheme>
    saveUiTheme(theme: UiTheme): Promise<UiTheme>
    getTerminal(): Promise<TerminalSettings>
    saveTerminal(settings: TerminalSettings): Promise<TerminalSettings>
    chooseTerminalBackground(): Promise<string | null>
    loadTerminalBackground(path: string): Promise<string>
    getAppIcons(): Promise<AppIconSettings>
    setAppIcon(icon: AppIconId): Promise<AppIconId>
  }
  onEvent(listener: (event: AppEvent) => void): () => void
}
