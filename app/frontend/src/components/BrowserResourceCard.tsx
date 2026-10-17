import { useId, useState } from 'react'
import { Bot, ChevronRight, ExternalLink, Globe2, Play, Power, RotateCcw, Settings, ShieldAlert } from 'lucide-react'
import { useI18n, type MessageKey } from '../i18n'
import type { AgentBrowserToolProfile, BrowserResource, BrowserRuntime, BrowserRuntimeStatus, BrowserTunnel, ChromeInstallation, MuxPane } from '../types'
import { BrowserProxySettings } from './BrowserProxySettings'

export interface BrowserResourceState extends BrowserResource { runtime?: BrowserRuntime; tunnel?: BrowserTunnel; status: BrowserRuntimeStatus | 'stopped'; error?: string }
const AGENT_BROWSER_TOOL_GROUPS: ReadonlyArray<{ id: AgentBrowserToolProfile; count: number; label: MessageKey; description: MessageKey }> = [
  { id: 'core', count: 29, label: 'app.browserToolsCore', description: 'app.browserToolsCoreDescription' },
  { id: 'tabs', count: 13, label: 'app.browserToolsTabs', description: 'app.browserToolsTabsDescription' },
  { id: 'network', count: 9, label: 'app.browserToolsNetwork', description: 'app.browserToolsNetworkDescription' },
  { id: 'debug', count: 40, label: 'app.browserToolsDebug', description: 'app.browserToolsDebugDescription' },
  { id: 'state', count: 27, label: 'app.browserToolsState', description: 'app.browserToolsStateDescription' },
  { id: 'react', count: 8, label: 'app.browserToolsReact', description: 'app.browserToolsReactDescription' },
  { id: 'mobile', count: 15, label: 'app.browserToolsMobile', description: 'app.browserToolsMobileDescription' },
  { id: 'webmcp', count: 4, label: 'app.browserToolsWebmcp', description: 'app.browserToolsWebmcpDescription' }
]

