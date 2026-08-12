import { useEffect, useMemo, useState } from 'react'
import { CircleHelp, Search, X } from 'lucide-react'
import { useI18n } from '../i18n'

export function HelpDialog({ onClose }: { onClose(): void }): React.JSX.Element {
  const { helpSections, t } = useI18n()
  const commandKey = window.api.platform === 'darwin' ? 'Cmd' : 'Ctrl'
  const sections = useMemo(() => helpSections(commandKey), [commandKey, helpSections])
  const [active, setActive] = useState('start')
  const [query, setQuery] = useState('')
  const normalizedQuery = query.trim().toLowerCase()
  const matches = normalizedQuery ? sections.filter((section) => `${section.title} ${section.searchText}`.toLowerCase().includes(normalizedQuery)) : sections
  const shown = normalizedQuery ? matches : sections.filter((section) => section.id === active)

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return <div className="help-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
    <section className="help-dialog" role="dialog" aria-modal="true" aria-label={t('common.help')}>
      <header><div className="help-title"><CircleHelp size={18} /><strong>{t('common.help')}</strong></div><label className="help-search"><Search size={15} /><input autoFocus placeholder={t('help.searchHelp')} value={query} onChange={(event) => setQuery(event.target.value)} /></label><button className="icon-button" title={t('help.closeHelp')} onClick={onClose}><X size={18} /></button></header>
      <div className="help-body">
        <nav aria-label={t('help.helpTopics')}>{matches.map((section) => { const Icon = section.icon; return <button key={section.id} className={!normalizedQuery && active === section.id ? 'active' : ''} onClick={() => { setActive(section.id); setQuery('') }}><Icon size={16} /><span>{section.title}</span></button> })}</nav>
        <main>{shown.length ? shown.map((section) => <article key={section.id} className="help-article">{section.content}</article>) : <div className="help-no-results"><Search size={24} /><strong>{t('help.noMatchingHelpTopics')}</strong><span>{t('help.tryConnectionSftpOrPortForwarding')}</span></div>}</main>
      </div>
    </section>
  </div>
}
