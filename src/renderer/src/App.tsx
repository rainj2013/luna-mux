import { useEffect, useMemo, useState } from 'react'
import { Bookmark as BookmarkIcon, CirclePlus, Edit3, KeyRound, MoreHorizontal, Plug, Power, Search, Server, Trash2, X } from 'lucide-react'
import type { AppEvent, Bookmark, BookmarkInput, ConflictResolution, HostKeyPrompt, SessionStatus, TransferTask } from '../../shared/types'
import { TerminalPane } from './components/TerminalPane'
import { SftpPane } from './components/SftpPane'

interface WorkspaceTab { key: string; bookmarkId: string; sessionId?: string; status: SessionStatus; error?: string }

const emptyBookmark: BookmarkInput = { name: '', host: '', port: 22, username: '', authType: 'password', privateKeyPath: '', note: '' }

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
  const [hostPrompt, setHostPrompt] = useState<HostKeyPrompt | null>(null)
  const [conflict, setConflict] = useState<{ taskId: string; sourcePath: string; destinationPath: string } | null>(null)
  const [transfers, setTransfers] = useState<TransferTask[]>([])
  const [transferView, setTransferView] = useState<'queue' | 'history'>('queue')
  const [toast, setToast] = useState('')

  const showError = (message: string): void => { setToast(message); setTimeout(() => setToast(''), 4500) }
  const reloadBookmarks = async (): Promise<void> => setBookmarks(await window.api.bookmarks.list())

  useEffect(() => {
    void Promise.all([window.api.bookmarks.list(), window.api.transfers.list(), window.api.state.getOpenBookmarks()]).then(([items, transferItems, openIds]) => {
      setBookmarks(items); setTransfers(transferItems)
      const restored = openIds.filter((id) => items.some((item) => item.id === id)).map((id) => ({ key: crypto.randomUUID(), bookmarkId: id, status: 'disconnected' as const }))
      setTabs(restored); if (restored[0]) setActiveKey(restored[0].key)
    }).catch((error) => showError(errorMessage(error)))
  }, [])

  useEffect(() => { void window.api.state.setOpenBookmarks(tabs.map((tab) => tab.bookmarkId)) }, [tabs])

  useEffect(() => window.api.onEvent((event: AppEvent) => {
    if (event.type === 'session') setTabs((current) => current.map((tab) => tab.sessionId === event.payload.id ? { ...tab, status: event.payload.status, error: event.payload.error } : tab))
    if (event.type === 'host-key') setHostPrompt(event.payload)
    if (event.type === 'transfer') setTransfers((current) => [event.payload, ...current.filter((item) => item.id !== event.payload.id)].sort((a, b) => b.createdAt.localeCompare(a.createdAt)))
    if (event.type === 'transfer-conflict') setConflict(event.payload)
  }), [])

  const bookmarkMap = useMemo(() => new Map(bookmarks.map((item) => [item.id, item])), [bookmarks])
  const activeTab = tabs.find((tab) => tab.key === activeKey)
  const activeBookmark = activeTab ? bookmarkMap.get(activeTab.bookmarkId) : undefined
  const filteredBookmarks = bookmarks.filter((item) => `${item.name} ${item.host} ${item.username}`.toLowerCase().includes(query.toLowerCase()))

  const connect = async (bookmark: Bookmark, credential?: string, rememberCredential = false): Promise<void> => {
    let key = tabs.find((tab) => tab.bookmarkId === bookmark.id)?.key
    if (!key) { key = crypto.randomUUID(); setTabs((current) => [...current, { key: key!, bookmarkId: bookmark.id, status: 'connecting' }]) }
    setActiveKey(key); setAuthBookmark(null)
    try {
      const summary = await window.api.sessions.connect({ bookmarkId: bookmark.id, credential, rememberCredential })
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, sessionId: summary.id, status: summary.status, error: undefined } : tab))
      if (rememberCredential) await reloadBookmarks()
    } catch (error) {
      setTabs((current) => current.map((tab) => tab.key === key ? { ...tab, status: 'error', error: errorMessage(error) } : tab))
      showError(errorMessage(error))
    }
  }

  const openBookmark = (bookmark: Bookmark): void => {
    const existing = tabs.find((tab) => tab.bookmarkId === bookmark.id)
    if (existing?.status === 'connected' || existing?.status === 'connecting') { setActiveKey(existing.key); return }
    if (bookmark.hasSavedCredential) void connect(bookmark)
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
    if (!window.confirm(`确定删除书签“${bookmark.name}”吗？`)) return
    for (const tab of tabs.filter((item) => item.bookmarkId === bookmark.id)) closeTab(tab)
    await window.api.bookmarks.remove(bookmark.id); await reloadBookmarks()
  }

  const statusText: Record<SessionStatus, string> = { connecting: '连接中', connected: '已连接', disconnected: '未连接', error: '连接失败' }

  return <div className="app-shell">
    <header className="titlebar"><div className="app-brand"><span className="app-logo">&gt;_</span><span>SSH Client</span></div></header>
    <div className="main-layout">
      <aside className="sidebar">
        <div className="sidebar-heading"><strong>连接管理</strong><div className="sidebar-actions">
          <button className="icon-button" title="新增书签" onClick={() => setBookmarkDialog('new')}><CirclePlus size={18} /></button>
          <button className="icon-button" title="编辑当前书签" disabled={!activeBookmark} onClick={() => activeBookmark && setBookmarkDialog(activeBookmark)}><Edit3 size={17} /></button>
          <button className="icon-button" title="删除当前书签" disabled={!activeBookmark} onClick={() => activeBookmark && void removeBookmark(activeBookmark)}><Trash2 size={17} /></button>
        </div></div>
        <label className="search-box"><Search size={16} /><input placeholder="搜索书签" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
        <div className="bookmark-list">
          {filteredBookmarks.length === 0 && <div className="sidebar-empty"><BookmarkIcon size={24} /><span>{bookmarks.length ? '没有匹配的书签' : '还没有书签'}</span></div>}
          {filteredBookmarks.map((bookmark) => {
            const tab = tabs.find((item) => item.bookmarkId === bookmark.id)
            return <button key={bookmark.id} className={`bookmark-item ${activeBookmark?.id === bookmark.id ? 'active' : ''}`} onClick={() => openBookmark(bookmark)}>
              <span className={`server-icon ${tab?.status ?? 'disconnected'}`}><Server size={17} /></span>
              <span className="bookmark-copy"><strong>{bookmark.name}</strong><small>{bookmark.username}@{bookmark.host}:{bookmark.port}</small></span>
              {bookmark.hasSavedCredential && <KeyRound size={13} className="saved-key" />}
            </button>
          })}
        </div>
      </aside>
      <main className="workspace">
        <div className="tabbar">
          {tabs.map((tab) => { const bookmark = bookmarkMap.get(tab.bookmarkId); return <button key={tab.key} className={`session-tab ${tab.key === activeKey ? 'active' : ''}`} onClick={() => setActiveKey(tab.key)}>
            <span className={`status-dot ${tab.status}`} /><span>{bookmark?.name ?? '已删除书签'}</span><X size={14} onClick={(event) => { event.stopPropagation(); closeTab(tab) }} />
          </button> })}
          <button className="icon-button tab-add" title="新增书签" onClick={() => setBookmarkDialog('new')}><CirclePlus size={17} /></button>
        </div>
        {tabs.length === 0 ? <div className="welcome-state"><div className="welcome-icon"><Plug size={30} /></div><h2>选择一个 SSH 书签</h2><p>连接后可在同一窗口使用终端和 SFTP 文件传输。</p><button className="primary-button" onClick={() => setBookmarkDialog('new')}><CirclePlus size={16} />新增书签</button></div> : <>
          <div className="session-toolbar"><div><span className={`status-dot ${activeTab?.status}`} /><strong>{activeBookmark?.name}</strong><span>{activeTab ? statusText[activeTab.status] : ''}</span>{activeTab?.error && <span className="error-text" title={activeTab.error}>{activeTab.error}</span>}</div><div>
            {activeTab?.status === 'connected' ? <button className="secondary-button" onClick={() => activeTab.sessionId && void window.api.sessions.disconnect(activeTab.sessionId)}><Power size={15} />断开</button>
              : activeBookmark && <button className="primary-button" disabled={activeTab?.status === 'connecting'} onClick={() => activeBookmark.hasSavedCredential ? void connect(activeBookmark) : setAuthBookmark(activeBookmark)}><Plug size={15} />连接</button>}
          </div></div>
          <div className="session-stack">
            {tabs.map((tab) => <div key={tab.key} className="session-view" style={{ display: tab.key === activeKey ? 'grid' : 'none' }}>
              <div className="terminal-region"><TerminalPane sessionId={tab.sessionId} visible={tab.key === activeKey} /></div>
              <div className="sftp-region"><SftpPane sessionId={tab.sessionId} connected={tab.status === 'connected'} visible={tab.key === activeKey} onError={showError} /></div>
            </div>)}
          </div>
        </>}
        <TransferPanel transfers={transfers} view={transferView} setView={setTransferView} onClear={async () => { await window.api.transfers.clearCompleted(); setTransfers((current) => current.filter((item) => !['completed', 'cancelled'].includes(item.status))) }} />
      </main>
    </div>
    {bookmarkDialog && <BookmarkDialog bookmark={bookmarkDialog === 'new' ? undefined : bookmarkDialog} onClose={() => setBookmarkDialog(null)} onSaved={async () => { setBookmarkDialog(null); await reloadBookmarks() }} onError={showError} />}
    {authBookmark && <AuthDialog bookmark={authBookmark} onClose={() => setAuthBookmark(null)} onConnect={(secret, remember) => void connect(authBookmark, secret, remember)} />}
    {hostPrompt && <HostKeyDialog prompt={hostPrompt} onDecision={(accept) => { window.api.sessions.hostKeyDecision(hostPrompt.sessionId, accept); setHostPrompt(null) }} />}
    {conflict && <ConflictDialog conflict={conflict} onDecision={(resolution, apply) => { window.api.transfers.resolveConflict(conflict.taskId, resolution, apply); setConflict(null) }} />}
    {toast && <div className="toast">{toast}</div>}
  </div>
}

