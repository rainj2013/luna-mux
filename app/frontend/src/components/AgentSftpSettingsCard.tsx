import { useEffect, useId, useRef, useState } from 'react'
import { useI18n } from '../i18n'

export function AgentSftpSettingsCard(): React.JSX.Element {
  const { t } = useI18n()
  const descriptionId = useId()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const pending = useRef(false)
  const mounted = useRef(false)

  useEffect(() => {
    mounted.current = true
    let active = true
    void window.api.settings.getAgentSftpEnabled().then((value) => {
      if (active) setEnabled(value)
    }).catch(() => { if (active) setError(t('app.agentSftpLoadFailed')) })
    return () => { active = false; mounted.current = false }
  }, [t])

  const save = async (value: boolean): Promise<void> => {
    if (enabled === null || pending.current) return
    pending.current = true
    setBusy(true)
    setError('')
    try {
      const saved = await window.api.settings.saveAgentSftpEnabled(value)
      if (mounted.current) setEnabled(saved)
    } catch {
      if (mounted.current) setError(t('app.agentSftpSaveFailed'))
    } finally {
      pending.current = false
      if (mounted.current) setBusy(false)
    }
  }

  return <section className="browser-settings-page">
    <header className="browser-page-heading"><h2>{t('app.agentSftpTitle')}</h2><p>{t('app.agentSftpDescription')}</p></header>
    <div className="browser-tools-list"><div className="browser-tool-row">
      <span className="browser-tool-copy"><strong>{t('app.agentSftpLabel')}</strong><small id={descriptionId}>{t('app.agentSftpHelp')}</small></span>
      <label className="switch-control">
        <input type="checkbox" role="switch" aria-label={t('app.agentSftpLabel')} aria-describedby={descriptionId} checked={enabled ?? false} disabled={enabled === null || busy} onChange={(event) => { void save(event.target.checked) }} />
        <span aria-hidden="true" />
      </label>
    </div></div>
    {error && <p className="browser-resource-error" role="alert">{error}</p>}
  </section>
}
