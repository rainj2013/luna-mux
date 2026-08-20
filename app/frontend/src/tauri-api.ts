import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWebview } from '@tauri-apps/api/webview'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { readImage, readText, writeText } from '@tauri-apps/plugin-clipboard-manager'
import type { AiSettingsInput, AiShell, AppApi, AppEvent, AppIconId, AppLanguage, BookmarkInput, BrowserRuntimeEvent, ConnectInput, ConflictResolution, ControlStateChangedEvent, DeploymentProfile, LunaRemoteImportSelection, ManagedAgentEvent, ManagedAgentNotificationActivation, NativeMenuLabels, Platform, PortForwardProfile, TerminalSettings, TransferRequest, UiTheme } from './types'
import type { TerminalRuntimeEvent } from './terminal-runtime-contract'

const call = <T>(command: string, args?: Record<string, unknown>): Promise<T> => invoke<T>(command, args).catch((error) => {
  throw new Error(typeof error === 'string' ? error : String(error))
})

const send = (command: string, args?: Record<string, unknown>): void => {
  void call(command, args).catch((error) => console.warn(`Tauri command ${command} failed`, error))
}

const readClipboardContent = async (): Promise<import('./types').ClipboardContent> => {
  try {
    const image = await readImage()
    try {
      const { width, height } = await image.size()
      if (width > 0 && height > 0) return { type: 'image' }
    } finally {
      await image.close()
    }
  } catch {
    // File drops and text-only clipboard contents commonly make image reads reject.
  }
  try {
    if (await call<boolean>('system_clipboard_has_image_file')) return { type: 'image' }
  } catch {
    // Keep text paste available if native file-drop inspection is unavailable.
  }
  try {
    const text = await readText()
    if (text) return { type: 'text', text }
  } catch {
    // Unsupported clipboard formats are treated as empty.
  }
  return { type: 'empty' }
}

