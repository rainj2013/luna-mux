import { useEffect, useRef, useState } from 'react'
import { Terminal, type IBufferLine } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ChevronDown, ChevronUp, ClipboardPaste, Copy, Palette, Plug, Search, X } from 'lucide-react'
import type { TerminalSettings } from '../types'
import { colorWithOpacity, terminalBackgroundStyle } from '../terminal-style'

const terminalHighWaterMark = 1024 * 1024
const terminalLowWaterMark = 256 * 1024
const terminalSelectionTheme = {
  selectionBackground: '#ffd43b',
  selectionInactiveBackground: '#ffd43b',
  selectionForeground: '#16181b'
}
interface TerminalSearchMatch { row: number; col: number; length: number }

function stringOffsetToBufferColumn(line: IBufferLine, offset: number): number {
  let stringOffset = 0
  for (let col = 0; col < line.length; col += 1) {
    if (stringOffset >= offset) return col
    const cell = line.getCell(col)
    if (!cell || cell.getWidth() === 0) continue
    stringOffset += cell.getChars().length || 1
  }
  return line.length
}

function findTerminalMatches(term: Terminal, query: string): TerminalSearchMatch[] {
  const needle = query.toLocaleLowerCase()
  const matches: TerminalSearchMatch[] = []
  const buffer = term.buffer.active
  for (let row = 0; row < buffer.length; row += 1) {
    const line = buffer.getLine(row)
    if (!line) continue
    const text = line.translateToString(true)
    const searchable = text.toLocaleLowerCase()
    let offset = searchable.indexOf(needle)
    while (offset >= 0) {
      const col = stringOffsetToBufferColumn(line, offset)
      const endCol = stringOffsetToBufferColumn(line, offset + query.length)
      matches.push({ row, col, length: Math.max(1, endCol - col) })
      offset = searchable.indexOf(needle, offset + Math.max(1, needle.length))
    }
  }
  return matches
}

interface TerminalPaneProps {
  sessionId?: string
  visible: boolean
  settings: TerminalSettings
  backgroundImage: string
  onConnect?: () => void
  onOpenSettings?: () => void
}

