import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { Terminal, type IBufferLine } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ChevronDown, ChevronUp, ClipboardPaste, Copy, Palette, Plug, Search, X } from 'lucide-react'
import type { TerminalRuntimeEvent, TerminalSettings } from '../types'
import { colorWithOpacity, terminalBackgroundStyle } from '../terminal-style'
import { useI18n } from '../i18n'

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

function recentTerminalText(term: Terminal, maxLines: number, maxChars: number): string {
  const buffer = term.buffer.active
  const lines: string[] = []
  for (let row = 0; row < buffer.length; row += 1) {
    const line = buffer.getLine(row)
    if (!line) continue
    const text = line.translateToString(true)
    if (line.isWrapped && lines.length) lines[lines.length - 1] += text
    else lines.push(text)
  }
  while (lines.length && !lines[lines.length - 1]!.trim()) lines.pop()
  const text = lines.slice(-Math.max(1, maxLines)).join('\n')
  const characters = Array.from(text)
  return characters.length > maxChars ? characters.slice(-maxChars).join('') : text
}

export interface TerminalPaneHandle {
  getRecentLines(maxLines: number, maxChars: number): string
}

interface TerminalPaneProps {
  runtimeId?: string
  connected: boolean
  visible: boolean
  settings: TerminalSettings
  backgroundImage: string
  onConnect?: () => void
  onOpenSettings?: () => void
}

