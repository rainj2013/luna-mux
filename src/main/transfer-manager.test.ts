import { Readable } from 'node:stream'
import { mkdtemp, readFile, readdir, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, it } from 'vitest'
import type { AppEvent, TransferTask } from '../shared/types'
import type { AppDatabase } from './database'
import type { SessionManager } from './session-manager'
import { TransferManager } from './transfer-manager'

const temporaryDirectories: string[] = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })))
})

function createHarness(read: (path: string) => Readable): {
  manager: TransferManager
  events: AppEvent[]
  tasks: Map<string, TransferTask>
} {
  const tasks = new Map<string, TransferTask>()
  const database = {
    saveTransfer: (task: TransferTask) => tasks.set(task.id, { ...task }),
    listTransfers: () => [...tasks.values()],
    clearCompletedTransfers: () => undefined
  } as unknown as AppDatabase
  const sftp = {
    lstat: (_path: string, callback: (error: Error | undefined, stats: { mode: number; size: number }) => void) => callback(undefined, { mode: 0o100644, size: 7 }),
    createReadStream: (path: string) => read(path)
  }
  const sessions = {
    getBookmarkId: () => 'connection',
    getSftp: async () => sftp
  } as unknown as SessionManager
  const events: AppEvent[] = []
  return { manager: new TransferManager(database, sessions, (event) => events.push(event)), events, tasks }
}

async function waitForStatus(tasks: Map<string, TransferTask>, ids: string[], status: TransferTask['status']): Promise<void> {
  const started = Date.now()
  while (!ids.every((id) => tasks.get(id)?.status === status)) {
    if (Date.now() - started > 2_000) throw new Error(`等待传输状态超时：${status}`)
    await new Promise((resolve) => setTimeout(resolve, 5))
  }
}

describe('TransferManager reliability', () => {
  it('settles conservatively when deployment task records are missing', async () => {
    const harness = createHarness(() => Readable.from(''))
    await expect(harness.manager.whenSettled(['missing'])).resolves.toBe(false)
  })

  it('serializes conflicts and applies one batch decision to queued conflicts', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'ssh-client-transfer-'))
    temporaryDirectories.push(directory)
    await writeFile(join(directory, 'a.txt'), 'old-a')
    await writeFile(join(directory, 'b.txt'), 'old-b')
    const harness = createHarness((path) => Readable.from(path.endsWith('a.txt') ? 'new-a!!' : 'new-b!!'))
    const queued = harness.manager.enqueue({ sessionId: 'session', direction: 'download', sourcePaths: ['/a.txt', '/b.txt'], destinationDirectory: directory })

    const started = Date.now()
    while (!harness.events.some((event) => event.type === 'transfer-conflict')) {
      if (Date.now() - started > 1_000) throw new Error('未收到冲突提示')
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
    await new Promise((resolve) => setTimeout(resolve, 20))
    const prompts = harness.events.filter((event) => event.type === 'transfer-conflict')
    expect(prompts).toHaveLength(1)
    const prompt = prompts[0]
    if (prompt?.type !== 'transfer-conflict') throw new Error('冲突事件格式错误')
    harness.manager.resolveConflict(prompt.payload.taskId, 'overwrite', true)

    await waitForStatus(harness.tasks, queued.map((item) => item.id), 'completed')
    expect(harness.events.filter((event) => event.type === 'transfer-conflict')).toHaveLength(1)
    expect(await readFile(join(directory, 'a.txt'), 'utf8')).toBe('new-a!!')
    expect(await readFile(join(directory, 'b.txt'), 'utf8')).toBe('new-b!!')
  })

  it('keeps the original file when an overwrite transfer fails', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'ssh-client-transfer-'))
    temporaryDirectories.push(directory)
    const destination = join(directory, 'a.txt')
    await writeFile(destination, 'original')
    const harness = createHarness(() => Readable.from((async function* () {
      yield Buffer.from('partial')
      throw new Error('network interrupted')
    })()))
    const [queued] = harness.manager.enqueue({ sessionId: 'session', direction: 'download', sourcePaths: ['/a.txt'], destinationDirectory: directory })
    if (!queued) throw new Error('未创建传输任务')

    const started = Date.now()
    let conflictId = ''
    while (!conflictId) {
      const conflict = harness.events.find((event) => event.type === 'transfer-conflict')
      if (conflict?.type === 'transfer-conflict') conflictId = conflict.payload.taskId
      if (Date.now() - started > 1_000) throw new Error('未收到冲突提示')
      if (!conflictId) await new Promise((resolve) => setTimeout(resolve, 5))
    }
    harness.manager.resolveConflict(conflictId, 'overwrite', false)
    await waitForStatus(harness.tasks, [queued.id], 'failed')

    expect(await readFile(destination, 'utf8')).toBe('original')
    expect((await readdir(directory)).filter((name) => name.endsWith('.part'))).toEqual([])
  })
})