export function TerminalPane({ sessionId, visible, settings, backgroundImage, onConnect, onOpenSettings }: TerminalPaneProps): React.JSX.Element {
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const fitAddon = useRef<FitAddon | null>(null)
  const lastSearchTerm = useRef('')
  const searchMatches = useRef<TerminalSearchMatch[]>([])
  const activeSearchIndex = useRef(-1)
  const webglAddon = useRef<WebglAddon | null>(null)
  const sessionIdRef = useRef(sessionId)
  const boundSessionId = useRef<string | undefined>(undefined)
  const [started, setStarted] = useState(Boolean(sessionId))
  const [searchOpen, setSearchOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [searchResult, setSearchResult] = useState({ index: -1, count: 0 })
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; hasSelection: boolean } | null>(null)
  const searchInput = useRef<HTMLInputElement>(null)

  const shouldRenderTerminal = Boolean(sessionId) || started
  const rendererBackground = settings.backgroundImagePath && backgroundImage ? 'rgba(0, 0, 0, 0)' : colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity)

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
      theme: { background: rendererBackground, foreground: settings.foregroundColor, cursor: '#78d64b', ...terminalSelectionTheme }
    })
    const fit = new FitAddon()
    fitAddon.current = fit
    term.loadAddon(fit)
    term.loadAddon(new WebLinksAddon((_event, uri) => { void window.api.system.openExternal(uri).catch((error) => console.warn('Failed to open terminal link', error)) }))
    term.open(container.current)
    fit.fit()
    terminal.current = term
    boundSessionId.current = sessionIdRef.current
    let pendingOutput = 0
    let paused = false

    term.attachCustomKeyEventHandler((event) => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (commandKey && event.key.toLowerCase() === 'f' && event.type === 'keydown') {
        event.preventDefault()
        event.stopPropagation()
        setSearchOpen(true)
        return false
      }
      if (commandKey && event.key.toLowerCase() === 'c' && term.hasSelection() && event.type === 'keydown') {
        event.preventDefault()
        event.stopPropagation()
        void window.api.system.writeClipboard(term.getSelection()).catch((error) => console.warn('Failed to copy terminal selection', error))
        return false
      }
      if (commandKey && event.key.toLowerCase() === 'v' && event.type === 'keydown') {
        event.preventDefault()
        event.stopPropagation()
        void window.api.system.readClipboard().then((text) => term.paste(text)).catch((error) => console.warn('Failed to paste into terminal', error))
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
      webglAddon.current = null; terminal.current = null; fitAddon.current = null
    }
  }, [shouldRenderTerminal])

  useEffect(() => {
    const term = terminal.current
    if (!term) return
    term.options.fontFamily = settings.fontFamily
    term.options.fontSize = settings.fontSize
    term.options.theme = { ...term.options.theme, background: rendererBackground, foreground: settings.foregroundColor }
    requestAnimationFrame(() => fitAddon.current?.fit())
  }, [settings, backgroundImage])

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
    if (searchOpen) setTimeout(() => { searchInput.current?.focus(); searchInput.current?.select() }, 0)
    else {
      lastSearchTerm.current = ''
      searchMatches.current = []
      activeSearchIndex.current = -1
      terminal.current?.clearSelection()
      setSearchResult({ index: -1, count: 0 })
    }
  }, [searchOpen])

  useEffect(() => {
    if (!visible) return
    const interceptFind = (event: KeyboardEvent): void => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (!commandKey || event.key.toLowerCase() !== 'f') return
      event.preventDefault()
      event.stopImmediatePropagation()
      setSearchOpen(true)
    }
    window.addEventListener('keydown', interceptFind, true)
    return () => window.removeEventListener('keydown', interceptFind, true)
  }, [visible])

  useEffect(() => {
    if (!contextMenu) return
    const close = (): void => setContextMenu(null)
    const closeOnKey = (event: KeyboardEvent): void => { if (event.key === 'Escape') close() }
    document.addEventListener('pointerdown', close)
    window.addEventListener('blur', close)
    window.addEventListener('resize', close)
    document.addEventListener('keydown', closeOnKey)
    return () => {
      document.removeEventListener('pointerdown', close)
      window.removeEventListener('blur', close)
      window.removeEventListener('resize', close)
      document.removeEventListener('keydown', closeOnKey)
    }
  }, [contextMenu])

  const search = (term = query, previous = false, reset = false): void => {
    const value = term.trim()
    const currentTerminal = terminal.current
    if (!value || !currentTerminal) {
      lastSearchTerm.current = ''
      searchMatches.current = []
      activeSearchIndex.current = -1
      currentTerminal?.clearSelection()
      setSearchResult({ index: -1, count: 0 })
      return
    }
    const isNewSearch = reset || value !== lastSearchTerm.current
    searchMatches.current = findTerminalMatches(currentTerminal, value)
    lastSearchTerm.current = value
    const count = searchMatches.current.length
    if (!count) {
      activeSearchIndex.current = -1
      currentTerminal.clearSelection()
      setSearchResult({ index: -1, count: 0 })
      return
    }
    if (isNewSearch) activeSearchIndex.current = previous ? count - 1 : 0
    else activeSearchIndex.current = (activeSearchIndex.current + (previous ? -1 : 1) + count) % count
    const match = searchMatches.current[activeSearchIndex.current]
    if (!match) return
    currentTerminal.select(match.col, match.row, match.length)
    currentTerminal.scrollToLine(match.row)
    setSearchResult({ index: activeSearchIndex.current, count })
  }

  if (!shouldRenderTerminal) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>尚未连接</strong><span>连接后即可开始 SSH 会话</span>{onConnect && <button className="terminal-connect-button" onClick={onConnect}><Plug size={15} />连接</button>}</div>
  return <div className="terminal-shell" style={terminalBackgroundStyle(settings, backgroundImage)} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 210)), hasSelection: Boolean(terminal.current?.hasSelection()) }) }}>
    {searchOpen && <form className="terminal-search" onSubmit={(event) => { event.preventDefault(); search() }}>
      <Search size={14} /><input ref={searchInput} value={query} aria-label="搜索终端内容" onChange={(event) => { setQuery(event.target.value); search(event.target.value, false, true) }} onKeyDown={(event) => { if (event.key === 'Escape') setSearchOpen(false) }} />
      <output className={query && !searchResult.count ? 'no-results' : ''}>{query ? searchResult.count ? `${Math.max(0, searchResult.index) + 1}/${searchResult.count}` : '无结果' : ''}</output>
      <button type="button" className="icon-button" title="上一个结果" onClick={() => search(query, true)}><ChevronUp size={15} /></button>
      <button type="button" className="icon-button" title="下一个结果" onClick={() => search()}><ChevronDown size={15} /></button>
      <button type="button" className="icon-button" title="关闭搜索" onClick={() => setSearchOpen(false)}><X size={15} /></button>
    </form>}
    <div className="terminal" ref={container} />
    {contextMenu && <div className="sidebar-context-menu terminal-context-menu" role="menu" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      <button role="menuitem" disabled={!contextMenu.hasSelection} onClick={() => { const text = terminal.current?.getSelection(); setContextMenu(null); if (text) void window.api.system.writeClipboard(text).catch((error) => console.warn('Failed to copy terminal selection', error)) }}><Copy size={15} />复制</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); void window.api.system.readClipboard().then((text) => { terminal.current?.paste(text); terminal.current?.focus() }).catch((error) => console.warn('Failed to paste into terminal', error)) }}><ClipboardPaste size={15} />粘贴</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); setSearchOpen(true) }}><Search size={15} />搜索</button>
      <div className="context-menu-separator" />
      <button role="menuitem" onClick={() => { setContextMenu(null); onOpenSettings?.() }}><Palette size={15} />外观设置</button>
    </div>}
  </div>
}
