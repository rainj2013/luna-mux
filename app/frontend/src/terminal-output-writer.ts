import type { IDisposable, Terminal } from '@xterm/xterm'

const backgroundSameColumnSettleMs = 150
const backgroundRelocateSettleMs = 500

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
  __lunaMuxCursorStabilized?: boolean
}

interface PrivateRenderService {
  handleCursorMove(): void
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
  wrapRenderer(): void
  dispose(): void
}

function includesParameter(params: (number | number[])[], expected: number): boolean {
  return params.some((param) => Array.isArray(param) ? param.includes(expected) : param === expected)
}

export function createTerminalOutputWriter(term: Terminal): TerminalOutputWriter {
  const internals = term as Terminal & TerminalInternals
  const disposables: IDisposable[] = []
  let pending: PendingWrite[] = []
  let scheduledFrame = 0
  let scheduledTimer = 0
  let disposed = false
  let writeInProgress = false
  let unsafeBatch = false
  let interactiveUntil = 0
  let settleTimer = 0
  let committedX = term.buffer.active.cursorX
  let committedY = term.buffer.active.cursorY
  let committedHidden = Boolean(internals._core?.coreService?.isCursorHidden)

  const privateBuffer = (): PrivateBuffer | undefined => internals._core?._bufferService?.buffer
  const synchronizedOutput = (): boolean => Boolean(internals._core?.coreService?.decPrivateModes?.synchronizedOutput)
  const pinned = (): boolean => writeInProgress || settleTimer !== 0 || synchronizedOutput()
  const clampX = (value: number): number => Math.max(0, Math.min(term.cols - 1, value))
  const clampY = (value: number): number => Math.max(0, Math.min(term.rows - 1, value))
  const settleDelay = (): number => {
    return privateBuffer()?.x === committedX ? backgroundSameColumnSettleMs : backgroundRelocateSettleMs
  }

  const renderService = internals._core?._renderService
  const originalHandleCursorMove = renderService?.handleCursorMove.bind(renderService)
  let guardedHandleCursorMove: (() => void) | undefined
  if (renderService && originalHandleCursorMove) {
    guardedHandleCursorMove = () => {
      if (!pinned()) originalHandleCursorMove()
    }
    renderService.handleCursorMove = guardedHandleCursorMove
  }

  const refreshCursorRows = (previousY: number, nextY: number): void => {
    const lastRow = Math.max(0, term.rows - 1)
    term.refresh(
      Math.max(0, Math.min(lastRow, previousY, nextY)),
      Math.max(0, Math.min(lastRow, Math.max(previousY, nextY)))
    )
  }

  const commit = (): void => {
    const buffer = privateBuffer()
    if (!buffer) return
    const previousX = committedX
    const previousY = committedY
    const previousHidden = committedHidden
    committedX = buffer.x
    committedY = buffer.y
    committedHidden = Boolean(internals._core?.coreService?.isCursorHidden)
    requestAnimationFrame(() => {
      if (disposed || pinned()) return
      originalHandleCursorMove?.()
      if (previousX !== committedX || previousY !== committedY || previousHidden !== committedHidden) refreshCursorRows(previousY, committedY)
    })
  }

  const commitNow = (): void => {
    if (settleTimer) window.clearTimeout(settleTimer)
    settleTimer = 0
    commit()
  }

  const armSettle = (delay: number): void => {
    if (settleTimer) window.clearTimeout(settleTimer)
    settleTimer = window.setTimeout(() => {
      settleTimer = 0
      if (disposed) return
      if (writeInProgress || synchronizedOutput()) {
        armSettle(16)
        return
      }
      commit()
    }, delay)
  }

  const unsafeCursorFinals = ['H', 'f', 'A', 'B', 'E', 'F', 'G', '`', 'd', 'r']
  for (const final of unsafeCursorFinals) {
    disposables.push(term.parser.registerCsiHandler({ final }, () => {
      unsafeBatch = true
      return false
    }))
  }
  for (const final of ['C', 'D']) {
    disposables.push(term.parser.registerCsiHandler({ final }, (params) => {
      const buffer = privateBuffer()
      const first = params[0]
      const amount = Number(Array.isArray(first) ? first[0] : first) || 1
      const echoLike = buffer?.x === committedX && buffer.y === committedY && amount <= 8
      if (!echoLike) unsafeBatch = true
      return false
    }))
  }
  for (const final of ['h', 'l']) {
    disposables.push(term.parser.registerCsiHandler({ final, prefix: '?' }, (params) => {
      if (includesParameter(params, 25) || includesParameter(params, 2026) || includesParameter(params, 47) || includesParameter(params, 1047) || includesParameter(params, 1049)) {
        unsafeBatch = true
      }
      return false
    }))
  }

  const wrapRenderer = (): void => {
    const renderer = internals._core?._renderService?._renderer?.value
    if (!renderer || renderer.__lunaMuxCursorStabilized) return
    const renderRows = renderer.renderRows.bind(renderer)
    renderer.renderRows = (start, end) => {
      const buffer = privateBuffer()
      if (!buffer || !pinned()) {
        renderRows(start, end)
        return
      }
      const actualX = buffer.x
      const actualY = buffer.y
      const coreService = internals._core?.coreService
      const actualHidden = coreService?.isCursorHidden
      const stableY = clampY(committedY)
      buffer.x = clampX(committedX)
      buffer.y = stableY
      if (coreService) coreService.isCursorHidden = committedHidden
      try {
        renderRows(Math.min(start, stableY), Math.max(end, stableY))
      } finally {
        buffer.x = actualX
        buffer.y = actualY
        if (coreService && actualHidden !== undefined) coreService.isCursorHidden = actualHidden
      }
    }
    renderer.__lunaMuxCursorStabilized = true
  }

  const scheduleFlush = (): void => {
    if (disposed || writeInProgress || !pending.length) return
    if (performance.now() < interactiveUntil) {
      if (scheduledFrame) cancelAnimationFrame(scheduledFrame)
      scheduledFrame = 0
      if (scheduledTimer) window.clearTimeout(scheduledTimer)
      scheduledTimer = window.setTimeout(flush, 4)
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
    unsafeBatch = false
    writeInProgress = true
    term.write(data, () => {
      writeInProgress = false
      if (!disposed) {
        const buffer = privateBuffer()
        const interactive = performance.now() < interactiveUntil
        const safeTextAtCommittedColumn = !unsafeBatch && buffer?.x === committedX && !synchronizedOutput()
        const ordinaryText = !unsafeBatch && settleTimer === 0 && !synchronizedOutput()
        if (interactive || safeTextAtCommittedColumn || ordinaryText) commitNow()
        else armSettle(settleDelay())
      }
      for (const write of writes) write.callback?.()
      scheduleFlush()
    })
  }

  wrapRenderer()

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
    wrapRenderer,
    dispose() {
      disposed = true
      if (scheduledFrame) cancelAnimationFrame(scheduledFrame)
      if (scheduledTimer) window.clearTimeout(scheduledTimer)
      if (settleTimer) window.clearTimeout(settleTimer)
      scheduledFrame = 0
      scheduledTimer = 0
      settleTimer = 0
      for (const write of pending) write.callback?.()
      pending = []
      for (const disposable of disposables) disposable.dispose()
      if (renderService && guardedHandleCursorMove && renderService.handleCursorMove === guardedHandleCursorMove && originalHandleCursorMove) {
        renderService.handleCursorMove = originalHandleCursorMove
      }
    }
  }
}
