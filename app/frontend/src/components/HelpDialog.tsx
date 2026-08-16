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
  const matches = normalizedQuery ? sections.filter((section) => `${section.group} ${section.title} ${section.searchText}`.toLowerCase().includes(normalizedQuery)) : sections
  const shown = normalizedQuery ? matches : sections.filter((section) => section.id === active)
  const groupedMatches = matches.reduce<Map<string, typeof matches>>((groups, section) => {
    const group = groups.get(section.group) ?? []
    group.push(section)
    groups.set(section.group, group)
    return groups
  }, new Map())

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent): void => { if (event.key === 'Escape') onClose() }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [onClose])

  return <div className="help-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
    <section className="help-dialog" role="dialog" aria-modal="true" aria-label={t('common.help')}>
      <header><div className="help-title"><CircleHelp size={18} /><strong>{t('common.help')}</strong></div><label className="help-search"><Search size={15} /><input autoFocus placeholder={t('help.searchHelp')} value={query} onChange={(event) => setQuery(event.target.value)} /></label><button className="icon-button" title={t('help.closeHelp')} onClick={onClose}><X size={18} /></button></header>
      <div className="help-body">
        <nav aria-label={t('help.helpTopics')}>{[...groupedMatches].map(([group, groupSections]) => <div className="help-nav-group" key={group}><span>{group}</span>{groupSections.map((section) => { const Icon = section.icon; return <button key={section.id} className={!normalizedQuery && active === section.id ? 'active' : ''} onClick={() => { setActive(section.id); setQuery('') }}><Icon size={16} /><span>{section.title}</span></button> })}</div>)}</nav>
        <main>{shown.length ? shown.map((section) => <article key={section.id} className="help-article">{section.content}</article>) : <div className="help-no-results"><Search size={24} /><strong>{t('help.noMatchingHelpTopics')}</strong><span>{t('help.tryConnectionSftpOrPortForwarding')}</span></div>}</main>
      </div>
    </section>
  </div>
}
