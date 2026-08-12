import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import { ArrowLeft, ArrowRight, ArrowUp, ChevronLeft, ChevronRight, Eye, EyeOff, File, FileText, Folder, FolderPlus, Link, Pencil, Plug, RefreshCw, Search, Server, Star, Trash2, X } from 'lucide-react'
import type { DirectoryEntry, FilePreview } from '../types'
import { useI18n } from '../i18n'

function remoteParentPath(path: string): string { return path === '/' ? '/' : path.slice(0, path.lastIndexOf('/')) || '/' }
function errorMessage(error: unknown): string { return error instanceof Error ? error.message : String(error) }
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
  onTransferDragStart(paths: string[], sourceRemote: boolean, event: React.PointerEvent<HTMLDivElement>): void;
  transferDropActive: boolean; transferDropDirectory?: string;
  onToggleFavorite(): void; onCreateDirectory(): void; onRename(path: string): void; onDelete(paths: string[]): void; onPreview(path: string): void;
}

interface TransferDropTarget { remote: boolean; directory?: string }
interface FileNameDialogState { mode: 'create' | 'rename'; remote: boolean; path?: string; value: string; error: string; submitting: boolean }
interface FileDeleteDialogState { remote: boolean; paths: string[]; error: string; submitting: boolean }

function transferDropTargetAt(x: number, y: number): TransferDropTarget | null {
  const element = document.elementFromPoint(x, y) as HTMLElement | null
  const browser = element?.closest<HTMLElement>('[data-file-browser-side]')
  if (!browser) return null
  const row = element?.closest<HTMLElement>('[data-entry-kind="directory"]')
  return {
    remote: browser.dataset.fileBrowserSide === 'remote',
    directory: row && browser.contains(row) ? row.dataset.entryPath : undefined
  }
}

