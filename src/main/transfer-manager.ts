import { createReadStream, createWriteStream } from 'node:fs'
import { lstat, mkdir, readdir } from 'node:fs/promises'
import { basename, join } from 'node:path'
import { posix } from 'node:path'
import { randomUUID } from 'node:crypto'
import { Transform } from 'node:stream'
import { pipeline } from 'node:stream/promises'
import type { Stats } from 'ssh2'
import type { AppDatabase } from './database'
import type { SessionManager } from './session-manager'
import { renamedPath } from './path-utils'
import type { AppEvent, ConflictResolution, TransferRequest, TransferTask } from '../shared/types'

interface InternalTask {
  task: TransferTask
  batchId: string
  controller: AbortController
  request: TransferRequest
}

interface ConflictWaiter { resolve: (value: ConflictResolution) => void; task: InternalTask }

export class TransferManager {
  private queue: InternalTask[] = []
  private running = new Map<string, InternalTask>()
  private conflicts = new Map<string, ConflictWaiter>()
  private batchResolution = new Map<string, ConflictResolution>()
  private readonly concurrency = 3

  constructor(private db: AppDatabase, private sessions: SessionManager, private emit: (event: AppEvent) => void) {}

  list(): TransferTask[] { return this.db.listTransfers() }

  enqueue(request: TransferRequest): TransferTask[] {
    const batchId = randomUUID()
    const now = new Date().toISOString()
    const items = request.sourcePaths.map((sourcePath) => {
      const name = request.direction === 'upload' ? basename(sourcePath) : posix.basename(sourcePath)
      const destinationPath = request.direction === 'upload' ? posix.join(request.destinationDirectory, name) : join(request.destinationDirectory, name)
      const task: TransferTask = {
        id: randomUUID(), sessionId: request.sessionId, direction: request.direction,
        sourcePath, destinationPath, displayName: name, status: 'queued', bytesTotal: 0,
        bytesTransferred: 0, speed: 0, createdAt: now, updatedAt: now
      }
      const internal = { task, batchId, controller: new AbortController(), request }
      this.queue.push(internal); this.update(internal)
      return task
    })
    this.pump()
    return items
  }

  cancel(id: string): void {
    const queuedIndex = this.queue.findIndex((item) => item.task.id === id)
    const internal = queuedIndex >= 0 ? this.queue.splice(queuedIndex, 1)[0] : this.running.get(id)
    if (!internal) return
    internal.controller.abort()
    this.conflicts.get(id)?.resolve('skip'); this.conflicts.delete(id)
    internal.task.status = 'cancelled'; internal.task.error = undefined; this.update(internal)
  }

  retry(id: string): void {
    const old = this.db.listTransfers().find((item) => item.id === id)
    if (!old || !['failed', 'interrupted', 'cancelled'].includes(old.status)) return
    const internal: InternalTask = {
      task: { ...old, status: 'queued', bytesTransferred: 0, speed: 0, error: undefined, updatedAt: new Date().toISOString() },
      batchId: randomUUID(), controller: new AbortController(),
      request: { sessionId: old.sessionId, direction: old.direction, sourcePaths: [old.sourcePath], destinationDirectory: old.direction === 'upload' ? posix.dirname(old.destinationPath) : join(old.destinationPath, '..') }
    }
    this.queue.push(internal); this.update(internal); this.pump()
  }

  clearCompleted(): void { this.db.clearCompletedTransfers() }

  resolveConflict(id: string, resolution: ConflictResolution, applyToBatch: boolean): void {
    const waiter = this.conflicts.get(id)
    if (!waiter) return
    if (applyToBatch) this.batchResolution.set(waiter.task.batchId, resolution)
    this.conflicts.delete(id); waiter.resolve(resolution)
  }

  private pump(): void {
    while (this.running.size < this.concurrency && this.queue.length) {
      const internal = this.queue.shift()!
      if (internal.task.status === 'cancelled') continue
      this.running.set(internal.task.id, internal)
      void this.run(internal).finally(() => { this.running.delete(internal.task.id); this.pump() })
    }
  }

