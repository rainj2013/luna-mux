import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
import type { AppApi, AppEvent, AppIconId, BookmarkInput, ConnectInput, ConflictResolution, DeploymentProfile, PortForwardProfile, TerminalSettings, TransferRequest, UiTheme } from './types'

const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => invoke<T>(command, args).catch((error) => {
  throw new Error(typeof error === 'string' ? error : String(error))
})

export async function createTauriApi(): Promise<AppApi> {
  const platform = await call<NodeJS.Platform>('platform')
  void getCurrentWebview().onDragDropEvent((event) => {
    if (event.payload.type !== 'drop') return
    const scale = window.devicePixelRatio || 1
    window.dispatchEvent(new CustomEvent('tauri-file-drop', { detail: {
      paths: event.payload.paths,
      x: event.payload.position.x / scale,
      y: event.payload.position.y / scale
    } }))
  })
  return {
    platform,
    system: {
      openExternal: (url) => call('system_open_external', { value: url }),
      readClipboard: () => readText(),
      writeClipboard: (text) => writeText(text.slice(0, 4 * 1024 * 1024))
    },
    bookmarks: {
      list: () => call('bookmarks_list'),
      save: (input: BookmarkInput & { id?: string }) => call('bookmarks_save', { input }),
      remove: (id) => call('bookmarks_remove', { id }),
      forgetCredential: (id) => call('bookmarks_forget_credential', { id }),
      previewSshConfig: () => call('bookmarks_preview_ssh_config'),
      importSshConfig: (path, aliases) => call('bookmarks_import_ssh_config', { path, aliases })
    },
    sessions: {
      list: () => call('sessions_list'),
      connect: (input: ConnectInput) => call('sessions_connect', { input }),
      disconnect: (id) => call('sessions_disconnect', { id }),
      write: (id, data) => { void call('sessions_write', { id, data }) },
      resize: (id, cols, rows) => { void call('sessions_resize', { id, cols, rows }) },
      flow: (id, paused) => { void call('sessions_flow', { id, paused }) },
      hostKeyDecision: (id, accept) => { void call('sessions_host_key_decision', { id, accept }) }
    },
    files: {
      home: () => call('files_home'),
      parentLocal: (path) => call('files_parent_local', { path }),
      remoteHome: (sessionId) => call('files_remote_home', { sessionId }),
      listLocal: (path) => call('files_list_local', { path }),
      listRemote: (sessionId, path) => call('files_list_remote', { sessionId, path }),
      createDirectory: (remote, sessionId, path) => call('files_create_directory', { remote, sessionId, path }),
      rename: (remote, sessionId, from, to) => call('files_rename', { remote, sessionId, from, to }),
      remove: (remote, sessionId, paths) => call('files_remove', { remote, sessionId, paths }),
      preview: (remote, sessionId, path, position) => call('files_preview', { remote, sessionId, path, position }),
      getFavorites: (bookmarkId) => call('files_get_favorites', { bookmarkId }),
      setFavorites: (bookmarkId, value) => call('files_set_favorites', { bookmarkId, value }),
      filePath: () => '',
      chooseLocalDirectory: () => call('files_choose_local_directory'),
      choosePrivateKey: () => call('files_choose_private_key')
    },
    transfers: {
      list: () => call('transfers_list'),
      enqueue: (request: TransferRequest) => call('transfers_enqueue', { request }),
      cancel: (id) => call('transfers_cancel', { id }),
      retry: (id, sessionId) => call('transfers_retry', { id, sessionId }),
      clearCompleted: () => call('transfers_clear_completed'),
      resolveConflict: (id, resolution: ConflictResolution, applyToBatch) => { void call('transfers_resolve_conflict', { id, resolution, applyToBatch }) }
    },
    deployments: {
      list: (bookmarkId) => call('deployments_list', { bookmarkId }),
      save: (profile: DeploymentProfile) => call('deployments_save', { profile }),
      remove: (id) => call('deployments_remove', { id }),
      preview: (id, sessionId) => call('deployments_preview', { id, sessionId }),
      execute: (id, sessionId) => call('deployments_execute', { id, sessionId })
    },
    tunnels: {
      listProfiles: (bookmarkId) => call('tunnels_list_profiles', { bookmarkId }),
      saveProfile: (profile: PortForwardProfile) => call('tunnels_save_profile', { profile }),
      removeProfile: (id) => call('tunnels_remove_profile', { id }),
      list: (sessionId) => call('tunnels_list', { sessionId }),
      start: (sessionId, profileId) => call('tunnels_start', { sessionId, profileId }),
      stop: (sessionId, tunnelId) => call('tunnels_stop', { sessionId, tunnelId })
    },
    diagnostics: { export: () => call('diagnostics_export') },
    state: {
      getOpenBookmarks: () => call('state_get_open_bookmarks'),
      setOpenBookmarks: (ids) => call('state_set_open_bookmarks', { ids }),
      getSidebarCollapsed: () => call('state_get_sidebar_collapsed'),
      setSidebarCollapsed: (collapsed) => call('state_set_sidebar_collapsed', { collapsed }),
      getSidebarWidth: () => call('state_get_sidebar_width'),
      setSidebarWidth: (width) => call('state_set_sidebar_width', { width })
    },
    settings: {
      getUiTheme: () => call('settings_get_ui_theme'),
      saveUiTheme: (theme: UiTheme) => call('settings_save_ui_theme', { theme }),
      getTerminal: () => call('settings_get_terminal'),
      saveTerminal: (settings: TerminalSettings) => call('settings_save_terminal', { settings }),
      chooseTerminalBackground: () => call('settings_choose_terminal_background'),
      loadTerminalBackground: (path) => call('settings_load_terminal_background', { path }),
      getAppIcons: () => call('settings_get_app_icons'),
      setAppIcon: (icon: AppIconId) => call('settings_set_app_icon', { icon })
    },
    onEvent: (listener: (event: AppEvent) => void) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<AppEvent>('app:event', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      })
      return () => { stopped = true; unlisten?.() }
    }
  }
}