export const TerminalPane = forwardRef<TerminalPaneHandle, TerminalPaneProps>(function TerminalPane({ runtimeId, connected, visible, settings, backgroundImage, onConnect, onOpenSettings }, ref): React.JSX.Element {
  const { t } = useI18n()
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const fitAddon = useRef<FitAddon | null>(null)
  const lastSearchTerm = useRef('')
  const searchMatches = useRef<TerminalSearchMatch[]>([])
  const activeSearchIndex = useRef(-1)
  const webglAddon = useRef<WebglAddon | null>(null)
  const runtimeIdRef = useRef(runtimeId)
  const connectedRef = useRef(connected)
  const boundRuntimeId = useRef<string | undefined>(undefined)
  const outputCursor = useRef(0)
  const [started, setStarted] = useState(Boolean(runtimeId))
  const [searchOpen, setSearchOpen] = useState(false)
  const [query, setQuery] = useState('')
  const [searchResult, setSearchResult] = useState({ index: -1, count: 0 })
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; hasSelection: boolean } | null>(null)
  const searchInput = useRef<HTMLInputElement>(null)

  const shouldRenderTerminal = Boolean(runtimeId) || started
  const rendererBackground = settings.backgroundImagePath && backgroundImage ? 'rgba(0, 0, 0, 0)' : colorWithOpacity(settings.backgroundColor, settings.backgroundOpacity)

  useImperativeHandle(ref, () => ({
    getRecentLines: (maxLines, maxChars) => terminal.current ? recentTerminalText(terminal.current, maxLines, maxChars) : ''
  }), [])

  useEffect(() => {
    runtimeIdRef.current = runtimeId
    connectedRef.current = connected
    if (runtimeId) setStarted(true)
    const term = terminal.current
    if (!term || !runtimeId || boundRuntimeId.current === runtimeId) return
    if (boundRuntimeId.current) term.writeln(`\r\n\x1b[90m--- ${t('terminal.newRuntimeEstablished')} ---\x1b[0m\r\n`)
    boundRuntimeId.current = runtimeId
    outputCursor.current = 0
    requestAnimationFrame(() => {
      fitAddon.current?.fit()
      void window.api.terminalRuntimes.resize(runtimeId, term.cols, term.rows)
    })
  }, [runtimeId, connected, t])

  useEffect(() => {
    connectedRef.current = connected
  }, [connected])

  useEffect(() => {
    if (!connected || !visible || !runtimeId) return
    const frame = requestAnimationFrame(() => {
      const term = terminal.current
      if (!term) return
      fitAddon.current?.fit()
      void window.api.terminalRuntimes.resize(runtimeId, term.cols, term.rows)
      term.focus()
    })
    return () => cancelAnimationFrame(frame)
  }, [connected, visible, runtimeId])

  useEffect(() => {
    if (!shouldRenderTerminal || !container.current || terminal.current) return
    const term = new Terminal({
      cursorBlink: true, convertEol: false, allowTransparency: true, fontFamily: settings.fontFamily,
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
    boundRuntimeId.current = runtimeIdRef.current
    let pendingOutput = 0
    let paused = false
    let disposed = false

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
    const input = term.onData((data) => { if (connectedRef.current && runtimeIdRef.current) void window.api.terminalRuntimes.write(runtimeIdRef.current, data) })
    const resize = term.onResize(({ cols, rows }) => { if (runtimeIdRef.current) void window.api.terminalRuntimes.resize(runtimeIdRef.current, cols, rows) })
    const stop = window.api.onTerminalRuntimeEvent((event: TerminalRuntimeEvent) => {
      if (event.type !== 'output' || event.payload.runtimeId !== runtimeIdRef.current) return
      if (event.payload.endCursor <= outputCursor.current) return
      const length = event.payload.data.length
      outputCursor.current = event.payload.endCursor
      pendingOutput += length
      if (!paused && pendingOutput >= terminalHighWaterMark) {
        paused = true
        void window.api.terminalRuntimes.flow(event.payload.runtimeId, true)
      }
      term.write(event.payload.data, () => {
        pendingOutput = Math.max(0, pendingOutput - length)
        if (paused && pendingOutput <= terminalLowWaterMark && runtimeIdRef.current) {
          paused = false
          void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
        }
      })
    })
    const catchUp = runtimeIdRef.current
      ? window.api.terminalRuntimes.readOutput(runtimeIdRef.current, outputCursor.current, terminalHighWaterMark)
          .then((result) => {
            if (disposed || result.runtimeId !== runtimeIdRef.current || !result.data || result.nextCursor <= outputCursor.current) return
            outputCursor.current = result.nextCursor
            term.write(result.data)
          })
          .catch(() => undefined)
      : Promise.resolve()
    const observer = new ResizeObserver(() => {
      if (container.current?.offsetParent) requestAnimationFrame(() => {
        fit.fit()
        if (runtimeIdRef.current) void window.api.terminalRuntimes.resize(runtimeIdRef.current, term.cols, term.rows)
      })
    })
    observer.observe(container.current)
    term.focus()
    return () => {
      disposed = true
      if (paused && runtimeIdRef.current) void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
      stop(); observer.disconnect(); input.dispose(); resize.dispose(); webglAddon.current?.dispose(); term.dispose()
      void catchUp
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

  if (!shouldRenderTerminal) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>{t('terminal.notConnected')}</strong><span>{t('terminal.connectToStartAnSshSession')}</span>{onConnect && <button className="terminal-connect-button" onClick={onConnect}><Plug size={15} />{t('common.connect')}</button>}</div>
  return <div className="terminal-shell" style={terminalBackgroundStyle(settings, backgroundImage)} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 210)), hasSelection: Boolean(terminal.current?.hasSelection()) }) }}>
    {searchOpen && <form className="terminal-search" onSubmit={(event) => { event.preventDefault(); search() }}>
      <Search size={14} /><input ref={searchInput} value={query} aria-label={t('terminal.searchTerminal')} onChange={(event) => { setQuery(event.target.value); search(event.target.value, false, true) }} onKeyDown={(event) => { if (event.key === 'Escape') setSearchOpen(false) }} />
      <output className={query && !searchResult.count ? 'no-results' : ''}>{query ? searchResult.count ? `${Math.max(0, searchResult.index) + 1}/${searchResult.count}` : t('terminal.noResults') : ''}</output>
      <button type="button" className="icon-button" title={t('terminal.previousResult')} onClick={() => search(query, true)}><ChevronUp size={15} /></button>
      <button type="button" className="icon-button" title={t('terminal.nextResult')} onClick={() => search()}><ChevronDown size={15} /></button>
      <button type="button" className="icon-button" title={t('terminal.closeSearch')} onClick={() => setSearchOpen(false)}><X size={15} /></button>
    </form>}
    <div className="terminal" ref={container} />
    {contextMenu && <div className="sidebar-context-menu terminal-context-menu" role="menu" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      <button role="menuitem" disabled={!contextMenu.hasSelection} onClick={() => { const text = terminal.current?.getSelection(); setContextMenu(null); if (text) void window.api.system.writeClipboard(text).catch((error) => console.warn('Failed to copy terminal selection', error)) }}><Copy size={15} />{t('common.copy')}</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); void window.api.system.readClipboard().then((text) => { terminal.current?.paste(text); terminal.current?.focus() }).catch((error) => console.warn('Failed to paste into terminal', error)) }}><ClipboardPaste size={15} />{t('terminal.paste')}</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); setSearchOpen(true) }}><Search size={15} />{t('terminal.search')}</button>
      <div className="context-menu-separator" />
      <button role="menuitem" onClick={() => { setContextMenu(null); onOpenSettings?.() }}><Palette size={15} />{t('terminal.appearanceSettings')}</button>
    </div>}
  </div>
})