function BookmarkDialog({ bookmark, onClose, onSaved, onError }: { bookmark?: Bookmark; onClose(): void; onSaved(): void; onError(message: string): void }): React.JSX.Element {
  const [form, setForm] = useState<BookmarkInput>(bookmark ? { name: bookmark.name, host: bookmark.host, port: bookmark.port, username: bookmark.username, authType: bookmark.authType, privateKeyPath: bookmark.privateKeyPath, note: bookmark.note } : emptyBookmark)
  const set = <K extends keyof BookmarkInput>(key: K, value: BookmarkInput[K]): void => setForm((current) => ({ ...current, [key]: value }))
  return <Modal title={bookmark ? '编辑书签' : '新增书签'} onClose={onClose}><form className="form-grid" onSubmit={async (event) => { event.preventDefault(); try { await window.api.bookmarks.save({ ...form, id: bookmark?.id }); onSaved() } catch (error) { onError(errorMessage(error)) } }}>
    <label>名称<input required value={form.name} onChange={(event) => set('name', event.target.value)} placeholder="Production 服务器" /></label>
    <div className="form-row"><label>Host（IP 或域名）<input required value={form.host} onChange={(event) => set('host', event.target.value)} placeholder="192.168.1.10" /></label><label className="port-field">端口<input type="number" min="1" max="65535" value={form.port} onChange={(event) => set('port', Number(event.target.value))} /></label></div>
    <label>用户名<input required value={form.username} onChange={(event) => set('username', event.target.value)} placeholder="root" /></label>
    <label>认证方式<select value={form.authType} onChange={(event) => set('authType', event.target.value as BookmarkInput['authType'])}><option value="password">密码</option><option value="privateKey">私钥</option></select></label>
    {form.authType === 'privateKey' && <label>私钥文件<div className="input-button"><input readOnly value={form.privateKeyPath} placeholder="选择 OpenSSH 私钥" /><button type="button" className="secondary-button" onClick={async () => { const path = await window.api.files.choosePrivateKey(); if (path) set('privateKeyPath', path) }}>选择</button></div></label>}
    <label>备注<textarea rows={3} value={form.note} onChange={(event) => set('note', event.target.value)} placeholder="可选" /></label>
    {bookmark?.hasSavedCredential && <button type="button" className="text-button danger" onClick={async () => { await window.api.bookmarks.forgetCredential(bookmark.id); onSaved() }}>清除已保存的凭据</button>}
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button className="primary-button" type="submit">保存</button></div>
  </form></Modal>
}

