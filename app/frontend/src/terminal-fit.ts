import type { Terminal } from '@xterm/xterm'
import type { FitAddon } from '@xterm/addon-fit'

/** Font/DPR changes can update xterm's cell height after ResizeObserver fitted
 * the container. Its box then stays unchanged while the last rows overflow it.
 * Reconcile against the committed renderer metrics after rendering, without
 * refitting or scheduling another frame when the terminal already fits. */
export function reconcileTerminalFit(
  term: Pick<Terminal, 'cols' | 'rows' | 'onRender'>,
  fit: Pick<FitAddon, 'proposeDimensions' | 'fit'>,
  options: {
    visible(): boolean
    screenFits(): boolean
    afterFit(): void
    requestFrame(callback: () => void): number
    cancelFrame(frame: number): void
  }
): { check(): void; dispose(): void } {
  let disposed = false
  let frame: number | undefined
  const needsFit = (): boolean => {
    if (disposed || !options.visible()) return false
    const size = fit.proposeDimensions()
    if (!size || !Number.isFinite(size.cols) || !Number.isFinite(size.rows)
      || size.cols < 2 || size.rows < 1 || (size.cols === term.cols && size.rows === term.rows)) return false
    // DOM canvas dimensions are rounded before being divided by rows/cols.
    // At a cell boundary FitAddon can alternate between N and N+1 forever.
    // Keep a screen that already fits when the disagreement is just one cell;
    // ordinary container resizes still use FitAddon directly as before.
    if (Math.abs(size.cols - term.cols) <= 1 && Math.abs(size.rows - term.rows) <= 1
      && options.screenFits()) return false
    return true
  }
  const check = (): void => {
    if (frame !== undefined || !needsFit()) return
    frame = options.requestFrame(() => {
      frame = undefined
      // Metrics and visibility can change again before this animation frame.
      if (!needsFit()) return
      fit.fit()
      options.afterFit()
    })
  }
  const render = term.onRender(check)
  return {
    check,
    dispose: () => {
      disposed = true
      render.dispose()
      if (frame !== undefined) options.cancelFrame(frame)
    }
  }
}
