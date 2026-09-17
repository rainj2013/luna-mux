import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react'
import { Terminal, type IBufferLine } from '@xterm/xterm'
import { FitAddon } from '@xterm/addon-fit'
import { SerializeAddon } from '@xterm/addon-serialize'
import { WebLinksAddon } from '@xterm/addon-web-links'
import { WebglAddon } from '@xterm/addon-webgl'
import { ChevronDown, ChevronUp, ClipboardPaste, Copy, KeyRound, Palette, Play, RefreshCw, Search, ShieldAlert, X } from 'lucide-react'
import type { TerminalRuntimeEvent, TerminalSettings, TerminalUiDiagnosticEvent, TerminalUiInputPath, TerminalUiKeyCategory } from '../types'
import { colorWithOpacity } from '../terminal-style'
import { createTerminalOutputWriter, type TerminalOutputWriter } from '../terminal-output-writer'
import { agentImagePasteInput, handleCodexMultilinePasteEvent, routeTerminalPaste } from '../terminal-input'
import { useI18n } from '../i18n'

const terminalHighWaterMark = 1024 * 1024
const terminalLowWaterMark = 256 * 1024
// Chromium/xterm can report the committed value of a Windows IME twice (once from
// the composition path and once from the textarea input path).  Keep this short:
// it is long enough to collapse that duplicate, while still allowing a user to
// intentionally commit the same CJK character twice in normal typing.
const duplicateImeInputWindowMs = 40
const terminalAccentColor = '#337fd6'
const terminalSelectionTheme = {
  selectionBackground: terminalAccentColor,
  selectionInactiveBackground: terminalAccentColor,
  selectionForeground: '#ffffff'
}
interface TerminalSearchMatch { row: number; col: number; length: number }
interface PendingImePunctuation { text: string; createdAt: number; timer: number }
interface TerminalSnapshot { runtimeId?: string; outputCursor: number; cols: number; rows: number; serialized: string }
interface TerminalRuntimeSize { runtimeId: string; cols: number; rows: number }
interface DiagnosticExtras { inputPath?: TerminalUiInputPath; reason?: string }
type DiagnosticRecorder = (kind: TerminalUiDiagnosticEvent['kind'], keyCategory?: TerminalUiKeyCategory, throttleMs?: number, extras?: DiagnosticExtras) => void

/**
 * A pane whose runtime id has been cleared cannot address its own runtime, so
 * its diagnostics would be dropped by the empty-id check. Recording them under
 * a sentinel keeps that state visible instead of silent.
 */
const unboundDiagnosticRuntimeId = 'unbound'

const terminalSnapshots = new Map<string, TerminalSnapshot>()
const discardedTerminalSnapshots = new Set<string>()
const mountedTerminalPanes = new Set<string>()

/**
 * Tauri rejects a failed command with the backend's error string, which is the
 * only description of why a call failed; recording it is what turns "the pane
 * went quiet" into a named cause.
 */
function describeError(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  return message.slice(0, 200) || 'unknown'
}

/**
 * A pane whose cursor blinks is a pane whose textarea holds DOM focus, so when
 * input stops working the first question is which textarea that is. Counting
 * the helper textareas in the document also exposes leaked instances of a
 * disposed terminal, which nothing else in the log would reveal.
 */
function describeFocusOwner(ownTextarea: HTMLTextAreaElement | undefined): string {
  const focused = document.activeElement
  if (!(focused instanceof HTMLElement)) return focused ? 'non-element' : 'none'
  const helpers = Array.from(document.querySelectorAll<HTMLTextAreaElement>('textarea.xterm-helper-textarea'))
  const index = helpers.indexOf(focused as HTMLTextAreaElement)
  if (index >= 0) return `${focused === ownTextarea ? 'own' : 'other'}-helper-textarea[${index + 1}/${helpers.length}]`
  const key = focused.className ? `${focused.tagName.toLowerCase()}.${String(focused.className).split(/\s+/)[0]}` : focused.tagName.toLowerCase()
  return `${key}[helper-textareas:${helpers.length}]`
}

function shouldOpenTerminalLink(event: MouseEvent): boolean {
  if (event.button !== 0) return false
  const commandClick = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
  return commandClick ? event.detail === 1 : event.detail === 2
}

function openTerminalLink(event: MouseEvent, uri: string): void {
  if (!shouldOpenTerminalLink(event)) return
  void window.api.system.openExternal(uri).catch((error) => console.warn('Failed to open terminal link', error))
}

export function discardTerminalSnapshot(paneId: string): void {
  terminalSnapshots.delete(paneId)
  if (mountedTerminalPanes.has(paneId)) discardedTerminalSnapshots.add(paneId)
}

const fullWidthPunctuationPattern = /^[\u00b7\u2014\u2018\u2019\u201c\u201d\u2026\u3000-\u303f\uff01-\uff0f\uff1a-\uff20\uff3b-\uff40\uff5b-\uff65\uffe5]+$/u

