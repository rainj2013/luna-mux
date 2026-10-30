import { useId, useState, type ReactNode } from 'react'
import { Bot, Settings } from 'lucide-react'
import { useI18n } from '../i18n'
import { AgentLaunchSettingsCard } from './AgentLaunchSettingsCard'
import { AgentSftpSettingsCard } from './AgentSftpSettingsCard'

export function AgentWorkbench({ children, approvals, approvalCount = 0 }: { children: ReactNode; approvals?: ReactNode; approvalCount?: number }): React.JSX.Element {
  const { t } = useI18n()
  const [page, setPage] = useState<'overview' | 'settings'>('overview')
  const pageId = useId()
  const pages = [
    { id: 'overview', title: t('app.agentOverview'), detail: t('app.agentOverviewNav'), icon: Bot },
    { id: 'settings', title: t('app.agentLaunchDefaults'), detail: t('app.agentSettingsNav'), icon: Settings }
  ] as const
  return <div className="agent-environment">
    <section className="browser-resource-detail">
      <header><div className="browser-resource-identity"><Bot size={18} /><span><strong>{t('app.agents')}</strong></span></div></header>
      <div className="browser-workbench">
        <nav className="browser-page-nav" role="tablist" aria-label={t('app.agents')} aria-orientation="vertical" onKeyDown={(event) => {
          const index = pages.findIndex((item) => item.id === page)
          const next = event.key === 'ArrowDown' || event.key === 'ArrowRight' ? (index + 1) % pages.length : event.key === 'ArrowUp' || event.key === 'ArrowLeft' ? (index + pages.length - 1) % pages.length : event.key === 'Home' ? 0 : event.key === 'End' ? pages.length - 1 : undefined
          if (next === undefined) return
          event.preventDefault()
          setPage(pages[next]!.id)
          event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]')[next]?.focus()
        }}>
          {pages.map(({ id, title, detail, icon: Icon }) => <button key={id} type="button" role="tab" id={`${pageId}-${id}-tab`} aria-controls={`${pageId}-${id}`} aria-selected={page === id} tabIndex={page === id ? 0 : -1} onClick={() => setPage(id)}><Icon size={17} /><span><strong>{title}{id === 'overview' && approvalCount > 0 && <span className="transfer-approval-badge">{approvalCount}</span>}</strong><small>{detail}</small></span></button>)}
        </nav>
        <div className="browser-page-area">
          <section className="browser-settings-page" id={`${pageId}-overview`} role="tabpanel" aria-labelledby={`${pageId}-overview-tab`} hidden={page !== 'overview'} tabIndex={0}>
            <header className="browser-page-heading"><h2>{t('app.agentOverview')}</h2><p>{t('app.agentOverviewDescription')}</p></header>
            {approvals}
            {children}
          </section>
          <section className="browser-settings-page" id={`${pageId}-settings`} role="tabpanel" aria-labelledby={`${pageId}-settings-tab`} hidden={page !== 'settings'} tabIndex={0}>
            <AgentLaunchSettingsCard />
            <AgentSftpSettingsCard />
          </section>
        </div>
      </div>
    </section>
  </div>
}
