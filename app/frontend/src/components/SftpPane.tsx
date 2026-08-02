import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowLeft, ArrowRight, ArrowUp, ChevronLeft, ChevronRight, Eye, File, Folder, FolderPlus, Link, Pencil, Plug, RefreshCw, Search, Server, Star, Trash2, X } from 'lucide-react'
import type { DirectoryEntry, FilePreview } from '../types'

function remoteParentPath(path: string): string { return path === '/' ? '/' : path.slice(0, path.lastIndexOf('/')) || '/' }
function errorMessage(error: unknown): string { return (error instanceof Error ? error.message : String(error)).replace(/^Error invoking remote method '[^']+': Error: /, '') }
function entryName(path: string): string { return path.replace(/[\\/]$/, '').split(/[\\/]/).pop() ?? path }
function childPath(directory: string, name: string, remote: boolean): string {
  if (remote) return directory === '/' ? `/${name}` : `${directory}/${name}`
  const separator = directory.includes('\\') ? '\\' : '/'
  return directory.endsWith(separator) ? `${directory}${name}` : `${directory}${separator}${name}`
}

function EntryIcon({ kind }: { kind: DirectoryEntry['kind'] }): React.JSX.Element {
  if (kind === 'directory') return <Folder size={16} className="file-folder" />
  if (kind === 'symlink') return <Link size={16} />
  return <File size={16} />
}

interface BrowserProps {
  title: string; remote: boolean; path: string; entries: DirectoryEntry[]; selected: Set<string>;
  loading: boolean; error?: string; favorites: string[]; canBack: boolean; canForward: boolean;
  onPath(path: string): void; onBack(): void; onForward(): void; onParent(): void; onRefresh(): void;
  onSelect(path: string, additive: boolean, range: boolean, visiblePaths: string[]): void; onSelectAll(paths: string[]): void;
  onTransferDrop(paths: string[], sourceRemote: boolean): void; onPrefetch?(path: string): void;
  onToggleFavorite(): void; onCreateDirectory(): void; onRename(path: string): void; onDelete(paths: string[]): void; onPreview(path: string): void;
}

function FileBrowser(props: BrowserProps): React.JSX.Element {
  const [pathInput, setPathInput] = useState(props.path)
  const [filter, setFilter] = useState('')
  const [showHidden, setShowHidden] = useState(false)
  const scroll = useRef<HTMLDivElement>(null)
  const browser = useRef<HTMLElement>(null)
  useEffect(() => {
    if (!props.remote) return
    const drop = (event: Event): void => {
      const detail = (event as CustomEvent<{ paths: string[]; x: number; y: number }>).detail
      const bounds = browser.current?.getBoundingClientRect()
      if (!bounds || detail.x < bounds.left || detail.x > bounds.right || detail.y < bounds.top || detail.y > bounds.bottom) return
      if (detail.paths.length) props.onTransferDrop(detail.paths, false)
    }
    window.addEventListener('tauri-file-drop', drop)
    return () => window.removeEventListener('tauri-file-drop', drop)
  }, [props.remote, props.onTransferDrop])
  useEffect(() => setPathInput(props.path), [props.path])
  const shown = useMemo(() => [...props.entries]
    .filter((entry) => showHidden || !entry.name.startsWith('.'))
    .filter((entry) => entry.name.toLowerCase().includes(filter.trim().toLowerCase()))
    .sort((a, b) => Number(b.kind === 'directory') - Number(a.kind === 'directory') || a.name.localeCompare(b.name)), [props.entries, filter, showHidden])
  const paths = useMemo(() => shown.map((entry) => entry.path), [shown])
  const virtualizer = useVirtualizer({ count: shown.length, getScrollElement: () => scroll.current, estimateSize: () => 31, overscan: 12 })
  const selectedEntries = shown.filter((entry) => props.selected.has(entry.path))
  const previewable = selectedEntries.length === 1 && selectedEntries[0]?.kind === 'file'

  return <section ref={browser} className="file-browser" onDragOver={(event) => event.preventDefault()} onDrop={(event) => {
    event.preventDefault()
    const raw = event.dataTransfer.getData('application/x-ssh-client-files')
    if (!raw && props.remote && event.dataTransfer.files.length) {
      const paths = [...event.dataTransfer.files].map((file) => window.api.files.filePath(file)).filter(Boolean)
      if (paths.length) props.onTransferDrop(paths, false)
      return
    }
    if (!raw) return
    try { const data = JSON.parse(raw) as { paths: string[]; remote: boolean }; if (data.remote !== props.remote) props.onTransferDrop(data.paths, data.remote) }
    catch { /* Ignore unrelated drag payloads. */ }
  }}>
    <header className="file-toolbar">
      <strong>{props.remote ? <Server size={15} /> : <Folder size={15} />}{props.title}</strong>
      <button className="icon-button" title="后退" disabled={!props.canBack} onClick={props.onBack}><ChevronLeft size={16} /></button>
      <button className="icon-button" title="前进" disabled={!props.canForward} onClick={props.onForward}><ChevronRight size={16} /></button>
      <button className="icon-button" title="上级目录" onClick={props.onParent}><ArrowUp size={16} /></button>
      <button className={`icon-button ${props.loading ? 'is-loading' : ''}`} title="刷新" onClick={props.onRefresh}><RefreshCw size={16} /></button>
      <form onSubmit={(event) => { event.preventDefault(); props.onPath(pathInput) }}><input value={pathInput} onChange={(event) => setPathInput(event.target.value)} aria-label={`${props.title}路径`} /></form>
    </header>
    <div className="file-actions">
      <label className="file-filter"><Search size={14} /><input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder="筛选当前目录" /></label>
      <button className={`icon-button ${showHidden ? 'active' : ''}`} title={showHidden ? '隐藏点文件' : '显示点文件'} onClick={() => setShowHidden((value) => !value)}><Eye size={15} /></button>
      <button className={`icon-button ${props.favorites.includes(props.path) ? 'active' : ''}`} title="收藏当前路径" onClick={props.onToggleFavorite}><Star size={15} /></button>
      {props.favorites.length > 0 && <select aria-label={`${props.title}收藏路径`} value="" onChange={(event) => { if (event.target.value) props.onPath(event.target.value) }}><option value="">收藏路径</option>{props.favorites.map((path) => <option key={path} value={path}>{path}</option>)}</select>}
      <span className="file-action-spacer" />
      <button className="icon-button" title="新建目录" onClick={props.onCreateDirectory}><FolderPlus size={15} /></button>
      <button className="icon-button" title="预览文件" disabled={!previewable} onClick={() => previewable && props.onPreview(selectedEntries[0]!.path)}><Eye size={15} /></button>
      <button className="icon-button" title="重命名" disabled={selectedEntries.length !== 1} onClick={() => selectedEntries[0] && props.onRename(selectedEntries[0].path)}><Pencil size={15} /></button>
      <button className="icon-button danger" title="删除" disabled={selectedEntries.length === 0} onClick={() => props.onDelete(selectedEntries.map((entry) => entry.path))}><Trash2 size={15} /></button>
    </div>
    <div className="file-header"><span>名称</span></div>
    <div className="file-list" ref={scroll} tabIndex={0} onKeyDown={(event) => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.key.toLowerCase() === 'a') { event.preventDefault(); props.onSelectAll(paths) }
      else if (event.key === 'Delete' && props.selected.size) { event.preventDefault(); props.onDelete([...props.selected]) }
      else if (event.key === 'F2' && props.selected.size === 1) { event.preventDefault(); props.onRename([...props.selected][0]!) }
    }}>
      {props.loading && shown.length === 0 && <div className="file-message">正在读取目录…</div>}
      {!props.loading && props.error && <div className="file-message error-text">{props.error}</div>}
      {!props.loading && !props.error && shown.length === 0 && <div className="file-message">此目录为空</div>}
      {shown.length > 0 && <div className="file-virtual-space" style={{ height: virtualizer.getTotalSize() }}>{virtualizer.getVirtualItems().map((item) => {
        const entry = shown[item.index]!
        return <div key={entry.path} className={`file-row ${props.selected.has(entry.path) ? 'selected' : ''}`} style={{ transform: `translateY(${item.start}px)` }}
          draggable onDragStart={(event) => event.dataTransfer.setData('application/x-ssh-client-files', JSON.stringify({ paths: props.selected.has(entry.path) ? [...props.selected] : [entry.path], remote: props.remote }))}
          onPointerDown={() => { if (entry.kind === 'directory') props.onPrefetch?.(entry.path) }}
          onClick={(event) => props.onSelect(entry.path, event.ctrlKey || event.metaKey, event.shiftKey, paths)}
          onDoubleClick={() => entry.kind === 'directory' ? props.onPath(entry.path) : props.onPreview(entry.path)}>
          <span className="file-name"><EntryIcon kind={entry.kind} /><span title={entry.name}>{entry.name}</span></span>
        </div>
      })}</div>}
    </div>
  </section>
}

