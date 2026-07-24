import { useEffect, useRef } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'

export function TerminalPane({ sessionId, visible }: { sessionId?: string; visible: boolean }): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)

  useEffect(() => {
    if (!sessionId || !container.current) return
    const term = new Terminal({
      cursorBlink: true, convertEol: true, fontFamily: 'Cascadia Mono, SFMono-Regular, Consolas, monospace',
      fontSize: 14, lineHeight: 1.25, scrollback: 5000,
      theme: { background: '#111315', foreground: '#e8eaed', cursor: '#78d64b', selectionBackground: '#315d8c' }
    })
    const fit = new FitAddon(); term.loadAddon(fit); term.open(container.current); fit.fit(); terminal.current = term
    const input = term.onData((data) => window.api.sessions.write(sessionId, data))
    const resize = term.onResize(({ cols, rows }) => window.api.sessions.resize(sessionId, cols, rows))
    const stop = window.api.onEvent((event) => { if (event.type === 'terminal-data' && event.payload.sessionId === sessionId) term.write(event.payload.data) })
    const observer = new ResizeObserver(() => { if (container.current?.offsetParent) requestAnimationFrame(() => { fit.fit(); window.api.sessions.resize(sessionId, term.cols, term.rows) }) })
    observer.observe(container.current)
    term.focus()
    return () => { stop(); observer.disconnect(); input.dispose(); resize.dispose(); term.dispose(); terminal.current = null }
  }, [sessionId])

  useEffect(() => { if (visible) setTimeout(() => terminal.current?.focus(), 0) }, [visible])

  if (!sessionId) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>尚未连接</strong><span>从左侧选择书签开始 SSH 会话</span></div>
  return <div className="terminal" ref={container} />
}
