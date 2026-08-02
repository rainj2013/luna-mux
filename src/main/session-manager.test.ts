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

class FakeSftp extends EventEmitter {
  end = vi.fn()
}

class FakeClient extends EventEmitter {
  static instances: FakeClient[] = []
  config?: Record<string, unknown>
  shellChannel = new FakeShell()
  tunnel = new EventEmitter()
  forwardArguments?: [string, number, string, number]
  ended = false
  sftpCalls = 0
  sftpChannel = new FakeSftp()
  forwardInCalls: [string, number][] = []
  unforwardInCalls: [string, number][] = []

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

  forwardIn(address: string, port: number, callback: (error: Error | undefined, assignedPort: number) => void): this {
    this.forwardInCalls.push([address, port])
    callback(undefined, port || 43001)
    return this
  }

  unforwardIn(address: string, port: number, callback: (error?: Error) => void): this {
    this.unforwardInCalls.push([address, port])
    callback()
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
  groupName: '', favorite: false, lastConnectedAt: '',
  keepaliveEnabled: true, keepaliveIntervalSeconds: 20, keepaliveCountMax: 4,
  createdAt: '2026-07-24T00:00:00.000Z', updatedAt: '2026-07-24T00:00:00.000Z'
}

const targetConnection: Bookmark = {
  id: 'target', name: '目标机器', host: '10.0.0.8', port: 2222, username: 'target-user',
  authType: 'password', privateKeyPath: '', jumpBookmarkId: jumpConnection.id, note: '', hasSavedCredential: false,
  groupName: 'Production', favorite: true, lastConnectedAt: '',
  keepaliveEnabled: true, keepaliveIntervalSeconds: 10, keepaliveCountMax: 2,
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
      markBookmarkConnected: vi.fn(),
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
    expect(jumpClient.config).toMatchObject({ host: jumpConnection.host, username: jumpConnection.username, password: 'jump-password', keepaliveInterval: 20_000, keepaliveCountMax: 4 })
    expect(jumpClient.forwardArguments).toEqual(['127.0.0.1', 0, targetConnection.host, targetConnection.port])
    expect(targetClient.config).toMatchObject({ host: targetConnection.host, username: targetConnection.username, password: 'target-password', sock: jumpClient.tunnel, keepaliveInterval: 10_000, keepaliveCountMax: 2 })
    expect(saveCredential).toHaveBeenCalledWith(jumpConnection.id, 'jump-password')
    expect(saveCredential).toHaveBeenCalledWith(targetConnection.id, 'target-password')
    expect(events).toContainEqual(expect.objectContaining({ type: 'session', payload: expect.objectContaining({ id: summary.id, status: 'connected' }) }))

    manager.write(summary.id, 'whoami\n')
    expect(targetClient.shellChannel.write).toHaveBeenCalledWith('whoami\n')
    manager.flow(summary.id, true)
    manager.flow(summary.id, false)
    expect(targetClient.shellChannel.pause).toHaveBeenCalledOnce()
    expect(targetClient.shellChannel.resume).toHaveBeenCalledOnce()

    manager.disconnect(summary.id)
    expect(() => targetClient.shellChannel.emit('error', new Error('socket closed during shutdown'))).not.toThrow()

    const replacement = await manager.connect({
      bookmarkId: targetConnection.id, jumpCredential: 'jump-password', credential: 'target-password'
    })
    await new Promise((resolve) => setTimeout(resolve, 0))

    const [firstSftp, secondSftp] = await Promise.all([manager.getSftp(replacement.id), manager.getSftp(replacement.id)])
    expect(firstSftp).toBe(secondSftp)
    expect(FakeClient.instances[2]!.sftpCalls).toBe(1)
    FakeClient.instances[2]!.sftpChannel.emit('close')
    await manager.getSftp(replacement.id)
    expect(FakeClient.instances[2]!.sftpCalls).toBe(2)

    const duplicate = await manager.connect({ bookmarkId: targetConnection.id, jumpCredential: 'jump-password', credential: 'target-password' })
    expect(duplicate.id).toBe(replacement.id)
    expect(FakeClient.instances).toHaveLength(4)

    const separate = await manager.connect({ bookmarkId: targetConnection.id, jumpCredential: 'jump-password', credential: 'target-password', newSession: true })
    await new Promise((resolve) => setTimeout(resolve, 0))
    expect(separate.id).not.toBe(replacement.id)
    expect(FakeClient.instances).toHaveLength(6)
    expect(manager.list().map((item) => item.id)).toEqual([replacement.id, separate.id])

    await expect(manager.getRemoteHome(replacement.id)).resolves.toBe('/home/target-user')
    await expect(manager.listRemote(replacement.id, '/home/target-user')).resolves.toEqual([
      expect.objectContaining({ name: 'logs', kind: 'directory' }),
      expect.objectContaining({ name: 'app.conf', kind: 'file' })
    ])

    const remoteTunnel = await manager.startTunnel(replacement.id, {
      id: 'forward', bookmarkId: targetConnection.id, name: 'Remote web', type: 'remote',
      bindAddress: '127.0.0.1', bindPort: 0, targetHost: '127.0.0.1', targetPort: 8080
    })
    expect(remoteTunnel).toMatchObject({ status: 'running', bindPort: 43001 })
    expect(manager.listTunnels(replacement.id)).toHaveLength(1)
    await manager.stopTunnel(replacement.id, remoteTunnel.id)
    expect(FakeClient.instances[2]!.unforwardInCalls).toEqual([['127.0.0.1', 43001]])

    manager.disconnect(replacement.id)
    expect(manager.list().map((item) => item.id)).toEqual([separate.id])
    expect(FakeClient.instances[2]!.sftpChannel.end).toHaveBeenCalledOnce()
    manager.disconnect(separate.id)
    expect(manager.list()).toEqual([])
  })

  it('does not let an earlier host-key timer reject a later prompt for the same session', async () => {
    vi.useFakeTimers()
    const database = {
      getBookmark: (id: string) => id === targetConnection.id ? targetConnection : jumpConnection,
      getCredential: () => undefined,
      getKnownHost: () => 'different',
      saveCredential: vi.fn(),
      markBookmarkConnected: vi.fn(),
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

  it('uses the configured SSH Agent without requesting a credential', async () => {
    const previousAgent = process.env['SSH_AUTH_SOCK']
    process.env['SSH_AUTH_SOCK'] = '/tmp/test-agent.sock'
    const agentConnection: Bookmark = { ...jumpConnection, id: 'agent', name: 'Agent host', authType: 'agent' }
    const markBookmarkConnected = vi.fn()
    const database = {
      getBookmark: (id: string) => id === agentConnection.id ? agentConnection : undefined,
      getCredential: vi.fn(), getKnownHost: () => 'known', saveCredential: vi.fn(),
      markBookmarkConnected, trustHost: vi.fn()
    } as unknown as AppDatabase
    const { SessionManager } = await import('./session-manager')
    const manager = new SessionManager(database, () => undefined)
    try {
      const summary = await manager.connect({ bookmarkId: agentConnection.id })
      await new Promise((resolve) => setTimeout(resolve, 0))
      expect(FakeClient.instances[0]!.config).toMatchObject({ agent: '/tmp/test-agent.sock' })
      expect(FakeClient.instances[0]!.config).not.toHaveProperty('password')
      expect(markBookmarkConnected).toHaveBeenCalledWith(agentConnection.id)
      manager.disconnect(summary.id)
    } finally {
      if (previousAgent === undefined) delete process.env['SSH_AUTH_SOCK']
      else process.env['SSH_AUTH_SOCK'] = previousAgent
    }
  })
})
