import { useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { FolderOpen, X } from 'lucide-react'
import { open as openNativeDialog, save as saveNativeDialog, type DialogFilter } from '@tauri-apps/plugin-dialog'
import { useI18n } from '../i18n'
import type { DatabaseSqlExportProgress } from '../types'

export interface DatabaseFileDialogFilter {
  label: string
  extensions: string[]
}

export interface DatabaseFileDialogDetail {
  label: string
  value: string
}

export interface DatabaseFileDialogCompletion {
  title: string
  details: DatabaseFileDialogDetail[]
}

export interface DatabaseFileDialogProps {
  mode: 'open' | 'save'
  title: string
  defaultPath?: string
  defaultFileName?: string
  filters?: DatabaseFileDialogFilter[]
  showSchemaOnly?: boolean
  schemaOnlyLabel?: string
  confirmLabel?: string
  onConfirm(path: string, schemaOnly: boolean, onProgress: (progress: DatabaseSqlExportProgress) => void): void | Promise<DatabaseFileDialogCompletion | void>
  onCancel(): void
}

function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

function elapsedText(milliseconds: number): string {
  return milliseconds >= 1000 ? `${(milliseconds / 1000).toFixed(1)} s` : `${milliseconds} ms`
}

export function DatabaseFileDialog({ mode, title, defaultPath = '', defaultFileName = '', filters = [], showSchemaOnly = false, schemaOnlyLabel, confirmLabel, onConfirm, onCancel }: DatabaseFileDialogProps): React.JSX.Element {
  const { t } = useI18n()
  const [path, setPath] = useState(defaultPath || defaultFileName)
  const [schemaOnly, setSchemaOnly] = useState(false)
  const [error, setError] = useState('')
  const [submitting, setSubmitting] = useState(false)
  const [completion, setCompletion] = useState<DatabaseFileDialogCompletion | null>(null)
  const [elapsedMs, setElapsedMs] = useState(0)
  const [progress, setProgress] = useState<DatabaseSqlExportProgress | null>(null)
  const nativeFilters = useMemo<DialogFilter[]>(() => filters
    .filter((filter) => filter.extensions.length > 0)
    .map((filter) => ({ name: filter.label, extensions: filter.extensions })), [filters])

  useEffect(() => {
    const escape = (event: KeyboardEvent): void => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      event.stopPropagation()
      if (submitting) return
      onCancel()
    }
    window.addEventListener('keydown', escape, true)
    return () => window.removeEventListener('keydown', escape, true)
  }, [onCancel, submitting])

  const chooseNativePath = async (): Promise<void> => {
    setError('')
    try {
      const options = { title, filters: nativeFilters, defaultPath: path.trim() || undefined }
      const selected = mode === 'open'
        ? await openNativeDialog({ ...options, multiple: false, directory: false })
        : await saveNativeDialog(options)
      if (typeof selected === 'string') setPath(selected)
    } catch (caught) {
      setError(errorText(caught))
    }
  }

  const submit = async (): Promise<void> => {
    const value = path.trim()
    if (!value || submitting) return
    setError('')
    setCompletion(null)
    setElapsedMs(0)
    setProgress(null)
    setSubmitting(true)
    const startedAt = performance.now()
    try {
      const result = await onConfirm(value, schemaOnly, setProgress)
      if (result) {
        setCompletion(result)
        setElapsedMs(Math.max(0, Math.round(performance.now() - startedAt)))
      }
    } catch (caught) {
      setError(errorText(caught))
    } finally {
      setSubmitting(false)
    }
  }

  const progressTotal = progress?.total ?? 0
  const progressCurrent = progress ? Math.min(progressTotal, Math.max(0, progress.current)) : 0
  const progressPercent = progressTotal > 0 ? Math.round((progressCurrent / progressTotal) * 100) : 0
  const determinate = Boolean(progress && progressTotal > 0)

  return createPortal(<div className="modal-backdrop database-file-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !submitting) onCancel() }}>
    <section className="modal database-file-dialog" role="dialog" aria-modal="true" aria-label={title}>
      <header><strong>{title}</strong><button type="button" className="icon-button" title={t('common.close')} aria-label={t('common.close')} disabled={submitting} onClick={onCancel}><X size={18} /></button></header>
      <form className="form-grid" onSubmit={(event) => { event.preventDefault(); void submit() }}>
        <label>{t('fileDialog.path')}<div className="input-button">
          <input autoFocus={!completion} value={path} disabled={submitting} onChange={(event) => { setPath(event.target.value); setError('') }} spellCheck={false} autoComplete="off" />
          <button type="button" className="secondary-button" disabled={submitting} onClick={() => void chooseNativePath()}><FolderOpen size={14} />{t('common.choose')}</button>
        </div></label>
        {showSchemaOnly && <label className="check-label"><input type="checkbox" checked={schemaOnly} disabled={submitting} onChange={(event) => setSchemaOnly(event.target.checked)} />{schemaOnlyLabel ?? t('database.schemaOnlyExport')}</label>}
        {submitting && <div className="database-file-dialog-status" role="status" aria-live="polite"><div className="database-file-dialog-status-heading"><strong>{t('fileDialog.exporting')}</strong><span>{determinate ? t('fileDialog.exportProgress', { current: progressCurrent, total: progressTotal, percent: progressPercent }) : t('fileDialog.exportInProgress')}</span></div><div className="database-file-progress-table" title={progress?.table ?? ''} aria-hidden={!progress?.table}>{progress?.table ?? ''}</div><div className={determinate ? 'database-file-progress determinate' : 'database-file-progress'} role="progressbar" aria-label={t('fileDialog.exporting')} aria-valuemin={determinate ? 0 : undefined} aria-valuemax={determinate ? 100 : undefined} aria-valuenow={determinate ? progressPercent : undefined}><span style={determinate ? { width: `${progressPercent}%` } : undefined} /></div></div>}
        {completion && <div className="database-file-dialog-status database-file-dialog-complete" role="status" aria-live="polite"><strong>{completion.title}</strong><dl>{completion.details.map((detail) => <div key={detail.label}><dt>{detail.label}</dt><dd title={detail.value}>{detail.value}</dd></div>)}<div><dt>{t('fileDialog.elapsed')}</dt><dd>{elapsedText(elapsedMs)}</dd></div></dl></div>}
        {error && <p className="error-text">{error}</p>}
        <div className="dialog-actions"><button type="button" className="secondary-button" disabled={submitting} onClick={onCancel}>{t('common.cancel')}</button><button type="submit" className="primary-button" disabled={submitting || !path.trim()}>{submitting ? t('fileDialog.exporting') : completion ? t('fileDialog.exportAgain') : confirmLabel ?? (mode === 'save' ? t('common.save') : t('common.choose'))}</button></div>
      </form>
    </section>
  </div>, document.querySelector('.app-shell') ?? document.body)
}
