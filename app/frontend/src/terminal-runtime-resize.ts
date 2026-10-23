type Resize = (runtimeId: string, cols: number, rows: number) => Promise<void>
interface Waiter { resolve(): void; reject(error: unknown): void }
interface PendingResize { cols: number; rows: number; waiters: Waiter[] }
interface ResizeQueue { pending?: PendingResize }

/**
 * Keep one native resize in flight per Runtime. Native backends use blocking
 * workers, whose execution order need not match IPC submission order. While a
 * resize is running, retain only the newest size; superseded callers settle
 * with that size's result. Other Runtimes continue resizing independently.
 * Own this queue at the API boundary so a pane remount cannot bypass it.
 */
export function createTerminalRuntimeResizer(send: Resize): Resize {
  const queues = new Map<string, ResizeQueue>()

  const drain = async (runtimeId: string, queue: ResizeQueue): Promise<void> => {
    while (queue.pending) {
      const request = queue.pending
      queue.pending = undefined
      try {
        await send(runtimeId, request.cols, request.rows)
        for (const waiter of request.waiters) waiter.resolve()
      } catch (error) {
        for (const waiter of request.waiters) waiter.reject(error)
      }
    }
    queues.delete(runtimeId)
  }

  return (runtimeId, cols, rows) => new Promise<void>((resolve, reject) => {
    const existing = queues.get(runtimeId)
    const queue = existing ?? {}
    const waiters = queue.pending?.waiters ?? []
    waiters.push({ resolve, reject })
    queue.pending = { cols, rows, waiters }
    if (!existing) {
      queues.set(runtimeId, queue)
      void drain(runtimeId, queue)
    }
  })
}