function FileBrowser(props: BrowserProps): React.JSX.Element {
  const { t } = useI18n()
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

  return <section ref={browser} data-file-browser-side={props.remote ? 'remote' : 'local'} className={`file-browser ${props.transferDropActive ? 'transfer-drop-target' : ''}`}>
    <header className="file-toolbar">
      <strong>{props.remote ? <Server size={15} /> : <Folder size={15} />}{props.title}</strong>
      <button className="icon-button" title={t('sftp.back')} disabled={!props.canBack} onClick={props.onBack}><ChevronLeft size={16} /></button>
      <button className="icon-button" title={t('sftp.forward')} disabled={!props.canForward} onClick={props.onForward}><ChevronRight size={16} /></button>
      <button className="icon-button" title={t('sftp.parentDirectory')} onClick={props.onParent}><ArrowUp size={16} /></button>
      <button className={`icon-button ${props.loading ? 'is-loading' : ''}`} title={t('sftp.refresh')} onClick={props.onRefresh}><RefreshCw size={16} /></button>
      <form onSubmit={(event) => { event.preventDefault(); props.onPath(pathInput) }}><input value={pathInput} onChange={(event) => setPathInput(event.target.value)} aria-label={t('sftp.valuePath', { value0: props.title })} /></form>
    </header>
    <div className="file-actions">
      <label className="file-filter"><Search size={14} /><input value={filter} onChange={(event) => setFilter(event.target.value)} placeholder={t('sftp.filterThisDirectory')} /></label>
      <button className={`icon-button ${showHidden ? 'active' : ''}`} title={showHidden ? t('common.hideHiddenFiles') : t('common.showHiddenFiles')} aria-label={showHidden ? t('common.hideHiddenFiles') : t('common.showHiddenFiles')} onClick={() => setShowHidden((value) => !value)}>{showHidden ? <EyeOff size={15} /> : <Eye size={15} />}</button>
      <button className={`icon-button ${props.favorites.includes(props.path) ? 'active' : ''}`} title={t('sftp.favoriteCurrentPath')} onClick={props.onToggleFavorite}><Star size={15} /></button>
      {props.favorites.length > 0 && <select aria-label={t('sftp.valueFavoritePaths', { value0: props.title })} value="" onChange={(event) => { if (event.target.value) props.onPath(event.target.value) }}><option value="">{t('sftp.favoritePaths')}</option>{props.favorites.map((path) => <option key={path} value={path}>{path}</option>)}</select>}
      <span className="file-action-spacer" />
      <button className="icon-button" title={t('sftp.newDirectory')} onClick={props.onCreateDirectory}><FolderPlus size={15} /></button>
      <button className="icon-button" title={t('common.previewFile')} aria-label={t('common.previewFile')} disabled={!previewable} onClick={() => previewable && props.onPreview(selectedEntries[0]!.path)}><FileText size={15} /></button>
      <button className="icon-button" title={t('sftp.rename')} disabled={selectedEntries.length !== 1} onClick={() => selectedEntries[0] && props.onRename(selectedEntries[0].path)}><Pencil size={15} /></button>
      <span className="file-action-divider" />
      <button className="icon-button danger" title={t('sftp.delete')} disabled={selectedEntries.length === 0} onClick={() => props.onDelete(selectedEntries.map((entry) => entry.path))}><Trash2 size={15} /></button>
    </div>
    <div className="file-header"><span>{t('common.name')}</span></div>
    <div className="file-list" ref={scroll} tabIndex={0} onKeyDown={(event) => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.key.toLowerCase() === 'a') { event.preventDefault(); props.onSelectAll(paths) }
      else if (event.key === 'Delete' && props.selected.size) { event.preventDefault(); props.onDelete([...props.selected]) }
      else if (event.key === 'F2' && props.selected.size === 1) { event.preventDefault(); props.onRename([...props.selected][0]!) }
    }}>
      {props.loading && shown.length === 0 && <div className="file-message">{t('sftp.loadingDirectory')}</div>}
      {!props.loading && props.error && <div className="file-message error-text">{props.error}</div>}
      {!props.loading && !props.error && shown.length === 0 && <div className="file-message">{t('sftp.thisDirectoryIsEmpty')}</div>}
      {shown.length > 0 && <div className="file-virtual-space" style={{ height: virtualizer.getTotalSize() }}>{virtualizer.getVirtualItems().map((item) => {
        const entry = shown[item.index]!
        const dragPaths = props.selected.has(entry.path) ? [...props.selected] : [entry.path]
        return <div key={entry.path} data-entry-kind={entry.kind} data-entry-path={entry.path} className={`file-row ${props.selected.has(entry.path) ? 'selected' : ''} ${props.transferDropDirectory === entry.path ? 'transfer-folder-target' : ''}`} style={{ transform: `translateY(${item.start}px)` }}
          onPointerDown={(event) => { if (entry.kind === 'directory') props.onPrefetch?.(entry.path); props.onTransferDragStart(dragPaths, props.remote, event) }}
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
  const { t } = useI18n()
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
  const [fileNameDialog, setFileNameDialog] = useState<FileNameDialogState | null>(null)
  const [fileDeleteDialog, setFileDeleteDialog] = useState<FileDeleteDialogState | null>(null)
  const [transferDrag, setTransferDrag] = useState<{ paths: string[]; sourceRemote: boolean; x: number; y: number; target: TransferDropTarget | null } | null>(null)
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
  const transferDragCleanup = useRef<(() => void) | null>(null)
  const suppressSelectionUntil = useRef(0)

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
    if (!sessionId || !connected) throw new Error(t('sftp.sshSessionIsNotConnected'))
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

  useEffect(() => { void window.api.files.getFavorites(bookmarkId).then(setFavorites).catch((error) => onError(errorMessage(error))) }, [bookmarkId])
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

  useEffect(() => () => transferDragCleanup.current?.(), [])

  useEffect(() => {
    if (!fileNameDialog) return
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !fileNameDialog.submitting) setFileNameDialog(null)
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => window.removeEventListener('keydown', closeOnEscape)
  }, [fileNameDialog])

  useEffect(() => {
    if (!fileDeleteDialog) return
    const closeOnEscape = (event: KeyboardEvent): void => {
      if (event.key === 'Escape' && !fileDeleteDialog.submitting) setFileDeleteDialog(null)
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => window.removeEventListener('keydown', closeOnEscape)
  }, [fileDeleteDialog])

  const enqueue = async (direction: 'upload' | 'download', paths: string[], destinationDirectory?: string): Promise<void> => {
    if (!sessionId || !connected || paths.length === 0) return
    try { await window.api.transfers.enqueue({ sessionId, direction, sourcePaths: paths, destinationDirectory: destinationDirectory ?? (direction === 'upload' ? remotePathRef.current : localPathRef.current) }) }
    catch (error) { onError(errorMessage(error)) }
  }
  const beginTransferDrag = (paths: string[], sourceRemote: boolean, event: React.PointerEvent<HTMLDivElement>): void => {
    if (event.button !== 0 || paths.length === 0) return
    transferDragCleanup.current?.()
    const pointerId = event.pointerId
    const startX = event.clientX
    const startY = event.clientY
    let active = false
    const cleanup = (): void => {
      window.removeEventListener('pointermove', move, true)
      window.removeEventListener('pointerup', finish, true)
      window.removeEventListener('pointercancel', cancel, true)
      transferDragCleanup.current = null
      document.documentElement.classList.remove('is-file-dragging')
    }
    const move = (pointerEvent: PointerEvent): void => {
      if (pointerEvent.pointerId !== pointerId) return
      if (!active && Math.hypot(pointerEvent.clientX - startX, pointerEvent.clientY - startY) < 5) return
      active = true
      document.documentElement.classList.add('is-file-dragging')
      const target = transferDropTargetAt(pointerEvent.clientX, pointerEvent.clientY)
      setTransferDrag({ paths, sourceRemote, x: pointerEvent.clientX, y: pointerEvent.clientY, target: target?.remote === sourceRemote ? null : target })
      pointerEvent.preventDefault()
    }
    const finish = (pointerEvent: PointerEvent): void => {
      if (pointerEvent.pointerId !== pointerId) return
      if (active) {
        const target = transferDropTargetAt(pointerEvent.clientX, pointerEvent.clientY)
        if (target && target.remote !== sourceRemote) {
          const destination = target.directory ?? (target.remote ? remotePathRef.current : localPathRef.current)
          void enqueue(target.remote ? 'upload' : 'download', paths, destination)
        }
        suppressSelectionUntil.current = Date.now() + 250
        pointerEvent.preventDefault()
      }
      cleanup()
      setTransferDrag(null)
    }
    const cancel = (pointerEvent: PointerEvent): void => {
      if (pointerEvent.pointerId !== pointerId) return
      cleanup()
      setTransferDrag(null)
    }
    window.addEventListener('pointermove', move, true)
    window.addEventListener('pointerup', finish, true)
    window.addEventListener('pointercancel', cancel, true)
    transferDragCleanup.current = cleanup
  }
  const select = (remote: boolean, path: string, additive: boolean, range: boolean, visiblePaths: string[]): void => {
    if (Date.now() < suppressSelectionUntil.current) return
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
  const mutate = async (remote: boolean, action: () => Promise<void>): Promise<string | null> => {
    try {
      await action()
      if (remote) { remoteCache.current.clear(); await loadRemote(remotePathRef.current, true) } else await loadLocal(localPathRef.current)
      return null
    } catch (error) {
      const message = errorMessage(error)
      onError(message)
      return message
    }
  }
  const createDirectory = (remote: boolean): void => {
    setFileNameDialog({ mode: 'create', remote, value: '', error: '', submitting: false })
  }
  const renameEntry = (remote: boolean, path: string): void => {
    setFileNameDialog({ mode: 'rename', remote, path, value: entryName(path), error: '', submitting: false })
  }
  const submitFileName = async (event: React.FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    if (!fileNameDialog || fileNameDialog.submitting) return
    const name = fileNameDialog.value.trim()
    if (!name) {
      setFileNameDialog((current) => current ? { ...current, error: t('sftp.nameIsRequired') } : null)
      return
    }
    if (name.includes('/') || (!fileNameDialog.remote && name.includes('\\'))) {
      setFileNameDialog((current) => current ? { ...current, error: t('sftp.nameCannotContainPathSeparators') } : null)
      return
    }
    if (fileNameDialog.mode === 'rename' && fileNameDialog.path && name === entryName(fileNameDialog.path)) {
      setFileNameDialog(null)
      return
    }
    setFileNameDialog((current) => current ? { ...current, error: '', submitting: true } : null)
    const currentDirectory = fileNameDialog.remote ? remotePathRef.current : localPathRef.current
    let error: string | null
    if (fileNameDialog.mode === 'create') {
      const path = childPath(currentDirectory, name, fileNameDialog.remote)
      error = await mutate(fileNameDialog.remote, () => window.api.files.createDirectory(fileNameDialog.remote, sessionId, path))
    } else {
      const source = fileNameDialog.path!
      const parent = fileNameDialog.remote ? remoteParentPath(source) : source.slice(0, Math.max(source.lastIndexOf('/'), source.lastIndexOf('\\'))) || currentDirectory
      error = await mutate(fileNameDialog.remote, () => window.api.files.rename(fileNameDialog.remote, sessionId, source, childPath(parent, name, fileNameDialog.remote)))
    }
    if (error) setFileNameDialog((current) => current ? { ...current, error, submitting: false } : null)
    else setFileNameDialog(null)
  }
  const requestDeleteEntries = (remote: boolean, paths: string[]): void => {
    if (paths.length) setFileDeleteDialog({ remote, paths, error: '', submitting: false })
  }
  const confirmDeleteEntries = async (event: React.FormEvent<HTMLFormElement>): Promise<void> => {
    event.preventDefault()
    if (!fileDeleteDialog || fileDeleteDialog.submitting) return
    const { remote, paths } = fileDeleteDialog
    setFileDeleteDialog((current) => current ? { ...current, error: '', submitting: true } : null)
    const error = await mutate(remote, () => window.api.files.remove(remote, sessionId, paths))
    if (error) setFileDeleteDialog((current) => current ? { ...current, error, submitting: false } : null)
    else {
      if (remote) setRemoteSelected(new Set()); else setLocalSelected(new Set())
      setFileDeleteDialog(null)
    }
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
    setFavorites(next)
    void window.api.files.setFavorites(bookmarkId, next).catch((error) => onError(errorMessage(error)))
  }

  if (!sessionId || !connected) return <div className="sftp-disabled"><Folder size={25} /><strong>{t('sftp.fileTransferIsNotConnected')}</strong><span>{t('sftp.connectToBrowseLocalAndRemoteDirectories')}</span>{onConnect && <button className="secondary-button" onClick={onConnect}><Plug size={15} />{t('common.connect')}</button>}</div>
  const common = (remote: boolean): Pick<BrowserProps, 'onSelect' | 'onSelectAll' | 'onCreateDirectory' | 'onRename' | 'onDelete' | 'onPreview' | 'onToggleFavorite'> => ({
    onSelect: (path, additive, range, paths) => select(remote, path, additive, range, paths),
    onSelectAll: (paths) => (remote ? setRemoteSelected : setLocalSelected)(new Set(paths)),
    onCreateDirectory: () => createDirectory(remote), onRename: (path) => renameEntry(remote, path),
    onDelete: (paths) => requestDeleteEntries(remote, paths), onPreview: (path) => void previewEntry(remote, path), onToggleFavorite: () => toggleFavorite(remote)
  })
  return <>
    <div className="sftp-pane">
      <FileBrowser title={t('sftp.local')} remote={false} path={localPath} entries={localEntries} selected={localSelected} loading={localState.loading} error={localState.error} favorites={favorites.local}
        canBack={localNavigation.index > 0} canForward={localNavigation.index < localNavigation.items.length - 1} onBack={() => moveHistory(false, -1)} onForward={() => moveHistory(false, 1)}
        onPath={navigateLocal} onParent={() => { void window.api.files.parentLocal(localPath).then(navigateLocal).catch((error) => onError(errorMessage(error))) }} onRefresh={() => void loadLocal(localPath)}
        onTransferDrop={(paths, sourceRemote) => { if (sourceRemote) void enqueue('download', paths) }} onTransferDragStart={beginTransferDrag}
        transferDropActive={transferDrag?.target?.remote === false} transferDropDirectory={transferDrag?.target?.remote === false ? transferDrag.target.directory : undefined} {...common(false)} />
      <div className="transfer-arrows">
        <button className="icon-button transfer-button" title={t('sftp.uploadToRemote')} disabled={!localSelected.size} onClick={() => void enqueue('upload', [...localSelected])}><ArrowRight size={18} /></button>
        <button className="icon-button transfer-button" title={t('sftp.downloadToLocal')} disabled={!remoteSelected.size} onClick={() => void enqueue('download', [...remoteSelected])}><ArrowLeft size={18} /></button>
      </div>
      <FileBrowser title={t('common.remote')} remote path={remotePath} entries={remoteEntries} selected={remoteSelected} loading={remoteState.loading} error={remoteState.error} favorites={favorites.remote}
        canBack={remoteNavigation.index > 0} canForward={remoteNavigation.index < remoteNavigation.items.length - 1} onBack={() => moveHistory(true, -1)} onForward={() => moveHistory(true, 1)}
        onPath={navigateRemote} onParent={() => navigateRemote(remoteParentPath(remotePath))} onRefresh={() => void loadRemote(remotePath, true)} onPrefetch={prefetchRemote}
        onTransferDrop={(paths, sourceRemote) => { if (!sourceRemote) void enqueue('upload', paths) }} onTransferDragStart={beginTransferDrag}
        transferDropActive={transferDrag?.target?.remote === true} transferDropDirectory={transferDrag?.target?.remote === true ? transferDrag.target.directory : undefined} {...common(true)} />
    </div>
    {transferDrag && <div className="transfer-drag-ghost" style={{ left: transferDrag.x + 14, top: transferDrag.y + 14 }}><File size={15} /><span>{transferDrag.paths.length === 1 ? entryName(transferDrag.paths[0]!) : t('sftp.valueItems', { value0: transferDrag.paths.length })}</span></div>}
    {fileNameDialog && <div className="modal-backdrop file-operation-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !fileNameDialog.submitting) setFileNameDialog(null) }}>
      <section className="modal file-operation-dialog" role="dialog" aria-modal="true" aria-labelledby="file-operation-title">
        <header><strong id="file-operation-title">{fileNameDialog.mode === 'create' ? t('sftp.newDirectoryOnSide', { side: fileNameDialog.remote ? t('common.remote') : t('common.local') }) : t('sftp.renameItem', { side: fileNameDialog.remote ? t('common.remote') : t('common.local') })}</strong><button type="button" className="icon-button" title={t('common.close')} disabled={fileNameDialog.submitting} onClick={() => setFileNameDialog(null)}><X size={17} /></button></header>
        <form className="file-operation-form" onSubmit={(event) => void submitFileName(event)}>
          <label>{t('common.name')}<input autoFocus maxLength={255} value={fileNameDialog.value} onFocus={(event) => { if (fileNameDialog.mode === 'rename') event.currentTarget.select() }} onChange={(event) => setFileNameDialog((current) => current ? { ...current, value: event.target.value, error: '' } : null)} /></label>
          {fileNameDialog.error && <small className="error-text">{fileNameDialog.error}</small>}
          <div className="dialog-actions"><button type="button" className="secondary-button" disabled={fileNameDialog.submitting} onClick={() => setFileNameDialog(null)}>{t('common.cancel')}</button><button className="primary-button" disabled={fileNameDialog.submitting}>{fileNameDialog.submitting ? t('sftp.processing') : t('sftp.ok')}</button></div>
        </form>
      </section>
    </div>}
    {fileDeleteDialog && <div className="modal-backdrop file-operation-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !fileDeleteDialog.submitting) setFileDeleteDialog(null) }}>
      <section className="modal file-operation-dialog" role="dialog" aria-modal="true" aria-labelledby="file-delete-title">
        <header><strong id="file-delete-title">{t('sftp.deleteFiles', { side: fileDeleteDialog.remote ? t('common.remote') : t('common.local') })}</strong><button type="button" className="icon-button" title={t('common.close')} disabled={fileDeleteDialog.submitting} onClick={() => setFileDeleteDialog(null)}><X size={17} /></button></header>
        <form className="file-delete-confirm" onSubmit={(event) => void confirmDeleteEntries(event)}>
          <div className="file-delete-heading"><Trash2 size={22} /><div><strong>{t('sftp.permanentlyDeleteValueSelectedItems', { value0: fileDeleteDialog.paths.length })}</strong><span>{t('sftp.thisCannotBeUndoneCheckTheSelectedItems')}</span></div></div>
          <ul>{fileDeleteDialog.paths.slice(0, 5).map((path) => <li key={path} title={path}>{entryName(path)}</li>)}{fileDeleteDialog.paths.length > 5 && <li>{t('sftp.valueMore', { value0: fileDeleteDialog.paths.length - 5 })}</li>}</ul>
          {fileDeleteDialog.error && <small className="error-text">{fileDeleteDialog.error}</small>}
          <div className="dialog-actions"><button type="button" autoFocus className="secondary-button" disabled={fileDeleteDialog.submitting} onClick={() => setFileDeleteDialog(null)}>{t('common.cancel')}</button><button type="submit" className="danger-button" disabled={fileDeleteDialog.submitting}><Trash2 size={15} />{fileDeleteDialog.submitting ? t('sftp.deleting') : t('sftp.deletePermanently')}</button></div>
        </form>
      </section>
    </div>}
    {preview && <div className="file-preview-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) setPreview(null) }}><section className="file-preview" role="dialog" aria-modal="true"><header><strong>{entryName(preview.path)}</strong><span>{preview.value.size.toLocaleString()} bytes</span><button className="icon-button" title={t('sftp.closePreview')} onClick={() => setPreview(null)}><X size={16} /></button></header>{preview.value.binary ? <div className="binary-preview"><File size={34} /><strong>{t('sftp.binaryFile')}</strong><span>{t('sftp.thisFileCannotBePreviewedAsText')}</span></div> : <pre>{preview.value.content}</pre>}{preview.value.truncated && !preview.value.binary && <footer><span>{t(preview.value.position === 'start' ? 'sftp.largeFileStart' : 'sftp.largeFileEnd')}</span><button className="secondary-button" onClick={() => void previewEntry(preview.remote, preview.path, preview.value.position === 'start' ? 'end' : 'start')}>{t(preview.value.position === 'start' ? 'sftp.viewEnd' : 'sftp.viewStart')}</button></footer>}</section></div>}
  </>
}
