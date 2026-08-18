import type { IDisposable, Terminal } from '@xterm/xterm'

/**
 * The terminal has one cursor in its buffer, but a repaint-heavy TUI uses
 * that cursor for two different jobs: drawing a footer and accepting input.
 * The buffer cursor must remain authoritative for input.  The renderer is
 * therefore given a small, display-only shadow cursor while a DEC 2026 frame
 * is being flushed.  A following DECTCEM show (`?25h`) is Codex's explicit
 * "park the visible cursor here" signal and is copied to the shadow cursor.
 */
const parkFallbackMs = 50

interface PendingWrite {
  data: string
  callback?: () => void
}

interface PrivateBuffer {
  x: number
  y: number
}

interface PrivateRenderer {
  renderRows(start: number, end: number): void
  __lunaMuxShadowCursor?: boolean
}

interface PrivateRenderService {
  handleCursorMove(): void
  dimensions?: { css?: { cell?: { width: number; height: number } } }
  _renderer?: { value?: PrivateRenderer }
}

interface PrivateCore {
  _bufferService?: { buffer?: PrivateBuffer }
  _renderService?: PrivateRenderService
  coreService?: {
    decPrivateModes?: { synchronizedOutput?: boolean }
    isCursorHidden?: boolean
  }
}

interface TerminalInternals {
  _core?: PrivateCore
}

export interface TerminalOutputWriter {
  write(data: string, callback?: () => void): void
  markInteractive(submitting?: boolean): void
  setComposing(active: boolean): void
  wrapRenderer(): void
  dispose(): void
}

function includesParameter(params: (number | number[])[], expected: number): boolean {
  return params.some((param) => Array.isArray(param) ? param.includes(expected) : param === expected)
}

function isDefaultCursorStyle(params: (number | number[])[]): boolean {
  if (!params.length) return true
  const first = params[0]
  return (Array.isArray(first) ? first[0] : first) === 0
}

