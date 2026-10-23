import assert from 'node:assert/strict'
import test from 'node:test'
import { createRequire } from 'node:module'
import { shouldUseWebglRenderer } from '../app/frontend/src/terminal-renderer.ts'
import { configureTerminalUnicode } from '../app/frontend/src/terminal-unicode.ts'
import { createTerminalRuntimeResizer } from '../app/frontend/src/terminal-runtime-resize.ts'
import { reconcileTerminalFit } from '../app/frontend/src/terminal-fit.ts'

function fitFixture() {
  const frames = new Map<number, () => void>()
  let frameId = 0
  let render: (() => void) | undefined
  let visible = true
  let cellHeight = 20
  let nativeRows = 46
  let fitCount = 0
  const term = { cols: 163, rows: 46, onRender: (callback: () => void) => {
    render = callback
    return { dispose: () => { render = undefined } }
  } }
  const fit = {
    proposeDimensions: () => ({ cols: 163, rows: Math.floor(922 / cellHeight) }),
    fit: () => { fitCount += 1; term.rows = Math.floor(922 / cellHeight) }
  }
  const reconciler = reconcileTerminalFit(term, fit, {
    visible: () => visible,
    screenFits: () => term.rows * cellHeight <= 922,
    afterFit: () => { nativeRows = term.rows },
    requestFrame: (callback) => { frames.set(++frameId, callback); return frameId },
    cancelFrame: (id) => { frames.delete(id) }
  })
  return {
    term, reconciler, frames,
    render: () => render?.(),
    cellHeight: (value: number) => { cellHeight = value },
    visible: (value: boolean) => { visible = value },
    nativeRows: () => nativeRows,
    fits: () => fitCount,
    flush: () => { const callbacks = [...frames.values()]; frames.clear(); for (const callback of callbacks) callback() }
  }
}

test('late fullscreen cell metrics refit the last row inside an unchanged container', () => {
  const pane = fitFixture()
  pane.cellHeight(22.5)
  assert.equal(pane.term.rows * 22.5, 1035, 'captured macOS overflow before repair')
  pane.render()
  pane.render()
  assert.equal(pane.frames.size, 1, 'coalesce repaint events')
  pane.flush()
  assert.equal(pane.term.rows, 40)
  assert.equal(pane.nativeRows(), 40, 'synchronize the corrected size to the Runtime')
  assert.ok(pane.term.rows * 22.5 <= 922, 'bottom row now fits')
  pane.render()
  assert.equal(pane.frames.size, 0, 'a stable render must not create a fit loop')
  pane.reconciler.dispose()
})

test('fit reconciliation uses current metrics and defers a hidden pane', () => {
  const pane = fitFixture()
  pane.cellHeight(22.5)
  pane.render()
  pane.cellHeight(25)
  pane.flush()
  assert.equal(pane.term.rows, 36, 'read the latest metrics in the animation frame')
  pane.cellHeight(20)
  pane.render()
  pane.visible(false)
  pane.flush()
  assert.equal(pane.term.rows, 36, 'never measure while hidden')
  pane.visible(true)
  pane.render()
  pane.flush()
  assert.equal(pane.term.rows, 46)
  pane.reconciler.dispose()
})

test('unmount cancels a queued font-metric repair', () => {
  const pane = fitFixture()
  pane.cellHeight(22.5)
  pane.render()
  pane.reconciler.dispose()
  pane.flush()
  pane.render()
  pane.reconciler.check()
  assert.equal(pane.fits(), 0)
  assert.equal(pane.frames.size, 0)
})

for (const initialRows of [44, 45, 46]) test(`rounded DOM metrics stabilize from ${initialRows} rows without a resize loop`, () => {
  let render!: () => void
  let frame: (() => void) | undefined
  let fits = 0
  const term = { cols: 163, rows: initialRows, onRender: (callback: () => void) => {
    render = callback
    return { dispose: () => {} }
  } }
  // Match DomRenderer's rounded canvas / rows at DPR 2, then FitAddon's
  // floor(container / cssCellHeight). 44 proposes 45; 45 proposes 44.
  const proposedRows = () => Math.floor(923 / (Math.round(41 * term.rows / 2) / term.rows))
  const fit = {
    proposeDimensions: () => ({ cols: 163, rows: proposedRows() }),
    fit: () => { fits += 1; term.rows = proposedRows() }
  }
  const reconciler = reconcileTerminalFit(term, fit, {
    visible: () => true,
    screenFits: () => Math.round(41 * term.rows / 2) <= 923,
    afterFit: () => {},
    requestFrame: (callback) => { frame = callback; return 1 },
    cancelFrame: () => { frame = undefined }
  })
  for (let i = 0; i < 10; i += 1) {
    render()
    const callback = frame
    frame = undefined
    callback?.()
  }
  assert.equal(fits, initialRows === 46 ? 1 : 0)
  assert.equal(frame, undefined, 'stable metrics must not leave another fit scheduled')
  assert.ok(Math.round(41 * term.rows / 2) <= 923)
  reconciler.dispose()
})

const { Terminal } = createRequire(import.meta.url)('@xterm/xterm') as typeof import('@xterm/xterm')
const { SerializeAddon } = createRequire(import.meta.url)('@xterm/addon-serialize') as typeof import('@xterm/addon-serialize')

