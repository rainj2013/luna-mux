import { app, BrowserWindow, clipboard, dialog, ipcMain, Menu, nativeImage, nativeTheme, shell, type MenuItemConstructorOptions } from 'electron'
import { dirname, extname, join, posix, relative } from 'node:path'
import { randomUUID } from 'node:crypto'
import { homedir } from 'node:os'
import { mkdir, open, readFile, readdir, rename, rm, stat, writeFile } from 'node:fs/promises'
import { AppDatabase } from './database'
import { SessionManager } from './session-manager'
import { TransferManager } from './transfer-manager'
import { localParentPath, validateBookmark } from './path-utils'
import { DEFAULT_TERMINAL_SETTINGS, type AppEvent, type AppIconId, type AppIconSettings, type BookmarkInput, type ConnectInput, type ConflictResolution, type DeploymentDiffEntry, type DeploymentProfile, type DirectoryEntry, type FilePreview, type TerminalSettings, type TransferRequest, type TransferTask, type UiTheme } from '../shared/types'
import { sanitizeTerminalSettings } from './terminal-settings'
import { parseSshConfig } from './ssh-config-import'
import type { Bookmark, PortForwardProfile } from '../shared/types'

let mainWindow: BrowserWindow | null = null
let database: AppDatabase
let sessions: SessionManager
let transfers: TransferManager
const allowedSshConfigImports = new Set<string>()

const appIconDefinitions: { id: AppIconId; name: string }[] = [
  { id: 'ssh', name: 'SSH 终端' },
  { id: 'classic', name: '蓝色传输' },
  { id: 'neon', name: '霓虹深色' },
  { id: 'light', name: '清新传输' }
]

const defaultSidebarWidth = 260
const minSidebarWidth = 200
const maxSidebarWidth = 480

function sanitizeSidebarWidth(value: unknown): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return defaultSidebarWidth
  return Math.min(maxSidebarWidth, Math.max(minSidebarWidth, Math.round(value)))
}

function requireSessionId(remote: boolean, sessionId: string | undefined): string {
  if (remote && !sessionId) throw new Error('SSH 会话未连接')
  return sessionId ?? ''
}

async function previewLocalFile(path: string, position: 'start' | 'end'): Promise<FilePreview> {
  const handle = await open(path, 'r')
  try {
    const info = await handle.stat()
    if (!info.isFile()) throw new Error('只能预览文件')
    const limit = 1024 * 1024
    const length = Math.min(limit, info.size)
    const buffer = Buffer.alloc(length)
    const offset = position === 'end' ? Math.max(0, info.size - length) : 0
    await handle.read(buffer, 0, length, offset)
    return filePreview(buffer, info.size, position)
  } finally { await handle.close() }
}

function filePreview(buffer: Buffer, size: number, position: 'start' | 'end'): FilePreview {
  const sample = buffer.subarray(0, Math.min(buffer.length, 8192))
  let controls = 0
  for (const byte of sample) if (byte === 0 || (byte < 9 || (byte > 13 && byte < 32))) controls++
  const binary = sample.length > 0 && controls / sample.length > 0.01
  return { content: binary ? '' : buffer.toString('utf8'), size, truncated: size > 1024 * 1024, position, binary }
}

function deploymentProfiles(): DeploymentProfile[] { return database.getSetting<DeploymentProfile[]>('deploymentProfiles', []) }
function portForwardProfiles(): PortForwardProfile[] { return database.getSetting<PortForwardProfile[]>('portForwardProfiles', []) }

function bookmarkInput(bookmark: Bookmark): BookmarkInput {
  const { id: _id, hasSavedCredential: _saved, lastConnectedAt: _last, createdAt: _created, updatedAt: _updated, ...input } = bookmark
  return input
}