function shiftEnterInput(platform: string, targetId: string, codexTui: boolean): string {
  const nativeWindowsPowershell = platform === 'win32' && (targetId === 'local:powershell' || targetId === 'local:powershell5')
  // In ConPTY Win32 input mode LF becomes Ctrl+Enter, which Codex does not
  // bind. ESC+CR becomes Alt+Enter, one of Codex's newline bindings.
  return nativeWindowsPowershell && codexTui ? '\x1b\r' : '\n'
}

function terminalKeyCategory(event: KeyboardEvent): TerminalUiKeyCategory {
  if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) return 'printable'
  if (event.key === 'Enter') return 'enter'
  if (event.key === 'Escape') return 'escape'
  if (event.ctrlKey || event.metaKey || event.altKey) return 'control'
  if (['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Home', 'End', 'PageUp', 'PageDown'].includes(event.key)) return 'navigation'
  return 'other'
}

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

interface PrivateRenderer {
  /** Only the DOM renderer keeps row elements; the WebGL renderer draws to a canvas. */
  _rowContainer?: HTMLElement
  /** Only the DOM renderer derives its advance from layout; WebGL has no such method. */
  _setDefaultSpacing?: () => void
}
interface PrivateRenderService { _renderer?: { value?: PrivateRenderer } }
interface TerminalInternals { _core?: { _renderService?: PrivateRenderService } }

/**
 * Repairs the doubled character advance a pane shows after it has been inactive.
 *
 * The DOM renderer takes one letter-spacing for the whole row container from a
 * layout measurement (`_setDefaultSpacing()`), which runs from its constructor
 * and from option changes. When it runs while the pane has no layout box, every
 * glyph measures 0, so the deviation is computed as a full cell width and lands
 * in both the row container and the row factory's default. Rows already on
 * screen have no inline override, so they inherit that value and the text
 * renders with its advance doubled while glyph widths and row heights stay
 * correct. Disposing the WebGL addon for an inactive pane is what triggers it:
 * the addon builds a fresh DOM renderer in the effect that runs while the pane
 * is still hidden. `fit()` cannot repair it, because `handleResize()` never
 * re-derives the default - the measurement has to be taken again once the pane
 * is laid out. The WebGL renderer is immune (it measures glyphs through
 * CharSizeService and adds only the letterSpacing option, which this app never
 * sets), so which renderer is active is what tells the two apart.
 *
 * Nothing inside xterm repairs it later, so the repair has to be called again:
 * the character size service rejects non-positive measurements, which leaves a
 * hidden pane holding its last valid size, and both `Terminal.resize()` and the
 * renderer's intersection observer skip re-measuring while that size is still
 * valid. A pane that comes back at the size it had is therefore never
 * re-measured by xterm itself.
 */
function resyncRendererDefaultSpacing(term: Terminal, element: HTMLElement): void {
  const renderer = (term as unknown as TerminalInternals)._core?._renderService?._renderer?.value
  // Ask the active renderer rather than the DOM: the DOM renderer `open()` builds is
  // never disposed when the WebGL addon takes over, so its row container outlives it
  // and a `.xterm-rows` lookup answers for a renderer that is no longer drawing.
  // WebGL needs no repair, and this is what keeps it out of the fallback below when
  // the pane is resized.
  if (!renderer?._rowContainer) return
  // Never measure an element that has no layout box: that measurement is exactly
  // what corrupts the value, so running here would plant the damage it repairs.
  if (element.getClientRects().length === 0) return
  if (renderer._setDefaultSpacing) {
    renderer._setDefaultSpacing()
    return
  }
  // Reached only by a DOM renderer whose private method a release renamed. A fresh
  // theme object is the app's existing idiom for forcing a renderer option change;
  // object identity is what makes the write observable to xterm.
  term.options.theme = { ...term.options.theme }
}

export interface TerminalPaneHandle {
  getRecentLines(maxLines: number, maxChars: number): string
}

interface TerminalPaneProps {
  paneId: string
  targetId: string
  activeAgentAdapterId?: string
  runtimeId?: string
  connected: boolean
  connecting: boolean
  focused: boolean
  visible: boolean
  settings: TerminalSettings
  backgroundImage: string
  stoppedState: {
    title: string
    description: string
    actionLabel: string
    actionIcon: 'credentials' | 'retry' | 'start'
    tone?: 'error'
  }
  onAgentAction?: () => void
  onRuntimeError?: (runtimeId: string, message: string) => void
  onStart?: () => void
  onClose?: () => void
  onOpenSettings?: () => void
}

