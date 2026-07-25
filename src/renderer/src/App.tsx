import { useEffect, useMemo, useRef, useState } from 'react'
import { ArrowRight, Bookmark as BookmarkIcon, CirclePlus, Edit3, Eye, EyeOff, FolderOpen, Image as ImageIcon, KeyRound, PanelLeftClose, PanelLeftOpen, Plug, Power, RotateCcw, Search, Server, Settings as SettingsIcon, SquareTerminal, Trash2, X } from 'lucide-react'
import { DEFAULT_TERMINAL_SETTINGS, type AppEvent, type AppIconId, type AppIconSettings, type Bookmark, type BookmarkInput, type ConflictResolution, type ConnectInput, type HostKeyPrompt, type SessionStatus, type TerminalSettings, type TransferTask } from '../../shared/types'
import { TerminalPane } from './components/TerminalPane'
import { SftpPane } from './components/SftpPane'
import { colorWithOpacity, terminalBackgroundStyle } from './terminal-style'

interface WorkspaceTab { key: string; bookmarkId: string; sessionId?: string; status: SessionStatus; error?: string }

const emptyBookmark: BookmarkInput = { name: '', host: '', port: 22, username: '', authType: 'password', privateKeyPath: '', jumpBookmarkId: '', note: '' }
type ConnectionCredentials = Omit<ConnectInput, 'bookmarkId'>

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message.replace(/^Error invoking remote method '[^']+': Error: /, '')
  return String(error)
}

