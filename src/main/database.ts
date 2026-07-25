import { app, safeStorage } from 'electron'
import { DatabaseSync } from 'node:sqlite'
import { join } from 'node:path'
import { randomUUID } from 'node:crypto'
import type { Bookmark, BookmarkInput, TransferTask } from '../shared/types'

type BookmarkRow = Omit<Bookmark, 'hasSavedCredential'> & { hasSavedCredential: number }

export class AppDatabase {
  private db: DatabaseSync

  constructor(file = join(app.getPath('userData'), 'ssh-client.db')) {
    this.db = new DatabaseSync(file)
    this.db.exec(`
      PRAGMA journal_mode = WAL;
      CREATE TABLE IF NOT EXISTS bookmarks (
        id TEXT PRIMARY KEY, name TEXT NOT NULL, host TEXT NOT NULL, port INTEGER NOT NULL,
        username TEXT NOT NULL, authType TEXT NOT NULL, privateKeyPath TEXT NOT NULL DEFAULT '',
        jumpBookmarkId TEXT NOT NULL DEFAULT '', note TEXT NOT NULL DEFAULT '',
        createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS credentials (bookmarkId TEXT PRIMARY KEY, encrypted TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS known_hosts (
        host TEXT NOT NULL, port INTEGER NOT NULL, fingerprint TEXT NOT NULL,
        PRIMARY KEY(host, port)
      );
      CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS transfers (
        id TEXT PRIMARY KEY, sessionId TEXT NOT NULL, bookmarkId TEXT NOT NULL DEFAULT '', direction TEXT NOT NULL,
        sourcePath TEXT NOT NULL, destinationPath TEXT NOT NULL, displayName TEXT NOT NULL,
        status TEXT NOT NULL, bytesTotal INTEGER NOT NULL, bytesTransferred INTEGER NOT NULL,
        speed REAL NOT NULL, error TEXT, createdAt TEXT NOT NULL, updatedAt TEXT NOT NULL
      );
    `)
    this.migrate()
    this.db.prepare("UPDATE transfers SET status = 'interrupted', error = '应用上次退出时传输未完成', updatedAt = ? WHERE status IN ('queued','scanning','running','conflict')").run(new Date().toISOString())
  }

  private migrate(): void {
    let version = (this.db.prepare('PRAGMA user_version').get() as { user_version: number }).user_version
    if (version < 1) {
      const columns = this.db.prepare('PRAGMA table_info(bookmarks)').all() as { name: string }[]
      this.db.exec('BEGIN')
      try {
        if (!columns.some((column) => column.name === 'jumpBookmarkId')) {
          this.db.exec("ALTER TABLE bookmarks ADD COLUMN jumpBookmarkId TEXT NOT NULL DEFAULT ''")
        }
        this.db.exec('PRAGMA user_version = 1; COMMIT')
        version = 1
      } catch (error) {
        this.db.exec('ROLLBACK')
        throw error
      }
    }
    if (version < 2) {
      const columns = this.db.prepare('PRAGMA table_info(transfers)').all() as { name: string }[]
      this.db.exec('BEGIN')
      try {
        if (!columns.some((column) => column.name === 'bookmarkId')) {
          this.db.exec("ALTER TABLE transfers ADD COLUMN bookmarkId TEXT NOT NULL DEFAULT ''")
        }
        this.db.exec('PRAGMA user_version = 2; COMMIT')
      } catch (error) {
        this.db.exec('ROLLBACK')
        throw error
      }
    }
  }

  listBookmarks(): Bookmark[] {
    const rows = this.db.prepare(`SELECT b.*, CASE WHEN c.bookmarkId IS NULL THEN 0 ELSE 1 END hasSavedCredential
      FROM bookmarks b LEFT JOIN credentials c ON c.bookmarkId = b.id ORDER BY b.updatedAt DESC`).all() as BookmarkRow[]
    return rows.map((row) => ({ ...row, hasSavedCredential: Boolean(row.hasSavedCredential) }))
  }

  getBookmark(id: string): Bookmark | undefined {
    return this.listBookmarks().find((item) => item.id === id)
  }

