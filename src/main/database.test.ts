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

  it('adds keepalive defaults to v2 bookmarks and persists custom values', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'ssh-client-db-'))
    temporaryDirectories.push(directory)
    const file = join(directory, 'keepalive.db')
    const legacy = new DatabaseSync(file)
    legacy.exec(`
      CREATE TABLE bookmarks (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
        username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
        jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '', createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
      );
      INSERT INTO bookmarks VALUES ('old','Old','host',22,'user','password','','','','2026-01-01','2026-01-01');
      PRAGMA user_version = 2;
    `)
    legacy.close()

    const { AppDatabase } = await import('./database')
    const database = new AppDatabase(file)
    expect(database.getBookmark('old')).toMatchObject({
      keepaliveEnabled: true, keepaliveIntervalSeconds: 15, keepaliveCountMax: 3,
      groupName: '', favorite: false, lastConnectedAt: ''
    })
    const saved = database.saveBookmark({
      id: 'old', name: 'Old', host: 'host', port: 22, username: 'user', authType: 'password',
      privateKeyPath: '', jumpBookmarkId: '', note: '', keepaliveEnabled: true,
      keepaliveIntervalSeconds: 2, keepaliveCountMax: 99, groupName: 'Production', favorite: true
    })
    expect(saved).toMatchObject({ keepaliveIntervalSeconds: 5, keepaliveCountMax: 10, groupName: 'Production', favorite: true })
    database.markBookmarkConnected('old')
    expect(database.getBookmark('old')?.lastConnectedAt).not.toBe('')
    database.saveCredential('old', 'password')
    const changedAuth = database.saveBookmark({ ...saved, authType: 'agent', privateKeyPath: '' })
    expect(changedAuth.hasSavedCredential).toBe(false)
    expect(database.getCredential('old')).toBeUndefined()
    ;(database as unknown as { db: { close(): void } }).db.close()
  })
})
