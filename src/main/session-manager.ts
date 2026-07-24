import { Client, type ClientChannel, type ConnectConfig, type SFTPWrapper } from 'ssh2'
import { readFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import type { AppDatabase } from './database'
import type { AppEvent, Bookmark, ConnectInput, DirectoryEntry, SessionSummary } from '../shared/types'

interface ActiveSession {
  summary: SessionSummary
  bookmark: Bookmark
  client: Client
  shell?: ClientChannel
  sftp?: SFTPWrapper
}

export class SessionManager {
  private sessions = new Map<string, ActiveSession>()
  private hostKeyCallbacks = new Map<string, { callback: (accept: boolean) => void; fingerprint: string }>()

  constructor(private db: AppDatabase, private emit: (event: AppEvent) => void) {}

  list(): SessionSummary[] { return [...this.sessions.values()].map((item) => item.summary) }

  async connect(input: ConnectInput): Promise<SessionSummary> {
    const bookmark = this.db.getBookmark(input.bookmarkId)
    if (!bookmark) throw new Error('书签不存在')
    const sessionId = randomUUID()
    const summary: SessionSummary = { id: sessionId, bookmarkId: bookmark.id, title: bookmark.name, status: 'connecting' }
    const client = new Client()
    const session: ActiveSession = { summary, bookmark, client }
    this.sessions.set(sessionId, session)
    this.emitSession(session)

    const credential = input.credential ?? this.db.getCredential(bookmark.id)
    if (input.rememberCredential && input.credential) this.db.saveCredential(bookmark.id, input.credential)
    // ssh2 supports an async callback with no return value; @types/ssh2 incorrectly
    // requires the fingerprint callback overload to return boolean.
    const hostVerifier = ((fingerprint: string, callback: (accept: boolean) => void): void => {
      this.verifyHost(sessionId, bookmark, fingerprint, callback)
    }) as unknown as NonNullable<ConnectConfig['hostVerifier']>
    const config: ConnectConfig = {
      host: bookmark.host, port: bookmark.port, username: bookmark.username,
      readyTimeout: 20_000, keepaliveInterval: 15_000, keepaliveCountMax: 3,
      hostHash: 'sha256', hostVerifier
    }
    if (bookmark.authType === 'password') {
      if (!credential) { this.sessions.delete(sessionId); throw new Error('需要输入密码') }
      config.password = credential
      config.tryKeyboard = true
    } else {
      if (!bookmark.privateKeyPath) { this.sessions.delete(sessionId); throw new Error('请选择私钥文件') }
      config.privateKey = await readFile(bookmark.privateKeyPath)
      if (credential) config.passphrase = credential
    }

    client.on('keyboard-interactive', (_name, _instructions, _lang, prompts, finish) => {
      finish(prompts.map(() => credential ?? ''))
    })
    client.on('ready', () => this.openShell(session))
    client.on('error', (error) => this.failSession(session, error.message))
    client.on('close', () => {
      if (session.summary.status !== 'error') session.summary = { ...session.summary, status: 'disconnected' }
      session.shell = undefined; session.sftp = undefined; this.emitSession(session)
    })
    client.connect(config)
    return summary
  }

  private verifyHost(sessionId: string, bookmark: Bookmark, fingerprint: string, callback: (accept: boolean) => void): void {
    const known = this.db.getKnownHost(bookmark.host, bookmark.port)
    if (known === fingerprint) { callback(true); return }
    this.hostKeyCallbacks.set(sessionId, { callback, fingerprint })
    this.emit({ type: 'host-key', payload: {
      sessionId, host: bookmark.host, port: bookmark.port, fingerprint,
      status: known ? 'changed' : 'unknown', previousFingerprint: known
    } })
    setTimeout(() => {
      const pending = this.hostKeyCallbacks.get(sessionId)
      if (pending) { this.hostKeyCallbacks.delete(sessionId); pending.callback(false) }
    }, 60_000)
  }

  hostKeyDecision(sessionId: string, accept: boolean): void {
    const pending = this.hostKeyCallbacks.get(sessionId)
    const session = this.sessions.get(sessionId)
    if (!pending || !session) return
    this.hostKeyCallbacks.delete(sessionId)
    if (accept) this.db.trustHost(session.bookmark.host, session.bookmark.port, pending.fingerprint)
    pending.callback(accept)
  }

  private openShell(session: ActiveSession): void {
    session.client.shell({ term: 'xterm-256color', cols: 100, rows: 30 }, (error, shell) => {
      if (error) { this.failSession(session, error.message); return }
      session.shell = shell
      session.summary = { ...session.summary, status: 'connected', error: undefined }
      shell.on('data', (data: Buffer) => this.emit({ type: 'terminal-data', payload: { sessionId: session.summary.id, data: data.toString('utf8') } }))
      shell.stderr.on('data', (data: Buffer) => this.emit({ type: 'terminal-data', payload: { sessionId: session.summary.id, data: data.toString('utf8') } }))
      this.emitSession(session)
    })
  }

  private failSession(session: ActiveSession, error: string): void {
    session.summary = { ...session.summary, status: 'error', error }
    this.emitSession(session)
  }

  private emitSession(session: ActiveSession): void { this.emit({ type: 'session', payload: session.summary }) }

  write(id: string, data: string): void { this.sessions.get(id)?.shell?.write(data) }
  resize(id: string, cols: number, rows: number): void { this.sessions.get(id)?.shell?.setWindow(rows, cols, 0, 0) }
  disconnect(id: string): void {
    const session = this.sessions.get(id)
    session?.shell?.close(); session?.client.end()
  }
  disconnectAll(): void { for (const session of this.sessions.values()) session.client.end() }

  async getSftp(id: string): Promise<SFTPWrapper> {
    const session = this.sessions.get(id)
    if (!session || session.summary.status !== 'connected') throw new Error('SSH 会话未连接')
    if (session.sftp) return session.sftp
    session.sftp = await new Promise<SFTPWrapper>((resolve, reject) => session.client.sftp((error, sftp) => error ? reject(error) : resolve(sftp)))
    return session.sftp
  }

  async listRemote(id: string, path: string): Promise<DirectoryEntry[]> {
    const sftp = await this.getSftp(id)
    return await new Promise((resolve, reject) => sftp.readdir(path, (error, list) => {
      if (error) { reject(error); return }
      resolve(list.filter((item) => item.filename !== '.' && item.filename !== '..').map((item) => {
        const mode = item.attrs.mode ?? 0
        const kind: DirectoryEntry['kind'] = (mode & 0o170000) === 0o040000 ? 'directory' : (mode & 0o170000) === 0o120000 ? 'symlink' : (mode & 0o170000) === 0o100000 ? 'file' : 'other'
        return { name: item.filename, path: path === '/' ? `/${item.filename}` : `${path}/${item.filename}`, kind, size: item.attrs.size ?? 0, modifiedAt: (item.attrs.mtime ?? 0) * 1000 }
      }))
    }))
  }
}
