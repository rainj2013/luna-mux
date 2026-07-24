export type AuthType = 'password' | 'privateKey'

export interface Bookmark {
  id: string
  name: string
  host: string
  port: number
  username: string
  authType: AuthType
  privateKeyPath: string
  note: string
  hasSavedCredential: boolean
  createdAt: string
  updatedAt: string
}

export type BookmarkInput = Omit<Bookmark, 'id' | 'hasSavedCredential' | 'createdAt' | 'updatedAt'>

export interface ConnectInput {
  bookmarkId: string
  credential?: string
  rememberCredential?: boolean
}

export type SessionStatus = 'connecting' | 'connected' | 'disconnected' | 'error'

export interface SessionSummary {
  id: string
  bookmarkId: string
  title: string
  status: SessionStatus
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
  size: number
  modifiedAt: number
}

export type TransferDirection = 'upload' | 'download'
export type TransferStatus = 'queued' | 'scanning' | 'running' | 'conflict' | 'completed' | 'failed' | 'cancelled' | 'interrupted'
export type ConflictResolution = 'overwrite' | 'skip' | 'rename'

export interface TransferTask {
  id: string
  sessionId: string
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

export type AppEvent =
  | { type: 'session'; payload: SessionSummary }
  | { type: 'terminal-data'; payload: { sessionId: string; data: string } }
  | { type: 'host-key'; payload: HostKeyPrompt }
  | { type: 'transfer'; payload: TransferTask }
  | { type: 'transfer-conflict'; payload: { taskId: string; sourcePath: string; destinationPath: string } }

export interface TransferRequest {
  sessionId: string
  direction: TransferDirection
  sourcePaths: string[]
  destinationDirectory: string
}

export interface AppApi {
  bookmarks: {
    list(): Promise<Bookmark[]>
    save(input: BookmarkInput & { id?: string }): Promise<Bookmark>
    remove(id: string): Promise<void>
    forgetCredential(id: string): Promise<void>
  }
  sessions: {
    list(): Promise<SessionSummary[]>
    connect(input: ConnectInput): Promise<SessionSummary>
    disconnect(id: string): Promise<void>
    write(id: string, data: string): void
    resize(id: string, cols: number, rows: number): void
    hostKeyDecision(id: string, accept: boolean): void
  }
  files: {
    home(): Promise<string>
    listLocal(path: string): Promise<DirectoryEntry[]>
    listRemote(sessionId: string, path: string): Promise<DirectoryEntry[]>
    choosePrivateKey(): Promise<string | null>
  }
  transfers: {
    list(): Promise<TransferTask[]>
    enqueue(request: TransferRequest): Promise<TransferTask[]>
    cancel(id: string): Promise<void>
    retry(id: string): Promise<void>
    clearCompleted(): Promise<void>
    resolveConflict(id: string, resolution: ConflictResolution, applyToBatch: boolean): void
  }
  state: {
    getOpenBookmarks(): Promise<string[]>
    setOpenBookmarks(ids: string[]): Promise<void>
  }
  onEvent(listener: (event: AppEvent) => void): () => void
}
