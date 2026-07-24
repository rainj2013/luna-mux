import { useCallback, useEffect, useMemo, useState } from 'react'
import { ArrowDown, ArrowLeft, ArrowRight, ArrowUp, File, Folder, Link, RefreshCw, Server } from 'lucide-react'
import type { DirectoryEntry } from '../../../shared/types'

function parentPath(path: string, remote: boolean): string {
  if (remote) return path === '/' ? '/' : path.slice(0, path.lastIndexOf('/')) || '/'
  const normalized = path.replace(/[\\/]$/, '')
  const split = Math.max(normalized.lastIndexOf('\\'), normalized.lastIndexOf('/'))
  return split <= 2 ? normalized.slice(0, split + 1) : normalized.slice(0, split)
}

function formatSize(size: number, directory: boolean): string {
  if (directory) return '--'
  if (size < 1024) return `${size} B`
  if (size < 1024 ** 2) return `${(size / 1024).toFixed(1)} KB`
  if (size < 1024 ** 3) return `${(size / 1024 ** 2).toFixed(1)} MB`
  return `${(size / 1024 ** 3).toFixed(1)} GB`
}

function EntryIcon({ kind }: { kind: DirectoryEntry['kind'] }): React.JSX.Element {
  if (kind === 'directory') return <Folder size={16} className="file-folder" />
  if (kind === 'symlink') return <Link size={16} />
  return <File size={16} />
}

interface BrowserProps {
  title: string; remote: boolean; path: string; entries: DirectoryEntry[]; selected: Set<string>;
  loading: boolean; error?: string; onPath(path: string): void; onRefresh(): void; onSelect(path: string, additive: boolean): void;
  onTransferDrop(paths: string[], sourceRemote: boolean): void
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
      <button className="icon-button" title="上级目录" onClick={() => props.onPath(parentPath(props.path, props.remote))}><ArrowUp size={16} /></button>
      <button className="icon-button" title="刷新" onClick={props.onRefresh}><RefreshCw size={16} /></button>
      <form onSubmit={(event) => { event.preventDefault(); props.onPath(pathInput) }}><input value={pathInput} onChange={(event) => setPathInput(event.target.value)} aria-label={`${props.title}路径`} /></form>
    </header>
    <div className="file-header"><span>名称</span><span>大小</span><span>修改时间</span></div>
    <div className="file-list">
      {props.loading && <div className="file-message">正在读取目录…</div>}
      {!props.loading && props.error && <div className="file-message error-text">{props.error}</div>}
      {!props.loading && !props.error && sorted.length === 0 && <div className="file-message">此目录为空</div>}
      {!props.loading && sorted.map((entry) => <div key={entry.path} className={`file-row ${props.selected.has(entry.path) ? 'selected' : ''}`}
        draggable onDragStart={(event) => event.dataTransfer.setData('application/x-ssh-client-files', JSON.stringify({ paths: props.selected.has(entry.path) ? [...props.selected] : [entry.path], remote: props.remote }))}
        onClick={(event) => props.onSelect(entry.path, event.ctrlKey || event.metaKey)}
        onDoubleClick={() => { if (entry.kind === 'directory') props.onPath(entry.path) }}>
        <span className="file-name"><EntryIcon kind={entry.kind} /><span title={entry.name}>{entry.name}</span></span>
        <span>{formatSize(entry.size, entry.kind === 'directory')}</span>
        <span>{entry.modifiedAt ? new Date(entry.modifiedAt).toLocaleString([], { dateStyle: 'short', timeStyle: 'short' }) : '--'}</span>
      </div>)}
    </div>
  </section>
}

export function SftpPane({ sessionId, connected, visible, onError }: { sessionId?: string; connected: boolean; visible: boolean; onError(message: string): void }): React.JSX.Element {
  const [localPath, setLocalPath] = useState('')
  const [remotePath, setRemotePath] = useState('/')
  const [localEntries, setLocalEntries] = useState<DirectoryEntry[]>([])
  const [remoteEntries, setRemoteEntries] = useState<DirectoryEntry[]>([])
  const [localSelected, setLocalSelected] = useState<Set<string>>(new Set())
  const [remoteSelected, setRemoteSelected] = useState<Set<string>>(new Set())
  const [localState, setLocalState] = useState({ loading: false, error: '' })
  const [remoteState, setRemoteState] = useState({ loading: false, error: '' })

  const loadLocal = useCallback(async (path: string) => {
    setLocalState({ loading: true, error: '' })
    try { setLocalEntries(await window.api.files.listLocal(path)); setLocalPath(path); setLocalSelected(new Set()); setLocalState({ loading: false, error: '' }) }
    catch (error) { setLocalState({ loading: false, error: error instanceof Error ? error.message : String(error) }) }
  }, [])
  const loadRemote = useCallback(async (path: string) => {
    if (!sessionId || !connected) return
    setRemoteState({ loading: true, error: '' })
    try { setRemoteEntries(await window.api.files.listRemote(sessionId, path)); setRemotePath(path); setRemoteSelected(new Set()); setRemoteState({ loading: false, error: '' }) }
    catch (error) { setRemoteState({ loading: false, error: error instanceof Error ? error.message : String(error) }) }
  }, [sessionId, connected])

  useEffect(() => { void window.api.files.home().then(loadLocal) }, [loadLocal])
  useEffect(() => { if (visible && connected) void loadRemote(remotePath) }, [connected, sessionId])

  const enqueue = async (direction: 'upload' | 'download', paths: string[]): Promise<void> => {
    if (!sessionId || !connected || paths.length === 0) return
    try { await window.api.transfers.enqueue({ sessionId, direction, sourcePaths: paths, destinationDirectory: direction === 'upload' ? remotePath : localPath }) }
    catch (error) { onError(error instanceof Error ? error.message : String(error)) }
  }
  const toggle = (setter: React.Dispatch<React.SetStateAction<Set<string>>>, path: string, additive: boolean): void => setter((current) => {
    const next = additive ? new Set(current) : new Set<string>(); if (next.has(path)) next.delete(path); else next.add(path); return next
  })

  if (!sessionId || !connected) return <div className="sftp-disabled"><Folder size={24} /><span>连接成功后可浏览和传输文件</span></div>
  return <div className="sftp-pane">
    <FileBrowser title="本地" remote={false} path={localPath} entries={localEntries} selected={localSelected} loading={localState.loading} error={localState.error}
      onPath={(path) => void loadLocal(path)} onRefresh={() => void loadLocal(localPath)} onSelect={(path, additive) => toggle(setLocalSelected, path, additive)}
      onTransferDrop={(paths, sourceRemote) => { if (sourceRemote) void enqueue('download', paths) }} />
    <div className="transfer-arrows">
      <button className="icon-button transfer-button" title="上传到远端" disabled={!localSelected.size} onClick={() => void enqueue('upload', [...localSelected])}><ArrowRight size={18} /></button>
      <button className="icon-button transfer-button" title="下载到本地" disabled={!remoteSelected.size} onClick={() => void enqueue('download', [...remoteSelected])}><ArrowLeft size={18} /></button>
    </div>
    <FileBrowser title="远端" remote path={remotePath} entries={remoteEntries} selected={remoteSelected} loading={remoteState.loading} error={remoteState.error}
      onPath={(path) => void loadRemote(path)} onRefresh={() => void loadRemote(remotePath)} onSelect={(path, additive) => toggle(setRemoteSelected, path, additive)}
      onTransferDrop={(paths, sourceRemote) => { if (!sourceRemote) void enqueue('upload', paths) }} />
  </div>
}
