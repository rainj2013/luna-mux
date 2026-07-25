import { Client, type ClientChannel, type ConnectConfig, type SFTPWrapper } from 'ssh2'
import { readFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import type { AppDatabase } from './database'
import { TerminalOutputBuffer } from './terminal-output-buffer'
import type { AppEvent, Bookmark, ConnectInput, DirectoryEntry, SessionSummary } from '../shared/types'

interface ActiveSession {
  summary: SessionSummary
  bookmark: Bookmark
  client: Client
  jumpClient?: Client
  shell?: ClientChannel
  sftp?: SFTPWrapper
  sftpPromise?: Promise<SFTPWrapper>
  remoteHome?: string
  remoteListMode?: 'exec' | 'sftp'
  terminalOutput?: TerminalOutputBuffer
  closing: boolean
}

interface HostKeyCallback {
  callback: (accept: boolean) => void
  fingerprint: string
  host: string
  port: number
}

class RemoteExecUnavailableError extends Error {}
class RemoteExecTimeoutError extends Error {}

const remoteListScript = `import base64,json,os,sys
path=os.fsdecode(base64.b64decode(sys.argv[1]))
items=[]
with os.scandir(path) as entries:
 for entry in entries:
  try:
   kind='symlink' if entry.is_symlink() else 'directory' if entry.is_dir(follow_symlinks=False) else 'file' if entry.is_file(follow_symlinks=False) else 'other'
   items.append({'name':entry.name,'path':os.path.join(path,entry.name),'kind':kind})
  except OSError:
   items.append({'name':entry.name,'path':os.path.join(path,entry.name),'kind':'other'})
sys.stdout.write(json.dumps(items))`

function shellQuote(value: string): string { return `'${value.replaceAll("'", `'"'"'`)}'` }

export class SessionManager {
  private sessions = new Map<string, ActiveSession>()
  private hostKeyCallbacks = new Map<string, HostKeyCallback>()

  constructor(private db: AppDatabase, private emit: (event: AppEvent) => void) {}

  list(): SessionSummary[] { return [...this.sessions.values()].map((item) => item.summary) }

  async connect(input: ConnectInput): Promise<SessionSummary> {
    const bookmark = this.db.getBookmark(input.bookmarkId)
    if (!bookmark) throw new Error('连接不存在')
    const active = [...this.sessions.values()].find((item) => item.bookmark.id === bookmark.id && !item.closing && ['connecting', 'connected'].includes(item.summary.status))
    if (active) return active.summary
    const jumpBookmark = bookmark.jumpBookmarkId ? this.db.getBookmark(bookmark.jumpBookmarkId) : undefined
    if (bookmark.jumpBookmarkId && !jumpBookmark) throw new Error('配置的跳板机连接不存在')
    if (jumpBookmark?.jumpBookmarkId) throw new Error('当前仅支持单级跳板机')

    const sessionId = randomUUID()
    const summary: SessionSummary = { id: sessionId, bookmarkId: bookmark.id, title: bookmark.name, status: 'connecting' }
    const client = new Client()
    const session: ActiveSession = { summary, bookmark, client, closing: false }
    this.sessions.set(sessionId, session)
    this.emitSession(session)

    try {
      const credential = input.credential ?? this.db.getCredential(bookmark.id)
      const targetConfig = await this.createConfig(sessionId, bookmark, credential, jumpBookmark ? '目标机器' : '连接')
      this.configureTargetClient(session, credential, Boolean(input.rememberCredential && input.credential), Boolean(jumpBookmark))

      if (!jumpBookmark) {
        client.connect(targetConfig)
        return summary
      }

      const jumpCredential = input.jumpCredential ?? this.db.getCredential(jumpBookmark.id)
      const jumpConfig = await this.createConfig(sessionId, jumpBookmark, jumpCredential, '跳板机')
      const jumpClient = new Client()
      session.jumpClient = jumpClient
      this.configureAuthentication(jumpClient, jumpCredential)
      jumpClient.on('ready', () => {
        if (input.rememberJumpCredential && input.jumpCredential) {
          try { this.db.saveCredential(jumpBookmark.id, input.jumpCredential) }
          catch (error) {
            this.failSession(session, error instanceof Error ? error.message : String(error))
            jumpClient.end()
            return
          }
        }
        jumpClient.forwardOut('127.0.0.1', 0, bookmark.host, bookmark.port, (error, stream) => {
          if (error) { this.failSession(session, `无法通过跳板机访问目标机器：${error.message}`); return }
          targetConfig.sock = stream
          client.connect(targetConfig)
        })
      })
      jumpClient.on('error', (error) => this.failSession(session, `跳板机连接失败：${error.message}`))
      jumpClient.on('close', () => {
        if (session.closing || this.sessions.get(sessionId) !== session) return
        this.closeSession(session, 'disconnected', '跳板机连接已断开')
      })
      jumpClient.connect(jumpConfig)
      return summary
    } catch (error) {
      this.releaseSession(session)
      throw error
    }
  }

  private async createConfig(sessionId: string, bookmark: Bookmark, credential: string | undefined, label: string): Promise<ConnectConfig> {
    const hostVerifier = ((fingerprint: string, callback: (accept: boolean) => void): void => {
      this.verifyHost(sessionId, bookmark, fingerprint, callback)
    }) as unknown as NonNullable<ConnectConfig['hostVerifier']>
    const config: ConnectConfig = {
      host: bookmark.host, port: bookmark.port, username: bookmark.username,
      readyTimeout: 20_000, keepaliveInterval: 15_000, keepaliveCountMax: 3,
      hostHash: 'sha256', hostVerifier
    }
    if (bookmark.authType === 'password') {
      if (!credential) throw new Error(`${label}需要输入密码`)
      config.password = credential
      config.tryKeyboard = true
    } else {
      if (!bookmark.privateKeyPath) throw new Error(`${label}未配置私钥文件`)
      config.privateKey = await readFile(bookmark.privateKeyPath)
      if (credential) config.passphrase = credential
    }
    return config
  }

  private configureAuthentication(client: Client, credential: string | undefined): void {
    client.on('keyboard-interactive', (_name, _instructions, _lang, prompts, finish) => {
      finish(prompts.map(() => credential ?? ''))
    })
  }

  private configureTargetClient(session: ActiveSession, credential: string | undefined, rememberCredential: boolean, viaJump: boolean): void {
    this.configureAuthentication(session.client, credential)
    session.client.on('ready', () => {
      if (rememberCredential && credential) {
        try { this.db.saveCredential(session.bookmark.id, credential) }
        catch (error) {
          this.failSession(session, error instanceof Error ? error.message : String(error))
          session.client.end()
          return
        }
      }
      this.openShell(session)
    })
    session.client.on('error', (error) => this.failSession(session, `${viaJump ? '目标机器连接失败：' : ''}${error.message}`))
    session.client.on('close', () => {
      if (session.closing || this.sessions.get(session.summary.id) !== session) return
      this.closeSession(session, 'disconnected')
    })
  }

  private verifyHost(sessionId: string, bookmark: Bookmark, fingerprint: string, callback: (accept: boolean) => void): void {
    const known = this.db.getKnownHost(bookmark.host, bookmark.port)
    if (known === fingerprint) { callback(true); return }
    const pending = { callback, fingerprint, host: bookmark.host, port: bookmark.port }
    this.hostKeyCallbacks.set(sessionId, pending)
    this.emit({ type: 'host-key', payload: {
      sessionId, host: bookmark.host, port: bookmark.port, fingerprint,
      status: known ? 'changed' : 'unknown', previousFingerprint: known
    } })
    setTimeout(() => {
      if (this.hostKeyCallbacks.get(sessionId) === pending) {
        this.hostKeyCallbacks.delete(sessionId)
        pending.callback(false)
      }
    }, 60_000)
  }

  hostKeyDecision(sessionId: string, accept: boolean): void {
    const pending = this.hostKeyCallbacks.get(sessionId)
    if (!pending || !this.sessions.has(sessionId)) return
    this.hostKeyCallbacks.delete(sessionId)
    if (accept) this.db.trustHost(pending.host, pending.port, pending.fingerprint)
    pending.callback(accept)
  }

  private openShell(session: ActiveSession): void {
    session.client.shell({ term: 'xterm-256color', cols: 100, rows: 30 }, (error, shell) => {
      if (error) { this.failSession(session, error.message); return }
      session.shell = shell
      session.summary = { ...session.summary, status: 'connected', error: undefined }
      const output = new TerminalOutputBuffer((data) => {
        this.emit({ type: 'terminal-data', payload: { sessionId: session.summary.id, data } })
      })
      session.terminalOutput = output
      shell.on('data', (data: Buffer) => output.write('stdout', data))
      shell.stderr.on('data', (data: Buffer) => output.write('stderr', data))
      shell.on('close', () => {
        output.end()
        if (session.terminalOutput === output) session.terminalOutput = undefined
      })
      this.emitSession(session)
    })
  }

  private failSession(session: ActiveSession, error: string): void {
    if (session.closing || session.summary.status === 'error') return
    this.closeSession(session, 'error', error)
  }

  private emitSession(session: ActiveSession): void { this.emit({ type: 'session', payload: session.summary }) }

  write(id: string, data: string): void { this.sessions.get(id)?.shell?.write(data) }
  resize(id: string, cols: number, rows: number): void { this.sessions.get(id)?.shell?.setWindow(rows, cols, 0, 0) }
  flow(id: string, paused: boolean): void {
    const shell = this.sessions.get(id)?.shell
    if (paused) shell?.pause()
    else shell?.resume()
  }
  disconnect(id: string): void {
    const session = this.sessions.get(id)
    if (!session) return
    this.closeSession(session, 'disconnected')
  }
  disconnectAll(): void { for (const id of this.sessions.keys()) this.disconnect(id) }

  getBookmarkId(id: string): string {
    return this.requireConnectedSession(id).bookmark.id
  }

  async getSftp(id: string): Promise<SFTPWrapper> {
    const session = this.sessions.get(id)
    if (!session || session.summary.status !== 'connected') throw new Error('SSH 会话未连接')
    if (session.sftp) return session.sftp
    if (session.sftpPromise) return await session.sftpPromise
    let settled = false
    const promise = new Promise<SFTPWrapper>((resolve, reject) => {
      const timer = setTimeout(() => {
        settled = true
        reject(new Error('初始化 SFTP 超时'))
      }, 12_000)
      session.client.sftp((error, sftp) => {
        if (settled) { sftp?.end(); return }
        settled = true
        clearTimeout(timer)
        if (error) reject(error)
        else resolve(sftp)
      })
    })
    session.sftpPromise = promise
    try {
      const sftp = await promise
      if (this.sessions.get(id) !== session || session.summary.status !== 'connected') {
        sftp.end()
        throw new Error('SSH 会话未连接')
      }
      session.sftp = sftp
      return sftp
    } finally {
      if (session.sftpPromise === promise) session.sftpPromise = undefined
    }
  }

  async getRemoteHome(id: string): Promise<string> {
    const session = this.requireConnectedSession(id)
    if (session.remoteHome) return session.remoteHome
    try {
      const path = (await this.execCommand(session, `printf '%s' "$HOME"`)).trim()
      if (path.startsWith('/')) { session.remoteHome = path; return path }
    } catch { /* Fall back to SFTP for restricted shells. */ }
    const sftp = await this.getSftp(id)
    const remoteHome = await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('读取远端初始目录超时')), 12_000)
      sftp.realpath('.', (error, path) => {
        clearTimeout(timer)
        if (error) reject(error)
        else resolve(path || '/')
      })
    })
    session.remoteHome = remoteHome
    return remoteHome
  }

  async listRemote(id: string, path: string): Promise<DirectoryEntry[]> {
    const session = this.requireConnectedSession(id)
    if (session.remoteListMode !== 'sftp') {
      try {
        const entries = await this.listRemoteWithExec(session, path)
        session.remoteListMode = 'exec'
        return entries
      } catch (error) {
        if (error instanceof RemoteExecUnavailableError) session.remoteListMode = 'sftp'
        else if (!(error instanceof RemoteExecTimeoutError)) throw error
      }
    }
    return await this.listRemoteWithSftp(id, path)
  }

  private requireConnectedSession(id: string): ActiveSession {
    const session = this.sessions.get(id)
    if (!session || session.summary.status !== 'connected') throw new Error('SSH 会话未连接')
    return session
  }

  private async listRemoteWithExec(session: ActiveSession, path: string): Promise<DirectoryEntry[]> {
    const encodedPath = Buffer.from(path).toString('base64')
    const output = await this.execCommand(session, `python3 -c ${shellQuote(remoteListScript)} ${shellQuote(encodedPath)}`)
    const parsed = JSON.parse(output) as unknown
    if (!Array.isArray(parsed)) throw new Error('远端目录响应格式错误')
    return parsed.map((value) => {
      if (!value || typeof value !== 'object') throw new Error('远端目录条目格式错误')
      const item = value as Partial<DirectoryEntry>
      if (typeof item.name !== 'string' || typeof item.path !== 'string' || !['file', 'directory', 'symlink', 'other'].includes(item.kind ?? '')) {
        throw new Error('远端目录条目格式错误')
      }
      return { name: item.name, path: item.path, kind: item.kind!, size: Number(item.size) || 0, modifiedAt: Number(item.modifiedAt) || 0 }
    })
  }

  private execCommand(session: ActiveSession, command: string): Promise<string> {
    return new Promise((resolve, reject) => {
      let settled = false
      let channel: ClientChannel | undefined
      const stdout: Buffer[] = []
      const stderr: Buffer[] = []
      let outputSize = 0
      const finish = (error?: Error): void => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        if (error) reject(error)
        else resolve(Buffer.concat(stdout).toString('utf8'))
      }
      const timer = setTimeout(() => {
        channel?.close()
        finish(new RemoteExecTimeoutError('读取远端目录超时'))
      }, 12_000)
      session.client.exec(command, (error, stream) => {
        if (error) { finish(new RemoteExecUnavailableError(error.message)); return }
        channel = stream
        stream.on('data', (data: Buffer) => {
          outputSize += data.length
          if (outputSize > 16 * 1024 * 1024) { stream.close(); finish(new Error('远端目录内容过大')) }
          else stdout.push(data)
        })
        stream.stderr.on('data', (data: Buffer) => stderr.push(data))
        stream.on('error', (streamError: Error) => finish(streamError))
        stream.on('close', (code: number | undefined) => {
          if (code && code !== 0) {
            const message = Buffer.concat(stderr).toString('utf8').trim() || `远端命令退出码 ${code}`
            finish(code === 127 ? new RemoteExecUnavailableError(message) : new Error(message))
          }
          else finish()
        })
      })
    })
  }

  private async listRemoteWithSftp(id: string, path: string): Promise<DirectoryEntry[]> {
    const sftp = await this.getSftp(id)
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`读取远端目录超时：${path}`)), 12_000)
      sftp.readdir(path, (error, list) => {
        clearTimeout(timer)
        if (error) { reject(error); return }
        resolve(list.filter((item) => item.filename !== '.' && item.filename !== '..').map((item) => {
          const mode = item.attrs.mode ?? 0
          const kind: DirectoryEntry['kind'] = (mode & 0o170000) === 0o040000 ? 'directory' : (mode & 0o170000) === 0o120000 ? 'symlink' : (mode & 0o170000) === 0o100000 ? 'file' : 'other'
          return { name: item.filename, path: path === '/' ? `/${item.filename}` : `${path}/${item.filename}`, kind, size: item.attrs.size ?? 0, modifiedAt: (item.attrs.mtime ?? 0) * 1000 }
        }))
      })
    })
  }

  private closeSession(session: ActiveSession, status: 'disconnected' | 'error', error?: string): void {
    if (this.sessions.get(session.summary.id) !== session) return
    session.summary = { ...session.summary, status, error }
    this.emitSession(session)
    this.releaseSession(session)
  }

  private releaseSession(session: ActiveSession): void {
    if (this.sessions.get(session.summary.id) !== session) return
    session.closing = true
    this.sessions.delete(session.summary.id)
    const pending = this.hostKeyCallbacks.get(session.summary.id)
    this.hostKeyCallbacks.delete(session.summary.id)
    pending?.callback(false)
    session.terminalOutput?.end()
    session.terminalOutput = undefined
    session.shell?.close()
    session.sftp?.end()
    session.sftp = undefined
    session.client.end()
    session.jumpClient?.end()
  }
}
