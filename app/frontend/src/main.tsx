import React from 'react'
import ReactDOM from 'react-dom/client'
import '@xterm/xterm/css/xterm.css'
import './styles.css'
import { App } from './App'
import { createTauriApi } from './tauri-api'

async function start(): Promise<void> {
  window.api = await createTauriApi()
  ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>)
}

void start().catch((error) => {
  const message = error instanceof Error ? error.message : String(error)
  ReactDOM.createRoot(document.getElementById('root')!).render(<div className="bootstrap-error" role="alert"><strong>应用启动失败</strong><span>{message}</span></div>)
})
