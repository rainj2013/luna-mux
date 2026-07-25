import { EventEmitter } from 'node:events'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import type { Bookmark } from '../shared/types'
import type { AppDatabase } from './database'

class FakeShell extends EventEmitter {
  stderr = new EventEmitter()
  write = vi.fn()
  setWindow = vi.fn()
  pause = vi.fn()
  resume = vi.fn()
  close = vi.fn()
}

class FakeExecChannel extends EventEmitter {
  stderr = new EventEmitter()
  close = vi.fn()
}

class FakeClient extends EventEmitter {
  static instances: FakeClient[] = []
  config?: Record<string, unknown>
  shellChannel = new FakeShell()
  tunnel = new EventEmitter()
  forwardArguments?: [string, number, string, number]
  ended = false
  sftpCalls = 0
  sftpChannel = { end: vi.fn() }

  constructor() {
    super()
    FakeClient.instances.push(this)
  }

  connect(config: Record<string, unknown>): this {
    this.config = config
    queueMicrotask(() => this.emit('ready'))
    return this
  }

  forwardOut(sourceHost: string, sourcePort: number, targetHost: string, targetPort: number, callback: (error: Error | undefined, stream: EventEmitter) => void): this {
    this.forwardArguments = [sourceHost, sourcePort, targetHost, targetPort]
    callback(undefined, this.tunnel)
    return this
  }

  shell(_options: unknown, callback: (error: Error | undefined, shell: FakeShell) => void): this {
    callback(undefined, this.shellChannel)
    return this
  }

  exec(command: string, callback: (error: Error | undefined, stream: FakeExecChannel) => void): this {
    const stream = new FakeExecChannel()
    callback(undefined, stream)
    queueMicrotask(() => {
      const output = command.startsWith('printf') ? '/home/target-user' : JSON.stringify([
        { name: 'logs', path: '/home/target-user/logs', kind: 'directory' },
        { name: 'app.conf', path: '/home/target-user/app.conf', kind: 'file' }
      ])
      stream.emit('data', Buffer.from(output))
      stream.emit('close', 0)
    })
    return this
  }

  sftp(callback: (error: Error | undefined, sftp: typeof this.sftpChannel) => void): this {
    this.sftpCalls++
    queueMicrotask(() => callback(undefined, this.sftpChannel))
    return this
  }

  end(): this { this.ended = true; return this }
}

const jumpConnection: Bookmark = {
  id: 'jump', name: '跳板机', host: 'jump.example.test', port: 22, username: 'jump-user',
  authType: 'password', privateKeyPath: '', jumpBookmarkId: '', note: '', hasSavedCredential: false,
  createdAt: '2026-07-24T00:00:00.000Z', updatedAt: '2026-07-24T00:00:00.000Z'
}

const targetConnection: Bookmark = {
  id: 'target', name: '目标机器', host: '10.0.0.8', port: 2222, username: 'target-user',
  authType: 'password', privateKeyPath: '', jumpBookmarkId: jumpConnection.id, note: '', hasSavedCredential: false,
  createdAt: '2026-07-24T00:00:00.000Z', updatedAt: '2026-07-24T00:00:00.000Z'
}

