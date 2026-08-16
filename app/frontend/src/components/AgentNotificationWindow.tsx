import { useEffect, useState } from 'react'
import { CircleAlert, CircleCheck, TriangleAlert, X } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import type { ManagedAgentDesktopNotification, UiTheme } from '../types'

const notificationWindow = getCurrentWindow()

export function AgentNotificationWindow(): React.JSX.Element {
  const [notification, setNotification] = useState<ManagedAgentDesktopNotification | null>(null)
  const [theme, setTheme] = useState<UiTheme>('system')
  const [systemDark, setSystemDark] = useState(() => window.matchMedia('(prefers-color-scheme: dark)').matches)
  const [paused, setPaused] = useState(false)

  useEffect(() => {
    void invoke<UiTheme>('settings_get_ui_theme').then(setTheme).catch((error) => console.warn('Failed to load notification theme', error))
    const stopNotification = listen<ManagedAgentDesktopNotification>('managed-agent:desktop-notification', (event) => {
      setPaused(false)
      setNotification(event.payload)
    })
    const stopTheme = listen<UiTheme>('ui-theme:changed', (event) => setTheme(event.payload))
    return () => {
      void stopNotification.then((stop) => stop())
      void stopTheme.then((stop) => stop())
    }
  }, [])

  useEffect(() => {
    const media = window.matchMedia('(prefers-color-scheme: dark)')
    const update = (): void => setSystemDark(media.matches)
    media.addEventListener('change', update)
    return () => media.removeEventListener('change', update)
  }, [])

  useEffect(() => {
    if (!notification || paused) return
    const timer = window.setTimeout(() => {
      setNotification(null)
      void notificationWindow.hide()
    }, 8_000)
    return () => window.clearTimeout(timer)
  }, [notification, paused])

  const hide = (): void => {
    setNotification(null)
    void notificationWindow.hide()
  }

  const activate = async (): Promise<void> => {
    if (!notification) return
    try {
      await invoke('managed_agents_activate_notification', {
        muxSessionId: notification.muxSessionId,
        paneId: notification.paneId,
        sequence: notification.sequence,
      })
    } catch (error) {
      console.error('Failed to activate Agent notification target', error)
    } finally {
      hide()
    }
  }

  const resolvedTheme = theme === 'system' ? (systemDark ? 'dark' : 'light') : theme
  const tone = notification?.status === 'error' ? 'error' : notification?.status === 'waiting' ? 'warning' : 'info'
  const Icon = tone === 'error' ? TriangleAlert : tone === 'warning' ? CircleAlert : CircleCheck

  return <main className="app-shell agent-notification-shell" data-theme={resolvedTheme}>
    {notification && <article
      className={`agent-notification-card tone-${tone}`}
      onClick={() => void activate()}
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
      role="button"
      aria-label={`${notification.title}: ${notification.body}`}
    >
      <span className="agent-notification-app-icon" aria-hidden="true">
        <svg viewBox="0 0 64 64">
          <rect x="2" y="2" width="60" height="60" rx="15" />
          <path className="moon" d="M28 11C17 14 10 24 10 35c0 13 11 24 24 24 5 0 10-2 14-4-2 .7-5 1-7 1-13 0-24-11-24-24 0-8 4-16 11-20z" />
          <path className="mux" d="M29 23h9l7 9h8M29 32h24M29 41h9l7-9" />
        </svg>
        <span className={`agent-notification-status tone-${tone}`}><Icon size={11} strokeWidth={2.6} /></span>
      </span>
      <span className="agent-notification-copy">
        <span className="agent-notification-source">Luna Mux <small>现在</small></span>
        <strong>{notification.title}</strong>
        <span className="agent-notification-body">{notification.body}</span>
      </span>
      <button className="agent-notification-close" type="button" aria-label="关闭" onClick={(event) => { event.stopPropagation(); hide() }}><X size={15} /></button>
    </article>}
  </main>
}
