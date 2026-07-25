import { afterEach, describe, expect, it, vi } from 'vitest'
import { TerminalOutputBuffer } from './terminal-output-buffer'

describe('TerminalOutputBuffer', () => {
  afterEach(() => vi.useRealTimers())

  it('batches output into one emission per flush window', () => {
    vi.useFakeTimers()
    const emit = vi.fn()
    const output = new TerminalOutputBuffer(emit)

    output.write('stdout', Buffer.from('one'))
    output.write('stderr', Buffer.from(' two'))
    expect(emit).not.toHaveBeenCalled()

    vi.advanceTimersByTime(16)
    expect(emit).toHaveBeenCalledOnce()
    expect(emit).toHaveBeenCalledWith('one two')
  })

  it('preserves UTF-8 characters split across SSH data chunks', () => {
    vi.useFakeTimers()
    const emit = vi.fn()
    const output = new TerminalOutputBuffer(emit)
    const bytes = Buffer.from('目录')

    output.write('stdout', bytes.subarray(0, 2))
    output.write('stdout', bytes.subarray(2, 4))
    output.write('stdout', bytes.subarray(4))
    vi.advanceTimersByTime(16)

    expect(emit).toHaveBeenCalledWith('目录')
  })

  it('flushes immediately at the batch size limit and when ending', () => {
    vi.useFakeTimers()
    const emit = vi.fn()
    const output = new TerminalOutputBuffer(emit, 16, 4)

    output.write('stdout', Buffer.from('1234'))
    expect(emit).toHaveBeenNthCalledWith(1, '1234')

    output.write('stdout', Buffer.from('last'))
    output.end()
    expect(emit).toHaveBeenNthCalledWith(2, 'last')
    vi.runAllTimers()
    expect(emit).toHaveBeenCalledTimes(2)
  })
})