  private async run(internal: InternalTask): Promise<void> {
    try {
      internal.task.status = 'scanning'; this.update(internal)
      internal.task.bytesTotal = internal.task.direction === 'upload'
        ? await this.localSize(internal.task.sourcePath)
        : await this.remoteSize(internal.task.sessionId, internal.task.sourcePath)
      internal.task.status = 'running'; this.update(internal)
      if (internal.task.direction === 'upload') await this.uploadEntry(internal, internal.task.sourcePath, internal.task.destinationPath)
      else await this.downloadEntry(internal, internal.task.sourcePath, internal.task.destinationPath)
      if (internal.controller.signal.aborted) throw new DOMException('已取消', 'AbortError')
      internal.task.status = 'completed'; internal.task.speed = 0; this.update(internal)
    } catch (error) {
      if (internal.controller.signal.aborted || (error instanceof DOMException && error.name === 'AbortError')) {
        internal.task.status = 'cancelled'; internal.task.error = undefined
      } else {
        internal.task.status = 'failed'; internal.task.error = error instanceof Error ? error.message : String(error)
      }
      internal.task.speed = 0; this.update(internal)
    }
  }

  private async localSize(path: string): Promise<number> {
    const stats = await lstat(path)
    if (stats.isSymbolicLink()) return 0
    if (stats.isFile()) return stats.size
    if (!stats.isDirectory()) return 0
    const children = await readdir(path)
    return (await Promise.all(children.map((name) => this.localSize(join(path, name))))).reduce((a, b) => a + b, 0)
  }

  private async remoteSize(sessionId: string, path: string): Promise<number> {
    const sftp = await this.sessions.getSftp(sessionId)
    const stats = await this.remoteLstat(sftp, path)
    if (this.isSymlink(stats)) return 0
    if (!this.isDirectory(stats)) return stats.size ?? 0
    const entries = await this.remoteReadDir(sftp, path)
    return (await Promise.all(entries.filter((entry) => entry.filename !== '.' && entry.filename !== '..').map((entry) => this.remoteSize(sessionId, posix.join(path, entry.filename))))).reduce((a, b) => a + b, 0)
  }

  private async uploadEntry(internal: InternalTask, source: string, destination: string): Promise<void> {
    this.assertActive(internal)
    const stats = await lstat(source)
    if (stats.isSymbolicLink()) return
    if (stats.isDirectory()) {
      const sftp = await this.sessions.getSftp(internal.task.sessionId)
      await this.ensureRemoteDirectory(sftp, destination)
      for (const name of await readdir(source)) await this.uploadEntry(internal, join(source, name), posix.join(destination, name))
      return
    }
    if (!stats.isFile()) return
    const sftp = await this.sessions.getSftp(internal.task.sessionId)
    const resolved = await this.resolveDestination(internal, destination, true)
    if (!resolved) return
    await this.ensureRemoteDirectory(sftp, posix.dirname(resolved))
    const read = createReadStream(source)
    const write = sftp.createWriteStream(resolved, { flags: 'w', mode: stats.mode })
    await pipeline(read, this.progressStream(internal), write, { signal: internal.controller.signal })
  }

  private async downloadEntry(internal: InternalTask, source: string, destination: string): Promise<void> {
    this.assertActive(internal)
    const sftp = await this.sessions.getSftp(internal.task.sessionId)
    const stats = await this.remoteLstat(sftp, source)
    if (this.isSymlink(stats)) return
    if (this.isDirectory(stats)) {
      await mkdir(destination, { recursive: true })
      const entries = await this.remoteReadDir(sftp, source)
      for (const entry of entries) if (entry.filename !== '.' && entry.filename !== '..') await this.downloadEntry(internal, posix.join(source, entry.filename), join(destination, entry.filename))
      return
    }
    const resolved = await this.resolveDestination(internal, destination, false)
    if (!resolved) return
    await mkdir(join(resolved, '..'), { recursive: true })
    const read = sftp.createReadStream(source)
    const write = createWriteStream(resolved, { flags: 'w', mode: stats.mode })
    await pipeline(read, this.progressStream(internal), write, { signal: internal.controller.signal })
  }