function AuthDialog({ bookmark, onClose, onConnect }: { bookmark: Bookmark; onClose(): void; onConnect(secret: string, remember: boolean): void }): React.JSX.Element {
  const [secret, setSecret] = useState(''); const [remember, setRemember] = useState(false); const [visible, setVisible] = useState(false)
  return <Modal title={`连接到 ${bookmark.name}`} onClose={onClose}><form className="form-grid" onSubmit={(event) => { event.preventDefault(); if (bookmark.authType === 'password' && !secret) return; onConnect(secret, remember) }}>
    <div className="connection-target"><Server size={18} /><strong>{bookmark.username}@{bookmark.host}:{bookmark.port}</strong></div>
    <label>{bookmark.authType === 'password' ? '密码' : '私钥口令（可选）'}<div className="password-input"><input autoFocus type={visible ? 'text' : 'password'} required={bookmark.authType === 'password'} value={secret} onChange={(event) => setSecret(event.target.value)} /><button className="icon-button" type="button" title="显示或隐藏" onClick={() => setVisible((value) => !value)}><MoreHorizontal size={17} /></button></div></label>
    <label className="check-label"><input type="checkbox" checked={remember} onChange={(event) => setRemember(event.target.checked)} />记住凭据（使用系统安全存储）</label>
    <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onClose}>取消</button><button type="submit" className="primary-button">连接</button></div>
  </form></Modal>
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

