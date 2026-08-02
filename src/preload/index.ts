import { contextBridge, ipcRenderer, webUtils } from 'electron'
import type { AppApi, AppEvent, AppIconId, BookmarkInput, ConnectInput, ConflictResolution, DeploymentProfile, PortForwardProfile, TerminalSettings, TransferRequest, UiTheme } from '../shared/types'

const api: AppApi = {
  platform: process.platform,
  system: {
    openExternal: (url: string) => ipcRenderer.invoke('system:open-external', url),
    readClipboard: () => ipcRenderer.invoke('system:read-clipboard'),
    writeClipboard: (text: string) => ipcRenderer.invoke('system:write-clipboard', text)
  },
  bookmarks: {
    list: () => ipcRenderer.invoke('bookmarks:list'),
    save: (input: BookmarkInput & { id?: string }) => ipcRenderer.invoke('bookmarks:save', input),
    remove: (id: string) => ipcRenderer.invoke('bookmarks:remove', id),
    forgetCredential: (id: string) => ipcRenderer.invoke('bookmarks:forget-credential', id),
    previewSshConfig: () => ipcRenderer.invoke('bookmarks:preview-ssh-config'),
    importSshConfig: (path: string, aliases: string[]) => ipcRenderer.invoke('bookmarks:import-ssh-config', path, aliases)
  },
  sessions: {
    list: () => ipcRenderer.invoke('sessions:list'),
    connect: (input: ConnectInput) => ipcRenderer.invoke('sessions:connect', input),
    disconnect: (id: string) => ipcRenderer.invoke('sessions:disconnect', id),
    write: (id: string, data: string) => ipcRenderer.send('sessions:write', id, data),
    resize: (id: string, cols: number, rows: number) => ipcRenderer.send('sessions:resize', id, cols, rows),
    flow: (id: string, paused: boolean) => ipcRenderer.send('sessions:flow', id, paused),
    hostKeyDecision: (id: string, accept: boolean) => ipcRenderer.send('sessions:host-key-decision', id, accept)
  },
  files: {
    home: () => ipcRenderer.invoke('files:home'),
    parentLocal: (path: string) => ipcRenderer.invoke('files:parent-local', path),
    remoteHome: (sessionId: string) => ipcRenderer.invoke('files:remote-home', sessionId),
    listLocal: (path: string) => ipcRenderer.invoke('files:list-local', path),
    listRemote: (sessionId: string, path: string) => ipcRenderer.invoke('files:list-remote', sessionId, path),
    createDirectory: (remote: boolean, sessionId: string | undefined, path: string) => ipcRenderer.invoke('files:create-directory', remote, sessionId, path),
    rename: (remote: boolean, sessionId: string | undefined, from: string, to: string) => ipcRenderer.invoke('files:rename', remote, sessionId, from, to),
    remove: (remote: boolean, sessionId: string | undefined, paths: string[]) => ipcRenderer.invoke('files:remove', remote, sessionId, paths),
    preview: (remote: boolean, sessionId: string | undefined, path: string, position: 'start' | 'end') => ipcRenderer.invoke('files:preview', remote, sessionId, path, position),
    getFavorites: (bookmarkId: string) => ipcRenderer.invoke('files:get-favorites', bookmarkId),
    setFavorites: (bookmarkId: string, value: { local: string[]; remote: string[] }) => ipcRenderer.invoke('files:set-favorites', bookmarkId, value),
    filePath: (file: File) => webUtils.getPathForFile(file),
    chooseLocalDirectory: () => ipcRenderer.invoke('files:choose-local-directory'),
    choosePrivateKey: () => ipcRenderer.invoke('files:choose-private-key')
  },
  transfers: {
    list: () => ipcRenderer.invoke('transfers:list'),
    enqueue: (request: TransferRequest) => ipcRenderer.invoke('transfers:enqueue', request),
    cancel: (id: string) => ipcRenderer.invoke('transfers:cancel', id),
    retry: (id: string, sessionId: string) => ipcRenderer.invoke('transfers:retry', id, sessionId),
    clearCompleted: () => ipcRenderer.invoke('transfers:clear-completed'),
    resolveConflict: (id: string, resolution: ConflictResolution, applyToBatch: boolean) => ipcRenderer.send('transfers:resolve-conflict', id, resolution, applyToBatch)
  },
  deployments: {
    list: (bookmarkId: string) => ipcRenderer.invoke('deployments:list', bookmarkId),
    save: (profile: DeploymentProfile) => ipcRenderer.invoke('deployments:save', profile),
    remove: (id: string) => ipcRenderer.invoke('deployments:remove', id),
    preview: (id: string, sessionId: string) => ipcRenderer.invoke('deployments:preview', id, sessionId),
    execute: (id: string, sessionId: string) => ipcRenderer.invoke('deployments:execute', id, sessionId)
  },
  tunnels: {
    listProfiles: (bookmarkId: string) => ipcRenderer.invoke('tunnels:list-profiles', bookmarkId),
    saveProfile: (profile: PortForwardProfile) => ipcRenderer.invoke('tunnels:save-profile', profile),
    removeProfile: (id: string) => ipcRenderer.invoke('tunnels:remove-profile', id),
    list: (sessionId: string) => ipcRenderer.invoke('tunnels:list', sessionId),
    start: (sessionId: string, profileId: string) => ipcRenderer.invoke('tunnels:start', sessionId, profileId),
    stop: (sessionId: string, tunnelId: string) => ipcRenderer.invoke('tunnels:stop', sessionId, tunnelId)
  },
  diagnostics: {
    export: () => ipcRenderer.invoke('diagnostics:export')
  },
  state: {
    getOpenBookmarks: () => ipcRenderer.invoke('state:get-open-bookmarks'),
    setOpenBookmarks: (ids: string[]) => ipcRenderer.invoke('state:set-open-bookmarks', ids),
    getSidebarCollapsed: () => ipcRenderer.invoke('state:get-sidebar-collapsed'),
    setSidebarCollapsed: (collapsed: boolean) => ipcRenderer.invoke('state:set-sidebar-collapsed', collapsed),
    getSidebarWidth: () => ipcRenderer.invoke('state:get-sidebar-width'),
    setSidebarWidth: (width: number) => ipcRenderer.invoke('state:set-sidebar-width', width)
  },
  settings: {
    getUiTheme: () => ipcRenderer.invoke('settings:get-ui-theme'),
    saveUiTheme: (theme: UiTheme) => ipcRenderer.invoke('settings:save-ui-theme', theme),
    getTerminal: () => ipcRenderer.invoke('settings:get-terminal'),
    saveTerminal: (settings: TerminalSettings) => ipcRenderer.invoke('settings:save-terminal', settings),
    chooseTerminalBackground: () => ipcRenderer.invoke('settings:choose-terminal-background'),
    loadTerminalBackground: (path: string) => ipcRenderer.invoke('settings:load-terminal-background', path),
    getAppIcons: () => ipcRenderer.invoke('settings:get-app-icons'),
    setAppIcon: (icon: AppIconId) => ipcRenderer.invoke('settings:set-app-icon', icon)
  },
  onEvent: (listener: (event: AppEvent) => void) => {
    const handler = (_event: Electron.IpcRendererEvent, payload: AppEvent): void => listener(payload)
    ipcRenderer.on('app:event', handler)
    return () => ipcRenderer.removeListener('app:event', handler)
  }
}

contextBridge.exposeInMainWorld('api', api)
