import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserResourceCard, type BrowserResourceState } from '../app/frontend/src/components/BrowserResourceCard'
import { I18nProvider } from '../app/frontend/src/i18n'
import '../app/frontend/src/styles.css'

const initial: BrowserResourceState = {
  id: 'fixture', muxSessionId: 'fixture', name: '开发预览', sourcePaneId: '', bookmarkId: '',
  url: 'about:blank', retainProfile: false, proxyServer: '', proxyEnabled: false,
  toolProfiles: ['core', 'tabs'], sortOrder: 0, createdAt: '', updatedAt: '', status: 'stopped'
}

function Fixture(): React.JSX.Element {
  const [resource, setResource] = useState(initial)
  const [dark, setDark] = useState(true)
  const [failSave, setFailSave] = useState(false)
  return <I18nProvider language="zh-CN" setLanguage={() => {}}>
    <div className="app-shell" data-theme={dark ? 'dark' : 'light'} style={{ display: 'block', padding: 20 }}>
      <div style={{ display: 'flex', gap: 8, marginBottom: 20 }}>
        <button onClick={() => setDark((value) => !value)}>切换主题</button>
        <button onClick={() => setFailSave(true)}>下次保存失败</button>
        <button onClick={() => setResource((value) => ({ ...value, status: value.status === 'starting' ? 'stopped' : 'starting' }))}>切换启动中</button>
      </div>
      <BrowserResourceCard resource={resource} chromeInstallation={undefined} onRefreshChrome={() => {}}
        onSaveProxy={async (_, proxyServer, proxyEnabled) => {
          await new Promise((resolve) => setTimeout(resolve, 150))
          if (failSave) { setFailSave(false); throw new Error('fixture save failed') }
          setResource((value) => ({ ...value, proxyServer, proxyEnabled }))
        }}
        onRetainProfile={(_, retainProfile) => setResource((value) => ({ ...value, retainProfile }))}
        onToolProfiles={(_, toolProfiles) => setResource((value) => ({ ...value, toolProfiles }))}
        onStart={() => {}} onFocus={() => {}} onRestart={() => {}} onStop={() => {}} />
      <output id="stored" style={{ display: 'none' }}>{JSON.stringify(resource)}</output>
    </div>
  </I18nProvider>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
