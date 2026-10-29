import { useState } from 'react'
import { Columns2, Globe2, Sparkles } from 'lucide-react'
import { createRoot } from 'react-dom/client'
import { AgentWorkbench } from '../app/frontend/src/components/AgentWorkbench'
import { I18nProvider } from '../app/frontend/src/i18n'
import type { AgentLaunchSettings, AppApi } from '../app/frontend/src/types'
import '../app/frontend/src/styles.css'

// A component fixture: the production card uses a mock of the persisted API.
// Exercise toggles, reload, delayed saves, save/load errors and both themes.
let stored: AgentLaunchSettings = { codexYolo: false, claudeCodeYolo: false }
let failSave = false
let failLoad = false
window.api = { settings: {
  async getAgentLaunch() {
    if (failLoad) { failLoad = false; throw new Error('load fixture failure') }
    return { ...stored }
  },
  async saveAgentLaunch(value: AgentLaunchSettings) {
    await new Promise((resolve) => setTimeout(resolve, 300))
    if (failSave) { failSave = false; throw new Error('save fixture failure') }
    stored = { ...value }
    return { ...stored }
  }
} } as AppApi

function Fixture(): React.JSX.Element {
  const [revision, setRevision] = useState(0)
  const [dark, setDark] = useState(false)
  const [view, setView] = useState('agents')
  return <I18nProvider language="zh-CN" setLanguage={() => {}}>
    <div className="app-shell" data-theme={dark ? 'dark' : 'light'} style={{ gridTemplateRows: 'auto auto minmax(0, 1fr)' }}>
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8, padding: 12 }}>
        <button onClick={() => setRevision((value) => value + 1)}>重新加载设置</button>
        <button onClick={() => { failSave = true }}>下次保存失败</button>
        <button onClick={() => { failLoad = true; setRevision((value) => value + 1) }}>下次加载失败</button>
        <button onClick={() => setDark((value) => !value)}>切换主题</button>
      </div>
      <div className="session-toolbar">
        <div className="session-toolbar-leading" />
        <div className="view-switcher" role="tablist" aria-label="会话视图">
          {([['terminal', '窗格', Columns2], ['agents', 'Agent', Sparkles], ['browser', '浏览器', Globe2]] as const).map(([id, label, Icon]) => <button key={id} role="tab" aria-selected={view === id} className={view === id ? 'active' : ''} onClick={() => setView(id)}><Icon size={15} />{label}</button>)}
        </div>
        <div className="session-actions" />
      </div>
      <AgentWorkbench key={revision}><section className="agent-environment-agents"><header><strong>活跃 Agent 运行环境</strong><span>0 个需要处理</span></header><div className="agent-environment-empty">当前会话没有活跃的 Agent</div></section></AgentWorkbench>
    </div>
  </I18nProvider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