function validatePortForward(input: PortForwardProfile): PortForwardProfile {
  const profile: PortForwardProfile = {
    id: input.id || randomUUID(), bookmarkId: input.bookmarkId, name: input.name.trim(), type: input.type,
    bindAddress: input.bindAddress.trim(), bindPort: Math.round(input.bindPort),
    targetHost: input.targetHost.trim(), targetPort: Math.round(input.targetPort)
  }
  if (!database.getBookmark(profile.bookmarkId)) throw new Error('连接不存在')
  if (!profile.name) throw new Error('请输入端口转发名称')
  if (!['local', 'remote', 'dynamic'].includes(profile.type)) throw new Error('不支持的端口转发类型')
  if (!profile.bindAddress || !/^[A-Za-z0-9:._-]+$/.test(profile.bindAddress)) throw new Error('监听地址格式不正确')
  if (!Number.isInteger(profile.bindPort) || profile.bindPort < 0 || profile.bindPort > 65535) throw new Error('监听端口必须在 0 到 65535 之间')
  if (profile.type !== 'dynamic') {
    if (!profile.targetHost || !/^[A-Za-z0-9:._-]+$/.test(profile.targetHost)) throw new Error('目标地址格式不正确')
    if (!Number.isInteger(profile.targetPort) || profile.targetPort < 1 || profile.targetPort > 65535) throw new Error('目标端口必须在 1 到 65535 之间')
  } else {
    profile.targetHost = ''
    profile.targetPort = 0
  }
  return profile
}

async function deploymentDiff(profile: DeploymentProfile, sessionId: string): Promise<DeploymentDiffEntry[]> {
  if (sessions.getBookmarkId(sessionId) !== profile.bookmarkId) throw new Error('部署配置与当前连接不匹配')
  const localFiles = new Map<string, { path: string; size: number; modifiedAt: number }>()
  const walkLocal = async (directory: string): Promise<void> => {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name)
      if (entry.isDirectory()) await walkLocal(path)
      else if (entry.isFile()) { const info = await stat(path); localFiles.set(relative(profile.localDirectory, path).split('\\').join('/'), { path, size: info.size, modifiedAt: info.mtimeMs }) }
    }
  }
  await walkLocal(profile.localDirectory)
  const remoteFiles = new Map((await sessions.listRemoteTree(sessionId, profile.remoteDirectory)).map((entry) => [entry.relativePath, entry]))
  const result: DeploymentDiffEntry[] = []
  for (const [relativePath, local] of localFiles) {
    const remote = remoteFiles.get(relativePath)
    const remotePath = profile.remoteDirectory === '/' ? `/${relativePath}` : `${profile.remoteDirectory}/${relativePath}`
    const same = remote && remote.size === local.size && Math.abs(remote.modifiedAt - local.modifiedAt) <= 2000
    result.push({ relativePath, localPath: local.path, remotePath, status: !remote ? 'new' : same ? 'same' : 'changed', size: local.size })
    remoteFiles.delete(relativePath)
  }
  for (const remote of remoteFiles.values()) result.push({ relativePath: remote.relativePath, remotePath: remote.path, status: 'remote-only', size: remote.size })
  return result.sort((a, b) => a.relativePath.localeCompare(b.relativePath))
}

function sanitizeAppIcon(value: unknown): AppIconId {
  return appIconDefinitions.some((item) => item.id === value) ? value as AppIconId : 'ssh'
}

