import React from 'react'
import ReactDOM from 'react-dom/client'
import '@xterm/xterm/css/xterm.css'
import './styles.css'
import { App } from './App'
import { getNativeMenuLabels, I18nProvider, resolveLanguage } from './i18n'
import { createTauriApi } from './tauri-api'
import type { AppLanguage } from './types'

function LocalizedApp({ initialLanguage }: { initialLanguage: AppLanguage }): React.JSX.Element {
  const [language, setLanguage] = React.useState(initialLanguage)
  return <I18nProvider language={language} setLanguage={setLanguage}><App /></I18nProvider>
}

async function start(): Promise<void> {
  window.api = await createTauriApi()
  const language = resolveLanguage(await window.api.settings.getLanguage())
  await window.api.settings.applyLanguage(getNativeMenuLabels(language))
  ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><LocalizedApp initialLanguage={language} /></React.StrictMode>)
}

void start().catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  ReactDOM.createRoot(document.getElementById('root')!).render(<div className="bootstrap-error" role="alert"><strong>应用启动失败</strong><span>{message}</span></div>)
})
