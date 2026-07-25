import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebglAddon } from '@xterm/addon-webgl'
import { Plug } from 'lucide-react'
import type { TerminalSettings } from '../../../shared/types'
import { colorWithOpacity, terminalBackgroundStyle } from '../terminal-style'

const terminalHighWaterMark = 1024 * 1024
const terminalLowWaterMark = 256 * 1024

export function TerminalPane({ sessionId, visible, settings, backgroundImage, onConnect }: { sessionId?: string; visible: boolean; settings: TerminalSettings; backgroundImage: string; onConnect?: () => void }): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const fitAddon = useRef<FitAddon | null>(null)

  useEffect(() => {
    if (!sessionId || !container.current) return
    const term = new Terminal({
      cursorBlink: true, convertEol: true, allowTransparency: true, fontFamily: settings.fontFamily,
      fontSize: settings.fontSize, lineHeight: 1.25, scrollback: 5000,
      theme: { background: colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity), foreground: settings.foregroundColor, cursor: '#78d64b', selectionBackground: '#315d8c' }
    })
    const fit = new FitAddon(); fitAddon.current = fit; term.loadAddon(fit); term.open(container.current)
    let webgl: WebglAddon | undefined
    try {
      webgl = new WebglAddon()
      webgl.onContextLoss(() => {
        webgl?.dispose()
        webgl = undefined
        if (container.current) container.current.dataset.renderer = 'dom'
      })
      term.loadAddon(webgl)
      container.current.dataset.renderer = 'webgl'
    } catch {
      webgl?.dispose()
      container.current.dataset.renderer = 'dom'
    }
    fit.fit(); terminal.current = term
    let pendingOutput = 0
    let paused = false
    const input = term.onData((data) => window.api.sessions.write(sessionId, data))
    const resize = term.onResize(({ cols, rows }) => window.api.sessions.resize(sessionId, cols, rows))
    const stop = window.api.onEvent((event) => {
      if (event.type !== 'terminal-data' || event.payload.sessionId !== sessionId) return
      const length = event.payload.data.length
      pendingOutput += length
      if (!paused && pendingOutput >= terminalHighWaterMark) {
        paused = true
        window.api.sessions.flow(sessionId, true)
      }
      term.write(event.payload.data, () => {
        pendingOutput = Math.max(0, pendingOutput - length)
        if (paused && pendingOutput <= terminalLowWaterMark) {
          paused = false
          window.api.sessions.flow(sessionId, false)
        }
      })
    })
    const observer = new ResizeObserver(() => { if (container.current?.offsetParent) requestAnimationFrame(() => { fit.fit(); window.api.sessions.resize(sessionId, term.cols, term.rows) }) })
    observer.observe(container.current)
    term.focus()
    return () => { if (paused) window.api.sessions.flow(sessionId, false); stop(); observer.disconnect(); input.dispose(); resize.dispose(); term.dispose(); terminal.current = null; fitAddon.current = null }
  }, [sessionId])

  useEffect(() => {
    const term = terminal.current
    if (!term) return
    term.options.fontFamily = settings.fontFamily
    term.options.fontSize = settings.fontSize
    term.options.theme = { ...term.options.theme, background: colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity), foreground: settings.foregroundColor }
    requestAnimationFrame(() => fitAddon.current?.fit())
  }, [settings])

  useEffect(() => { if (visible) setTimeout(() => terminal.current?.focus(), 0) }, [visible])

  if (!sessionId) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>尚未连接</strong><span>连接后即可开始 SSH 会话</span>{onConnect && <button className="terminal-connect-button" onClick={onConnect}><Plug size={15} />连接</button>}</div>
  return <div className="terminal" ref={container} style={terminalBackgroundStyle(settings, backgroundImage)} />
}