describe('SessionManager jump connection', () => {
  beforeEach(() => {
    FakeClient.instances = []
    vi.clearAllMocks()
    vi.resetModules()
    vi.doMock('ssh2', () => ({ Client: FakeClient }))
  })

  it('authenticates both hops and opens the target shell through forwardOut', async () => {
    const saveCredential = vi.fn()
    const database = {
      getBookmark: (id: string) => id === jumpConnection.id ? jumpConnection : id === targetConnection.id ? targetConnection : undefined,
      getCredential: () => undefined,
      getKnownHost: () => 'known',
      saveCredential,
      trustHost: vi.fn()
    } as unknown as AppDatabase
    const events: unknown[] = []
    const { SessionManager } = await import('./session-manager')
    const manager = new SessionManager(database, (event) => events.push(event))

    const summary = await manager.connect({
      bookmarkId: targetConnection.id,
      jumpCredential: 'jump-password', rememberJumpCredential: true,
      credential: 'target-password', rememberCredential: true
    })
    await new Promise((resolve) => setTimeout(resolve, 0))

    const targetClient = FakeClient.instances[0]!
    const jumpClient = FakeClient.instances[1]!
    expect(jumpClient.config).toMatchObject({ host: jumpConnection.host, username: jumpConnection.username, password: 'jump-password' })
    expect(jumpClient.forwardArguments).toEqual(['127.0.0.1', 0, targetConnection.host, targetConnection.port])
    expect(targetClient.config).toMatchObject({ host: targetConnection.host, username: targetConnection.username, password: 'target-password', sock: jumpClient.tunnel })
    expect(saveCredential).toHaveBeenCalledWith(jumpConnection.id, 'jump-password')
    expect(saveCredential).toHaveBeenCalledWith(targetConnection.id, 'target-password')
    expect(events).toContainEqual(expect.objectContaining({ type: 'session', payload: expect.objectContaining({ id: summary.id, status: 'connected' }) }))

    manager.write(summary.id, 'whoami\n')
    expect(targetClient.shellChannel.write).toHaveBeenCalledWith('whoami\n')
    manager.flow(summary.id, true)
    manager.flow(summary.id, false)
    expect(targetClient.shellChannel.pause).toHaveBeenCalledOnce()
    expect(targetClient.shellChannel.resume).toHaveBeenCalledOnce()

    const [firstSftp, secondSftp] = await Promise.all([manager.getSftp(summary.id), manager.getSftp(summary.id)])
    expect(firstSftp).toBe(secondSftp)
    expect(targetClient.sftpCalls).toBe(1)

    const duplicate = await manager.connect({ bookmarkId: targetConnection.id, jumpCredential: 'jump-password', credential: 'target-password' })
    expect(duplicate.id).toBe(summary.id)
    expect(FakeClient.instances).toHaveLength(2)

    await expect(manager.getRemoteHome(summary.id)).resolves.toBe('/home/target-user')
    await expect(manager.listRemote(summary.id, '/home/target-user')).resolves.toEqual([
      expect.objectContaining({ name: 'logs', kind: 'directory', size: 0, modifiedAt: 0 }),
      expect.objectContaining({ name: 'app.conf', kind: 'file', size: 0, modifiedAt: 0 })
    ])

    manager.disconnect(summary.id)
    expect(manager.list()).toEqual([])
    expect(targetClient.sftpChannel.end).toHaveBeenCalledOnce()
  })

  it('does not let an earlier host-key timer reject a later prompt for the same session', async () => {
    vi.useFakeTimers()
    const database = {
      getBookmark: (id: string) => id === targetConnection.id ? targetConnection : jumpConnection,
      getCredential: () => undefined,
      getKnownHost: () => 'different',
      saveCredential: vi.fn(),
      trustHost: vi.fn()
    } as unknown as AppDatabase
    const { SessionManager } = await import('./session-manager')
    const manager = new SessionManager(database, () => undefined)
    const sessionMap = (manager as unknown as { sessions: Map<string, unknown> }).sessions
    sessionMap.set('session', {})
    const verify = (manager as unknown as {
      verifyHost(id: string, bookmark: Bookmark, fingerprint: string, callback: (accept: boolean) => void): void
    }).verifyHost.bind(manager)
    const first = vi.fn()
    const second = vi.fn()

    verify('session', jumpConnection, 'jump-fingerprint', first)
    vi.advanceTimersByTime(50_000)
    manager.hostKeyDecision('session', true)
    verify('session', targetConnection, 'target-fingerprint', second)
    vi.advanceTimersByTime(10_000)

    expect(first).toHaveBeenCalledWith(true)
    expect(second).not.toHaveBeenCalled()
    manager.hostKeyDecision('session', false)
    expect(second).toHaveBeenCalledWith(false)
    vi.useRealTimers()
  })
})
