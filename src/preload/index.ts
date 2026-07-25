import { contextBridge, ipcRenderer } from 'electron'
import type { AppApi, AppEvent, AppIconId, BookmarkInput, ConnectInput, ConflictResolution, TerminalSettings, TransferRequest } from '../shared/types'

const api: AppApi = {
  platform: process.platform,
  bookmarks: {
    list: () => ipcRenderer.invoke('bookmarks:list'),
    save: (input: BookmarkInput & { id?: string }) => ipcRenderer.invoke('bookmarks:save', input),
    remove: (id: string) => ipcRenderer.invoke('bookmarks:remove', id),
    forgetCredential: (id: string) => ipcRenderer.invoke('bookmarks:forget-credential', id)
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
  state: {
    getOpenBookmarks: () => ipcRenderer.invoke('state:get-open-bookmarks'),
    setOpenBookmarks: (ids: string[]) => ipcRenderer.invoke('state:set-open-bookmarks', ids),
    getSidebarCollapsed: () => ipcRenderer.invoke('state:get-sidebar-collapsed'),
    setSidebarCollapsed: (collapsed: boolean) => ipcRenderer.invoke('state:set-sidebar-collapsed', collapsed)
  },
  settings: {
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
