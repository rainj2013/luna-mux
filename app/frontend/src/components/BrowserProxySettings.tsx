import { useEffect, useId, useRef, useState } from 'react'
import { ChevronRight } from 'lucide-react'
import { useI18n } from '../i18n'
import type { BrowserResource } from '../types'

const DEFAULT_PROXY_SERVER = 'http://127.0.0.1:7890'

export function BrowserProxySettings({ resource, onSave }: {
  resource: BrowserResource & { status: string }
  onSave(proxyServer: string, enabled: boolean): Promise<void>
}): React.JSX.Element {
  const { t } = useI18n()
  const descriptionId = useId()
  const [proxyServer, setProxyServer] = useState(resource.proxyServer || DEFAULT_PROXY_SERVER)
  const [enabled, setEnabled] = useState(resource.proxyEnabled)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const pending = useRef(false)
  const mounted = useRef(true)
  const pointerToggle = useRef(false)
  useEffect(() => { mounted.current = true; return () => { mounted.current = false } }, [])
  useEffect(() => {
    setProxyServer(resource.proxyServer || DEFAULT_PROXY_SERVER)
    setEnabled(resource.proxyEnabled)
  }, [resource.id, resource.proxyServer, resource.proxyEnabled])
  const save = async (server = proxyServer, nextEnabled = enabled): Promise<void> => {
    if (pending.current || resource.status === 'starting') return
    const normalizedServer = server.trim() || DEFAULT_PROXY_SERVER
    if (normalizedServer === resource.proxyServer && nextEnabled === resource.proxyEnabled) {
      setProxyServer(normalizedServer)
      return
    }
    pending.current = true
    setSaving(true)
    setError('')
    try {
      await onSave(normalizedServer, nextEnabled)
      if (mounted.current) setProxyServer(normalizedServer)
    }
    catch {
      if (mounted.current) { setEnabled(resource.proxyEnabled); setError(t('app.browserProxySaveFailed')) }
    } finally {
      pending.current = false
      if (mounted.current) setSaving(false)
    }
  }
  return <section className="browser-settings-item browser-proxy-settings" onBlur={(event) => {
    const switching = pointerToggle.current
    pointerToggle.current = false
    if (!switching && !event.currentTarget.contains(event.relatedTarget)) void save()
  }}>
    <div className="browser-tool-row">
      <span className="browser-tool-copy">
        <strong>{t('app.browserProxySettings')}</strong>
        <small id={descriptionId}>{enabled ? resource.proxyServer || DEFAULT_PROXY_SERVER : t('app.browserSystemProxy')}</small>
      </span>
      <label className="switch-control" onPointerDown={() => { if (!saving && resource.status !== 'starting') pointerToggle.current = true }} onPointerCancel={() => { pointerToggle.current = false }}>
        <input type="checkbox" role="switch" aria-label={t('app.browserProxyEnabled')} aria-describedby={descriptionId} checked={enabled} disabled={saving || resource.status === 'starting'} onChange={(event) => {
          const next = event.target.checked
          pointerToggle.current = false
          setEnabled(next)
          void save(proxyServer, next)
        }} />
        <span aria-hidden="true" />
      </label>
    </div>
    <details className="browser-proxy-editor">
      <summary><ChevronRight size={14} />{t('app.browserProxyEditAddress')}</summary>
      <div className="form-grid browser-proxy-form">
        <label>{t('app.browserProxyServer')}<span className="browser-proxy-controls"><input value={proxyServer} onChange={(event) => { setProxyServer(event.target.value); setError('') }} onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); void save() } }} placeholder={DEFAULT_PROXY_SERVER} disabled={saving || resource.status === 'starting'} autoComplete="off" spellCheck={false} maxLength={2048} /></span></label>
        <p className="browser-setting-copy">{t('app.browserProxyHelp')}</p>
      </div>
    </details>
    {error && <div className="browser-resource-error" role="alert">{error}</div>}
  </section>
}
