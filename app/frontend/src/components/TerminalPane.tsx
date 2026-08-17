import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { Terminal, type IBufferLine } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SerializeAddon } from '@xterm/addon-serialize'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ChevronDown, ChevronUp, ClipboardPaste, Copy, Palette, Play, Search, X } from 'lucide-react'
import type { TerminalRuntimeEvent, TerminalSettings } from '../types'
import { colorWithOpacity } from '../terminal-style'
import { createTerminalOutputWriter, type TerminalOutputWriter } from '../terminal-output-writer'
import { useI18n } from '../i18n'

const terminalHighWaterMark = 1024 * 1024
const terminalLowWaterMark = 256 * 1024
const terminalSelectionTheme = {
  selectionBackground: '#ffd43b',
  selectionInactiveBackground: '#ffd43b',
  selectionForeground: '#16181b'
}
interface TerminalSearchMatch { row: number; col: number; length: number }
interface PendingImePunctuation { text: string; createdAt: number; timer: number }
interface TerminalSnapshot { runtimeId?: string; outputCursor: number; cols: number; rows: number; serialized: string }

const terminalSnapshots = new Map<string, TerminalSnapshot>()
const discardedTerminalSnapshots = new Set<string>()
const mountedTerminalPanes = new Set<string>()

export function discardTerminalSnapshot(paneId: string): void {
  terminalSnapshots.delete(paneId)
  if (mountedTerminalPanes.has(paneId)) discardedTerminalSnapshots.add(paneId)
}

const fullWidthPunctuationPattern = /^[\u00b7\u2014\u2018\u2019\u201c\u201d\u2026\u3000-\u303f\uff01-\uff0f\uff1a-\uff20\uff3b-\uff40\uff5b-\uff65\uffe5]+$/u

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
  paneId: string
  targetId: string
  runtimeId?: string
  connected: boolean
  visible: boolean
  settings: TerminalSettings
  backgroundImage: string
  stoppedState: {
    title: string
    description: string
    actionLabel: string
  }
  onAgentAction?: () => void
  onRuntimeError?: (runtimeId: string, message: string) => void
  onStart?: () => void
  onOpenSettings?: () => void
}