export const TerminalPane = forwardRef<TerminalPaneHandle, TerminalPaneProps>(function TerminalPane({ paneId, targetId, activeAgentAdapterId, runtimeId, connected, connecting, focused, visible, settings, backgroundImage, stoppedState, onAgentAction, onRuntimeError, onStart, onClose, onOpenSettings }, ref): React.JSX.Element {
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
  const activeAgentAdapterIdRef = useRef(activeAgentAdapterId)
  activeAgentAdapterIdRef.current = activeAgentAdapterId
  const pasteClipboardRef = useRef<() => Promise<void>>(async () => undefined)
  const onAgentActionRef = useRef(onAgentAction)
  onAgentActionRef.current = onAgentAction
  const onRuntimeErrorRef = useRef(onRuntimeError)
  onRuntimeErrorRef.current = onRuntimeError
  const reportedInputErrorRuntimeId = useRef('')
  const connectedRef = useRef(connected)
  const connectingRef = useRef(connecting)
  const focusedRef = useRef(focused)
  focusedRef.current = focused
  const visibleRef = useRef(visible)
  visibleRef.current = visible
  const pendingRuntimeInput = useRef(new Map<string, string[]>())
  const boundRuntimeId = useRef<string | undefined>(undefined)
  const outputCursor = useRef(0)
  const renderedOutputCursor = useRef(0)
  const lastRuntimeSize = useRef<TerminalRuntimeSize | undefined>(undefined)
  // The recorder closes over per-mount state, so effects outside the terminal
  // effect reach it through a ref instead of duplicating its payload.
  const diagnosticRecorder = useRef<DiagnosticRecorder | null>(null)
  const catchUpOutputRef = useRef<(() => void) | null>(null)
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

  const reportRuntimeInputError = (failedRuntimeId: string, error: unknown): void => {
    if (reportedInputErrorRuntimeId.current === failedRuntimeId) return
    reportedInputErrorRuntimeId.current = failedRuntimeId
    onRuntimeErrorRef.current?.(failedRuntimeId, describeError(error))
  }

  const focusTerminalIfFocused = (term: Terminal): void => {
    if (focusedRef.current && terminal.current === term) term.focus()
  }

  const resizeRuntimeIfVisible = (term: Terminal, targetRuntimeId = runtimeIdRef.current): void => {
    // A split can remount a terminal while its DOM is between layout sizes.
    // A pane in the current split remains visible even when another pane owns
    // keyboard focus. Only panes without a layout box defer their PTY resize.
    if (!visibleRef.current || !targetRuntimeId) return
    const next = { runtimeId: targetRuntimeId, cols: term.cols, rows: term.rows }
    const previous = lastRuntimeSize.current
    if (previous?.runtimeId === next.runtimeId && previous.cols === next.cols && previous.rows === next.rows) return
    lastRuntimeSize.current = next
    void window.api.terminalRuntimes.resize(next.runtimeId, next.cols, next.rows).catch((error) => {
      if (lastRuntimeSize.current === next) lastRuntimeSize.current = undefined
      // The rejection reason is the only signal that the backend refused the
      // resize; a poisoned runtime looks idle otherwise.
      diagnosticRecorder.current?.('outputSkipped', undefined, 1000, { reason: `resize-failed: ${describeError(error)}` })
    })
  }

  useEffect(() => {
    const previousRuntimeId = runtimeIdRef.current
    runtimeIdRef.current = runtimeId
    if (previousRuntimeId !== runtimeId) lastRuntimeSize.current = undefined
    connectedRef.current = connected
    connectingRef.current = connecting
    if (previousRuntimeId && previousRuntimeId !== runtimeId) pendingRuntimeInput.current.delete(previousRuntimeId)
    if (runtimeId && connected) {
      const pending = pendingRuntimeInput.current.get(runtimeId)
      pendingRuntimeInput.current.delete(runtimeId)
      if (pending?.length) {
        void window.api.terminalRuntimes.write(runtimeId, pending.join('')).catch((error) => reportRuntimeInputError(runtimeId, error))
      }
    } else if (runtimeId && !connecting) {
      pendingRuntimeInput.current.delete(runtimeId)
    }
    if (runtimeId) setStarted(true)
    const term = terminal.current
    if (!term || !runtimeId || boundRuntimeId.current === runtimeId) return
    if (boundRuntimeId.current) term.writeln(`\r\n\x1b[90m--- ${t('terminal.newRuntimeEstablished')} ---\x1b[0m\r\n`)
    boundRuntimeId.current = runtimeId
    outputCursor.current = 0
    renderedOutputCursor.current = 0
    requestAnimationFrame(() => {
      fitAddon.current?.fit()
      resizeRuntimeIfVisible(term, runtimeId)
    })
  }, [runtimeId, connected, connecting, t])

  useEffect(() => {
    connectedRef.current = connected
    connectingRef.current = connecting
  }, [connected, connecting])

  useEffect(() => {
    if (!connected || !visible || !runtimeId) return
    const frame = requestAnimationFrame(() => {
      const term = terminal.current
      if (!term) return
      fitAddon.current?.fit()
      resizeRuntimeIfVisible(term, runtimeId)
      focusTerminalIfFocused(term)
    })
    return () => cancelAnimationFrame(frame)
  }, [connected, focused, visible, runtimeId])

  useEffect(() => {
    if (!shouldRenderTerminal || !container.current || terminal.current) return
    const snapshot = terminalSnapshots.get(paneId)
    const restorableSnapshot = snapshot && snapshot.runtimeId === runtimeIdRef.current ? snapshot : undefined
    const term = new Terminal({
      cursorBlink: true, convertEol: false, allowTransparency: true, fontFamily: settings.fontFamily,
      fontSize: settings.fontSize, lineHeight: 1.25, scrollback: 5000,
      linkHandler: { activate: openTerminalLink },
      ...(restorableSnapshot ? { cols: restorableSnapshot.cols, rows: restorableSnapshot.rows } : {}),
      theme: { background: rendererBackground, foreground: settings.foregroundColor, cursor: terminalAccentColor, ...terminalSelectionTheme }
    })
    const fit = new FitAddon()
    const serialize = new SerializeAddon()
    fitAddon.current = fit
    term.loadAddon(fit)
    term.loadAddon(serialize)
    term.loadAddon(new WebLinksAddon(openTerminalLink))
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
    let userScrolled = false
    let scrollAnchor = 0
    let lastTerminalInput = { data: '', timestamp: 0, runtimeId: '' }
    const pendingImePunctuation = new Set<PendingImePunctuation>()
    const lastUiDiagnosticAt = new Map<string, number>()

    const createUiDiagnostic = (kind: TerminalUiDiagnosticEvent['kind'], keyCategory?: TerminalUiKeyCategory, throttleMs = 0, extras: DiagnosticExtras = {}): TerminalUiDiagnosticEvent | undefined => {
      const now = performance.now()
      const throttleKey = `${kind}:${keyCategory ?? ''}`
      const previous = lastUiDiagnosticAt.get(throttleKey) ?? Number.NEGATIVE_INFINITY
      if (throttleMs > 0 && now - previous < throttleMs) return undefined
      // Unthrottled events share a throttle key with throttled ones of the same
      // kind, so they must not advance the window and swallow the next record.
      if (throttleMs > 0) lastUiDiagnosticAt.set(throttleKey, now)
      const buffer = term.buffer.active
      return {
        kind,
        connected: connectedRef.current,
        connecting: connectingRef.current,
        visible: visibleRef.current,
        documentVisible: document.visibilityState === 'visible',
        focused: document.activeElement === term.textarea,
        focusOwner: describeFocusOwner(term.textarea),
        alternateBuffer: buffer.type === 'alternate',
        pendingOutput,
        outputCursor: outputCursor.current,
        viewportY: buffer.viewportY,
        baseY: buffer.baseY,
        paneId,
        ...(keyCategory ? { keyCategory } : {}),
        ...(extras.inputPath ? { inputPath: extras.inputPath } : {}),
        ...(extras.reason ? { reason: extras.reason } : {})
      }
    }

    const recordUiDiagnostic = (kind: TerminalUiDiagnosticEvent['kind'], keyCategory?: TerminalUiKeyCategory, throttleMs = 0, extras: DiagnosticExtras = {}): void => {
      const event = createUiDiagnostic(kind, keyCategory, throttleMs, extras)
      if (!event) return
      // Deliberately no early return for a missing runtime id: a pane that lost
      // its runtime is one of the states under investigation.
      void window.api.terminalRuntimes.recordDiagnostic(runtimeIdRef.current || unboundDiagnosticRuntimeId, event).catch(() => undefined)
    }
    diagnosticRecorder.current = recordUiDiagnostic
    // One mount record per mounted terminal. Pairing these with dispose events
    // exposes remounts that no pane or runtime lifecycle change would explain.
    recordUiDiagnostic('mount', undefined, 0, { reason: restorableSnapshot ? 'snapshot' : 'fresh' })

    const writeTerminalInput = (data: string): void => {
      const now = performance.now()
      const activeRuntimeId = runtimeIdRef.current ?? ''
      const hasCommittedNonAscii = data.length > 0 && [...data].some((character) => {
        const codePoint = character.codePointAt(0) ?? 0
        return codePoint > 0x7f && !/\p{Control}/u.test(character)
      })
      if (hasCommittedNonAscii && lastTerminalInput.runtimeId === activeRuntimeId && lastTerminalInput.data === data && now - lastTerminalInput.timestamp < duplicateImeInputWindowMs) {
        // Do not send the same committed IME payload twice.  This is deliberately
        // limited to non-ASCII text so ordinary terminal key repeats are untouched.
        // Recorded anyway: a guard that misfires discards input just as silently
        // as a disconnected runtime does.
        recordUiDiagnostic('input', undefined, 0, { inputPath: 'dropped', reason: 'duplicate-ime' })
        return
      }
      lastTerminalInput = { data, timestamp: now, runtimeId: activeRuntimeId }
      const inputDiagnostic = createUiDiagnostic('input', undefined, 250, { inputPath: 'write' })
      for (const pending of pendingImePunctuation) {
        if (!data.includes(pending.text)) continue
        window.clearTimeout(pending.timer)
        pendingImePunctuation.delete(pending)
        break
      }
      if (connectedRef.current && activeRuntimeId) {
        void window.api.terminalRuntimes.write(activeRuntimeId, data, inputDiagnostic).catch((error) => reportRuntimeInputError(activeRuntimeId, error))
      } else if (connectingRef.current && activeRuntimeId) {
        // Unthrottled: this branch keeps the keystroke out of the PTY for as
        // long as the pane stays in the connecting state.
        recordUiDiagnostic('input', undefined, 0, { inputPath: 'buffered', reason: 'connecting' })
        const pending = pendingRuntimeInput.current.get(activeRuntimeId) ?? []
        pending.push(data)
        pendingRuntimeInput.current.set(activeRuntimeId, pending)
      } else {
        // Neither branch taken: the keystroke is discarded and nothing is sent
        // to the PTY. Previously this produced no record at all.
        recordUiDiagnostic('input', undefined, 0, { inputPath: 'dropped', reason: activeRuntimeId ? 'not-connected' : 'no-runtime-id' })
      }
    }

    const pasteTerminalText = (text: string): void => {
      routeTerminalPaste(text, activeAgentAdapterIdRef.current === 'codex', writeTerminalInput, (payload) => term.paste(payload))
    }

    const pasteClipboard = async (): Promise<void> => {
      try {
        const content = await window.api.system.readClipboard()
        if (disposed) return
        writer.markInteractive()
        if (content.type === 'text') {
          pasteTerminalText(content.text)
        } else if (content.type === 'image' && targetIdRef.current.startsWith('local:')) {
          writeTerminalInput(agentImagePasteInput(activeAgentAdapterIdRef.current, window.api.platform))
        }
      } catch (error) {
        console.warn('Failed to paste into terminal', error)
      } finally {
        if (!disposed) focusTerminalIfFocused(term)
      }
    }
    pasteClipboardRef.current = pasteClipboard
    const captureCodexMultilinePaste = (event: ClipboardEvent): void => {
      const text = event.clipboardData?.getData('text/plain')
      handleCodexMultilinePasteEvent(
        text,
        activeAgentAdapterIdRef.current === 'codex',
        () => event.preventDefault(),
        () => event.stopImmediatePropagation(),
        (payload) => {
          writer.markInteractive()
          writeTerminalInput(payload)
        }
      )
    }
    term.element?.addEventListener('paste', captureCodexMultilinePaste, true)

    term.attachCustomKeyEventHandler((event) => {
      if (event.type === 'keydown') recordUiDiagnostic('keydown', terminalKeyCategory(event), 250)
      if (event.key === 'Enter' && event.shiftKey) {
        if (event.type === 'keydown') {
          event.preventDefault()
          event.stopPropagation()
          onAgentActionRef.current?.()
          writer.markInteractive()
          writeTerminalInput(shiftEnterInput(window.api.platform, targetIdRef.current, activeAgentAdapterIdRef.current === 'codex'))
        } else if (event.type === 'keypress') {
          event.preventDefault()
          event.stopPropagation()
        }
        return event.type === 'keyup'
      }
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
          if (activeRuntimeId) writeTerminalInput(text)
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
    let compositionReleaseTimer = 0
    const startImeComposition = (): void => {
      if (compositionReleaseTimer) window.clearTimeout(compositionReleaseTimer)
      compositionReleaseTimer = 0
      writer.setComposing(true)
    }
    const finishImeComposition = (): void => {
      if (compositionReleaseTimer) window.clearTimeout(compositionReleaseTimer)
      compositionReleaseTimer = window.setTimeout(() => {
        compositionReleaseTimer = 0
        writer.setComposing(false)
      }, 0)
    }
    const cancelImeComposition = (): void => {
      if (compositionReleaseTimer) window.clearTimeout(compositionReleaseTimer)
      compositionReleaseTimer = 0
      writer.setComposing(false)
    }
    textarea?.addEventListener('beforeinput', rememberImeTextareaBeforeInput)
    textarea?.addEventListener('input', recoverDroppedImePunctuationFromInput)
    textarea?.addEventListener('compositionstart', startImeComposition)
    textarea?.addEventListener('compositionend', recoverDroppedImePunctuationFromComposition, true)
    textarea?.addEventListener('compositionend', finishImeComposition)
    textarea?.addEventListener('blur', cancelImeComposition)
    const recordFocus = (): void => recordUiDiagnostic('focus')
    const recordBlur = (): void => recordUiDiagnostic('blur')
    const recordPointerDown = (): void => recordUiDiagnostic('pointerDown', undefined, 250)
    const recordWheel = (): void => recordUiDiagnostic('wheel', undefined, 250)
    textarea?.addEventListener('focus', recordFocus)
    textarea?.addEventListener('blur', recordBlur)
    term.element?.addEventListener('pointerdown', recordPointerDown)
    term.element?.addEventListener('wheel', recordWheel, { passive: true })
    const resize = term.onResize(() => resizeRuntimeIfVisible(term))
    let catchingUp = false
    let needsCatchUp = true
    const queuedOutput: Array<Extract<TerminalRuntimeEvent, { type: 'output' }>['payload']> = []
    let queuedOutputSize = 0
    type TerminalOutputPayload = Extract<TerminalRuntimeEvent, { type: 'output' }>['payload']
    const outputFromCursor = (payload: TerminalOutputPayload, cursor: number): TerminalOutputPayload | undefined => {
      if (payload.endCursor <= cursor) return undefined
      if (payload.startCursor >= cursor) return payload
      const targetBytes = cursor - payload.startCursor
      const encoder = new TextEncoder()
      let offset = 0
      let consumedBytes = 0
      for (const character of payload.data) {
        const characterBytes = encoder.encode(character).length
        if (consumedBytes + characterBytes > targetBytes) break
        consumedBytes += characterBytes
        offset += character.length
      }
      if (offset >= payload.data.length) return undefined
      const data = payload.data.slice(offset)
      return {
        ...payload,
        startCursor: payload.startCursor + consumedBytes,
        data,
      }
    }
    const writeOutput = (rawPayload: TerminalOutputPayload): void => {
      const payload = outputFromCursor(rawPayload, outputCursor.current)
      if (!payload) {
        // Dropping output that the cursor has already passed is intentional,
        // but a cursor left ahead of the runtime's ring silences the pane for
        // good, so record which case this is.
        recordUiDiagnostic('outputSkipped', undefined, 250, { reason: rawPayload.endCursor <= outputCursor.current ? 'behind-cursor' : 'empty-slice' })
        return
      }
      if (payload.endCursor <= outputCursor.current) return
      const length = payload.data.length
      outputCursor.current = payload.endCursor
      pendingOutput += length
      // Background panes must keep draining their PTY. xterm rendering can be
      // throttled when a pane loses focus (for example when a database pane is
      // opened), and pausing the backend here would block the agent process on
      // its PTY output buffer. Only the active pane participates in UI flow
      // control; the runtime ring buffer remains the bounded catch-up source.
      if (visibleRef.current && !paused && pendingOutput >= terminalHighWaterMark) {
        paused = true
        recordUiDiagnostic('flowPause', undefined, 0, { reason: 'high-water' })
        void window.api.terminalRuntimes.flow(payload.runtimeId, true)
      }
      writer.write(payload.data, () => {
        if (userScrolled) term.scrollToLine(scrollAnchor)
        if (runtimeIdRef.current === payload.runtimeId) renderedOutputCursor.current = Math.max(renderedOutputCursor.current, payload.endCursor)
        pendingOutput = Math.max(0, pendingOutput - length)
        if (paused && pendingOutput <= terminalLowWaterMark && runtimeIdRef.current) {
          paused = false
          recordUiDiagnostic('flowResume', undefined, 0, { reason: 'low-water' })
          void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
        }
      })
    }
    const catchUpOutput = (): void => {
      if (disposed || catchingUp || !needsCatchUp || !visibleRef.current) return
      const catchUpRuntimeId = runtimeIdRef.current
      if (!catchUpRuntimeId) return
      catchingUp = true
      needsCatchUp = false
      let catchUpFailed = false
      void (async (): Promise<void> => {
        try {
          while (!disposed && catchUpRuntimeId === runtimeIdRef.current) {
            if (!visibleRef.current) {
              needsCatchUp = true
              break
            }
            const before = outputCursor.current
            const result = await window.api.terminalRuntimes.readOutput(catchUpRuntimeId, before, terminalHighWaterMark)
            if (result.runtimeId !== runtimeIdRef.current) {
              needsCatchUp = true
              recordUiDiagnostic('outputSkipped', undefined, 0, {
                reason: 'runtime-changed'
              })
              break
            }
            if (!result.data || result.nextCursor <= before) break
            outputCursor.current = result.nextCursor
            writer.write(result.data, () => {
              if (userScrolled) term.scrollToLine(scrollAnchor)
              if (runtimeIdRef.current === result.runtimeId) renderedOutputCursor.current = Math.max(renderedOutputCursor.current, result.nextCursor)
            })
          }
        } catch (error) {
          catchUpFailed = true
          needsCatchUp = true
          recordUiDiagnostic('outputSkipped', undefined, 0, { reason: `catch-up-failed: ${describeError(error)}` })
        } finally {
          catchingUp = false
          if (disposed) {
            queuedOutput.length = 0
            return
          }
          if (!visibleRef.current) needsCatchUp = true
          if (catchUpRuntimeId !== runtimeIdRef.current) needsCatchUp = true
          if (!needsCatchUp) {
            queuedOutput.sort((left, right) => left.startCursor - right.startCursor)
            for (const output of queuedOutput) {
              if (output.runtimeId === runtimeIdRef.current) writeOutput(output)
            }
          }
          queuedOutput.length = 0
          queuedOutputSize = 0
          if (needsCatchUp && visibleRef.current && !catchUpFailed) {
            queueMicrotask(catchUpOutput)
          }
        }
      })()
    }
    catchUpOutputRef.current = catchUpOutput
    const stop = window.api.onTerminalRuntimeEvent((event: TerminalRuntimeEvent) => {
      if (event.type !== 'output' || event.payload.runtimeId !== runtimeIdRef.current) return
      if (!visibleRef.current) {
        needsCatchUp = true
        return
      }
      if (catchingUp || needsCatchUp) {
        queuedOutputSize += event.payload.data.length
        if (queuedOutputSize > terminalHighWaterMark) {
          queuedOutput.length = 0
          queuedOutputSize = 0
          needsCatchUp = true
        } else {
          queuedOutput.push(event.payload)
        }
        if (!catchingUp) catchUpOutput()
        return
      }
      writeOutput(event.payload)
    })
    catchUpOutput()
    const observer = new ResizeObserver(() => {
      const element = container.current
      if (element?.offsetParent) requestAnimationFrame(() => {
        if (disposed || terminal.current !== term) return
        fit.fit()
        resizeRuntimeIfVisible(term)
        // The pane's layout box coming back is the signal the repair has to hang
        // off, not its visibility: a workspace that is not the shown session is
        // rendered with `hidden` and gives its panes an empty active key, so all
        // of them lose WebGL and re-measure while laid out nowhere, but only the
        // one that becomes active passes through the effect below. A pane whose
        // box returns at the size it had before is no help to `fit()` either -
        // that resize is a no-op, so it never re-measures the character size and
        // nothing else re-derives the advance. Taken after fit(), so the
        // measurement describes the layout the pane has now.
        resyncRendererDefaultSpacing(term, element)
        if (visibleRef.current) {
          term.refresh(0, Math.max(0, term.rows - 1))
          focusTerminalIfFocused(term)
        }
      })
    })
    const scroll = term.onScroll((position) => {
      const baseY = term.buffer.active.baseY
      userScrolled = position < baseY
      scrollAnchor = position
      recordUiDiagnostic('scroll', undefined, 250)
    })
    observer.observe(container.current)
    focusTerminalIfFocused(term)
    return () => {
      recordUiDiagnostic('dispose')
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
      if (paused && runtimeIdRef.current) {
        // An unpaired resume here would look identical to a low-water resume in
        // the log, even though this one is caused by the pane going away.
        recordUiDiagnostic('flowResume', undefined, 0, { reason: 'dispose' })
        void window.api.terminalRuntimes.flow(runtimeIdRef.current, false)
      }
      pendingRuntimeInput.current.clear()
      for (const pending of pendingImePunctuation) window.clearTimeout(pending.timer)
      pendingImePunctuation.clear()
      textarea?.removeEventListener('beforeinput', rememberImeTextareaBeforeInput)
      textarea?.removeEventListener('input', recoverDroppedImePunctuationFromInput)
      textarea?.removeEventListener('compositionstart', startImeComposition)
      textarea?.removeEventListener('compositionend', recoverDroppedImePunctuationFromComposition, true)
      textarea?.removeEventListener('compositionend', finishImeComposition)
      textarea?.removeEventListener('blur', cancelImeComposition)
      textarea?.removeEventListener('focus', recordFocus)
      textarea?.removeEventListener('blur', recordBlur)
      term.element?.removeEventListener('pointerdown', recordPointerDown)
      term.element?.removeEventListener('wheel', recordWheel)
      term.element?.removeEventListener('paste', captureCodexMultilinePaste, true)
      cancelImeComposition()
      stop(); observer.disconnect(); input.dispose(); resize.dispose(); scroll.dispose(); writer.dispose(); webglAddon.current?.dispose(); term.dispose()
      webglAddon.current = null; outputWriter.current = null; terminal.current = null; fitAddon.current = null
      // The recorder closes over this mount's refs; leaving it reachable after
      // dispose would let an outside effect report against a dead terminal.
      if (diagnosticRecorder.current === recordUiDiagnostic) diagnosticRecorder.current = null
      if (catchUpOutputRef.current === catchUpOutput) catchUpOutputRef.current = null
    }
  }, [paneId, shouldRenderTerminal])

  useEffect(() => {
    if (visible && runtimeId) catchUpOutputRef.current?.()
  }, [visible, runtimeId])

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
    if (!focused) {
      webglAddon.current?.dispose()
      webglAddon.current = null
      element.dataset.renderer = 'dom'
      outputWriter.current?.wrapRenderer()
    } else if (!webglAddon.current) {
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
    if (!visible) return
    requestAnimationFrame(() => {
      if (terminal.current !== term) return
      fitAddon.current?.fit()
      // After fit(), so the advance is measured against the layout the pane has
      // now and the repaint below already uses the repaired value.
      resyncRendererDefaultSpacing(term, element)
      term.refresh(0, Math.max(0, term.rows - 1))
      focusTerminalIfFocused(term)
      // The WebGL addon can finish attaching after the first frame when a
      // session was hidden for a while. Refresh again once its cell metrics
      // have been committed by the browser.
      requestAnimationFrame(() => {
        if (terminal.current !== term || !visibleRef.current) return
        fitAddon.current?.fit()
        resyncRendererDefaultSpacing(term, element)
        term.refresh(0, Math.max(0, term.rows - 1))
        focusTerminalIfFocused(term)
      })
    })
  }, [focused, visible, shouldRenderTerminal])

  useEffect(() => {
    // If a visible pane was flow-paused just before it became inactive, do not
    // leave its PTY reader suspended while another pane is being used.
    if (visible || !runtimeId) return
    // Recorded through the ref because this effect sits outside the terminal
    // effect's closure; an unlogged resume here hides the pane-inactive state.
    diagnosticRecorder.current?.('flowResume', undefined, 0, { reason: 'pane-inactive' })
    void window.api.terminalRuntimes.flow(runtimeId, false)
  }, [visible, runtimeId])

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
    if (!focused) return
    const interceptFind = (event: KeyboardEvent): void => {
      const commandKey = window.api.platform === 'darwin' ? event.metaKey : event.ctrlKey
      if (!commandKey || event.key.toLowerCase() !== 'f') return
      event.preventDefault()
      event.stopImmediatePropagation()
      setSearchOpen(true)
    }
    window.addEventListener('keydown', interceptFind, true)
    return () => window.removeEventListener('keydown', interceptFind, true)
  }, [focused])

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

  const ActionIcon = stoppedState.actionIcon === 'credentials' ? KeyRound : stoppedState.actionIcon === 'retry' ? RefreshCw : Play
  const stoppedContent = <div className={`terminal-stopped-state ${stoppedState.tone ?? ''}`} role={stoppedState.tone === 'error' ? 'alert' : 'status'}>
    <div className="terminal-stopped-icon">{stoppedState.tone === 'error' ? <ShieldAlert size={24} /> : <span>&gt;_</span>}</div>
    <strong>{stoppedState.title}</strong>
    <span className="terminal-stopped-description">{stoppedState.description}</span>
    <div className="terminal-stopped-actions">
      {onStart && <button type="button" className="terminal-connect-button" onClick={onStart}><ActionIcon size={15} />{stoppedState.actionLabel}</button>}
      {onClose && <button type="button" className="terminal-close-button" onClick={onClose}><X size={15} />{t('app.closePane')}</button>}
    </div>
  </div>

  if (!shouldRenderTerminal) return <div className="terminal-empty">{stoppedContent}</div>
  return <div className="terminal-shell" onPointerDownCapture={(event) => {
    if (event.button === 0 && visible) terminal.current?.focus()
    if (event.button === 2) event.stopPropagation()
  }} onContextMenu={(event) => { event.preventDefault(); setContextMenu({ x: Math.max(4, Math.min(event.clientX, window.innerWidth - 210)), y: Math.max(4, Math.min(event.clientY, window.innerHeight - 210)), hasSelection: Boolean(terminal.current?.hasSelection()) }) }}>
    {searchOpen && <form className="terminal-search" onSubmit={(event) => { event.preventDefault(); search() }}>
      <Search size={14} /><input ref={searchInput} value={query} aria-label={t('terminal.searchTerminal')} onChange={(event) => { setQuery(event.target.value); search(event.target.value, false, true) }} onKeyDown={(event) => { if (event.key === 'Escape') setSearchOpen(false) }} />
      <output className={query && !searchResult.count ? 'no-results' : ''}>{query ? searchResult.count ? `${Math.max(0, searchResult.index) + 1}/${searchResult.count}` : t('terminal.noResults') : ''}</output>
      <button type="button" className="icon-button" title={t('terminal.previousResult')} onClick={() => search(query, true)}><ChevronUp size={15} /></button>
      <button type="button" className="icon-button" title={t('terminal.nextResult')} onClick={() => search()}><ChevronDown size={15} /></button>
      <button type="button" className="icon-button" title={t('terminal.closeSearch')} onClick={() => setSearchOpen(false)}><X size={15} /></button>
    </form>}
    <div className="terminal" ref={container} />
    {!connected && !connecting && <div className="terminal-stopped-overlay">{stoppedContent}</div>}
    {contextMenu && <div className="sidebar-context-menu terminal-context-menu" role="menu" style={{ left: contextMenu.x, top: contextMenu.y }} onPointerDown={(event) => event.stopPropagation()}>
      <button role="menuitem" disabled={!contextMenu.hasSelection} onClick={() => { const text = terminal.current?.getSelection(); setContextMenu(null); if (text) void window.api.system.writeClipboard(text).catch((error) => console.warn('Failed to copy terminal selection', error)) }}><Copy size={15} />{t('common.copy')}</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); void pasteClipboardRef.current() }}><ClipboardPaste size={15} />{t('terminal.paste')}</button>
      <button role="menuitem" onClick={() => { setContextMenu(null); setSearchOpen(true) }}><Search size={15} />{t('terminal.search')}</button>
      <div className="context-menu-separator" />
      <button role="menuitem" onClick={() => { setContextMenu(null); onOpenSettings?.() }}><Palette size={15} />{t('terminal.appearanceSettings')}</button>
    </div>}
  </div>
})
