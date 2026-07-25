import { app, BrowserWindow, dialog, ipcMain, nativeImage, shell } from 'electron'
import { extname, join } from 'node:path'
import { homedir } from 'node:os'
import { readdir, stat } from 'node:fs/promises'
import { AppDatabase } from './database'
import { SessionManager } from './session-manager'
import { TransferManager } from './transfer-manager'
import { localParentPath, validateBookmark } from './path-utils'
import { DEFAULT_TERMINAL_SETTINGS, type AppEvent, type AppIconId, type AppIconSettings, type BookmarkInput, type ConnectInput, type ConflictResolution, type DirectoryEntry, type TerminalSettings, type TransferRequest } from '../shared/types'
import { sanitizeTerminalSettings } from './terminal-settings'

let mainWindow: BrowserWindow | null = null
let database: AppDatabase
let sessions: SessionManager
let transfers: TransferManager

const appIconDefinitions: { id: AppIconId; name: string }[] = [
  { id: 'ssh', name: 'SSH 终端' },
  { id: 'classic', name: '蓝色传输' },
  { id: 'neon', name: '霓虹深色' },
  { id: 'light', name: '清新传输' }
]

function sanitizeAppIcon(value: unknown): AppIconId {
  return appIconDefinitions.some((item) => item.id === value) ? value as AppIconId : 'ssh'
}

function appIconPath(id: AppIconId): string {
  const directory = app.isPackaged ? join(process.resourcesPath, 'icons') : join(app.getAppPath(), 'resources', 'icons')
  return join(directory, `${id}.png`)
}

function appIconImage(id: AppIconId): Electron.NativeImage {
  const image = nativeImage.createFromPath(appIconPath(id))
  return image.isEmpty() && id !== 'ssh' ? nativeImage.createFromPath(appIconPath('ssh')) : image
}

function currentAppIcon(): AppIconId {
  return sanitizeAppIcon(database.getSetting('appIcon', 'ssh'))
}

function appIconSettings(): AppIconSettings {
  return {
    selected: currentAppIcon(),
    options: appIconDefinitions.map((item) => ({ ...item, dataUrl: appIconImage(item.id).resize({ width: 128, height: 128, quality: 'good' }).toDataURL() }))
  }
}

function applyAppIcon(id: AppIconId): void {
  const image = appIconImage(id)
  if (process.platform === 'darwin') app.dock?.setIcon(image)
  else mainWindow?.setIcon(image)
}

function emit(event: AppEvent): void { mainWindow?.webContents.send('app:event', event) }

async function loadTerminalBackground(path: string): Promise<string> {
  if (!path || !['.png', '.jpg', '.jpeg', '.webp', '.bmp', '.gif'].includes(extname(path).toLowerCase())) throw new Error('请选择 PNG、JPEG、WebP、BMP 或 GIF 图片')
  const file = await stat(path)
  if (!file.isFile()) throw new Error('背景图片路径不是文件')
  if (file.size > 25 * 1024 * 1024) throw new Error('背景图片不能超过 25 MB')
  const image = nativeImage.createFromPath(path)
  if (image.isEmpty()) throw new Error('无法读取背景图片')
  const size = image.getSize()
  const scale = Math.min(1, 3840 / size.width, 2160 / size.height)
  const displayImage = scale < 1 ? image.resize({ width: Math.max(1, Math.round(size.width * scale)), height: Math.max(1, Math.round(size.height * scale)), quality: 'best' }) : image
  return displayImage.toDataURL()
}

