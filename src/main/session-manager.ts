import { Client, type ClientChannel, type ConnectConfig, type SFTPWrapper, type Stats } from 'ssh2'
import { readFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import { connect as connectTcp, createServer, type Server } from 'node:net'
import type { Duplex } from 'node:stream'
import socks, { type SocksServer } from 'socksv5'
import type { AppDatabase } from './database'
import { TerminalOutputBuffer } from './terminal-output-buffer'
import type { AppEvent, Bookmark, ConnectInput, DirectoryEntry, FilePreview, PortForwardProfile, SessionSummary, TunnelSummary } from '../shared/types'

interface ActiveTunnel {
  summary: TunnelSummary
  server?: Server | SocksServer
  remoteBinding?: { address: string; port: number }
  streams: Set<Duplex>
}

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
  tunnels: Map<string, ActiveTunnel>
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
    if (!input.newSession) {
      const active = [...this.sessions.values()].find((item) => item.bookmark.id === bookmark.id && !item.closing && ['connecting', 'connected'].includes(item.summary.status))
      if (active) return active.summary
    }
    const jumpBookmark = bookmark.jumpBookmarkId ? this.db.getBookmark(bookmark.jumpBookmarkId) : undefined
    if (bookmark.jumpBookmarkId && !jumpBookmark) throw new Error('配置的跳板机连接不存在')
    if (jumpBookmark?.jumpBookmarkId) throw new Error('当前仅支持单级跳板机')

    const sessionId = randomUUID()
    const summary: SessionSummary = { id: sessionId, bookmarkId: bookmark.id, title: bookmark.name, status: 'connecting' }
    const client = new Client()
    const session: ActiveSession = { summary, bookmark, client, tunnels: new Map(), closing: false }
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
      jumpClient.on('error', (error) => this.failSession(session, error.message === 'Keepalive timeout'
        ? '跳板机保活检测超时，连接已断开'
        : `跳板机连接失败：${error.message}`))
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
      readyTimeout: 20_000,
      keepaliveInterval: bookmark.keepaliveEnabled ? bookmark.keepaliveIntervalSeconds * 1000 : 0,
      keepaliveCountMax: bookmark.keepaliveCountMax,
      hostHash: 'sha256', hostVerifier
    }
    if (bookmark.authType === 'password') {
      if (!credential) throw new Error(`${label}需要输入密码`)
      config.password = credential
      config.tryKeyboard = true
    } else if (bookmark.authType === 'privateKey') {
      if (!bookmark.privateKeyPath) throw new Error(`${label}未配置私钥文件`)
      config.privateKey = await readFile(bookmark.privateKeyPath)
      if (credential) config.passphrase = credential
    } else {
      const agent = process.env['SSH_AUTH_SOCK'] || (process.platform === 'win32' ? '\\\\.\\pipe\\openssh-ssh-agent' : '')
      if (!agent) throw new Error(`${label}未检测到 SSH Agent，请确认 SSH_AUTH_SOCK 已设置`)
      config.agent = agent
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
    session.client.on('error', (error) => this.failSession(session, error.message === 'Keepalive timeout'
      ? `${viaJump ? '目标机器' : 'SSH'}保活检测超时，连接已断开`
      : `${viaJump ? '目标机器连接失败：' : ''}${error.message}`))
    session.client.on('close', () => {
      if (session.closing || this.sessions.get(session.summary.id) !== session) return
      this.closeSession(session, 'disconnected')
    })
    session.client.on('tcp connection', (details, accept, reject) => {
      const tunnel = [...session.tunnels.values()].find((item) => item.summary.type === 'remote'
        && item.remoteBinding?.port === details.destPort
        && (item.remoteBinding.address === details.destIP || item.remoteBinding.address === '0.0.0.0' || item.remoteBinding.address === '::'))
      if (!tunnel || tunnel.summary.status !== 'running') { reject(); return }
      const channel = accept()
      const socket = connectTcp(tunnel.summary.targetPort, tunnel.summary.targetHost)
      this.trackTunnelStreams(tunnel, channel, socket)
      channel.pipe(socket).pipe(channel)
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
      shell.on('error', (shellError: Error) => this.failSession(session, shellError.message))
      shell.on('close', () => {
        output.end()
        if (session.terminalOutput === output) session.terminalOutput = undefined
      })
      this.db.markBookmarkConnected(session.bookmark.id)
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

  listTunnels(sessionId: string): TunnelSummary[] {
    const session = this.sessions.get(sessionId)
    return session ? [...session.tunnels.values()].map((item) => item.summary) : []
  }

  async startTunnel(sessionId: string, profile: PortForwardProfile): Promise<TunnelSummary> {
    const session = this.requireConnectedSession(sessionId)
    if (profile.bookmarkId !== session.bookmark.id) throw new Error('端口转发配置与当前连接不匹配')
    const existing = [...session.tunnels.values()].find((item) => item.summary.profileId === profile.id)
    if (existing) return existing.summary
    const summary: TunnelSummary = {
      id: randomUUID(), profileId: profile.id, sessionId, name: profile.name, type: profile.type,
      bindAddress: profile.bindAddress, bindPort: profile.bindPort,
      targetHost: profile.targetHost, targetPort: profile.targetPort, status: 'starting'
    }
    const tunnel: ActiveTunnel = { summary, streams: new Set() }
    session.tunnels.set(summary.id, tunnel)
    this.emitTunnel(tunnel)
    try {
      if (profile.type === 'remote') await this.startRemoteTunnel(session, tunnel)
      else await this.startLocalTunnel(session, tunnel, profile.type === 'dynamic')
      tunnel.summary = { ...tunnel.summary, status: 'running', error: undefined }
      this.emitTunnel(tunnel)
      return tunnel.summary
    } catch (error) {
      tunnel.summary = { ...tunnel.summary, status: 'error', error: error instanceof Error ? error.message : String(error) }
      this.emitTunnel(tunnel)
      session.tunnels.delete(summary.id)
      tunnel.server?.close()
      throw error
    }
  }

  async stopTunnel(sessionId: string, tunnelId: string): Promise<void> {
    const session = this.sessions.get(sessionId)
    const tunnel = session?.tunnels.get(tunnelId)
    if (!session || !tunnel) return
    if (tunnel.remoteBinding) {
      await new Promise<void>((resolve, reject) => session.client.unforwardIn(tunnel.remoteBinding!.address, tunnel.remoteBinding!.port, (error) => error ? reject(error) : resolve()))
    }
    session.tunnels.delete(tunnelId)
    tunnel.server?.close()
    for (const stream of tunnel.streams) stream.destroy()
    tunnel.streams.clear()
    this.emit({ type: 'tunnel', payload: { ...tunnel.summary, removed: true } })
  }

  private async startLocalTunnel(session: ActiveSession, tunnel: ActiveTunnel, dynamic: boolean): Promise<void> {
    const onRuntimeError = (error: Error): void => {
      if (!session.tunnels.has(tunnel.summary.id)) return
      tunnel.summary = { ...tunnel.summary, status: 'error', error: error.message }
      this.emitTunnel(tunnel)
    }
    if (dynamic) {
      const server = socks.createServer((info, accept, deny) => {
        session.client.forwardOut(info.srcAddr, info.srcPort, info.dstAddr, info.dstPort, (error, stream) => {
          if (error) { deny(); return }
          const socket = accept(true)
          if (!socket) { stream.close(); return }
          this.trackTunnelStreams(tunnel, stream, socket)
          stream.pipe(socket).pipe(stream)
        })
      }).useAuth(socks.auth.None())
      tunnel.server = server
      await this.listen(server, tunnel.summary.bindPort, tunnel.summary.bindAddress)
      const address = server.address()
      if (address && typeof address !== 'string') tunnel.summary = { ...tunnel.summary, bindPort: address.port }
      server.on('error', onRuntimeError)
      return
    }
    const server = createServer((socket) => {
      session.client.forwardOut(socket.remoteAddress ?? '127.0.0.1', socket.remotePort ?? 0, tunnel.summary.targetHost, tunnel.summary.targetPort, (error, stream) => {
        if (error) { socket.destroy(error); return }
        this.trackTunnelStreams(tunnel, stream, socket)
        socket.pipe(stream).pipe(socket)
      })
    })
    tunnel.server = server
    await this.listen(server, tunnel.summary.bindPort, tunnel.summary.bindAddress)
    const address = server.address()
    if (address && typeof address !== 'string') tunnel.summary = { ...tunnel.summary, bindPort: address.port }
    server.on('error', onRuntimeError)
  }

  private async startRemoteTunnel(session: ActiveSession, tunnel: ActiveTunnel): Promise<void> {
    const assignedPort = await new Promise<number>((resolve, reject) => {
      session.client.forwardIn(tunnel.summary.bindAddress, tunnel.summary.bindPort, (error, port) => error ? reject(error) : resolve(port))
    })
    tunnel.remoteBinding = { address: tunnel.summary.bindAddress, port: assignedPort }
    tunnel.summary = { ...tunnel.summary, bindPort: assignedPort }
  }

  private listen(server: Server | SocksServer, port: number, address: string): Promise<void> {
    return new Promise((resolve, reject) => {
      const onError = (error: Error): void => { server.removeListener('error', onError); reject(error) }
      server.once('error', onError)
      server.listen(port, address, () => { server.removeListener('error', onError); resolve() })
    })
  }

  private emitTunnel(tunnel: ActiveTunnel): void { this.emit({ type: 'tunnel', payload: tunnel.summary }) }

  private trackTunnelStreams(tunnel: ActiveTunnel, first: Duplex, second: Duplex): void {
    tunnel.streams.add(first); tunnel.streams.add(second)
    const close = (): void => { first.destroy(); second.destroy(); tunnel.streams.delete(first); tunnel.streams.delete(second) }
    first.on('error', close); second.on('error', close)
    first.on('close', close); second.on('close', close)
  }

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
      const invalidate = (): void => { if (session.sftp === sftp) session.sftp = undefined }
      sftp.once('close', invalidate)
      sftp.on('error', invalidate)
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

  async createRemoteDirectory(id: string, path: string): Promise<void> {
    const sftp = await this.getSftp(id)
    await this.sftpCall<void>(`创建远端目录超时：${path}`, (done) => sftp.mkdir(path, (error) => done(error)))
  }

  async renameRemote(id: string, from: string, to: string): Promise<void> {
    const sftp = await this.getSftp(id)
    await this.sftpCall<void>(`重命名远端文件超时：${from}`, (done) => sftp.rename(from, to, (error) => done(error)))
  }

  async removeRemote(id: string, paths: string[]): Promise<void> {
    const sftp = await this.getSftp(id)
    for (const path of paths) await this.removeRemoteEntry(sftp, path)
  }

  async previewRemote(id: string, path: string, position: 'start' | 'end'): Promise<FilePreview> {
    const sftp = await this.getSftp(id)
    const info = await this.sftpCall<Stats>(`读取远端文件信息超时：${path}`, (done) => sftp.lstat(path, done))
    if ((info.mode & 0o170000) !== 0o100000) throw new Error('只能预览普通文件')
    const limit = 1024 * 1024
    const length = Math.min(limit, info.size)
    const start = position === 'end' ? Math.max(0, info.size - length) : 0
    const stream = sftp.createReadStream(path, { start, end: Math.max(start, start + length - 1) })
    const chunks: Buffer[] = []
    let size = 0
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => { stream.destroy(); reject(new Error(`预览远端文件超时：${path}`)) }, 20_000)
      stream.on('data', (chunk: Buffer) => { size += chunk.length; if (size <= limit) chunks.push(chunk) })
      stream.on('error', (error: Error) => { clearTimeout(timer); reject(error) })
      stream.on('close', () => { clearTimeout(timer); resolve() })
    })
    const buffer = Buffer.concat(chunks)
    const sample = buffer.subarray(0, Math.min(buffer.length, 8192))
    let controls = 0
    for (const byte of sample) if (byte === 0 || (byte < 9 || (byte > 13 && byte < 32))) controls++
    const binary = sample.length > 0 && controls / sample.length > 0.01
    return { content: binary ? '' : buffer.toString('utf8'), size: info.size, truncated: info.size > limit, position, binary }
  }

  async listRemoteTree(id: string, root: string): Promise<{ relativePath: string; path: string; size: number; modifiedAt: number }[]> {
    const sftp = await this.getSftp(id)
    const files: { relativePath: string; path: string; size: number; modifiedAt: number }[] = []
    const walk = async (directory: string, relativeDirectory: string): Promise<void> => {
      const entries = await this.sftpCall<Parameters<Parameters<SFTPWrapper['readdir']>[1]>[1]>(`读取远端目录超时：${directory}`, (done) => sftp.readdir(directory, done))
      for (const entry of entries ?? []) {
        if (entry.filename === '.' || entry.filename === '..') continue
        const mode = entry.attrs.mode ?? 0
        const path = directory === '/' ? `/${entry.filename}` : `${directory}/${entry.filename}`
        const relativePath = relativeDirectory ? `${relativeDirectory}/${entry.filename}` : entry.filename
        if ((mode & 0o170000) === 0o040000) await walk(path, relativePath)
        else if ((mode & 0o170000) === 0o100000) files.push({ relativePath, path, size: entry.attrs.size ?? 0, modifiedAt: (entry.attrs.mtime ?? 0) * 1000 })
      }
    }
    await walk(root, '')
    return files
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
      return { name: item.name, path: item.path, kind: item.kind! }
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

  private sftpCall<T>(timeoutMessage: string, invoke: (done: (error?: Error | null, value?: T) => void) => void): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      let settled = false
      const timer = setTimeout(() => { settled = true; reject(new Error(timeoutMessage)) }, 20_000)
      const done = (error?: Error | null, value?: T): void => {
        if (settled) return
        settled = true
        clearTimeout(timer)
        if (error) reject(error)
        else resolve(value as T)
      }
      try { invoke(done) }
      catch (error) { done(error instanceof Error ? error : new Error(String(error))) }
    })
  }

  private async removeRemoteEntry(sftp: SFTPWrapper, path: string): Promise<void> {
    const info = await this.sftpCall<Stats>(`读取远端文件信息超时：${path}`, (done) => sftp.lstat(path, done))
    const kind = info.mode & 0o170000
    if (kind !== 0o040000) {
      await this.sftpCall<void>(`删除远端文件超时：${path}`, (done) => sftp.unlink(path, (error) => done(error)))
      return
    }
    const children = await this.sftpCall<{ filename: string }[]>(`读取远端目录超时：${path}`, (done) => sftp.readdir(path, done as Parameters<SFTPWrapper['readdir']>[1]))
    for (const child of children) {
      if (child.filename === '.' || child.filename === '..') continue
      await this.removeRemoteEntry(sftp, path === '/' ? `/${child.filename}` : `${path}/${child.filename}`)
    }
    await this.sftpCall<void>(`删除远端目录超时：${path}`, (done) => sftp.rmdir(path, (error) => done(error)))
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
    for (const tunnel of session.tunnels.values()) {
      tunnel.server?.close()
      for (const stream of tunnel.streams) stream.destroy()
    }
    session.tunnels.clear()
    session.shell?.close()
    session.sftp?.end()
    session.sftp = undefined
    session.client.end()
    session.jumpClient?.end()
  }
}
