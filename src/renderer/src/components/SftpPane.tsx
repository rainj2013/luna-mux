import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, File, Folder, Link, Plug, RefreshCw, Server } from 'lucide-react'
import type { DirectoryEntry } from '../../../shared/types'

function remoteParentPath(path: string): string {
  return path === '/' ? '/' : path.slice(0, path.lastIndexOf('/')) || '/'
}

function errorMessage(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return message.replace(/^Error invoking remote method '[^']+': Error: /, '')
}

function EntryIcon({ kind }: { kind: DirectoryEntry['kind'] }): React.JSX.Element {
  if (kind === 'directory') return <Folder size={16} className="file-folder" />
  if (kind === 'symlink') return <Link size={16} />
  return <File size={16} />
}

interface BrowserProps {
  title: string; remote: boolean; path: string; entries: DirectoryEntry[]; selected: Set<string>;
  loading: boolean; error?: string; onPath(path: string): void; onParent(): void; onRefresh(): void; onSelect(path: string, additive: boolean): void;
  onTransferDrop(paths: string[], sourceRemote: boolean): void; onPrefetch?(path: string): void
}

function FileBrowser(props: BrowserProps): React.JSX.Element {
  const [pathInput, setPathInput] = useState(props.path)
  useEffect(() => setPathInput(props.path), [props.path])
  const sorted = useMemo(() => [...props.entries].sort((a, b) => Number(b.kind === 'directory') - Number(a.kind === 'directory') || a.name.localeCompare(b.name)), [props.entries])
  return <section className="file-browser" onDragOver={(event) => event.preventDefault()} onDrop={(event) => {
    event.preventDefault(); const raw = event.dataTransfer.getData('application/x-ssh-client-files'); if (!raw) return
    const data = JSON.parse(raw) as { paths: string[]; remote: boolean }; if (data.remote !== props.remote) props.onTransferDrop(data.paths, data.remote)
  }}>
    <header className="file-toolbar">
      <strong>{props.remote ? <Server size={15} /> : <Folder size={15} />}{props.title}</strong>
      <button className="icon-button" title="上级目录" onClick={props.onParent}><ArrowUp size={16} /></button>
      <button className={`icon-button ${props.loading ? 'is-loading' : ''}`} title="刷新" onClick={props.onRefresh}><RefreshCw size={16} /></button>
      <form onSubmit={(event) => { event.preventDefault(); props.onPath(pathInput) }}><input value={pathInput} onChange={(event) => setPathInput(event.target.value)} aria-label={`${props.title}路径`} /></form>
    </header>
    <div className="file-header"><span>名称</span></div>
    <div className="file-list">
      {props.loading && sorted.length === 0 && <div className="file-message">正在读取目录…</div>}
      {!props.loading && props.error && <div className="file-message error-text">{props.error}</div>}
      {!props.loading && !props.error && sorted.length === 0 && <div className="file-message">此目录为空</div>}
      {(!props.loading || sorted.length > 0) && sorted.map((entry) => <div key={entry.path} className={`file-row ${props.selected.has(entry.path) ? 'selected' : ''}`}
        draggable onDragStart={(event) => event.dataTransfer.setData('application/x-ssh-client-files', JSON.stringify({ paths: props.selected.has(entry.path) ? [...props.selected] : [entry.path], remote: props.remote }))}
        onPointerDown={() => { if (entry.kind === 'directory') props.onPrefetch?.(entry.path) }}
        onClick={(event) => props.onSelect(entry.path, event.ctrlKey || event.metaKey)}
        onDoubleClick={() => { if (entry.kind === 'directory') props.onPath(entry.path) }}>
        <span className="file-name"><EntryIcon kind={entry.kind} /><span title={entry.name}>{entry.name}</span></span>
      </div>)}
    </div>
  </section>
}

