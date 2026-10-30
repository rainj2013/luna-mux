import { useEffect, useRef, useState } from 'react'
import { ArrowRight, ShieldAlert, ShieldCheck } from 'lucide-react'
import { useI18n } from '../i18n'
import type { PendingTransferApproval } from '../types'

export function useTransferApprovals() {
  const [pending, setPending] = useState<PendingTransferApproval[]>([])
  const [busyIds, setBusyIds] = useState<Set<string>>(new Set())
  const [error, setError] = useState('')
  const [loadError, setLoadError] = useState('')
  const [refreshVersion, setRefreshVersion] = useState(0)
  const revision = useRef(0)
  const resolving = useRef(new Set<string>())
  const mounted = useRef(false)

  useEffect(() => {
    mounted.current = true
    let stopped = false
    let timer: ReturnType<typeof setTimeout>
    const refresh = async (): Promise<void> => {
      const version = revision.current
      try {
        const requests = await window.api.transferApprovals.list()
        if (!stopped && version === revision.current) { setPending(requests); setLoadError('') }
      } catch (cause) {
        if (!stopped) setLoadError(cause instanceof Error ? cause.message : String(cause))
      } finally {
        if (!stopped) timer = setTimeout(() => void refresh(), 1000)
      }
    }
    void refresh()
    return () => { stopped = true; mounted.current = false; clearTimeout(timer) }
  }, [refreshVersion])

  const resolve = async (approvalId: string, approved: boolean): Promise<void> => {
    if (resolving.current.has(approvalId)) return
    resolving.current.add(approvalId)
    revision.current += 1
    setBusyIds(new Set(resolving.current))
    setError('')
    try {
      await window.api.transferApprovals.resolve(approvalId, approved)
      if (mounted.current) setPending((items) => items.filter((item) => item.approval.approvalId !== approvalId))
    } catch (cause) {
      if (mounted.current) setError(cause instanceof Error ? cause.message : String(cause))
    } finally {
      resolving.current.delete(approvalId)
      revision.current += 1
      if (mounted.current) {
        setBusyIds(new Set(resolving.current))
        setRefreshVersion((value) => value + 1)
      }
    }
  }
  return { pending, busyIds, error: error || loadError, resolve, retry: () => { setError(''); setLoadError(''); setRefreshVersion((value) => value + 1) } }
}

export type TransferApprovalsState = ReturnType<typeof useTransferApprovals>

export function TransferApprovalCards({ state }: { state: TransferApprovalsState }): React.JSX.Element | null {
  const { t } = useI18n()
  if (!state.pending.length && !state.error) return null
  return <section className="transfer-approval-section" aria-label={t('app.transferApprovals')}>
    <header><ShieldCheck size={17} /><strong>{t('app.transferApprovals')}</strong><span>{state.pending.length}</span></header>
    {state.error && <div className="browser-resource-error" role="alert"><span>{state.error}</span><button type="button" className="text-button" onClick={state.retry}>{t('app.transferApprovalRetry')}</button></div>}
    {state.pending.map((item) => {
      const id = item.approval.approvalId
      const busy = state.busyIds.has(id)
      return <article className="transfer-approval-card" key={id}>
        <header><strong>{item.callerName}</strong><span>{t(item.request.direction === 'upload' ? 'app.transferApprovalUpload' : 'app.transferApprovalDownload')}</span></header>
        <p className="transfer-approval-target">{item.targetName}</p>
        <div className="transfer-approval-files">
          <div className="transfer-approval-file-heading"><span>{t('app.transferApprovalSource')}</span><span /><span>{t('app.transferApprovalDestination')}</span></div>
          {item.request.sourcePaths.map((path, index) => <div className="transfer-approval-file" key={index}><code>{path}</code><ArrowRight size={14} aria-hidden="true" /><code>{item.destinationPaths[index]}</code></div>)}
        </div>
        <p className="transfer-approval-help">{t('app.transferApprovalOnce', { time: new Date(item.approval.expiresAt).toLocaleTimeString() })}</p>
        <div className="dialog-actions">
          <button type="button" className="secondary-button" disabled={busy} onClick={() => void state.resolve(id, false)}>{t('app.transferApprovalReject')}</button>
          <button type="button" className="primary-button" disabled={busy} onClick={() => void state.resolve(id, true)}>{t('app.transferApprovalAllow')}</button>
        </div>
      </article>
    })}
  </section>
}

export function TransferApprovalDialog({ state }: { state: TransferApprovalsState }): React.JSX.Element | null {
  const { t } = useI18n()
  const item = state.pending[0]
  if (!item) return null
  const id = item.approval.approvalId
  const busy = state.busyIds.has(id)
  return <div className="modal-backdrop raised transfer-approval-modal">
    <section className="modal transfer-approval-dialog" role="dialog" aria-modal="true" aria-labelledby="transfer-approval-title">
      <header><div className="transfer-approval-dialog-title"><span className="transfer-approval-dialog-icon"><ShieldCheck size={17} /></span><div><strong id="transfer-approval-title">{t('app.transferApprovals')}</strong><small>{item.callerName}</small></div></div><span className="transfer-approval-dialog-count">1 / {state.pending.length}</span></header>
      <div className="transfer-approval-dialog-body">
        <div className="transfer-approval-dialog-heading"><strong>{t(item.request.direction === 'upload' ? 'app.transferApprovalUpload' : 'app.transferApprovalDownload')}</strong><span>{item.targetName}</span></div>
        <div className="transfer-approval-files">
          <div className="transfer-approval-file-heading"><span>{t('app.transferApprovalSource')}</span><span /><span>{t('app.transferApprovalDestination')}</span></div>
          {item.request.sourcePaths.map((path, index) => <div className="transfer-approval-file" key={index}><code>{path}</code><ArrowRight size={14} aria-hidden="true" /><code>{item.destinationPaths[index]}</code></div>)}
        </div>
        <p className="transfer-approval-help">{t('app.transferApprovalOnce', { time: new Date(item.approval.expiresAt).toLocaleTimeString() })}</p>
        {state.error && <div className="browser-resource-error" role="alert">{state.error}</div>}
        <div className="dialog-actions"><button type="button" className="secondary-button" disabled={busy} onClick={() => void state.resolve(id, false)}>{t('app.transferApprovalReject')}</button><button type="button" className="primary-button" disabled={busy} onClick={() => void state.resolve(id, true)}>{t('app.transferApprovalAllow')}</button></div>
      </div>
    </section>
  </div>
}

export function TransferApprovalNotice({ count, onOpen }: { count: number; onOpen(): void }): React.JSX.Element {
  const { t } = useI18n()
  return <div className="transfer-approval-notice" role="status"><ShieldAlert size={18} /><span>{t('app.transferApprovalNotice', { count })}</span><button type="button" className="text-button" onClick={onOpen}>{t('app.transferApprovalView')}</button></div>
}
