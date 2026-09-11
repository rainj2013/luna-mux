import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ArrowUp, Eye, EyeOff, File, Folder, Link as LinkIcon, RefreshCw, Search, X } from 'lucide-react'
import type { DirectoryEntry } from '../types'
import { useI18n } from '../i18n'
import { fileDialogExtension, fileDialogJoin, fileDialogSegments, fileDialogWithExtension } from '../file-dialog-path'

export interface FileDialogFilter { label: string; extensions: string[] }

export interface FileDialogOptions {
  /** `open` picks an existing file, `save` describes a file that may not exist yet, `directory` picks a folder. */
  mode: 'open' | 'save' | 'directory'
  title: string
  defaultFileName?: string
  filters?: FileDialogFilter[]
  initialDirectory?: string
  confirmLabel?: string
  onConfirm(path: string): void
  onCancel(): void
}

// Native save/open panels look and behave differently on every platform (macOS adds a Tags field
// to a save panel, for example). This dialog is the whole picker: a breadcrumb, an editable path,
// a folder list, and a file name field, so the experience is identical on macOS, Windows and Linux.
// Remembering the last directory also matches what people expect from a save panel.
let lastDirectory = ''

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function EntryIcon({ kind }: { kind: DirectoryEntry['kind'] }): React.JSX.Element {
  if (kind === 'directory') return <Folder size={15} className="file-folder" />
  if (kind === 'symlink') return <LinkIcon size={14} />
  return <File size={14} />
}

