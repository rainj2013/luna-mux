import { useEffect, useRef, useState } from 'react'
import { Terminal } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SearchAddon, type ISearchOptions } from '@xterm/addon-search'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ChevronDown, ChevronUp, Plug, Search, X } from 'lucide-react'
import type { TerminalSettings } from '../../../shared/types'
import { colorWithOpacity, terminalBackgroundStyle } from '../terminal-style'

const terminalHighWaterMark = 1024 * 1024
const terminalLowWaterMark = 256 * 1024
const terminalSearchOptions: ISearchOptions = {
  caseSensitive: false, regex: false, wholeWord: false, incremental: true,
  decorations: { matchBackground: '#725a19', activeMatchBackground: '#d28b20', matchOverviewRuler: '#c58c28', activeMatchColorOverviewRuler: '#f3b345' }
}

interface TerminalPaneProps {
  sessionId?: string
  visible: boolean
  settings: TerminalSettings
  backgroundImage: string
  onConnect?: () => void
}

export function TerminalPane({ sessionId, visible, settings, backgroundImage, onConnect }: TerminalPaneProps): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const fitAddon = useRef<FitAddon | null>(null)
  const searchAddon = useRef<SearchAddon | null>(null)
  const webglAddon = useRef<WebglAddon | null>(null)
  const sessionIdRef = useRef(sessionId)
  const boundSessionId = useRef<string | undefined>(undefined)
  const [started, setStarted] = useState(Boolean(sessionId))
  const [searchOpen, setSearchOpen] = useState(false)
  const [query, setQuery] = useState('')
  const searchInput = useRef<HTMLInputElement>(null)

  const shouldRenderTerminal = Boolean(sessionId) || started

  useEffect(() => {
    sessionIdRef.current = sessionId
    if (sessionId) setStarted(true)
    const term = terminal.current
    if (!term || !sessionId || boundSessionId.current === sessionId) return
    if (boundSessionId.current) term.writeln('\r\n\x1b[90m--- 已建立新的 SSH session ---\x1b[0m\r\n')
    boundSessionId.current = sessionId
    requestAnimationFrame(() => {
      fitAddon.current?.fit()
      window.api.sessions.resize(sessionId, term.cols, term.rows)
    })
  }, [sessionId])

  useEffect(() => {
    if (!shouldRenderTerminal || !container.current || terminal.current) return
    const term = new Terminal({
      cursorBlink: true, convertEol: true, allowTransparency: true, fontFamily: settings.fontFamily,
      fontSize: settings.fontSize, lineHeight: 1.25, scrollback: 5000,
      theme: { background: colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity), foreground: settings.foregroundColor, cursor: '#78d64b', selectionBackground: '#315d8c' }
    })
    const fit = new FitAddon()
    const search = new SearchAddon()
    fitAddon.current = fit
    searchAddon.current = search
    term.loadAddon(fit)
    term.loadAddon(search)
    term.loadAddon(new WebLinksAddon((_event, uri) => { void window.api.system.openExternal(uri) }))
    term.open(container.current)
    fit.fit()
    terminal.current = term
    boundSessionId.current = sessionIdRef.current
    let pendingOutput = 0
    let paused = false

    term.attachCustomKeyEventHandler((event) => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.key.toLowerCase() === 'f' && event.type === 'keydown') {
        setSearchOpen(true)
        return false
      }
      if (commandKey && event.key.toLowerCase() === 'c' && term.hasSelection() && event.type === 'keydown') {
        void window.api.system.writeClipboard(term.getSelection())
        return false
      }
      if (commandKey && event.key.toLowerCase() === 'v' && event.type === 'keydown') {
        void window.api.system.readClipboard().then((text) => term.paste(text))
        return false
      }
      return true
    })
    const input = term.onData((data) => { if (sessionIdRef.current) window.api.sessions.write(sessionIdRef.current, data) })
    const resize = term.onResize(({ cols, rows }) => { if (sessionIdRef.current) window.api.sessions.resize(sessionIdRef.current, cols, rows) })
    const stop = window.api.onEvent((event) => {
      if (event.type !== 'terminal-data' || event.payload.sessionId !== sessionIdRef.current) return
      const length = event.payload.data.length
      pendingOutput += length
      if (!paused && pendingOutput >= terminalHighWaterMark) {
        paused = true
        window.api.sessions.flow(event.payload.sessionId, true)
      }
      term.write(event.payload.data, () => {
        pendingOutput = Math.max(0, pendingOutput - length)
        if (paused && pendingOutput <= terminalLowWaterMark && sessionIdRef.current) {
          paused = false
          window.api.sessions.flow(sessionIdRef.current, false)
        }
      })
    })
    const observer = new ResizeObserver(() => {
      if (container.current?.offsetParent) requestAnimationFrame(() => {
        fit.fit()
        if (sessionIdRef.current) window.api.sessions.resize(sessionIdRef.current, term.cols, term.rows)
      })
    })
    observer.observe(container.current)
    term.focus()
    return () => {
      if (paused && sessionIdRef.current) window.api.sessions.flow(sessionIdRef.current, false)
      stop(); observer.disconnect(); input.dispose(); resize.dispose(); webglAddon.current?.dispose(); term.dispose()
      webglAddon.current = null; terminal.current = null; fitAddon.current = null; searchAddon.current = null
    }
  }, [shouldRenderTerminal])

  useEffect(() => {
    const term = terminal.current
    if (!term) return
    term.options.fontFamily = settings.fontFamily
    term.options.fontSize = settings.fontSize
    term.options.theme = { ...term.options.theme, background: colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity), foreground: settings.foregroundColor }
    requestAnimationFrame(() => fitAddon.current?.fit())
  }, [settings])

  useEffect(() => {
    const term = terminal.current
    const element = container.current
    if (!term || !element) return
    if (!visible) {
      webglAddon.current?.dispose()
      webglAddon.current = null
      element.dataset.renderer = 'dom'
      return
    }
    if (!webglAddon.current) {
      try {
        const webgl = new WebglAddon()
        webgl.onContextLoss(() => {
          webgl.dispose()
          if (webglAddon.current === webgl) webglAddon.current = null
          element.dataset.renderer = 'dom'
        })
        term.loadAddon(webgl)
        webglAddon.current = webgl
        element.dataset.renderer = 'webgl'
      } catch {
        webglAddon.current?.dispose()
        webglAddon.current = null
        element.dataset.renderer = 'dom'
      }
    }
    requestAnimationFrame(() => { fitAddon.current?.fit(); term.focus() })
  }, [visible, shouldRenderTerminal])

  useEffect(() => {
    if (searchOpen) setTimeout(() => searchInput.current?.focus(), 0)
    else searchAddon.current?.clearDecorations()
  }, [searchOpen])

  const search = (previous = false): void => {
    if (!query) { searchAddon.current?.clearDecorations(); return }
    if (previous) searchAddon.current?.findPrevious(query, terminalSearchOptions)
    else searchAddon.current?.findNext(query, terminalSearchOptions)
  }

  if (!shouldRenderTerminal) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>尚未连接</strong><span>连接后即可开始 SSH 会话</span>{onConnect && <button className="terminal-connect-button" onClick={onConnect}><Plug size={15} />连接</button>}</div>
  return <div className="terminal-shell" style={terminalBackgroundStyle(settings, backgroundImage)}>
    {searchOpen && <form className="terminal-search" onSubmit={(event) => { event.preventDefault(); search() }}>
      <Search size={14} /><input ref={searchInput} value={query} aria-label="搜索终端内容" onChange={(event) => { setQuery(event.target.value); searchAddon.current?.findNext(event.target.value, terminalSearchOptions) }} onKeyDown={(event) => { if (event.key === 'Escape') setSearchOpen(false) }} />
      <button type="button" className="icon-button" title="上一个结果" onClick={() => search(true)}><ChevronUp size={15} /></button>
      <button type="button" className="icon-button" title="下一个结果" onClick={() => search()}><ChevronDown size={15} /></button>
      <button type="button" className="icon-button" title="关闭搜索" onClick={() => setSearchOpen(false)}><X size={15} /></button>
    </form>}
    <div className="terminal" ref={container} />
  </div>
}