  saveBookmark(input: BookmarkInput & { id?: string }): Bookmark {
    const now = new Date().toISOString()
    const existing = input.id ? this.getBookmark(input.id) : undefined
    const bookmark: Bookmark = {
      id: input.id ?? randomUUID(), name: input.name.trim(), host: input.host.trim(),
      port: input.port, username: input.username.trim(), authType: input.authType,
      privateKeyPath: input.privateKeyPath.trim(), jumpBookmarkId: input.jumpBookmarkId.trim(), note: input.note.trim(),
      hasSavedCredential: existing?.hasSavedCredential ?? false,
      createdAt: existing?.createdAt ?? now, updatedAt: now
    }
    this.db.prepare(`INSERT INTO bookmarks (id,name,host,port,username,authType,privateKeyPath,jumpBookmarkId,note,createdAt,updatedAt)
      VALUES (?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET name=excluded.name,host=excluded.host,
      port=excluded.port,username=excluded.username,authType=excluded.authType,
      privateKeyPath=excluded.privateKeyPath,jumpBookmarkId=excluded.jumpBookmarkId,note=excluded.note,updatedAt=excluded.updatedAt`).run(
      bookmark.id, bookmark.name, bookmark.host, bookmark.port, bookmark.username,
      bookmark.authType, bookmark.privateKeyPath, bookmark.jumpBookmarkId, bookmark.note, bookmark.createdAt, bookmark.updatedAt
    )
    return bookmark
  }

  removeBookmark(id: string): void {
    this.db.prepare('DELETE FROM credentials WHERE bookmarkId = ?').run(id)
    this.db.prepare('DELETE FROM bookmarks WHERE id = ?').run(id)
  }

  saveCredential(bookmarkId: string, secret: string): void {
    if (!safeStorage.isEncryptionAvailable()) throw new Error('当前系统无法使用安全凭据存储')
    const encrypted = safeStorage.encryptString(secret).toString('base64')
    this.db.prepare('INSERT INTO credentials(bookmarkId, encrypted) VALUES (?,?) ON CONFLICT(bookmarkId) DO UPDATE SET encrypted=excluded.encrypted').run(bookmarkId, encrypted)
  }

  getCredential(bookmarkId: string): string | undefined {
    const row = this.db.prepare('SELECT encrypted FROM credentials WHERE bookmarkId = ?').get(bookmarkId) as { encrypted: string } | undefined
    if (!row || !safeStorage.isEncryptionAvailable()) return undefined
    return safeStorage.decryptString(Buffer.from(row.encrypted, 'base64'))
  }

  forgetCredential(bookmarkId: string): void {
    this.db.prepare('DELETE FROM credentials WHERE bookmarkId = ?').run(bookmarkId)
  }

  getKnownHost(host: string, port: number): string | undefined {
    return (this.db.prepare('SELECT fingerprint FROM known_hosts WHERE host=? AND port=?').get(host, port) as { fingerprint: string } | undefined)?.fingerprint
  }

  trustHost(host: string, port: number, fingerprint: string): void {
    this.db.prepare('INSERT INTO known_hosts(host,port,fingerprint) VALUES (?,?,?) ON CONFLICT(host,port) DO UPDATE SET fingerprint=excluded.fingerprint').run(host, port, fingerprint)
  }

  getSetting<T>(key: string, fallback: T): T {
    const row = this.db.prepare('SELECT value FROM settings WHERE key=?').get(key) as { value: string } | undefined
    if (!row) return fallback
    try { return JSON.parse(row.value) as T } catch { return fallback }
  }

  setSetting(key: string, value: unknown): void {
    this.db.prepare('INSERT INTO settings(key,value) VALUES (?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value').run(key, JSON.stringify(value))
  }

  listTransfers(): TransferTask[] {
    return this.db.prepare('SELECT * FROM transfers ORDER BY createdAt DESC LIMIT 500').all() as unknown as TransferTask[]
  }

  saveTransfer(task: TransferTask): void {
    this.db.prepare(`INSERT INTO transfers
      (id,sessionId,bookmarkId,direction,sourcePath,destinationPath,displayName,status,bytesTotal,bytesTransferred,speed,error,createdAt,updatedAt)
      VALUES (?,?,?,?,?,?,?,?,?,?,?,?,?,?) ON CONFLICT(id) DO UPDATE SET
      sessionId=excluded.sessionId,bookmarkId=excluded.bookmarkId,destinationPath=excluded.destinationPath,status=excluded.status,bytesTotal=excluded.bytesTotal,
      bytesTransferred=excluded.bytesTransferred,speed=excluded.speed,error=excluded.error,updatedAt=excluded.updatedAt`).run(
      task.id, task.sessionId, task.bookmarkId, task.direction, task.sourcePath, task.destinationPath,
      task.displayName, task.status, task.bytesTotal, task.bytesTransferred, task.speed,
      task.error ?? null, task.createdAt, task.updatedAt
    )
  }

  clearCompletedTransfers(): void {
    this.db.prepare("DELETE FROM transfers WHERE status IN ('completed','cancelled')").run()
  }
}