export function BrowserResourceCard({ resource, sourcePane, chromeInstallation, onRefreshChrome, onSaveProxy, onStart, onFocus, onRestart, onStop, onRetainProfile, onToolProfiles }: { resource: BrowserResourceState; sourcePane?: Pick<MuxPane, 'title'>; chromeInstallation: ChromeInstallation | null | undefined; onRefreshChrome(): void; onSaveProxy(resource: BrowserResourceState, proxyServer: string, proxyEnabled: boolean): Promise<void>; onStart(resource: BrowserResourceState): void; onFocus(resource: BrowserResourceState): void; onRestart(resource: BrowserResourceState): void; onStop(resource: BrowserResourceState): void; onRetainProfile(resource: BrowserResourceState, retain: boolean): void; onToolProfiles(resource: BrowserResourceState, profiles: AgentBrowserToolProfile[]): void }): React.JSX.Element {
  const { t } = useI18n()
  const [page, setPage] = useState<'overview' | 'settings' | 'tools'>('overview')
  const pageId = useId()
  const pages = [
    { id: 'overview', title: t('app.browserOverview'), detail: t('app.browserOverviewNav'), icon: Globe2 },
    { id: 'settings', title: t('app.browserSettings'), detail: t('app.browserSettingsNav'), icon: Settings },
    { id: 'tools', title: t('app.browserAgentToolSettings'), detail: t('app.browserAgentToolCount', { value0: String(AGENT_BROWSER_TOOL_GROUPS.filter((group) => resource.toolProfiles.includes(group.id)).length) }), icon: Bot },
  ] as const
  const live = resource.status === 'running' && Boolean(resource.runtime)
  const statusLabel = resource.status === 'starting' ? t('app.connecting') : live ? t('app.browserRunning') : resource.status === 'error' ? t('app.browserRuntimeFailed') : t('app.browserRuntimeStopped')
  const agentAccessLabel = live ? t('app.browserAgentAccessReady') : chromeInstallation ? t('app.browserAgentAccessOnStart') : t('app.browserAgentAccessBlocked')
  const serviceLabel = sourcePane ? `${t('app.viaSshPane')} ${sourcePane.title}` : t('app.localService')
  const profileLocked = Boolean(resource.runtime) || resource.status === 'starting'
  const selectedCount = AGENT_BROWSER_TOOL_GROUPS.filter((group) => resource.toolProfiles.includes(group.id)).length
  const proxySummary = resource.proxyEnabled && resource.proxyServer ? t('app.browserCustomProxy') : t('app.browserSystemProxy')
  const profileSummary = resource.retainProfile ? t('app.retainBrowserData') : t('app.browserFreshProfile')
  const openPage = (next: typeof page): void => {
    setPage(next)
    document.getElementById(`${pageId}-${next}-tab`)?.focus()
  }
  const toggleGroup = (id: AgentBrowserToolProfile, enabled: boolean): void => {
    if (id === 'core') return
    const selected = new Set(resource.toolProfiles)
    if (enabled) selected.add(id); else selected.delete(id)
    selected.add('core')
    onToolProfiles(resource, AGENT_BROWSER_TOOL_GROUPS.map((group) => group.id).filter((profile) => selected.has(profile)))
  }
  return <section className={`browser-resource-detail ${live ? 'running' : resource.status}`}>
    <header>
      <div className="browser-resource-identity"><span className={`status-dot ${live ? 'connected' : resource.status === 'error' ? 'error' : resource.status === 'starting' ? 'connecting' : 'disconnected'}`} /><Globe2 size={18} /><span><strong>{resource.name}</strong><small>{serviceLabel}</small></span></div>
      <div className="browser-resource-actions">
        {live ? <button className="secondary-button" onClick={() => onFocus(resource)}><ExternalLink size={15} />{t('app.openManagedChrome')}</button> : <button className="primary-button" disabled={!chromeInstallation || resource.status === 'starting'} onClick={() => onStart(resource)}><Play size={15} />{t('app.startBrowser')}</button>}
        <button className="icon-button" title={t('app.restartBrowser')} aria-label={t('app.restartBrowser')} disabled={!resource.runtime || resource.status === 'starting'} onClick={() => onRestart(resource)}><RotateCcw size={15} /></button>
        <button className="icon-button" title={t('app.stopBrowser')} aria-label={t('app.stopBrowser')} disabled={!resource.runtime} onClick={() => onStop(resource)}><Power size={15} /></button>
      </div>
    </header>
    {chromeInstallation === null && <div className="browser-resource-warning"><ShieldAlert size={16} /><span><strong>{t('app.chromeNotFound')}</strong><small>{t('app.installChromeFirst')}</small></span><button className="icon-button" title={t('app.recheckBrowser')} aria-label={t('app.recheckBrowser')} onClick={onRefreshChrome}><RotateCcw size={15} /></button></div>}
    {resource.error && <div className="browser-resource-error" title={resource.error}><ShieldAlert size={15} />{resource.error}</div>}
    <div className="browser-workbench">
      <nav className="browser-page-nav" role="tablist" aria-label={t('app.browserSettings')} aria-orientation="vertical" onKeyDown={(event) => {
        const index = pages.findIndex((item) => item.id === page)
        const next = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? (index + 1) % pages.length : event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? (index + pages.length - 1) % pages.length : event.key === 'Home' ? 0 : event.key === 'End' ? pages.length - 1 : undefined
        if (next === undefined) return
        event.preventDefault()
        setPage(pages[next]!.id)
        event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus()
      }}>
        {pages.map(({ id, title, detail, icon: Icon }) => <button key={id} type="button" role="tab" id={`${pageId}-${id}-tab`} aria-controls={`${pageId}-${id}`} aria-selected={page === id} tabIndex={page === id ? 0 : -1} onClick={() => setPage(id)}><Icon size={17} /><span><strong>{title}</strong><small>{detail}</small></span></button>)}
      </nav>
      <div className="browser-page-area">
      <section className="browser-settings-page" id={`${pageId}-overview`} role="tabpanel" aria-labelledby={`${pageId}-overview-tab`} hidden={page !== 'overview'} tabIndex={0}>
        <header className="browser-page-heading"><h2>{t('app.browserOverview')}</h2><p>{t('app.browserOverviewDescription')}</p></header>
        <div className="browser-status-banner"><span className={`status-dot ${live ? 'connected' : resource.status === 'error' ? 'error' : resource.status === 'starting' ? 'connecting' : 'disconnected'}`} /><span><strong>{statusLabel}</strong><small>{t('app.agentBrowserAccess')} · {agentAccessLabel}</small></span></div>
        <dl className="browser-value-list">
          <div><dt>{sourcePane ? t('app.remoteServiceUrl') : t('app.initialUrl')}</dt><dd><code title={resource.url || 'about:blank'}>{resource.url || 'about:blank'}</code></dd></div>
          <div><dt>{t('app.browserServiceLocation')}</dt><dd>{serviceLabel}</dd></div>
        </dl>
        <div className="browser-summary-list">
          <button onClick={() => openPage('settings')}><span>{t('app.browserProxySettings')}</span><small>{proxySummary}</small><ChevronRight size={15} /></button>
          <button onClick={() => openPage('settings')}><span>{t('app.browserProfileSettings')}</span><small>{profileSummary}</small><ChevronRight size={15} /></button>
          <button onClick={() => openPage('tools')}><span>{t('app.browserAgentToolSettings')}</span><small>{t('app.browserAgentToolCount', { value0: String(selectedCount) })}</small><ChevronRight size={15} /></button>
        </div>
        {resource.runtime && <p className="browser-process-note">{t('app.chromeProcess')} · PID {resource.runtime.processId} · CDP {resource.runtime.cdpPort}</p>}
      </section>
      <section className="browser-settings-page" id={`${pageId}-settings`} role="tabpanel" aria-labelledby={`${pageId}-settings-tab`} hidden={page !== 'settings'} tabIndex={0}>
        <header className="browser-page-heading"><h2>{t('app.browserSettings')}</h2><p>{t('app.browserSettingsDescription')}</p></header>
        <div className="browser-resource-settings">
        <BrowserProxySettings resource={resource} onSave={(server, enabled) => onSaveProxy(resource, server, enabled)} />
        <section className="browser-setting-block browser-profile-settings">
          <header><strong>{t('app.retainBrowserData')}</strong>
            <label className="switch-control" title={profileLocked ? t('app.stopBrowserBeforeProfileChange') : undefined}>
              <input type="checkbox" role="switch" aria-label={t('app.retainBrowserData')} checked={resource.retainProfile} disabled={profileLocked} onChange={(event) => onRetainProfile(resource, event.target.checked)} /><span aria-hidden="true" />
            </label>
          </header>
          <p className="browser-setting-copy">{resource.retainProfile ? t('app.browserProfileRetained') : t('app.browserProfileDisposable')}</p>
          {profileLocked && <p className="browser-setting-copy">{t('app.stopBrowserBeforeProfileChange')}</p>}
        </section>
        </div>
      </section>
      <section className="browser-settings-page" id={`${pageId}-tools`} role="tabpanel" aria-labelledby={`${pageId}-tools-tab`} hidden={page !== 'tools'} tabIndex={0}>
        <header className="browser-page-heading"><h2>{t('app.browserAgentToolSettings')}</h2><p>{t('app.browserAgentToolsDescription')}</p></header>
        <div className="browser-tools-list">{AGENT_BROWSER_TOOL_GROUPS.map((group) => {
          const checked = resource.toolProfiles.includes(group.id)
          const required = group.id === 'core'
          return <div className="browser-tool-row" key={group.id}>
            <span className="browser-tool-copy">
              <strong>{t(group.label)} <small>{required ? t('app.required') : t('app.browserAgentToolGroupCount', { value0: String(group.count) })}</small></strong>
              <small id={`${pageId}-${group.id}-description`}>{t(group.description)}</small>
            </span>
            <label className="switch-control"><input type="checkbox" role="switch" aria-label={t(group.label)} aria-describedby={`${pageId}-${group.id}-description`} checked={checked} disabled={required} onChange={(event) => toggleGroup(group.id, event.target.checked)} /><span aria-hidden="true" /></label>
          </div>
        })}</div>
        <p className="browser-setting-copy">{t('app.browserAgentToolsRestartNote')}</p>
      </section>
      </div>
    </div>
  </section>
}