export function App(): React.JSX.Element {
  const [bookmarks, setBookmarks] = useState<Bookmark[]>([])
  const [tabs, setTabs] = useState<WorkspaceTab[]>([])
  const [activeKey, setActiveKey] = useState('')
  const [query, setQuery] = useState('')
  const [bookmarkDialog, setBookmarkDialog] = useState<Bookmark | 'new' | null>(null)
  const [authBookmark, setAuthBookmark] = useState<Bookmark | null>(null)
  const [hostPrompts, setHostPrompts] = useState<HostKeyPrompt[]>([])
  const [conflict, setConflict] = useState<{ taskId: string; sourcePath: string; destinationPath: string } | null>(null)
  const [transfers, setTransfers] = useState<TransferTask[]>([])
  const [transferView, setTransferView] = useState<'queue' | 'history'>('queue')
  const [workspaceView, setWorkspaceView] = useState<'terminal' | 'files'>('terminal')
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)
  const [terminalSettings, setTerminalSettings] = useState<TerminalSettings>(DEFAULT_TERMINAL_SETTINGS)
  const [terminalBackground, setTerminalBackground] = useState('')
  const [appIcons, setAppIcons] = useState<AppIconSettings>({ selected: 'ssh', options: [] })
  const [settingsDialog, setSettingsDialog] = useState(false)
  const [toast, setToast] = useState('')
  const stateRestored = useRef(false)

  const showError = (message: string): void => { setToast(message); setTimeout(() => setToast(''), 4500) }
  const reloadBookmarks = async (): Promise<void> => setBookmarks(await window.api.bookmarks.list())

  useEffect(() => {
    void Promise.all([window.api.bookmarks.list(), window.api.transfers.list(), window.api.state.getOpenBookmarks(), window.api.state.getSidebarCollapsed(), window.api.settings.getTerminal(), window.api.settings.getAppIcons()]).then(([items, transferItems, openIds, collapsed, appearance, icons]) => {
      setBookmarks(items); setTransfers(transferItems)
      setSidebarCollapsed(collapsed)
      setTerminalSettings(appearance)
      setAppIcons(icons)
      if (appearance.backgroundImagePath) void window.api.settings.loadTerminalBackground(appearance.backgroundImagePath).then(setTerminalBackground).catch(() => setTerminalBackground(''))
      const restored = openIds.filter((id) => items.some((item) => item.id === id)).map((id) => ({ key: crypto.randomUUID(), bookmarkId: id, status: 'disconnected' as const }))
      stateRestored.current = true
      setTabs(restored); if (restored[0]) setActiveKey(restored[0].key)
    }).catch((error) => showError(errorMessage(error)))
  }, [])

  useEffect(() => { if (stateRestored.current) void window.api.state.setOpenBookmarks(tabs.map((tab) => tab.bookmarkId)) }, [tabs])
  useEffect(() => { if (stateRestored.current) void window.api.state.setSidebarCollapsed(sidebarCollapsed) }, [sidebarCollapsed])

  useEffect(() => window.api.onEvent((event: AppEvent) => {
    if (event.type === 'session') {
      setTabs((current) => current.map((tab) => tab.sessionId === event.payload.id ? { ...tab, status: event.payload.status, error: event.payload.error } : tab))
      if (['disconnected', 'error'].includes(event.payload.status)) setHostPrompts((current) => current.filter((item) => item.sessionId !== event.payload.id))
    }
    if (event.type === 'host-key') setHostPrompts((current) => current.some((item) => item.sessionId === event.payload.sessionId && item.fingerprint === event.payload.fingerprint) ? current : [...current, event.payload])
    if (event.type === 'transfer') {
      setTransfers((current) => [event.payload, ...current.filter((item) => item.id !== event.payload.id)].sort((a, b) => b.createdAt.localeCompare(a.createdAt)))
      if (event.payload.status !== 'conflict') setConflict((current) => current?.taskId === event.payload.id ? null : current)
    }
    if (event.type === 'transfer-conflict') setConflict(event.payload)
  }), [])

  const bookmarkMap = useMemo(() => new Map(bookmarks.map((item) => [item.id, item])), [bookmarks])
  const activeTab = tabs.find((tab) => tab.key === activeKey)
  const activeBookmark = activeTab ? bookmarkMap.get(activeTab.bookmarkId) : undefined
  const filteredBookmarks = bookmarks.filter((item) => `${item.name} ${item.host} ${item.username}`.toLowerCase().includes(query.toLowerCase()))

  const connect = async (bookmark: Bookmark, credentials: ConnectionCredentials = {}): Promise<void> => {
    let key = tabs.find((tab) => tab.bookmarkId === bookmark.id)?.key
    if (!key) { key = crypto.randomUUID(); setTabs((current) => [...current, { key: key!, bookmarkId: bookmark.id, status: 'connecting' }]) }
    else setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: undefined, status: 'connecting', error: undefined } : tab))
    setActiveKey(key); setAuthBookmark(null)
    try {
      const summary = await window.api.sessions.connect({ bookmarkId: bookmark.id, ...credentials })
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: summary.id, status: summary.status, error: undefined } : tab))
      if (credentials.rememberCredential || credentials.rememberJumpCredential) await reloadBookmarks()
    } catch (error) {
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, status: 'error', error: errorMessage(error) } : tab))
      showError(errorMessage(error))
    }
  }

  const openBookmark = (bookmark: Bookmark): void => {
    const existing = tabs.find((tab) => tab.bookmarkId === bookmark.id)
    if (existing?.status === 'connected' || existing?.status === 'connecting') { setActiveKey(existing.key); return }
    const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
    if (bookmark.hasSavedCredential && (!jumpBookmark || jumpBookmark.hasSavedCredential)) void connect(bookmark)
    else setAuthBookmark(bookmark)
  }

  const closeTab = (tab: WorkspaceTab): void => {
    if (tab.sessionId) void window.api.sessions.disconnect(tab.sessionId)
    setTabs((current) => {
      const index = current.findIndex((item) => item.key === tab.key)
      const next = current.filter((item) => item.key !== tab.key)
      if (activeKey === tab.key) setActiveKey(next[Math.min(index, next.length - 1)]?.key ?? '')
      return next
    })
  }

  const removeBookmark = async (bookmark: Bookmark): Promise<void> => {
    if (!window.confirm(`确定删除连接“${bookmark.name}”吗？`)) return
    try {
      await window.api.bookmarks.remove(bookmark.id)
      for (const tab of tabs.filter((item) => item.bookmarkId === bookmark.id)) closeTab(tab)
      await reloadBookmarks()
    } catch (error) { showError(errorMessage(error)) }
  }

  const retryTransfer = async (task: TransferTask): Promise<void> => {
    if (!task.bookmarkId) { showError('此旧传输记录缺少连接信息，无法自动重试'); return }
    const tab = tabs.find((item) => item.bookmarkId === task.bookmarkId && item.status === 'connected' && item.sessionId)
    if (!tab?.sessionId) {
      const bookmark = bookmarkMap.get(task.bookmarkId)
      const existing = tabs.find((item) => item.bookmarkId === task.bookmarkId)
      if (existing) setActiveKey(existing.key)
      else if (bookmark) openBookmark(bookmark)
      showError('请先连接此传输任务对应的连接，再点击重试')
      return
    }
    try { await window.api.transfers.retry(task.id, tab.sessionId) }
    catch (error) { showError(errorMessage(error)) }
  }

  const statusText: Record<SessionStatus, string> = { connecting: '连接中', connected: '已连接', disconnected: '未连接', error: '连接失败' }

  const saveSettings = async (settings: TerminalSettings, previewImage: string, appIcon: AppIconId): Promise<void> => {
    try {
      const saved = await window.api.settings.saveTerminal(settings)
      let image = saved.backgroundImagePath === settings.backgroundImagePath ? previewImage : ''
      if (saved.backgroundImagePath && !image) image = await window.api.settings.loadTerminalBackground(saved.backgroundImagePath)
      const selected = await window.api.settings.setAppIcon(appIcon)
      setTerminalSettings(saved)
      setTerminalBackground(image)
      setAppIcons((current) => ({ ...current, selected }))
      setSettingsDialog(false)
    } catch (error) { showError(errorMessage(error)) }
  }

  const selectedAppIcon = appIcons.options.find((item) => item.id === appIcons.selected)

  return <div className={`app-shell platform-${window.api.platform}`}>
    <header className="titlebar">
      <button className="icon-button sidebar-toggle" title={sidebarCollapsed ? '展开连接管理' : '折叠连接管理'} aria-label={sidebarCollapsed ? '展开连接管理' : '折叠连接管理'} aria-expanded={!sidebarCollapsed} onClick={() => setSidebarCollapsed((collapsed) => !collapsed)}>
        {sidebarCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
      </button>
      <div className="app-brand">{selectedAppIcon ? <img className="app-logo" src={selectedAppIcon.dataUrl} alt="" /> : <span className="app-logo app-logo-fallback">&gt;_</span>}<span>SSH Client</span></div>
      <span className="titlebar-spacer" />
      <button className="icon-button titlebar-settings" title="设置" aria-label="设置" onClick={() => setSettingsDialog(true)}><SettingsIcon size={18} /></button>
    </header>
    <div className={`main-layout ${sidebarCollapsed ? 'sidebar-collapsed' : ''}`}>
      <aside className="sidebar">
        <div className="sidebar-heading"><strong>连接管理</strong><div className="sidebar-actions">
          <button className="icon-button" title="新增连接" onClick={() => setBookmarkDialog('new')}><CirclePlus size={18} /></button>
          <button className="icon-button" title="编辑当前连接" disabled={!activeBookmark} onClick={() => activeBookmark && setBookmarkDialog(activeBookmark)}><Edit3 size={17} /></button>
          <button className="icon-button" title="删除当前连接" disabled={!activeBookmark} onClick={() => activeBookmark && void removeBookmark(activeBookmark)}><Trash2 size={17} /></button>
        </div></div>
        <label className="search-box"><Search size={16} /><input placeholder="搜索连接" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
        <div className="bookmark-list">
          {filteredBookmarks.length === 0 && <div className="sidebar-empty"><BookmarkIcon size={24} /><span>{bookmarks.length ? '没有匹配的连接' : '还没有连接'}</span></div>}
          {filteredBookmarks.map((bookmark) => {
            const tab = tabs.find((item) => item.bookmarkId === bookmark.id)
            const jumpBookmark = bookmark.jumpBookmarkId ? bookmarkMap.get(bookmark.jumpBookmarkId) : undefined
            return <button key={bookmark.id} className={`bookmark-item ${activeBookmark?.id === bookmark.id ? 'active' : ''}`} onClick={() => openBookmark(bookmark)}>
              <span className={`server-icon ${tab?.status ?? 'disconnected'}`}><Server size={17} /></span>
              <span className="bookmark-copy"><strong>{bookmark.name}</strong><small title={jumpBookmark ? `通过 ${jumpBookmark.name}` : undefined}>{bookmark.username}@{bookmark.host}:{bookmark.port}{jumpBookmark ? ` · 经 ${jumpBookmark.name}` : ''}</small></span>
              {bookmark.hasSavedCredential && <KeyRound size={13} className="saved-key" />}
            </button>
          })}
        </div>
      </aside>
      <main className={`workspace ${tabs.length && workspaceView === 'files' ? 'file-mode' : ''}`}>
        <div className="tabbar">
          {tabs.map((tab) => { const bookmark = bookmarkMap.get(tab.bookmarkId); return <div key={tab.key} className={`session-tab ${tab.key === activeKey ? 'active' : ''}`}>
            <button className="session-tab-select" title={bookmark ? `${bookmark.username}@${bookmark.host}:${bookmark.port}` : undefined} onClick={() => setActiveKey(tab.key)} onAuxClick={(event) => { if (event.button === 1) closeTab(tab) }}><span className={`status-dot ${tab.status}`} /><span>{bookmark?.name ?? '已删除连接'}</span></button>
            <button className="session-tab-close" title="关闭会话" aria-label={`关闭 ${bookmark?.name ?? '会话'}`} onClick={() => closeTab(tab)}><X size={14} /></button>
          </div> })}
          <button className="icon-button tab-add" title="新增连接" onClick={() => setBookmarkDialog('new')}><CirclePlus size={17} /></button>
        </div>
        {tabs.length === 0 ? <div className="welcome-state"><div className="welcome-icon"><Plug size={30} /></div><h2>选择一个 SSH 连接</h2><p>连接后可在同一窗口使用终端和 SFTP 文件传输。</p><button className="primary-button" onClick={() => setBookmarkDialog('new')}><CirclePlus size={16} />新增连接</button></div> : <>
          <div className="session-toolbar"><div className="session-info"><span className={`status-dot ${activeTab?.status}`} /><span className="session-status">{activeTab ? statusText[activeTab.status] : ''}</span>{activeBookmark && <span className="session-endpoint" title={`${activeBookmark.username}@${activeBookmark.host}:${activeBookmark.port}`}>{activeBookmark.username}@{activeBookmark.host}:{activeBookmark.port}</span>}{activeTab?.error && <span className="error-text" title={activeTab.error}>{activeTab.error}</span>}</div>
            <div className="view-switcher" role="tablist" aria-label="会话视图">
              <button role="tab" aria-selected={workspaceView === 'terminal'} className={workspaceView === 'terminal' ? 'active' : ''} onClick={() => setWorkspaceView('terminal')}><SquareTerminal size={15} />终端</button>
              <button role="tab" aria-selected={workspaceView === 'files'} className={workspaceView === 'files' ? 'active' : ''} onClick={() => setWorkspaceView('files')}><FolderOpen size={15} />文件传输</button>
            </div><div className="session-actions">
            {activeTab?.status === 'connected' ? <button className="secondary-button" onClick={() => activeTab.sessionId && void window.api.sessions.disconnect(activeTab.sessionId)}><Power size={15} />断开</button>
              : activeBookmark && <button className="primary-button" disabled={activeTab?.status === 'connecting'} onClick={() => openBookmark(activeBookmark)}><Plug size={15} />连接</button>}
          </div></div>
          <div className="session-stack">
            {tabs.map((tab) => { const bookmark = bookmarkMap.get(tab.bookmarkId); const reconnect = bookmark ? () => openBookmark(bookmark) : undefined; return <div key={tab.key} className={`session-view ${tab.key === activeKey ? 'active' : ''} ${workspaceView}`}>
              <div className="terminal-region"><TerminalPane sessionId={tab.sessionId} visible={tab.key === activeKey && workspaceView === 'terminal'} settings={terminalSettings} backgroundImage={terminalBackground} onConnect={reconnect} /></div>
              <div className="sftp-region"><SftpPane sessionId={tab.sessionId} connected={tab.status === 'connected'} visible={tab.key === activeKey && workspaceView === 'files'} onError={showError} onConnect={reconnect} /></div>
            </div> })}
          </div>
        </>}
        {tabs.length > 0 && workspaceView === 'files' && <TransferPanel transfers={transfers} view={transferView} setView={setTransferView} onRetry={(task) => void retryTransfer(task)} onClear={async () => { await window.api.transfers.clearCompleted(); setTransfers((current) => current.filter((item) => !['completed', 'cancelled'].includes(item.status))) }} />}
      </main>
    </div>
    {bookmarkDialog && <BookmarkDialog bookmark={bookmarkDialog === 'new' ? undefined : bookmarkDialog} connections={bookmarks} onClose={() => setBookmarkDialog(null)} onSaved={async () => { setBookmarkDialog(null); await reloadBookmarks() }} onError={showError} />}
    {authBookmark && <AuthDialog bookmark={authBookmark} jumpBookmark={authBookmark.jumpBookmarkId ? bookmarkMap.get(authBookmark.jumpBookmarkId) : undefined} onClose={() => setAuthBookmark(null)} onConnect={(credentials) => void connect(authBookmark, credentials)} />}
    {hostPrompts[0] && <HostKeyDialog prompt={hostPrompts[0]} onDecision={(accept) => { const prompt = hostPrompts[0]; if (!prompt) return; window.api.sessions.hostKeyDecision(prompt.sessionId, accept); setHostPrompts((current) => current.filter((item) => item !== prompt)) }} />}
    {conflict && <ConflictDialog conflict={conflict} onDecision={(resolution, apply) => { window.api.transfers.resolveConflict(conflict.taskId, resolution, apply); setConflict(null) }} />}
    {settingsDialog && <SettingsDialog settings={terminalSettings} backgroundImage={terminalBackground} appIcons={appIcons} onClose={() => setSettingsDialog(false)} onSave={(settings, image, icon) => void saveSettings(settings, image, icon)} onError={showError} />}
    {toast && <div className="toast">{toast}</div>}
  </div>
}