export function FileDialog({ mode, title, defaultFileName = '', filters, initialDirectory, confirmLabel, onConfirm, onCancel }: FileDialogOptions): React.JSX.Element {
  const { t } = useI18n()
  const normalizedFilters = useMemo<FileDialogFilter[]>(() => filters?.length ? filters : [{ label: t('fileDialog.allFiles'), extensions: [] }], [filters, t])
  const [directory, setDirectory] = useState('')
  const [entries, setEntries] = useState<DirectoryEntry[]>([])
  const [state, setState] = useState({ loading: true, error: '' })
  const [selected, setSelected] = useState('')
  const [fileName, setFileName] = useState(defaultFileName)
  const [filterIndex, setFilterIndex] = useState(0)
  const [search, setSearch] = useState('')
  const [showHidden, setShowHidden] = useState(false)
  const [pathInput, setPathInput] = useState('')
  const requestId = useRef(0)
  const listRef = useRef<HTMLDivElement>(null)
  const nameRef = useRef<HTMLInputElement>(null)

  const load = useCallback(async (path: string): Promise<boolean> => {
    const id = ++requestId.current
    setState((current) => ({ ...current, loading: true, error: '' }))
    try {
      const list = await window.api.files.listLocal(path)
      if (id !== requestId.current) return false
      lastDirectory = path
      setDirectory(path); setEntries(list); setSelected(''); setPathInput(path)
      setState({ loading: false, error: '' })
      return true
    } catch (error) {
      if (id !== requestId.current) return false
      setState({ loading: false, error: errorText(error) })
      return false
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      const home = await window.api.files.home().catch(() => '')
      // A remembered or suggested directory may be gone by now, so fall back instead of failing.
      for (const candidate of [initialDirectory, lastDirectory, home]) {
        if (!candidate) continue
        const opened = await load(candidate)
        if (cancelled) return
        if (opened) return
      }
      if (!cancelled) setState({ loading: false, error: '' })
    })()
    return () => { cancelled = true }
  }, [initialDirectory, load])

  useEffect(() => {
    const escape = (event: KeyboardEvent): void => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onCancel() } }
    window.addEventListener('keydown', escape, true)
    return () => window.removeEventListener('keydown', escape, true)
  }, [onCancel])

  useEffect(() => {
    if (mode === 'save') nameRef.current?.focus({ preventScroll: true })
    else listRef.current?.focus({ preventScroll: true })
  }, [mode])

  const allowedKey = (normalizedFilters[Math.min(filterIndex, normalizedFilters.length - 1)]?.extensions ?? []).join(',')
  const shown = useMemo(() => {
    const allowed = allowedKey ? allowedKey.split(',') : []
    const visible = entries
      .filter((entry) => showHidden || !entry.name.startsWith('.'))
      .filter((entry) => entry.kind === 'directory' || !allowed.length || allowed.includes(fileDialogExtension(entry.name)))
      .sort((left, right) => Number(right.kind === 'directory') - Number(left.kind === 'directory') || left.name.localeCompare(right.name))
    const needle = search.trim().toLowerCase()
    return needle ? visible.filter((entry) => entry.name.toLowerCase().includes(needle)) : visible
  }, [entries, showHidden, allowedKey, search])

  const selectedEntry = shown.find((entry) => entry.path === selected)
  const selectedIndex = shown.findIndex((entry) => entry.path === selected)
  const crumbs = fileDialogSegments(directory)

  const select = (entry: DirectoryEntry): void => {
    setSelected(entry.path)
    if (entry.kind !== 'directory' && mode !== 'directory') setFileName(entry.name)
  }
  const selectIndex = (index: number): void => {
    const entry = shown[index]
    if (!entry) return
    select(entry)
    listRef.current?.querySelector<HTMLElement>(`[data-file-index="${index}"]`)?.scrollIntoView({ block: 'nearest' })
  }
  const openEntry = (entry: DirectoryEntry): void => {
    if (entry.kind === 'directory') { void load(entry.path); return }
    select(entry)
    // Double-clicking a file confirms it, the way a native picker does — except in save mode,
    // where it only fills the name field.
    if (mode !== 'save') onConfirm(entry.path)
  }
  const submit = (): void => {
    if (mode === 'directory') {
      const target = selectedEntry?.kind === 'directory' ? selectedEntry.path : directory
      if (target) onConfirm(target)
      return
    }
    const name = fileName.trim()
    if (!name) return
    if (name.includes('/') || name.includes('\\')) { onConfirm(name); return }
    onConfirm(fileDialogJoin(directory, mode === 'save' ? fileDialogWithExtension(name, allowedKey ? allowedKey.split(',') : []) : name))
  }
  const goParent = (): void => {
    if (!directory) return
    void window.api.files.parentLocal(directory).then((parent) => parent && parent !== directory ? load(parent) : undefined).catch((error) => setState({ loading: false, error: errorText(error) }))
  }
  const canConfirm = mode === 'directory' ? Boolean(directory) : Boolean(fileName.trim())

  // The dialog is portaled into the app shell so it keeps the theme variables, which live on
  // `.app-shell` rather than `:root`, and stays above every pane regardless of pane transforms.
  return createPortal(<div className="modal-backdrop file-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel() }}>
    <section className="modal file-dialog" role="dialog" aria-modal="true" aria-label={title}>
      <header><strong>{title}</strong><button type="button" className="icon-button" title={t('common.close')} aria-label={t('common.close')} onClick={onCancel}><X size={18} /></button></header>
      <div className="file-dialog-toolbar">
        <button type="button" className="icon-button" title={t('fileDialog.parent')} aria-label={t('fileDialog.parent')} disabled={!directory || crumbs.length <= 1} onClick={goParent}><ArrowUp size={15} /></button>
        <button type="button" className={`icon-button ${state.loading ? 'is-loading' : ''}`} title={t('fileDialog.refresh')} aria-label={t('fileDialog.refresh')} disabled={!directory} onClick={() => void load(directory)}><RefreshCw size={15} /></button>
        <form className="file-dialog-path" onSubmit={(event) => { event.preventDefault(); const next = pathInput.trim(); if (next) void load(next) }}>
          <input value={pathInput} onChange={(event) => setPathInput(event.target.value)} aria-label={t('fileDialog.path')} spellCheck={false} autoComplete="off" />
        </form>
      </div>
      <nav className="file-dialog-crumbs" aria-label={t('fileDialog.path')}>
        {crumbs.map((crumb, index) => <span key={crumb.path}>
          {index > 0 && <small>/</small>}
          <button type="button" className={index === crumbs.length - 1 ? 'active' : ''} onClick={() => void load(crumb.path)}>{crumb.name}</button>
        </span>)}
      </nav>
      <div className="file-dialog-search">
        <label className="file-filter"><Search size={13} /><input value={search} onChange={(event) => setSearch(event.target.value)} placeholder={t('fileDialog.filterPlaceholder')} /></label>
        {normalizedFilters.length > 1 && <select aria-label={t('fileDialog.fileType')} value={String(filterIndex)} onChange={(event) => { setFilterIndex(Number(event.target.value)); setSelected('') }}>{normalizedFilters.map((filter, index) => <option key={filter.label} value={index}>{filter.label}</option>)}</select>}
        <button type="button" className={`icon-button ${showHidden ? 'active' : ''}`} title={showHidden ? t('fileDialog.hideHidden') : t('fileDialog.showHidden')} aria-label={showHidden ? t('fileDialog.hideHidden') : t('fileDialog.showHidden')} onClick={() => setShowHidden((value) => !value)}>{showHidden ? <EyeOff size={15} /> : <Eye size={15} />}</button>
      </div>
      <div className="file-dialog-list" ref={listRef} tabIndex={0} role="listbox" aria-label={title} onKeyDown={(event) => {
        if (event.key === 'ArrowDown') { event.preventDefault(); selectIndex(selectedIndex < 0 ? 0 : Math.min(shown.length - 1, selectedIndex + 1)) }
        else if (event.key === 'ArrowUp') { event.preventDefault(); selectIndex(selectedIndex < 0 ? shown.length - 1 : Math.max(0, selectedIndex - 1)) }
        else if (event.key === 'Home') { event.preventDefault(); selectIndex(0) }
        else if (event.key === 'End') { event.preventDefault(); selectIndex(shown.length - 1) }
        else if (event.key === 'Enter') { event.preventDefault(); if (selectedEntry) openEntry(selectedEntry); else submit() }
      }}>
        {state.loading && !shown.length && <p className="file-message">{t('fileDialog.loading')}</p>}
        {!state.loading && state.error && <p className="file-message error-text">{state.error}</p>}
        {!state.loading && !state.error && !shown.length && <p className="file-message">{search.trim() ? t('fileDialog.noMatches') : t('fileDialog.empty')}</p>}
        {shown.map((entry, index) => <div key={entry.path} data-file-index={index} data-entry-kind={entry.kind} role="option" aria-selected={entry.path === selected} className={`file-row ${entry.path === selected ? 'selected' : ''}`}
          onClick={() => select(entry)} onDoubleClick={() => openEntry(entry)}>
          <span className="file-name"><EntryIcon kind={entry.kind} /><span title={entry.name}>{entry.name}</span></span>
        </div>)}
      </div>
      <footer className="file-dialog-footer">
        {mode !== 'directory' && <form className="file-dialog-name" onSubmit={(event) => { event.preventDefault(); submit() }}>
          <label>{t('fileDialog.fileName')}<input ref={nameRef} value={fileName} onChange={(event) => setFileName(event.target.value)} spellCheck={false} autoComplete="off" /></label>
        </form>}
        <div className="dialog-actions"><button type="button" className="secondary-button" onClick={onCancel}>{t('common.cancel')}</button><button type="button" className="primary-button" disabled={!canConfirm} onClick={submit}>{confirmLabel ?? (mode === 'save' ? t('common.save') : t('common.choose'))}</button></div>
      </footer>
    </section>
  </div>, document.querySelector('.app-shell') ?? document.body)
}
