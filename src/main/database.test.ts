import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { DatabaseSync } from 'node:sqlite'
import { afterEach, describe, expect, it, vi } from 'vitest'
import type { TransferTask } from '../shared/types'

vi.mock('electron', () => ({
  app: { getPath: () => tmpdir() },
  safeStorage: {
    isEncryptionAvailable: () => true,
    encryptString: (value: string) => Buffer.from(value),
    decryptString: (value: Buffer) => value.toString()
  }
}))

const temporaryDirectories: string[] = []

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })))
})

describe('AppDatabase migrations', () => {
  it('adds bookmarkId to v1 transfer records and persists it for new tasks', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'ssh-client-db-'))
    temporaryDirectories.push(directory)
    const file = join(directory, 'test.db')
    const legacy = new DatabaseSync(file)
    legacy.exec(`
      CREATE TABLE transfers (
        id TEXT PRIMARY KEY, sessionId TEXT NOT NULL, direction TEXT NOT NULL,
        sourcePath TEXT NOT NULL, destinationPath TEXT NOT NULL, displayName TEXT NOT NULL,
        status TEXT NOT NULL, bytesTotal INTEGER NOT NULL, bytesTransferred INTEGER NOT NULL,
        speed REAL NOT NULL, error TEXT, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
      );
      INSERT INTO transfers VALUES ('old','stale','upload','/a','/b','a','failed',1,0,0,'failed','2026-01-01','2026-01-01');
      PRAGMA user_version = 1;
    `)
    legacy.close()

    const { AppDatabase } = await import('./database')
    const database = new AppDatabase(file)
    expect(database.listTransfers().find((item) => item.id === 'old')?.bookmarkId).toBe('')

    const task: TransferTask = {
      id: 'new', sessionId: 'session', bookmarkId: 'connection', direction: 'download',
      sourcePath: '/remote/a', destinationPath: '/local/a', displayName: 'a', status: 'failed',
      bytesTotal: 1, bytesTransferred: 0, speed: 0,
      createdAt: '2026-01-02', updatedAt: '2026-01-02'
    }
    database.saveTransfer(task)
    expect(database.listTransfers().find((item) => item.id === 'new')).toMatchObject({ sessionId: 'session', bookmarkId: 'connection' })
    ;(database as unknown as { db: { close(): void } }).db.close()
  })
})