function SettingsDialog({ settings, backgroundImage, appIcons, onClose, onSave, onError }: { settings: TerminalSettings; backgroundImage: string; appIcons: AppIconSettings; onClose(): void; onSave(settings: TerminalSettings, backgroundImage: string, appIcon: AppIconId): void; onError(message: string): void }): React.JSX.Element {
  const [form, setForm] = useState<TerminalSettings>({ ...settings })
  const [previewImage, setPreviewImage] = useState(backgroundImage)
  const [appIcon, setAppIcon] = useState(appIcons.selected)
  const [section, setSection] = useState<'terminal' | 'application'>('terminal')
  const set = <K extends keyof TerminalSettings>(key: K, value: TerminalSettings[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const chooseBackground = async (): Promise<void> => {
    try {
      const path = await window.api.settings.chooseTerminalBackground()
      if (!path) return
      const image = await window.api.settings.loadTerminalBackground(path)
      set('backgroundImagePath', path)
      setPreviewImage(image)
    } catch (error) { onError(errorMessage(error)) }
  }
  const reset = (): void => { setForm({ ...DEFAULT_TERMINAL_SETTINGS }); setPreviewImage(''); setAppIcon('ssh') }
  return <Modal title="设置" onClose={onClose}><form className="form-grid terminal-settings-form" onSubmit={(event) => { event.preventDefault(); onSave(form, previewImage, appIcon) }}>
    <div className="settings-tabs" role="tablist" aria-label="设置类别"><button type="button" role="tab" aria-selected={section === 'terminal'} className={section === 'terminal' ? 'active' : ''} onClick={() => setSection('terminal')}><SquareTerminal size={15} />终端</button><button type="button" role="tab" aria-selected={section === 'application'} className={section === 'application' ? 'active' : ''} onClick={() => setSection('application')}><ImageIcon size={15} />应用图标</button></div>
    {section === 'application' ? <fieldset className="app-icon-settings"><legend>选择应用图标</legend><div className="app-icon-options">{appIcons.options.map((icon) => <label key={icon.id} className={appIcon === icon.id ? 'selected' : ''}><input type="radio" name="app-icon" checked={appIcon === icon.id} onChange={() => setAppIcon(icon.id)} /><img src={icon.dataUrl} alt="" /><span>{icon.name}</span></label>)}</div></fieldset> : <>
      <div className="terminal-preview" style={terminalBackgroundStyle(form, previewImage)}><div style={{ color: form.foregroundColor, background: colorWithOpacity(form.backgroundColor, form.backgroundOpacity), fontFamily: form.fontFamily, fontSize: form.fontSize }}><span>user@host:~$</span> ls -la<br />drwxr-xr-x&nbsp;&nbsp;projects</div></div>
      <label>字体<select value={form.fontFamily} onChange={(event) => set('fontFamily', event.target.value)}><option value="Cascadia Mono, SFMono-Regular, Consolas, monospace">默认等宽字体</option><option value="Cascadia Mono, Consolas, monospace">Cascadia Mono</option><option value="SFMono-Regular, Menlo, Monaco, monospace">SF Mono</option><option value="JetBrains Mono, monospace">JetBrains Mono</option><option value="Fira Code, monospace">Fira Code</option><option value="Consolas, monospace">Consolas</option><option value="Menlo, monospace">Menlo</option><option value="monospace">系统等宽字体</option></select></label>
      <label>字号<div className="range-control"><input type="range" min="10" max="30" step="1" value={form.fontSize} onChange={(event) => set('fontSize', Number(event.target.value))} /><output>{form.fontSize} px</output></div></label>
      <div className="color-settings"><label>文字颜色<div className="color-control"><input type="color" value={form.foregroundColor} onChange={(event) => set('foregroundColor', event.target.value)} /><span>{form.foregroundColor}</span></div></label><label>背景颜色<div className="color-control"><input type="color" value={form.backgroundColor} onChange={(event) => set('backgroundColor', event.target.value)} /><span>{form.backgroundColor}</span></div></label></div>
      <label>背景颜色不透明度<div className="range-control"><input type="range" min="0" max="1" step="0.05" value={form.backgroundOpacity} onChange={(event) => set('backgroundOpacity', Number(event.target.value))} /><output>{Math.round(form.backgroundOpacity * 100)}%</output></div></label>
      <label>背景图片<div className="input-button background-image-input"><input readOnly value={form.backgroundImagePath} placeholder="未选择" /><button type="button" className="secondary-button" onClick={() => void chooseBackground()}><ImageIcon size={15} />选择</button><button type="button" className="icon-button background-clear" title="清除背景图片" disabled={!form.backgroundImagePath} onClick={() => { set('backgroundImagePath', ''); setPreviewImage('') }}><Trash2 size={15} /></button></div></label>
      <label>图片显示方式<select value={form.backgroundImageFit} onChange={(event) => set('backgroundImageFit', event.target.value as TerminalSettings['backgroundImageFit'])}><option value="cover">覆盖窗口</option><option value="contain">完整显示</option><option value="stretch">拉伸</option><option value="tile">平铺</option></select></label>
    </>}
    <div className="dialog-actions spread"><button type="button" className="secondary-button" onClick={reset}><RotateCcw size={15} />恢复默认</button><div><button type="button" className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" type="submit">保存</button></div></div>
  </form></Modal>
}

function BookmarkDialog({ bookmark, connections, onClose, onSaved, onError }: { bookmark?: Bookmark; connections: Bookmark[]; onClose(): void; onSaved(): void; onError(message: string): void }): React.JSX.Element {
  const [form, setForm] = useState<BookmarkInput>(bookmark ? { name: bookmark.name, host: bookmark.host, port: bookmark.port, username: bookmark.username, authType: bookmark.authType, privateKeyPath: bookmark.privateKeyPath, jumpBookmarkId: bookmark.jumpBookmarkId, note: bookmark.note } : emptyBookmark)
  const set = <K extends keyof BookmarkInput>(key: K, value: BookmarkInput[K]): void => setForm((current) => ({ ...current, [key]: value }))
  const jumpCandidates = connections.filter((item) => item.id !== bookmark?.id && !item.jumpBookmarkId)
  return <Modal title={bookmark ? '编辑连接' : '新增连接'} onClose={onClose}><form className="form-grid" onSubmit={async (event) => { event.preventDefault(); try { await window.api.bookmarks.save({ ...form, id: bookmark?.id }); onSaved() } catch (error) { onError(errorMessage(error)) } }}>
    <label>连接名称<input required value={form.name} onChange={(event) => set('name', event.target.value)} placeholder="Production 服务器" /></label>
    <div className="form-row"><label>Host（IP 或域名）<input required value={form.host} onChange={(event) => set('host', event.target.value)} placeholder="192.168.1.10" /></label><label className="port-field">端口<input type="number" min="1" max="65535" value={form.port} onChange={(event) => set('port', Number(event.target.value))} /></label></div>
    <label>用户名<input required value={form.username} onChange={(event) => set('username', event.target.value)} placeholder="root" /></label>
    <label>认证方式<select value={form.authType} onChange={(event) => set('authType', event.target.value as BookmarkInput['authType'])}><option value="password">密码</option><option value="privateKey">私钥</option></select></label>
    {form.authType === 'privateKey' && <label>私钥文件<div className="input-button"><input readOnly value={form.privateKeyPath} placeholder="选择 OpenSSH 私钥" /><button type="button" className="secondary-button" onClick={async () => { const path = await window.api.files.choosePrivateKey(); if (path) set('privateKeyPath', path) }}>选择</button></div></label>}
    <label>连接方式<select value={form.jumpBookmarkId} onChange={(event) => set('jumpBookmarkId', event.target.value)}><option value="">直接连接</option>{jumpCandidates.map((item) => <option key={item.id} value={item.id}>通过 {item.name}（{item.username}@{item.host}）</option>)}</select></label>
    <label>备注<textarea rows={3} value={form.note} onChange={(event) => set('note', event.target.value)} placeholder="可选" /></label>
    {bookmark?.hasSavedCredential && <button type="button" className="text-button danger" onClick={async () => { await window.api.bookmarks.forgetCredential(bookmark.id); onSaved() }}>清除已保存的凭据</button>}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" type="submit">保存</button></div>
  </form></Modal>
}

function AuthDialog({ bookmark, jumpBookmark, onClose, onConnect }: { bookmark: Bookmark; jumpBookmark?: Bookmark; onClose(): void; onConnect(credentials: ConnectionCredentials): void }): React.JSX.Element {
  const [secret, setSecret] = useState(''); const [remember, setRemember] = useState(false); const [visible, setVisible] = useState(false)
  const [jumpSecret, setJumpSecret] = useState(''); const [rememberJump, setRememberJump] = useState(false); const [jumpVisible, setJumpVisible] = useState(false)
  const required = bookmark.authType === 'password' && !bookmark.hasSavedCredential
  const jumpRequired = Boolean(jumpBookmark?.authType === 'password' && !jumpBookmark.hasSavedCredential)
  return <Modal title={`连接到 ${bookmark.name}`} onClose={onClose}><form className="form-grid" onSubmit={(event) => { event.preventDefault(); if ((required && !secret) || (jumpRequired && !jumpSecret)) return; onConnect({ credential: secret || undefined, rememberCredential: remember, jumpCredential: jumpSecret || undefined, rememberJumpCredential: rememberJump }) }}>
    {jumpBookmark ? <div className="connection-route"><span><small>跳板机</small><strong>{jumpBookmark.name}</strong></span><ArrowRight size={16} /><span><small>目标机器</small><strong>{bookmark.name}</strong></span></div> : <div className="connection-target"><Server size={18} /><strong>{bookmark.username}@{bookmark.host}:{bookmark.port}</strong></div>}
    {jumpBookmark && <CredentialField bookmark={jumpBookmark} role="跳板机" secret={jumpSecret} setSecret={setJumpSecret} remember={rememberJump} setRemember={setRememberJump} visible={jumpVisible} setVisible={setJumpVisible} autoFocus />}
    <CredentialField bookmark={bookmark} role={jumpBookmark ? '目标机器' : ''} secret={secret} setSecret={setSecret} remember={remember} setRemember={setRemember} visible={visible} setVisible={setVisible} autoFocus={!jumpBookmark} />
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button type="submit" className="primary-button">连接</button></div>
  </form></Modal>
}

function CredentialField({ bookmark, role, secret, setSecret, remember, setRemember, visible, setVisible, autoFocus }: { bookmark: Bookmark; role: string; secret: string; setSecret(value: string): void; remember: boolean; setRemember(value: boolean): void; visible: boolean; setVisible(value: boolean): void; autoFocus: boolean }): React.JSX.Element {
  const credentialName = bookmark.authType === 'password' ? '密码' : '私钥口令（可选）'
  return <div className="credential-block"><div className="credential-heading"><strong>{role || '连接'}凭据</strong><span>{bookmark.username}@{bookmark.host}:{bookmark.port}</span></div>
    <label>{role}{credentialName}<div className="password-input"><input autoFocus={autoFocus} type={visible ? 'text' : 'password'} required={bookmark.authType === 'password' && !bookmark.hasSavedCredential} value={secret} placeholder={bookmark.hasSavedCredential ? '已保存，不填写则继续使用' : undefined} onChange={(event) => setSecret(event.target.value)} /><button className="icon-button" type="button" title={visible ? '隐藏密码' : '显示密码'} onClick={() => setVisible(!visible)}>{visible ? <EyeOff size={16} /> : <Eye size={16} />}</button></div></label>
    <label className="check-label"><input type="checkbox" checked={remember} disabled={!secret} onChange={(event) => setRemember(event.target.checked)} />记住{role || '连接'}凭据（使用系统安全存储）</label>
  </div>
}

function HostKeyDialog({ prompt, onDecision }: { prompt: HostKeyPrompt; onDecision(accept: boolean): void }): React.JSX.Element {
  const changed = prompt.status === 'changed'
  return <Modal title={changed ? '主机指纹已变化' : '确认主机指纹'} onClose={() => onDecision(false)}><div className="dialog-copy">
    <p>{changed ? '服务器返回的主机指纹与此前记录不一致。除非你确认服务器密钥已更换，否则不要继续。' : '这是首次连接该服务器。确认指纹与服务器管理员提供的信息一致后再继续。'}</p>
    <dl><dt>服务器</dt><dd>{prompt.host}:{prompt.port}</dd>{prompt.previousFingerprint && <><dt>原指纹</dt><dd>{prompt.previousFingerprint}</dd></>}<dt>当前指纹</dt><dd>{prompt.fingerprint}</dd></dl>
    <div className="dialog-actions"><button className="secondary-button" onClick={() => onDecision(false)}>取消连接</button><button className={changed ? 'danger-button' : 'primary-button'} onClick={() => onDecision(true)}>信任并继续</button></div>
  </div></Modal>
}

function ConflictDialog({ conflict, onDecision }: { conflict: { sourcePath: string; destinationPath: string }; onDecision(value: ConflictResolution, apply: boolean): void }): React.JSX.Element {
  const [apply, setApply] = useState(false)
  return <Modal title="目标位置已有同名文件" onClose={() => onDecision('skip', false)}><div className="dialog-copy"><p className="path-line">{conflict.destinationPath}</p><label className="check-label"><input type="checkbox" checked={apply} onChange={(event) => setApply(event.target.checked)} />应用到当前批次中的所有冲突</label><div className="dialog-actions spread"><button className="secondary-button" onClick={() => onDecision('skip', apply)}>跳过</button><button className="secondary-button" onClick={() => onDecision('rename', apply)}>自动重命名</button><button className="danger-button" onClick={() => onDecision('overwrite', apply)}>覆盖</button></div></div></Modal>
}

function Modal({ title, onClose, children }: { title: string; onClose(): void; children: React.ReactNode }): React.JSX.Element {
  return <div className="modal-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}><section className="modal" role="dialog" aria-modal="true"><header><strong>{title}</strong><button className="icon-button" onClick={onClose}><X size={18} /></button></header>{children}</section></div>
}

function TransferPanel({ transfers, view, setView, onRetry, onClear }: { transfers: TransferTask[]; view: 'queue' | 'history'; setView(value: 'queue' | 'history'): void; onRetry(task: TransferTask): void; onClear(): void }): React.JSX.Element {
  const activeStatuses = ['queued', 'scanning', 'running', 'conflict']
  const shown = transfers.filter((item) => view === 'queue' ? activeStatuses.includes(item.status) || item.status === 'failed' : !activeStatuses.includes(item.status))
  return <section className="transfer-panel"><header><div className="transfer-tabs"><button className={view === 'queue' ? 'active' : ''} onClick={() => setView('queue')}>传输队列 <span>{transfers.filter((item) => activeStatuses.includes(item.status)).length}</span></button><button className={view === 'history' ? 'active' : ''} onClick={() => setView('history')}>传输记录</button></div><button className="text-button" onClick={onClear}>清除已完成</button></header>
    <div className="transfer-list">{shown.length === 0 ? <div className="transfer-empty">{view === 'queue' ? '当前没有传输任务' : '暂无传输记录'}</div> : shown.slice(0, 40).map((task) => {
      const percent = task.bytesTotal ? Math.min(100, Math.round(task.bytesTransferred / task.bytesTotal * 100)) : 0
      return <div className="transfer-row" key={task.id}><span className={`transfer-direction ${task.direction}`}>{task.direction === 'upload' ? '↑' : '↓'}</span><div className="transfer-info"><strong>{task.displayName}</strong><small>{task.error ?? `${task.sourcePath} → ${task.destinationPath}`}</small>{['running', 'scanning'].includes(task.status) && <div className="progress"><span style={{ width: `${percent}%` }} /></div>}</div><span className={`transfer-status ${task.status}`}>{task.status === 'running' ? `${percent}%` : ({ queued: '等待中', scanning: '扫描中', conflict: '等待确认', completed: '已完成', failed: '失败', cancelled: '已取消', interrupted: '已中断' } as Record<string, string>)[task.status]}</span><div className="transfer-actions">{activeStatuses.includes(task.status) && <button className="icon-button" title="取消" onClick={() => void window.api.transfers.cancel(task.id)}><X size={15} /></button>}{['failed', 'cancelled', 'interrupted'].includes(task.status) && <button className="text-button" onClick={() => onRetry(task)}>重试</button>}</div></div>
    })}</div>
  </section>
}