  private async resolveDestination(internal: InternalTask, destination: string, remote: boolean): Promise<string | null> {
    if (!(await this.exists(internal.task.sessionId, destination, remote))) return destination
    const resolution = this.batchResolution.get(internal.batchId) ?? await this.askConflict(internal, destination)
    if (resolution === 'skip') return null
    if (resolution === 'overwrite') return destination
    let attempt = 1
    while (await this.exists(internal.task.sessionId, renamedPath(destination, attempt, remote), remote)) attempt++
    return renamedPath(destination, attempt, remote)
  }

  private askConflict(internal: InternalTask, destinationPath: string): Promise<ConflictResolution> {
    internal.task.status = 'conflict'; this.update(internal)
    this.emit({ type: 'transfer-conflict', payload: { taskId: internal.task.id, sourcePath: internal.task.sourcePath, destinationPath } })
    return new Promise((resolve) => this.conflicts.set(internal.task.id, { task: internal, resolve: (value) => {
      internal.task.status = 'running'; this.update(internal); resolve(value)
    } }))
  }

  private async exists(sessionId: string, path: string, remote: boolean): Promise<boolean> {
    if (!remote) { try { await lstat(path); return true } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error } }
    const sftp = await this.sessions.getSftp(sessionId)
    try { await this.remoteLstat(sftp, path); return true } catch (error) { const code = (error as { code?: string | number }).code; if (code === 2 || code === 'ENOENT') return false; throw error }
  }

  private progressStream(internal: InternalTask): Transform {
    let lastAt = Date.now(), lastBytes = internal.task.bytesTransferred
    return new Transform({ transform: (chunk: Buffer, _encoding, callback) => {
      internal.task.bytesTransferred += chunk.length
      const now = Date.now()
      if (now - lastAt >= 250) {
        internal.task.speed = ((internal.task.bytesTransferred - lastBytes) * 1000) / (now - lastAt)
        lastAt = now; lastBytes = internal.task.bytesTransferred; this.update(internal)
      }
      callback(null, chunk)
    } })
  }

  private update(internal: InternalTask): void {
    internal.task.updatedAt = new Date().toISOString()
    this.db.saveTransfer(internal.task)
    this.emit({ type: 'transfer', payload: { ...internal.task } })
  }

  private assertActive(internal: InternalTask): void {
    if (internal.controller.signal.aborted) throw new DOMException('已取消', 'AbortError')
  }

  private isDirectory(stats: Stats): boolean { return ((stats.mode ?? 0) & 0o170000) === 0o040000 }
  private isSymlink(stats: Stats): boolean { return ((stats.mode ?? 0) & 0o170000) === 0o120000 }
  private remoteLstat(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<Stats> {
    return new Promise((resolve, reject) => sftp.lstat(path, (error, stats) => error ? reject(error) : resolve(stats)))
  }
  private remoteReadDir(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<{ filename: string; attrs: Stats }[]> {
    return new Promise((resolve, reject) => sftp.readdir(path, (error, list) => error ? reject(error) : resolve(list)))
  }
  private async ensureRemoteDirectory(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<void> {
    if (!path || path === '/' || path === '.') return
    try { const stats = await this.remoteLstat(sftp, path); if (this.isDirectory(stats)) return; throw new Error(`${path} 不是目录`) }
    catch (error) {
      const code = (error as { code?: string | number }).code
      if (code !== 2 && code !== 'ENOENT') throw error
      await this.ensureRemoteDirectory(sftp, posix.dirname(path))
      await new Promise<void>((resolve, reject) => sftp.mkdir(path, (mkdirError) => mkdirError ? reject(mkdirError) : resolve()))
    }
  }
}