export async function createTauriApi(): Promise<AppApi> {
  const platform = await call<Platform>('platform')
  const currentWindow = getCurrentWindow()
  await getCurrentWebview().onDragDropEvent((event) => {
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
      readClipboard: readClipboardContent,
      writeClipboard: (text) => writeText(text.slice(0, 4 * 1024 * 1024)),
      minimizeWindow: () => currentWindow.minimize(),
      toggleMaximizeWindow: () => currentWindow.toggleMaximize(),
      closeWindow: () => currentWindow.close(),
    },
    bookmarks: {
      list: () => call('bookmarks_list'),
      save: (input: BookmarkInput & { id?: string }) => call('bookmarks_save', { input }),
      reorder: (ids) => call('bookmarks_reorder', { ids }),
      moveToGroup: (id, groupName) => call('bookmarks_move_to_group', { id, groupName }),
      duplicate: (id) => call('bookmarks_duplicate', { id }),
      remove: (id) => call('bookmarks_remove', { id }),
      forgetCredential: (id) => call('bookmarks_forget_credential', { id }),
      previewSshConfig: () => call('bookmarks_preview_ssh_config'),
      importSshConfig: (path, aliases) => call('bookmarks_import_ssh_config', { path, aliases }),
      exportArchive: () => call('bookmarks_export_archive'),
      previewArchive: () => call('bookmarks_preview_archive'),
      importArchive: (previewId, connectionIds, groups) => call('bookmarks_import_archive', { previewId, connectionIds, groups }),
      discoverLunaRemoteSources: () => call('bookmarks_discover_luna_remote_sources'),
      previewLunaRemote: (path) => call('bookmarks_preview_luna_remote', { path }),
      chooseLunaRemoteDatabase: () => call('bookmarks_choose_luna_remote_database'),
      importLunaRemote: (selection: LunaRemoteImportSelection) => call('bookmarks_import_luna_remote', { selection })
    },
    muxSessions: {
      list: () => call('mux_sessions_list'),
      save: (input) => call('mux_sessions_save', { input }),
      remove: (id) => call('mux_sessions_remove', { id })
    },
    muxPanes: {
      list: (muxSessionId) => call('mux_panes_list', { muxSessionId }),
      save: (input) => call('mux_panes_save', { input }),
      remove: (id) => call('mux_panes_remove', { id })
    },
    browserResources: {
      list: (muxSessionId) => call('browser_resources_list', { muxSessionId }),
      save: (input) => call('browser_resources_save', { input }),
      remove: (id) => call('browser_resources_remove', { id })
    },
    browserRuntimes: {
      discoverChrome: () => call('browser_chrome_discover'),
      create: (request) => call('browser_runtime_create', { request }),
      list: () => call('browser_runtimes_list'),
      close: (runtimeId) => call('browser_runtime_close', { runtimeId }),
      navigate: (runtimeId, url) => call('browser_runtime_navigate', { runtimeId, url }),
      focusExternal: (runtimeId) => call('browser_runtime_focus_external', { runtimeId }),
      resize: (runtimeId, width, height) => call('browser_runtime_resize', { runtimeId, width, height }),
      mouse: (runtimeId, event) => call('browser_runtime_mouse', { runtimeId, event }),
      key: (runtimeId, event) => call('browser_runtime_key', { runtimeId, event })
    },
    bookmarkGroups: {
      list: () => call('bookmark_groups_list'),
      create: (name) => call('bookmark_groups_create', { name }),
      rename: (oldName, newName) => call('bookmark_groups_rename', { oldName, newName }),
      delete: (name) => call('bookmark_groups_delete', { name }),
      reorder: (groups) => call('bookmark_groups_reorder', { groups })
    },
    sessions: {
      connect: (input: ConnectInput) => call('sessions_connect', { input }),
      disconnect: (id) => call('sessions_disconnect', { id }),
      write: (id, data) => send('sessions_write', { id, data }),
      writeCommand: (id, data) => call('sessions_write', { id, data }),
      resize: (id, cols, rows) => send('sessions_resize', { id, cols, rows }),
      flow: (id, paused) => send('sessions_flow', { id, paused }),
      hostKeyDecision: (id, accept) => send('sessions_host_key_decision', { id, accept })
    },
    terminalRuntimes: {
      targets: () => call('terminal_targets_list'),
      list: () => call('terminal_runtimes_list'),
      create: (request) => call('terminal_runtime_create', { request }),
      readOutput: (runtimeId, fromCursor, maxBytes) => call('terminal_runtime_read_output', { runtimeId, fromCursor, maxBytes }),
      write: (runtimeId, data) => call('terminal_runtime_write', { runtimeId, data }),
      resize: (runtimeId, cols, rows) => call('terminal_runtime_resize', { runtimeId, cols, rows }),
      flow: (runtimeId, paused) => call('terminal_runtime_flow', { runtimeId, paused }),
      interrupt: (runtimeId) => call('terminal_runtime_interrupt', { runtimeId }),
      close: (runtimeId) => call('terminal_runtime_close', { runtimeId })
    },
    managedAgents: {
      profiles: () => call('managed_agent_profiles_list'),
      availability: (profileId, targetId) => call('managed_agent_profile_availability', { profileId, targetId }),
      events: () => call('managed_agents_events'),
      setNotificationFocus: (muxSessionId, paneId, terminalVisible) => call('managed_agents_set_notification_focus', { muxSessionId, paneId, terminalVisible })
    },
    controlAudit: {
      list: (limit) => call('control_audit_list', { limit }),
      clear: () => call('control_audit_clear')
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
      chooseLocalDirectory: () => call('files_choose_local_directory'),
      choosePrivateKey: () => call('files_choose_private_key')
    },
    transfers: {
      list: () => call('transfers_list'),
      enqueue: (request: TransferRequest) => call('transfers_enqueue', { request }),
      cancel: (id) => call('transfers_cancel', { id }),
      retry: (id, sessionId) => call('transfers_retry', { id, sessionId }),
      clearCompleted: () => call('transfers_clear_completed'),
      resolveConflict: (id, resolution: ConflictResolution, applyToBatch) => send('transfers_resolve_conflict', { id, resolution, applyToBatch })
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
      startBrowser: (sessionId, browserResourceId, sourcePaneId, remoteUrl) => call('browser_tunnel_start', { sessionId, browserResourceId, sourcePaneId, remoteUrl }),
      stop: (sessionId, tunnelId) => call('tunnels_stop', { sessionId, tunnelId })
    },
    diagnostics: {
      run: (filter) => call('diagnostics_run', filter === undefined ? {} : { filter }),
      repair: (runtimeId, action) => call('diagnostics_repair', { runtimeId, action }),
      export: () => call('diagnostics_export')
    },
    ai: {
      getSettings: () => call('ai_settings_get'),
      saveSettings: (settings: AiSettingsInput) => call('ai_settings_save', { settings }),
      deleteApiKey: () => call('ai_settings_delete_key'),
      testSettings: (settings: AiSettingsInput) => call('ai_settings_test', { settings }),
      generate: (requirement, shell: AiShell, terminalContext, redactTerminalContext) => call('ai_command_generate', { request: { requirement, shell, terminalContext, redactTerminalContext } }),
      analyze: (command) => call('ai_command_analyze', { command }),
      listHistory: () => call('ai_command_history_list'),
      clearHistory: () => call('ai_command_history_clear'),
      getLastExchange: () => call('ai_diagnostics_get'),
      clearLastExchange: () => call('ai_diagnostics_clear')
    },
    state: {
      getSidebarCollapsed: () => call('state_get_sidebar_collapsed'),
      setSidebarCollapsed: (collapsed) => call('state_set_sidebar_collapsed', { collapsed }),
      getCollapsedBookmarkGroups: () => call('state_get_collapsed_bookmark_groups'),
      setCollapsedBookmarkGroups: (groups) => call('state_set_collapsed_bookmark_groups', { groups }),
      getSidebarWidth: () => call('state_get_sidebar_width'),
      setSidebarWidth: (width) => call('state_set_sidebar_width', { width })
    },
    settings: {
      getLanguage: () => call('settings_get_language'),
      applyLanguage: (menu: NativeMenuLabels) => call('settings_apply_language', { menu }),
      saveLanguage: (language: AppLanguage, menu: NativeMenuLabels) => call('settings_save_language', { language, menu }),
      getUiTheme: () => call('settings_get_ui_theme'),
      saveUiTheme: (theme: UiTheme) => call('settings_save_ui_theme', { theme }),
      getRemoteAgentIntegrationEnabled: () => call('settings_get_remote_agent_integration_enabled'),
      saveRemoteAgentIntegrationEnabled: (enabled: boolean) => call('settings_save_remote_agent_integration_enabled', { enabled }),
      getTerminal: () => call('settings_get_terminal'),
      saveTerminal: (settings: TerminalSettings) => call('settings_save_terminal', { settings }),
      listSystemFonts: () => call('settings_list_system_fonts'),
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
      }).catch((error) => console.error('Failed to register Tauri event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onTerminalRuntimeEvent: (listener: (event: TerminalRuntimeEvent) => void) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<TerminalRuntimeEvent>('terminal-runtime:event', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register terminal runtime event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onManagedAgentEvent: (listener: (event: ManagedAgentEvent) => void) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<ManagedAgentEvent>('managed-agent:event', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register managed agent event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onManagedAgentNotificationActivate: (listener: (event: ManagedAgentNotificationActivation) => void) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<ManagedAgentNotificationActivation>('managed-agent:activate-pane', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register Agent notification activation listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onBrowserRuntimeEvent: (listener) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<BrowserRuntimeEvent>('browser-runtime:event', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register browser runtime event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onUiThemeChanged: (listener) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<UiTheme>('ui-theme:changed', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register UI theme event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onTerminalSettingsChanged: (listener) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<TerminalSettings>('terminal-settings:changed', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register terminal settings event listener', error))
      return () => { stopped = true; unlisten?.() }
    },
    onControlStateChanged: (listener) => {
      let stopped = false
      let unlisten: (() => void) | undefined
      void listen<ControlStateChangedEvent>('control-state:changed', (event) => { if (!stopped) listener(event.payload) }).then((stop) => {
        if (stopped) stop(); else unlisten = stop
      }).catch((error) => console.error('Failed to register control state event listener', error))
      return () => { stopped = true; unlisten?.() }
    }
  }
}
