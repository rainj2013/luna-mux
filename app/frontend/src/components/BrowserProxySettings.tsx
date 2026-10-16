import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../i18n'
import type { BrowserResource } from '../types'

export function BrowserProxySettings({ resource, onSave }: {
  resource: BrowserResource & { status: string }
  onSave(proxyServer: string, enabled: boolean): Promise<void>
}): React.JSX.Element {
  const { t } = useI18n()
  const [proxyServer, setProxyServer] = useState(resource.proxyServer || '')
  const [enabled, setEnabled] = useState(resource.proxyEnabled)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const pending = useRef(false)
  const mounted = useRef(true)
  const pointerToggle = useRef(false)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  useEffect(() => {
    setProxyServer(resource.proxyServer || '')
    setEnabled(resource.proxyEnabled)
  }, [resource.id, resource.proxyServer, resource.proxyEnabled])
  const save = async (server = proxyServer, nextEnabled = enabled): Promise<void> => {
    if (pending.current || resource.status === 'starting') return
    if (server.trim() === resource.proxyServer && nextEnabled === resource.proxyEnabled) return
    pending.current = true
    setSaving(true)
    setError('')
    try { await onSave(server, nextEnabled) }
    catch {
      if (mounted.current) { setEnabled(resource.proxyEnabled); setError(t('app.browserProxySaveFailed')) }
    } finally {
      pending.current = false
      if (mounted.current) setSaving(false)
    }
  }
  return <section className="browser-setting-block browser-proxy-settings" onBlur={(event) => {
    const switching = pointerToggle.current
    pointerToggle.current = false
    if (!switching && !event.currentTarget.contains(event.relatedTarget)) void save()
  }}>
    <header>
      <strong>{t('app.browserProxySettings')}</strong>
      <label className="switch-control" onPointerDown={() => { if (!saving && resource.status !== 'starting') pointerToggle.current = true }} onPointerCancel={() => { pointerToggle.current = false }}>
        <input type="checkbox" role="switch" aria-label={t('app.browserProxyEnabled')} checked={enabled} disabled={saving || resource.status === 'starting'} onChange={(event) => {
          const next = event.target.checked
          pointerToggle.current = false
          setEnabled(next)
          void save(proxyServer, next)
        }} />
        <span aria-hidden="true" />
      </label>
    </header>
    <div className="form-grid browser-proxy-form">
      <label>{t('app.browserProxyServer')}<span className="browser-proxy-controls"><input value={proxyServer} onChange={(event) => { setProxyServer(event.target.value); setError('') }} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void save() } }} placeholder="http://127.0.0.1:7890" disabled={!enabled || saving || resource.status === 'starting'} autoComplete="off" spellCheck={false} maxLength={2048} /></span></label>
      <p className="browser-setting-copy">{t('app.browserProxyHelp')}</p>
      <p className="browser-setting-copy">{t('app.browserProxyRestart')}</p>
      {error && <div className="browser-resource-error" role="alert">{error}</div>}
    </div>
  </section>
}