export function createTerminalOutputWriter(term: Terminal): TerminalOutputWriter {
  const internals = term as Terminal & TerminalInternals
  const disposables: IDisposable[] = []
  let pending: PendingWrite[] = []
  let scheduledFrame = 0
  let scheduledTimer = 0
  let disposed = false
  let writeInProgress = false
  let composing = false
  let interactiveUntil = 0
  let syncOutputActive = false
  let altBufferActive = false
  let codexRepaintDetected = false
  let shadowActive = false
  let parkPending = false
  let parkTimer = 0
  let cursorHidden = Boolean(internals._core?.coreService?.isCursorHidden)
  let shadowHidden = cursorHidden
  let shadowX = term.buffer.active.cursorX
  let shadowY = term.buffer.active.cursorY
  let frameSavedX = shadowX
  let frameSavedY = shadowY

  const privateBuffer = (): PrivateBuffer | undefined => internals._core?._bufferService?.buffer
  const coreService = (): NonNullable<PrivateCore['coreService']> | undefined => internals._core?.coreService
  const clampX = (value: number): number => Math.max(0, Math.min(Math.max(0, term.cols - 1), value))
  const clampY = (value: number): number => Math.max(0, Math.min(Math.max(0, term.rows - 1), value))
  const interactive = (): boolean => composing || performance.now() < interactiveUntil
  const shouldUseShadow = (): boolean => (shadowActive || composing) && !altBufferActive

  const renderService = internals._core?._renderService
  const originalHandleCursorMove = renderService?.handleCursorMove.bind(renderService)
  const clearCompositionAnchor = (): void => {
    const element = term.element
    if (!element) return
    element.classList.remove('luna-ime-composing')
    element.style.removeProperty('--luna-ime-anchor-left')
    element.style.removeProperty('--luna-ime-anchor-top')
  }

  const pinCompositionAnchor = (): void => {
    const element = term.element
    if (!element) return
    const cell = renderService?.dimensions?.css?.cell
    const buffer = privateBuffer()
    const anchorX = buffer?.x ?? shadowX
    const anchorY = buffer?.y ?? shadowY
    const textarea = term.textarea
    const left = cell && cell.width > 0
      ? `${clampX(anchorX) * cell.width}px`
      : textarea?.style.left || '0px'
    const top = cell && cell.height > 0
      ? `${clampY(anchorY) * cell.height}px`
      : textarea?.style.top || '0px'
    element.style.setProperty('--luna-ime-anchor-left', left)
    element.style.setProperty('--luna-ime-anchor-top', top)
    element.classList.add('luna-ime-composing')
  }
  const refreshCursorRows = (previousY: number, nextY: number): void => {
    const lastRow = Math.max(0, term.rows - 1)
    term.refresh(
      Math.max(0, Math.min(lastRow, previousY, nextY)),
      Math.max(0, Math.min(lastRow, Math.max(previousY, nextY)))
    )
  }

  const refreshShadowCursor = (previousX: number, previousY: number, previousHidden: boolean): void => {
    if (disposed) return
    if (previousX !== shadowX || previousY !== shadowY || previousHidden !== shadowHidden) {
      refreshCursorRows(previousY, shadowY)
    }
  }

  const copyBufferToShadow = (force = false): void => {
    const buffer = privateBuffer()
    if (!buffer || (!force && (syncOutputActive || parkPending))) return
    const previousX = shadowX
    const previousY = shadowY
    const previousHidden = shadowHidden
    shadowX = buffer.x
    shadowY = buffer.y
    shadowHidden = cursorHidden
    refreshShadowCursor(previousX, previousY, previousHidden)
  }

  const armParkFallback = (): void => {
    if (parkTimer) window.clearTimeout(parkTimer)
    parkTimer = window.setTimeout(() => {
      parkTimer = 0
      if (disposed || !parkPending) return
      const previousX = shadowX
      const previousY = shadowY
      const previousHidden = shadowHidden
      parkPending = false
      shadowX = frameSavedX
      shadowY = frameSavedY
      shadowHidden = cursorHidden
      refreshShadowCursor(previousX, previousY, previousHidden)
    }, parkFallbackMs)
  }

  const clearParkFallback = (): void => {
    if (parkTimer) window.clearTimeout(parkTimer)
    parkTimer = 0
  }

  const markCursorHidden = (hidden: boolean): void => {
    cursorHidden = hidden
    if (!syncOutputActive && !parkPending && !altBufferActive) {
      const previousHidden = shadowHidden
      shadowHidden = hidden
      if (previousHidden !== shadowHidden) refreshCursorRows(shadowY, shadowY)
    }
  }

  const onSyncFrameStart = (): void => {
    const buffer = privateBuffer()
    if (buffer && !parkPending) {
      frameSavedX = buffer.x
      frameSavedY = buffer.y
    }
    syncOutputActive = true
    shadowActive = codexRepaintDetected
  }

  const onSyncFrameEnd = (): void => {
    syncOutputActive = false
    if (codexRepaintDetected && !altBufferActive) {
      shadowActive = true
      parkPending = true
      armParkFallback()
      // A frame that intentionally ends hidden must hide the display cursor
      // immediately; the park freeze only applies while it is visible.
      if (cursorHidden && !shadowHidden) {
        const previousHidden = shadowHidden
        shadowHidden = true
        refreshShadowCursor(shadowX, shadowY, previousHidden)
      }
    }
  }

  const onCursorPark = (): void => {
    const buffer = privateBuffer()
    if (!buffer || altBufferActive || syncOutputActive) {
      markCursorHidden(false)
      return
    }
    const previousX = shadowX
    const previousY = shadowY
    const previousHidden = shadowHidden
    shadowActive = true
    shadowX = buffer.x
    shadowY = buffer.y
    shadowHidden = false
    cursorHidden = false
    parkPending = false
    clearParkFallback()
    refreshShadowCursor(previousX, previousY, previousHidden)
  }

  const onCursorMove = (): void => {
    // During a repaint the buffer cursor belongs to the frame.  Outside it,
    // this is the low-latency path that keeps typing, delete and arrows
    // perfectly synchronized with the real xterm cursor.
    if (!privateBuffer() || composing || syncOutputActive || altBufferActive) return
    if (parkPending && !interactive()) return
    if (parkPending) {
      parkPending = false
      clearParkFallback()
    }
    copyBufferToShadow(true)
  }

  let guardedHandleCursorMove: (() => void) | undefined
  if (renderService && originalHandleCursorMove) {
    guardedHandleCursorMove = () => {
      // WebGL restarts the CSS blink animation for every cursor move.  Codex
      // moves the buffer cursor repeatedly while drawing a synchronized
      // frame, so forwarding those renderer notifications is the fast blink
      // seen by the user.  Input/output parsing still updates the real buffer.
      if (!syncOutputActive && !composing) originalHandleCursorMove()
    }
    renderService.handleCursorMove = guardedHandleCursorMove
  }

  const wrapRenderer = (): void => {
    const renderer = internals._core?._renderService?._renderer?.value
    if (!renderer || renderer.__lunaMuxShadowCursor) return
    const renderRows = renderer.renderRows.bind(renderer)
    renderer.renderRows = (start, end) => {
      const buffer = privateBuffer()
      if (!buffer || !shouldUseShadow()) {
        renderRows(start, end)
        return
      }
      const actualX = buffer.x
      const actualY = buffer.y
      const service = coreService()
      const actualHidden = service?.isCursorHidden
      buffer.x = clampX(shadowX)
      buffer.y = clampY(shadowY)
      if (service) service.isCursorHidden = shadowHidden
      try {
        renderRows(start, end)
      } finally {
        buffer.x = actualX
        buffer.y = actualY
        if (service && actualHidden !== undefined) service.isCursorHidden = actualHidden
      }
    }
    renderer.__lunaMuxShadowCursor = true
  }

  // Codex emits `CSI 0 SP q` on every repaint.  xterm interprets it as a
  // cursor-style reset, restarting its blink animation each time.  A zero
  // style means "default" and has no useful effect for this terminal, so
  // consume only that form; explicit bar/underline/block styles remain intact.
  disposables.push(term.parser.registerCsiHandler({ final: 'q', intermediates: ' ' }, (params) => {
    if (!isDefaultCursorStyle(params) || !syncOutputActive) return false
    codexRepaintDetected = true
    shadowActive = true
    return true
  }))

  for (const final of ['h', 'l']) {
    disposables.push(term.parser.registerCsiHandler({ final, prefix: '?' }, (params) => {
      if (includesParameter(params, 2026)) {
        if (final === 'h') onSyncFrameStart()
        else onSyncFrameEnd()
      }
      if (includesParameter(params, 25)) {
        markCursorHidden(final === 'l')
        if (final === 'h') onCursorPark()
      }
      if (includesParameter(params, 47) || includesParameter(params, 1047) || includesParameter(params, 1049)) {
        altBufferActive = final === 'h'
        clearParkFallback()
        parkPending = false
        shadowActive = false
        if (final === 'l') {
          cursorHidden = false
          copyBufferToShadow(true)
        }
      }
      return false
    }))
  }

  disposables.push(term.onCursorMove(onCursorMove))

  wrapRenderer()

  const scheduleFlush = (): void => {
    if (disposed || writeInProgress || !pending.length) return
    if (performance.now() < interactiveUntil) {
      if (scheduledFrame) cancelAnimationFrame(scheduledFrame)
      scheduledFrame = 0
      if (scheduledTimer) window.clearTimeout(scheduledTimer)
      scheduledTimer = window.setTimeout(flush, 0)
      return
    }
    if (!scheduledFrame) scheduledFrame = requestAnimationFrame(flush)
  }

  const flush = (): void => {
    scheduledFrame = 0
    scheduledTimer = 0
    if (disposed || writeInProgress || !pending.length) return
    const writes = pending
    pending = []
    const data = writes.map((write) => write.data).join('')
    writeInProgress = true
    term.write(data, () => {
      writeInProgress = false
      if (!disposed) {
        // onCursorMove normally updates this during parsing.  This final
        // synchronization also covers plain text writes that do not emit a
        // cursor-move event, without delaying the next interactive frame.
        if (!syncOutputActive && !parkPending && !altBufferActive) copyBufferToShadow()
      }
      for (const write of writes) write.callback?.()
      scheduleFlush()
    })
  }

  return {
    write(data, callback) {
      if (disposed || !data) {
        callback?.()
        return
      }
      pending.push({ data, callback })
      scheduleFlush()
    },
    markInteractive(submitting = false) {
      interactiveUntil = submitting ? 0 : performance.now() + 500
    },
    setComposing(active) {
      if (disposed || composing === active) return
      if (active) {
        composing = true
        copyBufferToShadow(true)
        pinCompositionAnchor()
      } else {
        composing = false
        clearCompositionAnchor()
        copyBufferToShadow(true)
      }
    },
    wrapRenderer,
    dispose() {
      disposed = true
      composing = false
      clearCompositionAnchor()
      if (scheduledFrame) cancelAnimationFrame(scheduledFrame)
      if (scheduledTimer) window.clearTimeout(scheduledTimer)
      clearParkFallback()
      scheduledFrame = 0
      scheduledTimer = 0
      for (const write of pending) write.callback?.()
      pending = []
      for (const disposable of disposables) disposable.dispose()
      if (renderService && guardedHandleCursorMove && renderService.handleCursorMove === guardedHandleCursorMove && originalHandleCursorMove) {
        renderService.handleCursorMove = originalHandleCursorMove
      }
    }
  }
}