function TransferPanel({ transfers, view, setView, onClear }: { transfers: TransferTask[]; view: 'queue' | 'history'; setView(value: 'queue' | 'history'): void; onClear(): void }): React.JSX.Element {
  const activeStatuses = ['queued', 'scanning', 'running', 'conflict']
  const shown = transfers.filter((item) => view === 'queue' ? activeStatuses.includes(item.status) || item.status === 'failed' : !activeStatuses.includes(item.status))
  return <section className="transfer-panel"><header><div className="transfer-tabs"><button className={view === 'queue' ? 'active' : ''} onClick={() => setView('queue')}>传输队列 <span>{transfers.filter((item) => activeStatuses.includes(item.status)).length}</span></button><button className={view === 'history' ? 'active' : ''} onClick={() => setView('history')}>传输记录</button></div><button className="text-button" onClick={onClear}>清除已完成</button></header>
    <div className="transfer-list">{shown.length === 0 ? <div className="transfer-empty">{view === 'queue' ? '当前没有传输任务' : '暂无传输记录'}</div> : shown.slice(0, 40).map((task) => {
      const percent = task.bytesTotal ? Math.min(100, Math.round(task.bytesTransferred / task.bytesTotal * 100)) : 0
      return <div className="transfer-row" key={task.id}><span className={`transfer-direction ${task.direction}`}>{task.direction === 'upload' ? '↑' : '↓'}</span><div className="transfer-info"><strong>{task.displayName}</strong><small>{task.error ?? `${task.sourcePath} → ${task.destinationPath}`}</small>{['running', 'scanning'].includes(task.status) && <div className="progress"><span style={{ width: `${percent}%` }} /></div>}</div><span className={`transfer-status ${task.status}`}>{task.status === 'running' ? `${percent}%` : ({ queued: '等待中', scanning: '扫描中', conflict: '等待确认', completed: '已完成', failed: '失败', cancelled: '已取消', interrupted: '已中断' } as Record<string, string>)[task.status]}</span><div className="transfer-actions">{activeStatuses.includes(task.status) && <button className="icon-button" title="取消" onClick={() => void window.api.transfers.cancel(task.id)}><X size={15} /></button>}{['failed', 'cancelled', 'interrupted'].includes(task.status) && <button className="text-button" onClick={() => void window.api.transfers.retry(task.id)}>重试</button>}</div></div>
    })}</div>
  </section>
}