export const TerminalPane = forwardRef<TerminalPaneHandle, TerminalPaneProps>(function TerminalPane({ paneId, targetId, runtimeId, connected, visible, settings, backgroundImage, stoppedState, onAgentAction, onRuntimeError, onStart, onOpenSettings }, ref): React.JSX.Element {
  const { t } = useI18n()
  const container = useRef<HTMLDivElement>(null)
  const terminal = useRef<Terminal | null>(null)
  const fitAddon = useRef<FitAddon | null>(null)
  const lastSearchTerm = useRef('')
  const searchMatches = useRef<TerminalSearchMatch[]>([])
  const activeSearchIndex = useRef(-1)
  const webglAddon = useRef<WebglAddon | null>(null)
  const outputWriter = useRef<TerminalOutputWriter | null>(null)
  const runtimeIdRef = useRef(runtimeId)
  const targetIdRef = useRef(targetId)
  targetIdRef.current = targetId
  const pasteClipboardRef = useRef<() => Promise<void>>(async () => undefined)
  const onAgentActionRef = useRef(onAgentAction)
  onAgentActionRef.current = onAgentAction
  const onRuntimeErrorRef = useRef(onRuntimeError)
  onRuntimeErrorRef.current = onRuntimeError
  const reportedInputErrorRuntimeId = useRef('')
  const connectedRef = useRef(connected)
  const boundRuntimeId = useRef<string | undefined>(undefined)
  const outputCursor = useRef(0)
  const renderedOutputCursor = useRef(0)
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
    renderedOutputCursor.current = 0
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
    const snapshot = terminalSnapshots.get(paneId)
    const restorableSnapshot = snapshot && snapshot.runtimeId === runtimeIdRef.current ? snapshot : undefined
    const term = new Terminal({
      cursorBlink: true, convertEol: false, allowTransparency: true, fontFamily: settings.fontFamily,
      fontSize: settings.fontSize, lineHeight: 1.25, scrollback: 5000,
      ...(restorableSnapshot ? { cols: restorableSnapshot.cols, rows: restorableSnapshot.rows } : {}),
      theme: { background: rendererBackground, foreground: settings.foregroundColor, cursor: '#78d64b', ...terminalSelectionTheme }
    })
    const fit = new FitAddon()
    const serialize = new SerializeAddon()
    fitAddon.current = fit
    term.loadAddon(fit)
    term.loadAddon(serialize)
    term.loadAddon(new WebLinksAddon((_event, uri) => { void window.api.system.openExternal(uri).catch((error) => console.warn('Failed to open terminal link', error)) }))
    if (restorableSnapshot) {
      outputCursor.current = restorableSnapshot.outputCursor
      renderedOutputCursor.current = restorableSnapshot.outputCursor
      term.write(restorableSnapshot.serialized)
      terminalSnapshots.delete(paneId)
    }
    term.open(container.current)
    fit.fit()
    mountedTerminalPanes.add(paneId)
    terminal.current = term
    const writer = createTerminalOutputWriter(term)
    outputWriter.current = writer
    boundRuntimeId.current = runtimeIdRef.current
    let pendingOutput = 0
    let paused = false
    let disposed = false
    let lastTerminalInput = { data: '', timestamp: 0 }
    const pendingImePunctuation = new Set<PendingImePunctuation>()

    const reportRuntimeInputError = (failedRuntimeId: string, error: unknown): void => {
      if (reportedInputErrorRuntimeId.current === failedRuntimeId) return
      reportedInputErrorRuntimeId.current = failedRuntimeId
      const message = error instanceof Error ? error.message : String(error)
      onRuntimeErrorRef.current?.(failedRuntimeId, message)
    }

    const writeTerminalInput = (data: string): void => {
      lastTerminalInput = { data, timestamp: performance.now() }
      for (const pending of pendingImePunctuation) {
        if (!data.includes(pending.text)) continue
        window.clearTimeout(pending.timer)
        pendingImePunctuation.delete(pending)
        break
      }
      const activeRuntimeId = runtimeIdRef.current
      if (connectedRef.current && activeRuntimeId) {
        void window.api.terminalRuntimes.write(activeRuntimeId, data).catch((error) => reportRuntimeInputError(activeRuntimeId, error))
      }
    }

    const pasteClipboard = async (): Promise<void> => {
      try {
        const content = await window.api.system.readClipboard()
        if (disposed) return
        writer.markInteractive()
        if (content.type === 'text') {
          term.paste(content.text)
        } else if (content.type === 'image' && targetIdRef.current.startsWith('local:')) {
          writeTerminalInput('\x16')
        }
      } catch (error) {
        console.warn('Failed to paste into terminal', error)
      } finally {
        if (!disposed) term.focus()
      }
    }
    pasteClipboardRef.current = pasteClipboard

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
        void pasteClipboard()
        return false
      }
      if (event.type === 'keydown' && (event.key === 'Enter' || event.key === 'Escape' || (event.ctrlKey && event.key.toLowerCase() === 'c'))) onAgentActionRef.current?.()
      if (event.type === 'keydown') writer.markInteractive(event.key === 'Enter')
      return true
    })
    const input = term.onData(writeTerminalInput)
    const textarea = term.textarea
    let textareaValueBeforeInput: string | undefined
    let lastTextareaValue = textarea?.value ?? ''
    const insertedText = (before: string, after: string): string => {
      let prefix = 0
      while (prefix < before.length && prefix < after.length && before[prefix] === after[prefix]) prefix += 1
      let beforeSuffix = before.length
      let afterSuffix = after.length
      while (beforeSuffix > prefix && afterSuffix > prefix && before[beforeSuffix - 1] === after[afterSuffix - 1]) {
        beforeSuffix -= 1
        afterSuffix -= 1
      }
      return after.slice(prefix, afterSuffix)
    }
    const queueDroppedImePunctuation = (text: string | null): void => {
      if (window.api.platform !== 'win32' || !text || !fullWidthPunctuationPattern.test(text)) return
      const now = performance.now()
      if (lastTerminalInput.data.includes(text) && now - lastTerminalInput.timestamp < 16) return
      if ([...pendingImePunctuation].some((pending) => pending.text === text && now - pending.createdAt < 8)) return
      const pending: PendingImePunctuation = {
        text,
        createdAt: now,
        timer: window.setTimeout(() => {
          pendingImePunctuation.delete(pending)
          if (disposed || !connectedRef.current || !runtimeIdRef.current) return
          writer.markInteractive()
          const activeRuntimeId = runtimeIdRef.current
          if (activeRuntimeId) void window.api.terminalRuntimes.write(activeRuntimeId, text).catch((error) => reportRuntimeInputError(activeRuntimeId, error))
        }, 32)
      }
      pendingImePunctuation.add(pending)
    }
    const rememberImeTextareaBeforeInput = (rawEvent: Event): void => {
      const event = rawEvent as InputEvent
      textareaValueBeforeInput = textarea?.value ?? lastTextareaValue
      if (!['insertText', 'insertCompositionText', 'insertFromComposition'].includes(event.inputType)) return
      queueDroppedImePunctuation(event.data)
    }
    const recoverDroppedImePunctuationFromInput = (rawEvent: Event): void => {
      const event = rawEvent as InputEvent
      const currentValue = textarea?.value ?? ''
      if (['insertText', 'insertCompositionText', 'insertFromComposition'].includes(event.inputType)) {
        queueDroppedImePunctuation(event.data || insertedText(textareaValueBeforeInput ?? lastTextareaValue, currentValue))
      }
      lastTextareaValue = currentValue
      textareaValueBeforeInput = undefined
    }
    const recoverDroppedImePunctuationFromComposition = (rawEvent: Event): void => {
      queueDroppedImePunctuation((rawEvent as CompositionEvent).data)
    }
    textarea?.addEventListener('beforeinput', rememberImeTextareaBeforeInput)
    textarea?.addEventListener('input', recoverDroppedImePunctuationFromInput)
    textarea?.addEventListener('compositionend', recoverDroppedImePunctuationFromComposition, true)
    const resize = term.onResize(({ cols, rows }) => { if (runtimeIdRef.current) void window.api.terminalRuntimes.resize(runtimeIdRef.current, cols, rows) })
    let catchingUp = true
    const queuedOutput: Array<Extract<TerminalRuntimeEvent, { type: 'output' }>['payload']> = []
    const writeOutput = (payload: Extract<TerminalRuntimeEvent, { type: 'output' }>['payload']): void => {
      if (payload.endCursor <= outputCursor.current) return
      const length = payload.data.length
      outputCursor.current = payload.endCursor
      pendingOutput += length
      if (!paused && pendingOutput >= terminalHighWaterMark) {
        paused = true
        void window.api.terminalRuntimes.flow(payload.runtimeId, true)
      }
      writer.write(payload.data, () => {
        if (runtimeIdRef.current === payload.runtimeId) renderedOutputCursor.current = Math.max(renderedOutputCursor.current, payload.endCursor)
        pendingOutput = Math.max(0, pendingOutput - length)
        if (paused && pendingOutput <= terminalLowWaterMark && runtimeIdRef.current) {
          paused = false
          void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
        }
      })
    }
    const stop = window.api.onTerminalRuntimeEvent((event: TerminalRuntimeEvent) => {
      if (event.type !== 'output' || event.payload.runtimeId !== runtimeIdRef.current) return
      if (catchingUp) queuedOutput.push(event.payload)
      else writeOutput(event.payload)
    })
    const catchUp = (async (): Promise<void> => {
      const catchUpRuntimeId = runtimeIdRef.current
      try {
        if (catchUpRuntimeId) {
          const result = await window.api.terminalRuntimes.readOutput(catchUpRuntimeId, outputCursor.current, terminalHighWaterMark)
          if (!disposed && result.runtimeId === runtimeIdRef.current && result.data && result.nextCursor > outputCursor.current) {
            outputCursor.current = result.nextCursor
            writer.write(result.data, () => {
              if (runtimeIdRef.current === result.runtimeId) renderedOutputCursor.current = Math.max(renderedOutputCursor.current, result.nextCursor)
            })
          }
        }
      } catch {
        // Live events below still keep the terminal current if the bounded replay is unavailable.
      } finally {
        catchingUp = false
        if (!disposed) {
          queuedOutput.sort((left, right) => left.startCursor - right.startCursor)
          for (const output of queuedOutput) writeOutput(output)
        }
        queuedOutput.length = 0
      }
    })()
    const observer = new ResizeObserver(() => {
      if (container.current?.offsetParent) requestAnimationFrame(() => {
        fit.fit()
        if (runtimeIdRef.current) void window.api.terminalRuntimes.resize(runtimeIdRef.current, term.cols, term.rows)
      })
    })
    observer.observe(container.current)
    if (visible) term.focus()
    return () => {
      disposed = true
      if (pasteClipboardRef.current === pasteClipboard) pasteClipboardRef.current = async () => undefined
      mountedTerminalPanes.delete(paneId)
      if (discardedTerminalSnapshots.delete(paneId)) terminalSnapshots.delete(paneId)
      else {
        try {
          terminalSnapshots.set(paneId, {
            runtimeId: runtimeIdRef.current,
            outputCursor: renderedOutputCursor.current,
            cols: term.cols,
            rows: term.rows,
            serialized: serialize.serialize({ scrollback: 5000 })
          })
        } catch {
          terminalSnapshots.delete(paneId)
        }
      }
      if (paused && runtimeIdRef.current) void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
      for (const pending of pendingImePunctuation) window.clearTimeout(pending.timer)
      pendingImePunctuation.clear()
      textarea?.removeEventListener('beforeinput', rememberImeTextareaBeforeInput)
      textarea?.removeEventListener('input', recoverDroppedImePunctuationFromInput)
      textarea?.removeEventListener('compositionend', recoverDroppedImePunctuationFromComposition, true)
      stop(); observer.disconnect(); input.dispose(); resize.dispose(); writer.dispose(); webglAddon.current?.dispose(); term.dispose()
      void catchUp
      webglAddon.current = null; outputWriter.current = null; terminal.current = null; fitAddon.current = null
    }
  }, [paneId, shouldRenderTerminal])

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
      outputWriter.current?.wrapRenderer()
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
        outputWriter.current?.wrapRenderer()
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

  if (!shouldRenderTerminal) return <div className="terminal-empty"><div className="terminal-mark">&gt;_</div><strong>{stoppedState.title}</strong><span>{stoppedState.description}</span>{onStart && <button className="terminal-connect-button" onClick={onStart}><Play size={15} />{stoppedState.actionLabel}</button>}</div>
  return <div className="terminal-shell" onContextMenu={(event) => { event.preventDefault(); setContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 210)), hasSelection: Boolean(terminal.current?.hasSelection()) }) }}>
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
      <button role="menuitem" onClick={() => { setContextMenu(null); void pasteClipboardRef.current() }}><ClipboardPaste size={15} />{t('terminal.paste')}</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); setSearchOpen(true) }}><Search size={15} />{t('terminal.search')}</button>
      <div className="context-menu-separator" />
      <button role="menuitem" onClick={() => { setContextMenu(null); onOpenSettings?.() }}><Palette size={15} />{t('terminal.appearanceSettings')}</button>
    </div>}
  </div>
})