function sanitizeUiTheme(value: unknown): UiTheme {
  return value === 'light' || value === 'dark' ? value : 'system'
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

function emit(event: AppEvent): void {
  if (!mainWindow || mainWindow.isDestroyed() || mainWindow.webContents.isDestroyed()) return
  mainWindow.webContents.send('app:event', event)
}

function installMacApplicationMenu(): void {
  if (process.platform !== 'darwin') return
  const applicationName = app.getName() === 'ssh-client' ? 'SSH Client' : app.getName()
  const command = (payload: Extract<AppEvent, { type: 'menu-command' }>['payload']): void => emit({ type: 'menu-command', payload })
  const template: MenuItemConstructorOptions[] = [
    {
      label: applicationName,
      submenu: [
        { role: 'about', label: `关于 ${applicationName}` },
        { type: 'separator' },
        { label: '设置...', accelerator: 'CommandOrControl+,', click: () => command('settings') },
        { type: 'separator' },
        { role: 'services', label: '服务' },
        { type: 'separator' },
        { role: 'hide', label: `隐藏 ${applicationName}` },
        { role: 'hideOthers', label: '隐藏其他' },
        { role: 'unhide', label: '全部显示' },
        { type: 'separator' },
        { role: 'quit', label: `退出 ${applicationName}` }
      ]
    },
    {
      label: '连接',
      submenu: [
        { label: '新增连接', accelerator: 'CommandOrControl+N', click: () => command('new-connection') },
        { label: '导入 OpenSSH Config...', click: () => command('import-connections') },
        { type: 'separator' },
        { label: '为当前机器新建会话', accelerator: 'CommandOrControl+Shift+T', click: () => command('new-session') },
        { label: '关闭当前标签', accelerator: 'CommandOrControl+W', click: () => command('close-tab') }
      ]
    },
    {
      label: '编辑',
      submenu: [
        { role: 'undo', label: '撤销' },
        { role: 'redo', label: '重做' },
        { type: 'separator' },
        { role: 'cut', label: '剪切' },
        { role: 'copy', label: '复制' },
        { role: 'paste', label: '粘贴' },
        { role: 'pasteAndMatchStyle', label: '粘贴并匹配样式' },
        { role: 'delete', label: '删除' },
        { role: 'selectAll', label: '全选' }
      ]
    },
    {
      label: '视图',
      submenu: [
        { label: '终端', accelerator: 'CommandOrControl+1', click: () => command('show-terminal') },
        { label: '文件传输', accelerator: 'CommandOrControl+2', click: () => command('show-files') },
        { type: 'separator' },
        { label: '展开/折叠连接管理', accelerator: 'CommandOrControl+B', click: () => command('toggle-sidebar') },
        { type: 'separator' },
        { role: 'resetZoom', label: '实际大小', accelerator: 'CommandOrControl+0' },
        { role: 'zoomIn', label: '放大', accelerator: 'CommandOrControl+=' },
        { role: 'zoomOut', label: '缩小', accelerator: 'CommandOrControl+-' },
        { type: 'separator' },
        { role: 'togglefullscreen', label: '进入/退出全屏' }
      ]
    },
    {
      label: '窗口',
      submenu: [
        { role: 'minimize', label: '最小化' },
        { role: 'zoom', label: '缩放' },
        { type: 'separator' },
        { role: 'front', label: '前置全部窗口' }
      ]
    },
    {
      role: 'help',
      label: '帮助',
      submenu: [
        { label: `${applicationName} 使用帮助`, accelerator: 'F1', click: () => command('help') }
      ]
    }
  ]
  Menu.setApplicationMenu(Menu.buildFromTemplate(template))
}

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
  mainWindow.on('closed', () => { mainWindow = null })
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
  ipcMain.handle('system:open-external', async (_event, value: string) => {
    const url = new URL(value)
    if (!['http:', 'https:'].includes(url.protocol)) throw new Error('只允许打开 HTTP 或 HTTPS 链接')
    await shell.openExternal(url.toString())
  })
  ipcMain.handle('system:read-clipboard', () => clipboard.readText())
  ipcMain.handle('system:write-clipboard', (_event, text: string) => clipboard.writeText(String(text).slice(0, 4 * 1024 * 1024)))

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
  ipcMain.handle('bookmarks:preview-ssh-config', async () => {
    const result = await dialog.showOpenDialog(mainWindow!, {
      title: '导入 OpenSSH Config', defaultPath: join(homedir(), '.ssh', 'config'), properties: ['openFile']
    })
    const path = result.canceled ? undefined : result.filePaths[0]
    if (!path) return null
    const info = await stat(path)
    if (!info.isFile() || info.size > 5 * 1024 * 1024) throw new Error('SSH Config 必须是小于 5 MB 的普通文件')
    const entries = parseSshConfig(await readFile(path, 'utf8'), homedir())
    allowedSshConfigImports.add(path)
    return { path, entries }
  })
  ipcMain.handle('bookmarks:import-ssh-config', async (_event, path: string, aliases: string[]) => {
    if (!allowedSshConfigImports.delete(path)) throw new Error('请重新选择 SSH Config 文件')
    const entries = parseSshConfig(await readFile(path, 'utf8'), homedir())
    const selected = new Set(aliases.map((value) => value.toLowerCase()))
    const chosen = entries.filter((entry) => selected.has(entry.alias.toLowerCase()))
    const existing = database.listBookmarks()
    const imported = new Map<string, Bookmark>()
    for (const entry of chosen) {
      const current = existing.find((item) => item.name.toLowerCase() === entry.alias.toLowerCase())
      const saved = database.saveBookmark({
        ...(current ? bookmarkInput(current) : {
          groupName: 'SSH Config', favorite: false, keepaliveEnabled: true,
          keepaliveIntervalSeconds: 15, keepaliveCountMax: 3, note: ''
        }),
        id: current?.id, name: entry.name, host: entry.host, port: entry.port, username: entry.username,
        authType: entry.privateKeyPath ? 'privateKey' : 'agent', privateKeyPath: entry.privateKeyPath,
        jumpBookmarkId: ''
      })
      imported.set(entry.alias.toLowerCase(), saved)
    }
    for (const entry of chosen) {
      const saved = imported.get(entry.alias.toLowerCase())
      if (!saved || !entry.proxyJumpAlias) continue
      const jump = imported.get(entry.proxyJumpAlias.toLowerCase())
        ?? existing.find((item) => item.name.toLowerCase() === entry.proxyJumpAlias.toLowerCase())
      if (jump && jump.id !== saved.id && !jump.jumpBookmarkId) {
        imported.set(entry.alias.toLowerCase(), database.saveBookmark({ ...bookmarkInput(saved), id: saved.id, jumpBookmarkId: jump.id }))
      }
    }
    return [...imported.values()]
  })

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
      return { name: entry.name, path: fullPath, kind }
    })
  })
  ipcMain.handle('files:list-remote', (_event, sessionId: string, path: string) => sessions.listRemote(sessionId, path))
  ipcMain.handle('files:create-directory', async (_event, remote: boolean, sessionId: string | undefined, path: string) => {
    if (remote) await sessions.createRemoteDirectory(requireSessionId(true, sessionId), path)
    else await mkdir(path)
  })
  ipcMain.handle('files:rename', async (_event, remote: boolean, sessionId: string | undefined, from: string, to: string) => {
    if (remote) await sessions.renameRemote(requireSessionId(true, sessionId), from, to)
    else await rename(from, to)
  })
  ipcMain.handle('files:remove', async (_event, remote: boolean, sessionId: string | undefined, paths: string[]) => {
    if (remote) await sessions.removeRemote(requireSessionId(true, sessionId), paths)
    else for (const path of paths) await rm(path, { recursive: true, force: false })
  })
  ipcMain.handle('files:preview', (_event, remote: boolean, sessionId: string | undefined, path: string, position: 'start' | 'end') => remote
    ? sessions.previewRemote(requireSessionId(true, sessionId), path, position)
    : previewLocalFile(path, position))
  ipcMain.handle('files:get-favorites', (_event, bookmarkId: string) => database.getSetting(`fileFavorites:${bookmarkId}`, { local: [] as string[], remote: [] as string[] }))
  ipcMain.handle('files:set-favorites', (_event, bookmarkId: string, value: { local: string[]; remote: string[] }) => database.setSetting(`fileFavorites:${bookmarkId}`, {
    local: [...new Set(value.local.map(String))].slice(0, 30), remote: [...new Set(value.remote.map(String))].slice(0, 30)
  }))
  ipcMain.handle('files:choose-local-directory', async () => {
    const result = await dialog.showOpenDialog(mainWindow!, { title: '选择本地部署目录', properties: ['openDirectory'] })
    return result.canceled ? null : result.filePaths[0] ?? null
  })
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

  ipcMain.handle('deployments:list', (_event, bookmarkId: string) => deploymentProfiles().filter((profile) => profile.bookmarkId === bookmarkId))
  ipcMain.handle('deployments:save', (_event, input: DeploymentProfile) => {
    if (!database.getBookmark(input.bookmarkId)) throw new Error('连接不存在')
    const profile: DeploymentProfile = {
      id: input.id || randomUUID(), bookmarkId: input.bookmarkId, name: input.name.trim(),
      localDirectory: input.localDirectory.trim(), remoteDirectory: input.remoteDirectory.trim(),
      deleteExtraneous: Boolean(input.deleteExtraneous)
    }
    if (!profile.name || !profile.localDirectory || !profile.remoteDirectory.startsWith('/')) throw new Error('部署名称、本地目录和远端绝对路径不能为空')
    const profiles = deploymentProfiles()
    database.setSetting('deploymentProfiles', [profile, ...profiles.filter((item) => item.id !== profile.id)])
    return profile
  })
  ipcMain.handle('deployments:remove', (_event, id: string) => database.setSetting('deploymentProfiles', deploymentProfiles().filter((profile) => profile.id !== id)))
  ipcMain.handle('deployments:preview', async (_event, id: string, sessionId: string) => {
    const profile = deploymentProfiles().find((item) => item.id === id)
    if (!profile) throw new Error('部署配置不存在')
    return await deploymentDiff(profile, sessionId)
  })
  ipcMain.handle('deployments:execute', async (_event, id: string, sessionId: string): Promise<TransferTask[]> => {
    const profile = deploymentProfiles().find((item) => item.id === id)
    if (!profile) throw new Error('部署配置不存在')
    const diff = await deploymentDiff(profile, sessionId)
    const groups = new Map<string, string[]>()
    for (const entry of diff) {
      if (!entry.localPath || !['new', 'changed'].includes(entry.status)) continue
      const destination = posix.dirname(entry.remotePath)
      groups.set(destination, [...(groups.get(destination) ?? []), entry.localPath])
    }
    const tasks = [...groups].flatMap(([destinationDirectory, sourcePaths]) => transfers.enqueue({ sessionId, direction: 'upload', sourcePaths, destinationDirectory }))
    const extraneous = diff.filter((entry) => entry.status === 'remote-only').map((entry) => entry.remotePath)
    if (profile.deleteExtraneous && extraneous.length) {
      void transfers.whenSettled(tasks.map((task) => task.id)).then((succeeded) => { if (succeeded) return sessions.removeRemote(sessionId, extraneous) }).catch(() => undefined)
    }
    return tasks
  })

  ipcMain.handle('tunnels:list-profiles', (_event, bookmarkId: string) => portForwardProfiles().filter((profile) => profile.bookmarkId === bookmarkId))
  ipcMain.handle('tunnels:save-profile', (_event, input: PortForwardProfile) => {
    const profile = validatePortForward(input)
    const profiles = portForwardProfiles()
    database.setSetting('portForwardProfiles', [profile, ...profiles.filter((item) => item.id !== profile.id)])
    return profile
  })
  ipcMain.handle('tunnels:remove-profile', (_event, id: string) => database.setSetting('portForwardProfiles', portForwardProfiles().filter((profile) => profile.id !== id)))
  ipcMain.handle('tunnels:list', (_event, sessionId: string) => sessions.listTunnels(sessionId))
  ipcMain.handle('tunnels:start', (_event, sessionId: string, profileId: string) => {
    const profile = portForwardProfiles().find((item) => item.id === profileId)
    if (!profile) throw new Error('端口转发配置不存在')
    return sessions.startTunnel(sessionId, profile)
  })
  ipcMain.handle('tunnels:stop', (_event, sessionId: string, tunnelId: string) => sessions.stopTunnel(sessionId, tunnelId))

  ipcMain.handle('diagnostics:export', async () => {
    const result = await dialog.showSaveDialog(mainWindow!, {
      title: '导出诊断信息', defaultPath: `ssh-client-diagnostics-${new Date().toISOString().replaceAll(':', '-').slice(0, 19)}.json`,
      filters: [{ name: 'JSON', extensions: ['json'] }]
    })
    if (result.canceled || !result.filePath) return null
    const bookmarkList = database.listBookmarks()
    const transferList = transfers.list()
    const diagnostics = {
      generatedAt: new Date().toISOString(),
      application: { name: app.getName(), version: app.getVersion(), platform: process.platform, arch: process.arch, electron: process.versions.electron, node: process.versions.node },
      connections: bookmarkList.map((item) => ({
        id: item.id, name: item.name, host: item.host, port: item.port, username: item.username,
        authType: item.authType, groupName: item.groupName, favorite: item.favorite,
        usesJumpHost: Boolean(item.jumpBookmarkId), keepaliveEnabled: item.keepaliveEnabled,
        keepaliveIntervalSeconds: item.keepaliveIntervalSeconds, keepaliveCountMax: item.keepaliveCountMax,
        hasSavedCredential: item.hasSavedCredential, lastConnectedAt: item.lastConnectedAt
      })),
      sessions: sessions.list(),
      transfers: {
        total: transferList.length,
        byStatus: Object.fromEntries([...new Set(transferList.map((item) => item.status))].map((status) => [status, transferList.filter((item) => item.status === status).length])),
        activeBytes: transferList.filter((item) => ['queued', 'scanning', 'running', 'conflict'].includes(item.status)).reduce((sum, item) => sum + item.bytesTransferred, 0)
      },
      portForwardProfileCount: portForwardProfiles().length
    }
    await writeFile(result.filePath, `${JSON.stringify(diagnostics, null, 2)}\n`, { encoding: 'utf8', mode: 0o600 })
    return result.filePath
  })

  ipcMain.handle('state:get-open-bookmarks', () => database.getSetting<string[]>('openBookmarks', []))
  ipcMain.handle('state:set-open-bookmarks', (_event, ids: string[]) => database.setSetting('openBookmarks', ids))
  ipcMain.handle('state:get-sidebar-collapsed', () => database.getSetting<boolean>('sidebarCollapsed', false))
  ipcMain.handle('state:set-sidebar-collapsed', (_event, collapsed: boolean) => database.setSetting('sidebarCollapsed', Boolean(collapsed)))
  ipcMain.handle('state:get-sidebar-width', () => sanitizeSidebarWidth(database.getSetting('sidebarWidth', defaultSidebarWidth)))
  ipcMain.handle('state:set-sidebar-width', (_event, width: number) => database.setSetting('sidebarWidth', sanitizeSidebarWidth(width)))

  ipcMain.handle('settings:get-ui-theme', () => sanitizeUiTheme(database.getSetting('uiTheme', 'system')))
  ipcMain.handle('settings:save-ui-theme', (_event, value: UiTheme) => {
    const theme = sanitizeUiTheme(value)
    database.setSetting('uiTheme', theme)
    nativeTheme.themeSource = theme
    return theme
  })
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
  nativeTheme.themeSource = sanitizeUiTheme(database.getSetting('uiTheme', 'system'))
  sessions = new SessionManager(database, emit)
  transfers = new TransferManager(database, sessions, emit)
  registerIpc(); installMacApplicationMenu(); createWindow()
  app.on('activate', () => { if (BrowserWindow.getAllWindows().length === 0) createWindow() })
})

app.on('before-quit', () => sessions?.disconnectAll())
app.on('window-all-closed', () => { if (process.platform !== 'darwin') app.quit() })