export function SftpPane({ sessionId, connected, visible, onError, onConnect }: { sessionId?: string; connected: boolean; visible: boolean; onError(message: string): void; onConnect?: () => void }): React.JSX.Element {
  const [localPath, setLocalPath] = useState('')
  const [remotePath, setRemotePath] = useState('/')
  const [localEntries, setLocalEntries] = useState<DirectoryEntry[]>([])
  const [remoteEntries, setRemoteEntries] = useState<DirectoryEntry[]>([])
  const [localSelected, setLocalSelected] = useState<Set<string>>(new Set())
  const [remoteSelected, setRemoteSelected] = useState<Set<string>>(new Set())
  const [localState, setLocalState] = useState({ loading: false, error: '' })
  const [remoteState, setRemoteState] = useState({ loading: false, error: '' })
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

  const loadLocal = useCallback(async (path: string) => {
    const request = ++localRequest.current
    const navigating = path !== localPathRef.current
    localPathRef.current = path
    setLocalPath(path)
    if (navigating) { setLocalEntries([]); setLocalSelected(new Set()) }
    setLocalState({ loading: true, error: '' })
    try {
      const entries = await window.api.files.listLocal(path)
      if (request !== localRequest.current) return
      setLocalEntries(entries)
      setLocalState({ loading: false, error: '' })
    } catch (error) {
      if (request === localRequest.current) setLocalState({ loading: false, error: errorMessage(error) })
    }
  }, [])
  const fetchRemote = useCallback(async (path: string, force = false): Promise<DirectoryEntry[]> => {
    if (!sessionId || !connected) throw new Error('SSH 会话未连接')
    const key = `${sessionId}\0${path}`
    if (!force) {
      const pending = remoteInflight.current.get(key)
      if (pending) return await pending
    }
    const request = (async () => {
      try { return await window.api.files.listRemote(sessionId, path) }
      catch (error) {
        if (!errorMessage(error).includes('超时')) throw error
        return await window.api.files.listRemote(sessionId, path)
      }
    })()
    remoteInflight.current.set(key, request)
    try {
      const entries = await request
      remoteCache.current.set(key, { entries, updatedAt: Date.now() })
      return entries
    } finally {
      if (remoteInflight.current.get(key) === request) remoteInflight.current.delete(key)
    }
  }, [sessionId, connected])

  const loadRemote = useCallback(async (path: string, force = false) => {
    if (!sessionId || !connected) return
    const request = ++remoteRequest.current
    const navigating = path !== remotePathRef.current
    const key = `${sessionId}\0${path}`
    const cached = remoteCache.current.get(key)
    remotePathRef.current = path
    setRemotePath(path)
    if (cached) setRemoteEntries(cached.entries)
    else if (navigating) setRemoteEntries([])
    if (navigating) setRemoteSelected(new Set())
    if (!force && cached && Date.now() - cached.updatedAt < 10_000) {
      setRemoteState({ loading: false, error: '' })
      return
    }
    setRemoteState({ loading: true, error: '' })
    try {
      const entries = await fetchRemote(path, force)
      if (request !== remoteRequest.current) return
      setRemoteEntries(entries)
      setRemoteState({ loading: false, error: '' })
    } catch (error) {
      if (request === remoteRequest.current) setRemoteState({ loading: false, error: errorMessage(error) })
    }
  }, [sessionId, connected, fetchRemote])

  const prefetchRemote = useCallback((path: string): void => {
    if (!sessionId || !connected) return
    const cached = remoteCache.current.get(`${sessionId}\0${path}`)
    if (cached && Date.now() - cached.updatedAt < 10_000) return
    void fetchRemote(path).catch(() => undefined)
  }, [sessionId, connected, fetchRemote])

  useEffect(() => {
    if (localInitialized.current) return
    localInitialized.current = true
    void window.api.files.home().then((path) => loadLocal(path)).catch((error) => setLocalState({ loading: false, error: errorMessage(error) }))
  }, [loadLocal])
  useEffect(() => {
    if (!visible || !connected || !sessionId || initializedSession.current === sessionId) return
    initializedSession.current = sessionId
    remoteCache.current.clear()
    remoteInflight.current.clear()
    void window.api.files.remoteHome(sessionId).then((path) => loadRemote(path)).catch(() => loadRemote('/'))
  }, [visible, connected, sessionId, loadRemote])

  useEffect(() => {
    if (!sessionId) return
    const stop = window.api.onEvent((event) => {
      if (event.type !== 'transfer' || event.payload.sessionId !== sessionId || event.payload.status !== 'completed') return
      if (event.payload.direction === 'upload') pendingRefresh.current.remote = true
      else pendingRefresh.current.local = true
      if (refreshTimer.current) clearTimeout(refreshTimer.current)
      refreshTimer.current = setTimeout(() => {
        const pending = pendingRefresh.current
        pendingRefresh.current = { local: false, remote: false }
        refreshTimer.current = null
        if (pending.remote) void loadRemote(remotePathRef.current, true)
        if (pending.local) void loadLocal(localPathRef.current)
      }, 350)
    })
    return () => {
      stop()
      if (refreshTimer.current) clearTimeout(refreshTimer.current)
      refreshTimer.current = null
      pendingRefresh.current = { local: false, remote: false }
    }
  }, [sessionId, loadLocal, loadRemote])

  const enqueue = async (direction: 'upload' | 'download', paths: string[]): Promise<void> => {
    if (!sessionId || !connected || paths.length === 0) return
    try { await window.api.transfers.enqueue({ sessionId, direction, sourcePaths: paths, destinationDirectory: direction === 'upload' ? remotePath : localPath }) }
    catch (error) { onError(error instanceof Error ? error.message : String(error)) }
  }
  const toggle = (setter: React.Dispatch<React.SetStateAction<Set<string>>>, path: string, additive: boolean): void => setter((current) => {
    const next = additive ? new Set(current) : new Set<string>(); if (next.has(path)) next.delete(path); else next.add(path); return next
  })

  if (!sessionId || !connected) return <div className="sftp-disabled"><Folder size={25} /><strong>文件传输尚未连接</strong><span>连接后可浏览本地和远端目录</span>{onConnect && <button className="secondary-button" onClick={onConnect}><Plug size={15} />连接</button>}</div>
  return <div className="sftp-pane">
    <FileBrowser title="本地" remote={false} path={localPath} entries={localEntries} selected={localSelected} loading={localState.loading} error={localState.error}
      onPath={(path) => void loadLocal(path)} onParent={() => { void window.api.files.parentLocal(localPath).then(loadLocal).catch((error) => onError(errorMessage(error))) }} onRefresh={() => void loadLocal(localPath)} onSelect={(path, additive) => toggle(setLocalSelected, path, additive)}
      onTransferDrop={(paths, sourceRemote) => { if (sourceRemote) void enqueue('download', paths) }} />
    <div className="transfer-arrows">
      <button className="icon-button transfer-button" title="上传到远端" disabled={!localSelected.size} onClick={() => void enqueue('upload', [...localSelected])}><ArrowRight size={18} /></button>
      <button className="icon-button transfer-button" title="下载到本地" disabled={!remoteSelected.size} onClick={() => void enqueue('download', [...remoteSelected])}><ArrowLeft size={18} /></button>
    </div>
    <FileBrowser title="远端" remote path={remotePath} entries={remoteEntries} selected={remoteSelected} loading={remoteState.loading} error={remoteState.error}
      onPath={(path) => void loadRemote(path)} onParent={() => void loadRemote(remoteParentPath(remotePath))} onRefresh={() => void loadRemote(remotePath, true)} onSelect={(path, additive) => toggle(setRemoteSelected, path, additive)} onPrefetch={prefetchRemote}
      onTransferDrop={(paths, sourceRemote) => { if (!sourceRemote) void enqueue('upload', paths) }} />
  </div>
}
