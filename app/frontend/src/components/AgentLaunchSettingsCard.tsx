import { useEffect, useId, useRef, useState } from 'react'
import { useI18n } from '../i18n'
import type { AgentLaunchSettings } from '../types'

export function AgentLaunchSettingsCard(): React.JSX.Element {
  const { t } = useI18n()
  const descriptionId = useId()
  const [settings, setSettings] = useState<AgentLaunchSettings | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const pending = useRef(false)
  const mounted = useRef(false)
  useEffect(() => {
    mounted.current = true
    let active = true
    void window.api.settings.getAgentLaunch().then((value) => {
      if (active) setSettings(value)
    }).catch(() => { if (active) setError(t('app.agentLaunchLoadFailed')) })
    return () => { active = false; mounted.current = false }
  }, [t])

  const save = async (key: keyof AgentLaunchSettings, enabled: boolean): Promise<void> => {
    if (!settings || pending.current) return
    pending.current = true
    setBusy(true)
    setError('')
    try {
      const value = await window.api.settings.saveAgentLaunch({ ...settings, [key]: enabled })
      if (mounted.current) setSettings(value)
    } catch {
      if (mounted.current) setError(t('app.agentLaunchSaveFailed'))
    } finally {
      pending.current = false
      if (mounted.current) setBusy(false)
    }
  }
  return <section className="browser-settings-page agent-launch-settings">
    <header className="browser-page-heading"><h2>{t('app.agentLaunchDefaults')}</h2><p>{t('app.agentLaunchYoloHelp')}</p></header>
    <div className="browser-tools-list">{([
      ['codexYolo', 'Codex', 'app.agentLaunchCodexHelp'],
      ['claudeCodeYolo', 'Claude Code', 'app.agentLaunchClaudeHelp']
    ] as const).map(([key, name, help]) => <div className="browser-tool-row" key={key}>
      <span className="browser-tool-copy">
        <strong>{name}</strong>
        <small id={`${descriptionId}-${key}`}>{t(help)}</small>
      </span>
      <label className="switch-control">
        <input type="checkbox" role="switch" aria-label={t('app.agentLaunchYoloLabel', { name })} aria-describedby={`${descriptionId}-${key}`} checked={settings?.[key] ?? false} disabled={!settings || busy} onChange={(event) => { void save(key, event.target.checked) }} />
        <span aria-hidden="true" />
      </label>
    </div>)}</div>
    {error && <p className="browser-resource-error" role="alert">{error}</p>}
  </section>
}
