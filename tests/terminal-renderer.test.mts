import assert from 'node:assert/strict'
import test from 'node:test'
import { createRequire } from 'node:module'
import { shouldUseWebglRenderer } from '../app/frontend/src/terminal-renderer.ts'
import { configureTerminalUnicode } from '../app/frontend/src/terminal-unicode.ts'

const { Terminal } = createRequire(import.meta.url)('@xterm/xterm') as typeof import('@xterm/xterm')
const { SerializeAddon } = createRequire(import.meta.url)('@xterm/addon-serialize') as typeof import('@xterm/addon-serialize')

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
