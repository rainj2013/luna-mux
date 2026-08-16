import React from 'react'
import ReactDOM from 'react-dom/client'
import '@xterm/xterm/css/xterm.css'
import './styles.css'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { AppLanguage } from './types'

async function start(): Promise<void> {
  if (getCurrentWindow().label === 'agent-notification') {
    const { AgentNotificationWindow } = await import('./components/AgentNotificationWindow')
    document.documentElement.classList.add('notification-window-document')
    ReactDOM.createRoot(document.getElementById('root')!).render(<AgentNotificationWindow />)
    return
  }
  const [{ App }, { getNativeMenuLabels, I18nProvider, resolveLanguage }, { createTauriApi }] = await Promise.all([
    import('./App'),
    import('./i18n'),
    import('./tauri-api'),
  ])
  function LocalizedApp({ initialLanguage }: { initialLanguage: AppLanguage }): React.JSX.Element {
    const [language, setLanguage] = React.useState(initialLanguage)
    return <I18nProvider language={language} setLanguage={setLanguage}><App /></I18nProvider>
  }
  window.api = await createTauriApi()
  const language = resolveLanguage(await window.api.settings.getLanguage())
  await window.api.settings.applyLanguage(getNativeMenuLabels(language))
  ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><LocalizedApp initialLanguage={language} /></React.StrictMode>)
}

void start().catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  ReactDOM.createRoot(document.getElementById('root')!).render(<div className="bootstrap-error" role="alert"><strong>应用启动失败</strong><span>{message}</span></div>)
})