function createWindow(): void {
  const icon = appIconImage(currentAppIcon())
  mainWindow = new BrowserWindow({
    width: 1360, height: 860, minWidth: 980, minHeight: 640,
    backgroundColor: '#f5f7fa', show: false, icon,
    titleBarStyle: 'hidden',
    titleBarOverlay: process.platform === 'win32' ? { color: '#ffffff', symbolColor: '#273142', height: 42 } : false,
    trafficLightPosition: { x: 16, y: 13 },
    webPreferences: {
      preload: join(__dirname, '../preload/index.cjs'),
      sandbox: true, contextIsolation: true, nodeIntegration: false
    }
  })
  applyAppIcon(currentAppIcon())
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
    const jumpBookmarkId = input.jumpBookmarkId.trim()
    const connections = database.listBookmarks()
    if (input.id && jumpBookmarkId === input.id) throw new Error('连接不能将自身设置为跳板机')
    if (jumpBookmarkId) {
      const jump = connections.find((item) => item.id === jumpBookmarkId)
      if (!jump) throw new Error('选择的跳板机连接不存在')
      if (jump.jumpBookmarkId) throw new Error('当前仅支持单级跳板机，请选择一个直连连接')
      if (input.id && connections.some((item) => item.jumpBookmarkId === input.id)) {
        throw new Error('此连接已被其他连接用作跳板机，不能再为它配置跳板机')
      }
    }
    return database.saveBookmark({ ...input, jumpBookmarkId })
  })
  ipcMain.handle('bookmarks:remove', (_event, id: string) => {
    const dependent = database.listBookmarks().find((item) => item.jumpBookmarkId === id)
    if (dependent) throw new Error(`连接“${dependent.name}”正在使用此跳板机，请先修改该连接`)
    database.removeBookmark(id)
  })
  ipcMain.handle('bookmarks:forget-credential', (_event, id: string) => database.forgetCredential(id))

  ipcMain.handle('sessions:list', () => sessions.list())
  ipcMain.handle('sessions:connect', (_event, input: ConnectInput) => sessions.connect(input))
  ipcMain.handle('sessions:disconnect', (_event, id: string) => sessions.disconnect(id))
  ipcMain.on('sessions:write', (_event, id: string, data: string) => sessions.write(id, data))
  ipcMain.on('sessions:resize', (_event, id: string, cols: number, rows: number) => sessions.resize(id, cols, rows))
  ipcMain.on('sessions:flow', (_event, id: string, paused: boolean) => sessions.flow(id, paused))
  ipcMain.on('sessions:host-key-decision', (_event, id: string, accept: boolean) => sessions.hostKeyDecision(id, accept))

  ipcMain.handle('files:home', () => homedir())
  ipcMain.handle('files:parent-local', (_event, path: string) => localParentPath(path))
  ipcMain.handle('files:remote-home', (_event, sessionId: string) => sessions.getRemoteHome(sessionId))
  ipcMain.handle('files:list-local', async (_event, path: string): Promise<DirectoryEntry[]> => {
    const entries = await readdir(path, { withFileTypes: true })
    return entries.map((entry) => {
      const fullPath = join(path, entry.name)
      const kind: DirectoryEntry['kind'] = entry.isDirectory() ? 'directory' : entry.isSymbolicLink() ? 'symlink' : entry.isFile() ? 'file' : 'other'
      return { name: entry.name, path: fullPath, kind, size: 0, modifiedAt: 0 }
    })
  })
  ipcMain.handle('files:list-remote', (_event, sessionId: string, path: string) => sessions.listRemote(sessionId, path))
  ipcMain.handle('files:choose-private-key', async () => {
    const result = await dialog.showOpenDialog(mainWindow!, { title: '选择 SSH 私钥', properties: ['openFile'] })
    return result.canceled ? null : result.filePaths[0] ?? null
  })

  ipcMain.handle('transfers:list', () => transfers.list())
  ipcMain.handle('transfers:enqueue', (_event, request: TransferRequest) => transfers.enqueue(request))
  ipcMain.handle('transfers:cancel', (_event, id: string) => transfers.cancel(id))
  ipcMain.handle('transfers:retry', (_event, id: string, sessionId: string) => transfers.retry(id, sessionId))
  ipcMain.handle('transfers:clear-completed', () => transfers.clearCompleted())
  ipcMain.on('transfers:resolve-conflict', (_event, id: string, resolution: ConflictResolution, applyToBatch: boolean) => transfers.resolveConflict(id, resolution, applyToBatch))

  ipcMain.handle('state:get-open-bookmarks', () => database.getSetting<string[]>('openBookmarks', []))
  ipcMain.handle('state:set-open-bookmarks', (_event, ids: string[]) => database.setSetting('openBookmarks', [...new Set(ids)]))
  ipcMain.handle('state:get-sidebar-collapsed', () => database.getSetting<boolean>('sidebarCollapsed', false))
  ipcMain.handle('state:set-sidebar-collapsed', (_event, collapsed: boolean) => database.setSetting('sidebarCollapsed', Boolean(collapsed)))

  ipcMain.handle('settings:get-terminal', () => sanitizeTerminalSettings(database.getSetting('terminalSettings', DEFAULT_TERMINAL_SETTINGS)))
  ipcMain.handle('settings:save-terminal', (_event, input: TerminalSettings) => {
    const settings = sanitizeTerminalSettings(input)
    database.setSetting('terminalSettings', settings)
    return settings
  })
  ipcMain.handle('settings:choose-terminal-background', async () => {
    const result = await dialog.showOpenDialog(mainWindow!, {
      title: '选择终端背景图片', properties: ['openFile'],
      filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'webp', 'bmp', 'gif'] }]
    })
    return result.canceled ? null : result.filePaths[0] ?? null
  })
  ipcMain.handle('settings:load-terminal-background', (_event, path: string) => loadTerminalBackground(path))
  ipcMain.handle('settings:get-app-icons', () => appIconSettings())
  ipcMain.handle('settings:set-app-icon', (_event, value: AppIconId) => {
    const icon = sanitizeAppIcon(value)
    database.setSetting('appIcon', icon)
    applyAppIcon(icon)
    return icon
  })
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
