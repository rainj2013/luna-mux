import { app, BrowserWindow, dialog, ipcMain, shell } from 'electron'
import { join, basename } from 'node:path'
import { homedir } from 'node:os'
import { lstat, readdir } from 'node:fs/promises'
import { AppDatabase } from './database'
import { SessionManager } from './session-manager'
import { TransferManager } from './transfer-manager'
import { validateBookmark } from './path-utils'
import type { AppEvent, BookmarkInput, ConnectInput, ConflictResolution, DirectoryEntry, TransferRequest } from '../shared/types'

let mainWindow: BrowserWindow | null = null
let database: AppDatabase
let sessions: SessionManager
let transfers: TransferManager

function emit(event: AppEvent): void { mainWindow?.webContents.send('app:event', event) }

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1360, height: 860, minWidth: 980, minHeight: 640,
    backgroundColor: '#f5f7fa', show: false,
    titleBarStyle: 'hidden',
    titleBarOverlay: process.platform === 'win32' ? { color: '#ffffff', symbolColor: '#273142', height: 42 } : false,
    trafficLightPosition: { x: 16, y: 13 },
    webPreferences: {
      preload: join(__dirname, '../preload/index.mjs'),
      sandbox: true, contextIsolation: true, nodeIntegration: false
    }
  })
  mainWindow.on('ready-to-show', () => mainWindow?.show())
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    if (/^https?:/.test(url)) void shell.openExternal(url)
    return { action: 'deny' }
  })
  mainWindow.webContents.on('will-navigate', (event, url) => {
    if (url !== mainWindow?.webContents.getURL()) event.preventDefault()
  })
  if (process.env['ELECTRON_RENDERER_URL']) mainWindow.loadURL(process.env['ELECTRON_RENDERER_URL'])
  else mainWindow.loadFile(join(__dirname, '../renderer/index.html'))
}

function registerIpc(): void {
  ipcMain.handle('bookmarks:list', () => database.listBookmarks())
  ipcMain.handle('bookmarks:save', (_event, input: BookmarkInput & { id?: string }) => {
    const errors = validateBookmark(input)
    if (errors.length) throw new Error(errors[0])
    return database.saveBookmark(input)
  })
  ipcMain.handle('bookmarks:remove', (_event, id: string) => database.removeBookmark(id))
  ipcMain.handle('bookmarks:forget-credential', (_event, id: string) => database.forgetCredential(id))

  ipcMain.handle('sessions:list', () => sessions.list())
  ipcMain.handle('sessions:connect', (_event, input: ConnectInput) => sessions.connect(input))
  ipcMain.handle('sessions:disconnect', (_event, id: string) => sessions.disconnect(id))
  ipcMain.on('sessions:write', (_event, id: string, data: string) => sessions.write(id, data))
  ipcMain.on('sessions:resize', (_event, id: string, cols: number, rows: number) => sessions.resize(id, cols, rows))
  ipcMain.on('sessions:host-key-decision', (_event, id: string, accept: boolean) => sessions.hostKeyDecision(id, accept))

  ipcMain.handle('files:home', () => homedir())
  ipcMain.handle('files:list-local', async (_event, path: string): Promise<DirectoryEntry[]> => {
    const entries = await readdir(path, { withFileTypes: true })
    return await Promise.all(entries.map(async (entry) => {
      const fullPath = join(path, entry.name)
      const stats = await lstat(fullPath)
      const kind: DirectoryEntry['kind'] = entry.isDirectory() ? 'directory' : entry.isSymbolicLink() ? 'symlink' : entry.isFile() ? 'file' : 'other'
      return { name: entry.name, path: fullPath, kind, size: stats.size, modifiedAt: stats.mtimeMs }
    }))
  })
  ipcMain.handle('files:list-remote', (_event, sessionId: string, path: string) => sessions.listRemote(sessionId, path))
  ipcMain.handle('files:choose-private-key', async () => {
    const result = await dialog.showOpenDialog(mainWindow!, { title: '选择 SSH 私钥', properties: ['openFile'] })
    return result.canceled ? null : result.filePaths[0] ?? null
  })

  ipcMain.handle('transfers:list', () => transfers.list())
  ipcMain.handle('transfers:enqueue', (_event, request: TransferRequest) => transfers.enqueue(request))
  ipcMain.handle('transfers:cancel', (_event, id: string) => transfers.cancel(id))
  ipcMain.handle('transfers:retry', (_event, id: string) => transfers.retry(id))
  ipcMain.handle('transfers:clear-completed', () => transfers.clearCompleted())
  ipcMain.on('transfers:resolve-conflict', (_event, id: string, resolution: ConflictResolution, applyToBatch: boolean) => transfers.resolveConflict(id, resolution, applyToBatch))

  ipcMain.handle('state:get-open-bookmarks', () => database.getSetting<string[]>('openBookmarks', []))
  ipcMain.handle('state:set-open-bookmarks', (_event, ids: string[]) => database.setSetting('openBookmarks', [...new Set(ids)]))
}

app.whenReady().then(() => {
  database = new AppDatabase()
  sessions = new SessionManager(database, emit)
  transfers = new TransferManager(database, sessions, emit)
  registerIpc(); createWindow()
  app.on('activate', () => { if (BrowserWindow.getAllWindows().length === 0) createWindow() })
})

app.on('before-quit', () => sessions?.disconnectAll())
app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit() })