test('fullscreen resizes finish at the latest size even when native resizing is slow', async () => {
  const applied: number[][] = []
  let releaseFirst!: () => void
  const first = new Promise<void>(resolve => { releaseFirst = resolve })
  const resize = createTerminalRuntimeResizer(async (_id, cols, rows) => {
    if (cols === 100) await first
    applied.push([cols, rows])
  })
  const windowed = resize('pane', 100, 30)
  const intermediate = resize('pane', 160, 60)
  const fullscreen = resize('pane', 180, 50)
  await Promise.resolve()
  assert.deepEqual(applied, [], 'newer requests must not overtake the slow first request')
  releaseFirst()
  await Promise.all([windowed, intermediate, fullscreen])
  assert.deepEqual(applied, [[100, 30], [180, 50]], 'skip obsolete animation sizes and apply the final size last')
})

test('a blocked resize does not block a different Runtime', async () => {
  let release!: () => void
  const blocked = new Promise<void>(resolve => { release = resolve })
  const applied: string[] = []
  const resize = createTerminalRuntimeResizer(async (id) => {
    if (id === 'slow') await blocked
    applied.push(id)
  })
  const slow = resize('slow', 100, 30)
  await resize('other', 120, 40)
  assert.deepEqual(applied, ['other'])
  release()
  await slow
  assert.deepEqual(applied, ['other', 'slow'])
})

test('a rejected resize does not discard a queued size or prevent a later retry', async () => {
  let rejectFirst!: (error: Error) => void
  const first = new Promise<void>((_resolve, reject) => { rejectFirst = reject })
  let attempts = 0
  const applied: number[] = []
  const resize = createTerminalRuntimeResizer(async (_id, _cols, rows) => {
    attempts += 1
    if (attempts === 1) await first
    applied.push(rows)
  })
  const failed = assert.rejects(resize('pane', 100, 30), /temporarily unavailable/)
  const queued = resize('pane', 120, 40)
  rejectFirst(new Error('temporarily unavailable'))
  await Promise.all([failed, queued])
  await resize('pane', 100, 30)
  assert.deepEqual(applied, [40, 30])
})

test('coalesced callers observe failure and can retry their final size', async () => {
  let release!: () => void
  const first = new Promise<void>(resolve => { release = resolve })
  let attempts = 0
  const resize = createTerminalRuntimeResizer(async () => {
    attempts += 1
    if (attempts === 1) await first
    if (attempts === 2) throw new Error('resize failed')
  })
  const initial = resize('pane', 100, 30)
  const intermediate = assert.rejects(resize('pane', 120, 40), /resize failed/)
  const final = assert.rejects(resize('pane', 140, 50), /resize failed/)
  release()
  await Promise.all([initial, intermediate, final])
  await resize('pane', 140, 50)
  assert.equal(attempts, 3)
})

test('a focused macOS terminal stays on the DOM renderer', () => {
  assert.equal(shouldUseWebglRenderer('darwin', true), false)
})

test('focused Windows and Linux terminals retain the WebGL renderer', () => {
  assert.equal(shouldUseWebglRenderer('win32', true), true)
  assert.equal(shouldUseWebglRenderer('linux', true), true)
})

test('unfocused terminals use the DOM renderer on every platform', () => {
  assert.equal(shouldUseWebglRenderer('darwin', false), false)
  assert.equal(shouldUseWebglRenderer('win32', false), false)
  assert.equal(shouldUseWebglRenderer('linux', false), false)
})

test('emoji widths agree with modern TUIs and incremental erases leave no ASCII residue', async () => {
  const term = new Terminal({ cols: 90, rows: 40, allowProposedApi: true })
  configureTerminalUnicode(term)
  const write = (data: string): Promise<void> => new Promise(resolve => term.write(data, resolve))
  try {
    for (const emoji of ['🌐', '🖥️', '💾', '🎮', '🧠', '🇨🇳', '👩‍💻', '👍🏽']) {
      await write('\x1b[2J\x1b[H' + emoji + 'm')
      assert.equal(term.buffer.active.cursorX, 3, emoji)
      assert.equal(term.buffer.active.getLine(0)?.getCell(2)?.getChars(), 'm')
      // A TUI uses its two-cell emoji layout to erase the following glyph.
      await write('\x1b[1;3H\x1b[K')
      assert.equal(term.buffer.active.getLine(0)?.translateToString(true), emoji)
    }
    await write('\x1b[2J\x1b[H中文 ABC')
    assert.equal(term.buffer.active.cursorX, 8)
    assert.equal(term.buffer.active.getLine(0)?.translateToString(true), '中文 ABC')
  } finally {
    term.dispose()
  }
})

test('split graphemes and ANSI survive a terminal snapshot restore with the same cell widths', async () => {
  for (const text of ['🌐🖥️m', 'e\u0301中文', '👩‍💻🇨🇳👍🏽', '123456789🌐m', '\x1b[31m🎮\x1b[0m done']) {
    const source = new Terminal({ cols: 10, rows: 4, allowProposedApi: true })
    const restored = new Terminal({ cols: 10, rows: 4, allowProposedApi: true })
    configureTerminalUnicode(source)
    configureTerminalUnicode(restored)
    const serialize = new SerializeAddon()
    source.loadAddon(serialize)
    try {
      for (const part of text) await new Promise<void>(resolve => source.write(part, resolve))
      await new Promise<void>(resolve => restored.write(serialize.serialize(), resolve))
      assert.equal(restored.buffer.active.cursorX, source.buffer.active.cursorX, text)
      assert.equal(restored.buffer.active.cursorY, source.buffer.active.cursorY, text)
      for (let y = 0; y < 4; y++) for (let x = 0; x < 10; x++) {
        const before = source.buffer.active.getLine(y)?.getCell(x)
        const after = restored.buffer.active.getLine(y)?.getCell(x)
        assert.equal(after?.getChars(), before?.getChars(), `${text} cell ${x},${y}`)
        assert.equal(after?.getWidth(), before?.getWidth(), `${text} width ${x},${y}`)
      }
    } finally {
      source.dispose()
      restored.dispose()
    }
  }
})