interface NavigationState { items: string[]; index: number }

export function SftpPane({ sessionId, bookmarkId, connected, visible, onError, onConnect }: { sessionId?: string; bookmarkId: string; connected: boolean; visible: boolean; onError(message: string): void; onConnect?: () => void }): React.JSX.Element {
  const [localPath, setLocalPath] = useState('')
  const [remotePath, setRemotePath] = useState('/')
  const [localEntries, setLocalEntries] = useState<DirectoryEntry[]>([])
  const [remoteEntries, setRemoteEntries] = useState<DirectoryEntry[]>([])
  const [localSelected, setLocalSelected] = useState<Set<string>>(new Set())
  const [remoteSelected, setRemoteSelected] = useState<Set<string>>(new Set())
  const [localState, setLocalState] = useState({ loading: false, error: '' })
  const [remoteState, setRemoteState] = useState({ loading: false, error: '' })
  const [localNavigation, setLocalNavigation] = useState<NavigationState>({ items: [], index: -1 })
  const [remoteNavigation, setRemoteNavigation] = useState<NavigationState>({ items: [], index: -1 })
  const [favorites, setFavorites] = useState({ local: [] as string[], remote: [] as string[] })
  const [preview, setPreview] = useState<{ remote: boolean; path: string; value: FilePreview } | null>(null)
  const localPathRef = useRef('')
  const remotePathRef = useRef('/')
  const localRequest = useRef(0)
  const remoteRequest = useRef(0)
  const localInitialized = useRef(false)
  const initializedSession = useRef('')
  const refreshTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const pendingRefresh = useRef({ local: false, remote: false })
  const remoteCache = useRef(new Map<string, { entries: DirectoryEntry[]; updatedAt: number }>())
  const remoteInflight = useRef(new Map<string, Promise<DirectoryEntry[]>>())
  const localAnchor = useRef('')
  const remoteAnchor = useRef('')

  const loadLocal = useCallback(async (path: string) => {
    const request = ++localRequest.current
    const navigating = path !== localPathRef.current
    localPathRef.current = path; setLocalPath(path)
    if (navigating) { setLocalEntries([]); setLocalSelected(new Set()) }
    setLocalState({ loading: true, error: '' })
    try { const entries = await window.api.files.listLocal(path); if (request === localRequest.current) { setLocalEntries(entries); setLocalState({ loading: false, error: '' }) } }
    catch (error) { if (request === localRequest.current) setLocalState({ loading: false, error: errorMessage(error) }) }
  }, [])

  const fetchRemote = useCallback(async (path: string, force = false): Promise<DirectoryEntry[]> => {
    if (!sessionId || !connected) throw new Error('SSH 会话未连接')
    const key = `${sessionId}\0${path}`
    if (!force) { const pending = remoteInflight.current.get(key); if (pending) return await pending }
    const request = (async () => { try { return await window.api.files.listRemote(sessionId, path) } catch (error) { if (!errorMessage(error).includes('超时')) throw error; return await window.api.files.listRemote(sessionId, path) } })()
    remoteInflight.current.set(key, request)
    try {
      const entries = await request
      remoteCache.current.delete(key); remoteCache.current.set(key, { entries, updatedAt: Date.now() })
      while (remoteCache.current.size > 100) remoteCache.current.delete(remoteCache.current.keys().next().value!)
      return entries
    } finally { if (remoteInflight.current.get(key) === request) remoteInflight.current.delete(key) }
  }, [sessionId, connected])

  const loadRemote = useCallback(async (path: string, force = false) => {
    if (!sessionId || !connected) return
    const request = ++remoteRequest.current
    const navigating = path !== remotePathRef.current
    const key = `${sessionId}\0${path}`
    const cached = remoteCache.current.get(key)
    remotePathRef.current = path; setRemotePath(path)
    if (cached) setRemoteEntries(cached.entries); else if (navigating) setRemoteEntries([])
    if (navigating) setRemoteSelected(new Set())
    if (!force && cached && Date.now() - cached.updatedAt < 10_000) { setRemoteState({ loading: false, error: '' }); return }
    setRemoteState({ loading: true, error: '' })
    try { const entries = await fetchRemote(path, force); if (request === remoteRequest.current) { setRemoteEntries(entries); setRemoteState({ loading: false, error: '' }) } }
    catch (error) { if (request === remoteRequest.current) setRemoteState({ loading: false, error: errorMessage(error) }) }
  }, [sessionId, connected, fetchRemote])

  const pushNavigation = (setNavigation: React.Dispatch<React.SetStateAction<NavigationState>>, path: string): void => setNavigation((current) => {
    if (current.items[current.index] === path) return current
    const items = [...current.items.slice(0, current.index + 1), path]
    return { items: items.slice(-100), index: Math.min(items.length - 1, 99) }
  })
  const navigateLocal = (path: string): void => { pushNavigation(setLocalNavigation, path); void loadLocal(path) }
  const navigateRemote = (path: string): void => { pushNavigation(setRemoteNavigation, path); void loadRemote(path) }
  const moveHistory = (remote: boolean, offset: number): void => {
    const current = remote ? remoteNavigation : localNavigation
    const index = current.index + offset
    const path = current.items[index]
    if (!path) return
    if (remote) { setRemoteNavigation({ ...current, index }); void loadRemote(path) }
    else { setLocalNavigation({ ...current, index }); void loadLocal(path) }
  }

  const prefetchRemote = useCallback((path: string): void => {
    if (!sessionId || !connected) return
    const cached = remoteCache.current.get(`${sessionId}\0${path}`)
    if (!cached || Date.now() - cached.updatedAt >= 10_000) void fetchRemote(path).catch(() => undefined)
  }, [sessionId, connected, fetchRemote])

  useEffect(() => { void window.api.files.getFavorites(bookmarkId).then(setFavorites) }, [bookmarkId])
  useEffect(() => {
    if (!visible || localInitialized.current) return
    localInitialized.current = true
    void window.api.files.home().then((path) => { setLocalNavigation({ items: [path], index: 0 }); return loadLocal(path) }).catch((error) => setLocalState({ loading: false, error: errorMessage(error) }))
  }, [visible, loadLocal])
  useEffect(() => {
    if (!visible || !connected || !sessionId || initializedSession.current === sessionId) return
    initializedSession.current = sessionId; remoteCache.current.clear(); remoteInflight.current.clear()
    void window.api.files.remoteHome(sessionId).then((path) => { setRemoteNavigation({ items: [path], index: 0 }); return loadRemote(path) }).catch(() => { setRemoteNavigation({ items: ['/'], index: 0 }); return loadRemote('/') })
  }, [visible, connected, sessionId, loadRemote])

  useEffect(() => {
    if (!sessionId) return
    const stop = window.api.onEvent((event) => {
      if (event.type !== 'transfer' || event.payload.sessionId !== sessionId || event.payload.status !== 'completed') return
      if (event.payload.direction === 'upload') pendingRefresh.current.remote = true; else pendingRefresh.current.local = true
      if (refreshTimer.current) clearTimeout(refreshTimer.current)
      refreshTimer.current = setTimeout(() => {
        const pending = pendingRefresh.current; pendingRefresh.current = { local: false, remote: false }; refreshTimer.current = null
        if (pending.remote) { remoteCache.current.clear(); void loadRemote(remotePathRef.current, true) }
        if (pending.local) void loadLocal(localPathRef.current)
      }, 350)
    })
    return () => { stop(); if (refreshTimer.current) clearTimeout(refreshTimer.current); refreshTimer.current = null; pendingRefresh.current = { local: false, remote: false } }
  }, [sessionId, loadLocal, loadRemote])

  const enqueue = async (direction: 'upload' | 'download', paths: string[]): Promise<void> => {
    if (!sessionId || !connected || paths.length === 0) return
    try { await window.api.transfers.enqueue({ sessionId, direction, sourcePaths: paths, destinationDirectory: direction === 'upload' ? remotePath : localPath }) }
    catch (error) { onError(errorMessage(error)) }
  }
  const select = (remote: boolean, path: string, additive: boolean, range: boolean, visiblePaths: string[]): void => {
    const setter = remote ? setRemoteSelected : setLocalSelected
    const anchor = remote ? remoteAnchor : localAnchor
    setter((current) => {
      if (range && anchor.current) {
        const start = visiblePaths.indexOf(anchor.current); const end = visiblePaths.indexOf(path)
        if (start >= 0 && end >= 0) return new Set(visiblePaths.slice(Math.min(start, end), Math.max(start, end) + 1))
      }
      anchor.current = path
      const next = additive ? new Set(current) : new Set<string>()
      if (next.has(path)) next.delete(path); else next.add(path)
      return next
    })
  }
  const mutate = async (remote: boolean, action: () => Promise<void>): Promise<void> => {
    try {
      await action()
      if (remote) { remoteCache.current.clear(); await loadRemote(remotePathRef.current, true) } else await loadLocal(localPathRef.current)
    } catch (error) { onError(errorMessage(error)) }
  }
  const createDirectory = (remote: boolean): void => {
    const name = window.prompt('新目录名称')?.trim()
    if (!name || name.includes('/') || (!remote && name.includes('\\'))) return
    const path = childPath(remote ? remotePath : localPath, name, remote)
    void mutate(remote, () => window.api.files.createDirectory(remote, sessionId, path))
  }
  const renameEntry = (remote: boolean, path: string): void => {
    const name = window.prompt('新名称', entryName(path))?.trim()
    if (!name || name === entryName(path) || name.includes('/') || (!remote && name.includes('\\'))) return
    const parent = remote ? remoteParentPath(path) : path.slice(0, Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))) || localPath
    void mutate(remote, () => window.api.files.rename(remote, sessionId, path, childPath(parent, name, remote)))
  }
  const deleteEntries = (remote: boolean, paths: string[]): void => {
    if (!paths.length || !window.confirm(`确定永久删除选中的 ${paths.length} 项吗？`)) return
    void mutate(remote, () => window.api.files.remove(remote, sessionId, paths))
  }
  const previewEntry = async (remote: boolean, path: string, position: 'start' | 'end' = 'start'): Promise<void> => {
    try { setPreview({ remote, path, value: await window.api.files.preview(remote, sessionId, path, position) }) }
    catch (error) { onError(errorMessage(error)) }
  }
  const toggleFavorite = (remote: boolean): void => {
    const path = remote ? remotePath : localPath
    const list = remote ? favorites.remote : favorites.local
    const nextList = list.includes(path) ? list.filter((item) => item !== path) : [...list, path]
    const next = remote ? { ...favorites, remote: nextList } : { ...favorites, local: nextList }
    setFavorites(next); void window.api.files.setFavorites(bookmarkId, next)
  }

  if (!sessionId || !connected) return <div className="sftp-disabled"><Folder size={25} /><strong>文件传输尚未连接</strong><span>连接后可浏览本地和远端目录</span>{onConnect && <button className="secondary-button" onClick={onConnect}><Plug size={15} />连接</button>}</div>
  const common = (remote: boolean): Pick<BrowserProps, 'onSelect' | 'onSelectAll' | 'onCreateDirectory' | 'onRename' | 'onDelete' | 'onPreview' | 'onToggleFavorite'> => ({
    onSelect: (path, additive, range, paths) => select(remote, path, additive, range, paths),
    onSelectAll: (paths) => (remote ? setRemoteSelected : setLocalSelected)(new Set(paths)),
    onCreateDirectory: () => createDirectory(remote), onRename: (path) => renameEntry(remote, path),
    onDelete: (paths) => deleteEntries(remote, paths), onPreview: (path) => void previewEntry(remote, path), onToggleFavorite: () => toggleFavorite(remote)
  })
  return <>
    <div className="sftp-pane">
      <FileBrowser title="本地" remote={false} path={localPath} entries={localEntries} selected={localSelected} loading={localState.loading} error={localState.error} favorites={favorites.local}
        canBack={localNavigation.index > 0} canForward={localNavigation.index < localNavigation.items.length - 1} onBack={() => moveHistory(false, -1)} onForward={() => moveHistory(false, 1)}
        onPath={navigateLocal} onParent={() => { void window.api.files.parentLocal(localPath).then(navigateLocal).catch((error) => onError(errorMessage(error))) }} onRefresh={() => void loadLocal(localPath)}
        onTransferDrop={(paths, sourceRemote) => { if (sourceRemote) void enqueue('download', paths) }} {...common(false)} />
      <div className="transfer-arrows">
        <button className="icon-button transfer-button" title="上传到远端" disabled={!localSelected.size} onClick={() => void enqueue('upload', [...localSelected])}><ArrowRight size={18} /></button>
        <button className="icon-button transfer-button" title="下载到本地" disabled={!remoteSelected.size} onClick={() => void enqueue('download', [...remoteSelected])}><ArrowLeft size={18} /></button>
      </div>
      <FileBrowser title="远端" remote path={remotePath} entries={remoteEntries} selected={remoteSelected} loading={remoteState.loading} error={remoteState.error} favorites={favorites.remote}
        canBack={remoteNavigation.index > 0} canForward={remoteNavigation.index < remoteNavigation.items.length - 1} onBack={() => moveHistory(true, -1)} onForward={() => moveHistory(true, 1)}
        onPath={navigateRemote} onParent={() => navigateRemote(remoteParentPath(remotePath))} onRefresh={() => void loadRemote(remotePath, true)} onPrefetch={prefetchRemote}
        onTransferDrop={(paths, sourceRemote) => { if (!sourceRemote) void enqueue('upload', paths) }} {...common(true)} />
    </div>
    {preview && <div className="file-preview-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setPreview(null) }}><section className="file-preview" role="dialog" aria-modal="true"><header><strong>{entryName(preview.path)}</strong><span>{preview.value.size.toLocaleString()} bytes</span><button className="icon-button" title="关闭预览" onClick={() => setPreview(null)}><X size={16} /></button></header>{preview.value.binary ? <div className="binary-preview"><File size={34} /><strong>二进制文件</strong><span>此文件不适合文本预览</span></div> : <pre>{preview.value.content}</pre>}{preview.value.truncated && !preview.value.binary && <footer><span>文件较大，当前显示{preview.value.position === 'start' ? '开头' : '末尾'} 1 MiB</span><button className="secondary-button" onClick={() => void previewEntry(preview.remote, preview.path, preview.value.position === 'start' ? 'end' : 'start')}>查看{preview.value.position === 'start' ? '末尾' : '开头'}</button></footer>}</section></div>}
  </>
}
