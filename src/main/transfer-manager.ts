import { createReadStream, createWriteStream } from 'node:fs'
import { lstat, mkdir, readdir, rename, rm } from 'node:fs/promises'
import { basename, dirname, join } from 'node:path'
import { posix } from 'node:path'
import { randomUUID } from 'node:crypto'
import { Transform, type Readable, type Writable } from 'node:stream'
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

interface ConflictWaiter {
  resolve: (value: ConflictResolution) => void
  task: InternalTask
  destinationPath: string
}

interface DestinationResolution { path: string; overwrite: boolean }

const remoteOperationTimeout = 20_000
const remoteOperationLimit = 8

export class TransferManager {
  private queue: InternalTask[] = []
  private running = new Map<string, InternalTask>()
  private conflicts = new Map<string, ConflictWaiter>()
  private conflictQueue: ConflictWaiter[] = []
  private activeConflictId?: string
  private batchResolution = new Map<string, ConflictResolution>()
  private remoteOperations = 0
  private remoteOperationWaiters: (() => void)[] = []
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
        id: randomUUID(), sessionId: request.sessionId, bookmarkId: this.sessions.getBookmarkId(request.sessionId), direction: request.direction,
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
    const conflict = this.conflicts.get(id)
    if (conflict) this.settleConflict(conflict, 'skip')
    internal.task.status = 'cancelled'; internal.task.error = undefined; this.update(internal)
  }

  retry(id: string, sessionId: string): void {
    const old = this.db.listTransfers().find((item) => item.id === id)
    if (!old || !['failed', 'interrupted', 'cancelled'].includes(old.status)) return
    if (!old.bookmarkId) throw new Error('此旧传输记录缺少连接信息，无法自动重试')
    if (this.sessions.getBookmarkId(sessionId) !== old.bookmarkId) throw new Error('请先连接此传输任务对应的连接')
    const internal: InternalTask = {
      task: { ...old, sessionId, status: 'queued', bytesTransferred: 0, speed: 0, error: undefined, updatedAt: new Date().toISOString() },
      batchId: randomUUID(), controller: new AbortController(),
      request: { sessionId, direction: old.direction, sourcePaths: [old.sourcePath], destinationDirectory: old.direction === 'upload' ? posix.dirname(old.destinationPath) : dirname(old.destinationPath) }
    }
    this.queue.push(internal); this.update(internal); this.pump()
  }

  clearCompleted(): void { this.db.clearCompletedTransfers() }

  resolveConflict(id: string, resolution: ConflictResolution, applyToBatch: boolean): void {
    const waiter = this.conflicts.get(id)
    if (!waiter || this.activeConflictId !== id) return
    if (applyToBatch) this.batchResolution.set(waiter.task.batchId, resolution)
    this.settleConflict(waiter, resolution)
  }

  private pump(): void {
    while (this.running.size < this.concurrency && this.queue.length) {
      const internal = this.queue.shift()!
      if (internal.task.status === 'cancelled') continue
      this.running.set(internal.task.id, internal)
      void this.run(internal).finally(() => {
        this.running.delete(internal.task.id)
        if (!this.queue.some((item) => item.batchId === internal.batchId) && ![...this.running.values()].some((item) => item.batchId === internal.batchId)) {
          this.batchResolution.delete(internal.batchId)
        }
        this.pump()
      })
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
    if (stats.isSymbolicLink()) throw new Error(`暂不支持传输符号链接：${path}`)
    if (stats.isFile()) return stats.size
    if (!stats.isDirectory()) return 0
    let total = 0
    for (const name of await readdir(path)) total += await this.localSize(join(path, name))
    return total
  }

  private async remoteSize(sessionId: string, path: string): Promise<number> {
    const sftp = await this.sessions.getSftp(sessionId)
    const stats = await this.remoteLstat(sftp, path)
    if (this.isSymlink(stats)) throw new Error(`暂不支持传输符号链接：${path}`)
    if (!this.isDirectory(stats)) return stats.size ?? 0
    let total = 0
    for (const entry of await this.remoteReadDir(sftp, path)) {
      if (entry.filename !== '.' && entry.filename !== '..') total += await this.remoteSize(sessionId, posix.join(path, entry.filename))
    }
    return total
  }

  private async uploadEntry(internal: InternalTask, source: string, destination: string): Promise<void> {
    this.assertActive(internal)
    const stats = await lstat(source)
    if (stats.isSymbolicLink()) throw new Error(`暂不支持传输符号链接：${source}`)
    if (stats.isDirectory()) {
      const sftp = await this.sessions.getSftp(internal.task.sessionId)
      await this.ensureRemoteDirectory(sftp, destination)
      for (const name of await readdir(source)) await this.uploadEntry(internal, join(source, name), posix.join(destination, name))
      return
    }
    if (!stats.isFile()) return
    const sftp = await this.sessions.getSftp(internal.task.sessionId)
    const resolved = await this.resolveDestination(internal, destination, true)
    if (!resolved) { this.markSkipped(internal, stats.size); return }
    await this.ensureRemoteDirectory(sftp, posix.dirname(resolved.path))
    const temporary = this.remoteTemporaryPath(resolved.path, internal.task.id)
    const read = createReadStream(source)
    const write = sftp.createWriteStream(temporary, { flags: 'wx', mode: stats.mode })
    try {
      await this.pipelineTransfer(internal, read, write)
      await this.commitRemoteFile(sftp, temporary, resolved.path, resolved.overwrite)
    } catch (error) {
      await this.remoteUnlink(sftp, temporary, true).catch(() => undefined)
      throw error
    }
  }

  private async downloadEntry(internal: InternalTask, source: string, destination: string): Promise<void> {
    this.assertActive(internal)
    const sftp = await this.sessions.getSftp(internal.task.sessionId)
    const stats = await this.remoteLstat(sftp, source)
    if (this.isSymlink(stats)) throw new Error(`暂不支持传输符号链接：${source}`)
    if (this.isDirectory(stats)) {
      await mkdir(destination, { recursive: true })
      const entries = await this.remoteReadDir(sftp, source)
      for (const entry of entries) if (entry.filename !== '.' && entry.filename !== '..') await this.downloadEntry(internal, posix.join(source, entry.filename), join(destination, entry.filename))
      return
    }
    const resolved = await this.resolveDestination(internal, destination, false)
    if (!resolved) { this.markSkipped(internal, stats.size ?? 0); return }
    await mkdir(dirname(resolved.path), { recursive: true })
    const temporary = this.localTemporaryPath(resolved.path, internal.task.id)
    const read = sftp.createReadStream(source)
    const write = createWriteStream(temporary, { flags: 'wx', mode: stats.mode })
    try {
      await this.pipelineTransfer(internal, read, write)
      await this.commitLocalFile(temporary, resolved.path, resolved.overwrite)
    } catch (error) {
      await rm(temporary, { force: true }).catch(() => undefined)
      throw error
    }
  }

  private async resolveDestination(internal: InternalTask, destination: string, remote: boolean): Promise<DestinationResolution | null> {
    if (!(await this.exists(internal.task.sessionId, destination, remote))) return { path: destination, overwrite: false }
    const resolution = this.batchResolution.get(internal.batchId) ?? await this.askConflict(internal, destination)
    if (resolution === 'skip') return null
    if (resolution === 'overwrite') return { path: destination, overwrite: true }
    let attempt = 1
    while (await this.exists(internal.task.sessionId, renamedPath(destination, attempt, remote), remote)) attempt++
    return { path: renamedPath(destination, attempt, remote), overwrite: false }
  }

  private askConflict(internal: InternalTask, destinationPath: string): Promise<ConflictResolution> {
    internal.task.status = 'conflict'; this.update(internal)
    return new Promise((resolve) => {
      const waiter: ConflictWaiter = { task: internal, destinationPath, resolve }
      this.conflicts.set(internal.task.id, waiter)
      this.conflictQueue.push(waiter)
      this.showNextConflict()
    })
  }

  private showNextConflict(): void {
    if (this.activeConflictId) return
    while (this.conflictQueue.length) {
      const waiter = this.conflictQueue.shift()!
      if (!this.conflicts.has(waiter.task.task.id)) continue
      const batchResolution = this.batchResolution.get(waiter.task.batchId)
      if (batchResolution) { this.settleConflict(waiter, batchResolution, false); continue }
      this.activeConflictId = waiter.task.task.id
      this.emit({ type: 'transfer-conflict', payload: {
        taskId: waiter.task.task.id,
        sourcePath: waiter.task.task.sourcePath,
        destinationPath: waiter.destinationPath
      } })
      return
    }
  }

  private settleConflict(waiter: ConflictWaiter, resolution: ConflictResolution, showNext = true): void {
    const id = waiter.task.task.id
    if (!this.conflicts.delete(id)) return
    if (this.activeConflictId === id) this.activeConflictId = undefined
    waiter.task.task.status = 'running'
    this.update(waiter.task)
    waiter.resolve(resolution)
    if (showNext) this.showNextConflict()
  }

  private async exists(sessionId: string, path: string, remote: boolean): Promise<boolean> {
    if (!remote) { try { await lstat(path); return true } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error } }
    const sftp = await this.sessions.getSftp(sessionId)
    try { await this.remoteLstat(sftp, path); return true } catch (error) { const code = (error as { code?: string | number }).code; if (code === 2 || code === 'ENOENT') return false; throw error }
  }

  private progressStream(internal: InternalTask, onProgress?: () => void): Transform {
    let lastAt = Date.now(), lastBytes = internal.task.bytesTransferred
    return new Transform({ transform: (chunk: Buffer, _encoding, callback) => {
      internal.task.bytesTransferred += chunk.length
      onProgress?.()
      const now = Date.now()
      if (now - lastAt >= 250) {
        internal.task.speed = ((internal.task.bytesTransferred - lastBytes) * 1000) / (now - lastAt)
        lastAt = now; lastBytes = internal.task.bytesTransferred; this.update(internal)
      }
      callback(null, chunk)
    } })
  }

  private async pipelineTransfer(internal: InternalTask, read: Readable, write: Writable): Promise<void> {
    let timer: ReturnType<typeof setTimeout>
    const resetTimeout = (): void => {
      clearTimeout(timer)
      timer = setTimeout(() => {
        const error = new Error('文件传输 60 秒没有进展，已中止')
        read.destroy(error)
        write.destroy(error)
      }, 60_000)
    }
    resetTimeout()
    try {
      await pipeline(read, this.progressStream(internal, resetTimeout), write, { signal: internal.controller.signal })
    } finally {
      clearTimeout(timer!)
    }
  }

  private markSkipped(internal: InternalTask, size: number): void {
    internal.task.bytesTransferred = Math.min(internal.task.bytesTotal, internal.task.bytesTransferred + size)
    this.update(internal)
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
    return this.remoteCall(`读取远端属性超时：${path}`, (callback) => sftp.lstat(path, callback))
  }
  private remoteReadDir(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<{ filename: string; attrs: Stats }[]> {
    return this.remoteCall(`读取远端目录超时：${path}`, (callback) => sftp.readdir(path, callback))
  }
  private async ensureRemoteDirectory(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<void> {
    if (!path || path === '/' || path === '.') return
    try { const stats = await this.remoteLstat(sftp, path); if (this.isDirectory(stats)) return; throw new Error(`${path} 不是目录`) }
    catch (error) {
      const code = (error as { code?: string | number }).code
      if (code !== 2 && code !== 'ENOENT') throw error
      await this.ensureRemoteDirectory(sftp, posix.dirname(path))
      try {
        await this.remoteCall<void>(`创建远端目录超时：${path}`, (callback) => sftp.mkdir(path, callback))
      } catch (mkdirError) {
        const stats = await this.remoteLstat(sftp, path).catch(() => undefined)
        if (!stats || !this.isDirectory(stats)) throw mkdirError
      }
    }
  }

  private localTemporaryPath(destination: string, taskId: string): string {
    return join(dirname(destination), `.${basename(destination)}.${taskId}.${randomUUID()}.part`)
  }

  private remoteTemporaryPath(destination: string, taskId: string): string {
    return posix.join(posix.dirname(destination), `.${posix.basename(destination)}.${taskId}.${randomUUID()}.part`)
  }

  private async commitLocalFile(temporary: string, destination: string, overwrite: boolean): Promise<void> {
    if (!overwrite) {
      if (await this.localExists(destination)) throw new Error(`目标文件在传输期间已出现：${destination}`)
      await rename(temporary, destination)
      return
    }
    const destinationStats = await lstat(destination).catch((error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT') return undefined
      throw error
    })
    if (destinationStats?.isDirectory()) throw new Error(`不能用文件覆盖目录：${destination}`)
    const backup = `${temporary}.backup`
    let hasBackup = false
    try {
      if (destinationStats) { await rename(destination, backup); hasBackup = true }
      await rename(temporary, destination)
    } catch (error) {
      if (hasBackup) {
        try { await rename(backup, destination) }
        catch (restoreError) { throw new Error(`提交失败且无法恢复原文件：${destination}；${String(restoreError)}`) }
      }
      throw error
    }
    if (hasBackup) await rm(backup, { force: true }).catch(() => undefined)
  }

  private async commitRemoteFile(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, temporary: string, destination: string, overwrite: boolean): Promise<void> {
    if (!overwrite) {
      if (await this.remoteExists(sftp, destination)) throw new Error(`目标文件在传输期间已出现：${destination}`)
      await this.remoteRename(sftp, temporary, destination)
      return
    }
    const destinationStats = await this.remoteLstat(sftp, destination).catch((error) => this.isMissing(error) ? undefined : Promise.reject(error))
    if (destinationStats && this.isDirectory(destinationStats)) throw new Error(`不能用文件覆盖目录：${destination}`)
    try {
      await this.remoteCall<void>(`提交远端文件超时：${destination}`, (callback) => sftp.ext_openssh_rename(temporary, destination, callback))
      return
    } catch (error) {
      if (!String(error).includes('does not support')) throw error
    }
    const backup = `${temporary}.backup`
    let hasBackup = false
    try {
      if (destinationStats) { await this.remoteRename(sftp, destination, backup); hasBackup = true }
      await this.remoteRename(sftp, temporary, destination)
    } catch (error) {
      if (hasBackup) {
        try { await this.remoteRename(sftp, backup, destination) }
        catch (restoreError) { throw new Error(`提交失败且无法恢复远端原文件：${destination}；${String(restoreError)}`) }
      }
      throw error
    }
    if (hasBackup) await this.remoteUnlink(sftp, backup).catch(() => undefined)
  }

  private localExists(path: string): Promise<boolean> {
    return lstat(path).then(() => true).catch((error: NodeJS.ErrnoException) => {
      if (error.code === 'ENOENT') return false
      throw error
    })
  }

  private async remoteExists(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string): Promise<boolean> {
    try { await this.remoteLstat(sftp, path); return true }
    catch (error) { if (this.isMissing(error)) return false; throw error }
  }

  private remoteRename(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, source: string, destination: string): Promise<void> {
    return this.remoteCall(`重命名远端文件超时：${source}`, (callback) => sftp.rename(source, destination, callback))
  }

  private async remoteUnlink(sftp: Awaited<ReturnType<SessionManager['getSftp']>>, path: string, ignoreMissing = false): Promise<void> {
    try { await this.remoteCall<void>(`清理远端临时文件超时：${path}`, (callback) => sftp.unlink(path, callback)) }
    catch (error) { if (!ignoreMissing || !this.isMissing(error)) throw error }
  }

  private isMissing(error: unknown): boolean {
    const code = (error as { code?: string | number }).code
    return code === 2 || code === 'ENOENT'
  }

  private async remoteCall<T>(message: string, invoke: (callback: (error?: Error | null, value?: T) => void) => void): Promise<T> {
    await this.acquireRemoteOperation()
    try {
      return await new Promise<T>((resolve, reject) => {
        let settled = false
        const timer = setTimeout(() => { settled = true; reject(new Error(message)) }, remoteOperationTimeout)
        const callback = (error?: Error | null, value?: T): void => {
          if (settled) return
          settled = true
          clearTimeout(timer)
          if (error) reject(error)
          else resolve(value as T)
        }
        try { invoke(callback) }
        catch (error) { clearTimeout(timer); settled = true; reject(error) }
      })
    } finally {
      this.releaseRemoteOperation()
    }
  }

  private async acquireRemoteOperation(): Promise<void> {
    if (this.remoteOperations < remoteOperationLimit) {
      this.remoteOperations++
      return
    }
    await new Promise<void>((resolve) => this.remoteOperationWaiters.push(resolve))
  }

  private releaseRemoteOperation(): void {
    const next = this.remoteOperationWaiters.shift()
    if (next) next()
    else this.remoteOperations--
  }
}
