import assert from 'node:assert/strict'
import { test } from 'node:test'
import xterm from '@xterm/xterm'
import { SerializeAddon } from '@xterm/addon-serialize'
import { serializeTerminalSnapshot } from '../app/frontend/src/terminal-snapshot.ts'

type TestTerminal = InstanceType<typeof xterm.Terminal> & {
  _core: {
    coreMouseService: {
      activeEncoding: string
      triggerMouseEvent(event: {
        col: number; row: number; x: number; y: number
        button: number; action: number; ctrl: boolean; alt: boolean; shift: boolean
      }): boolean
    }
  }
}

function write(term: TestTerminal, data: string): Promise<void> {
  return new Promise((resolve) => term.write(data, resolve))
}

for (const [mode, encoding, prefix] of [
  ['\u001b[?1006h', 'SGR', '\u001b[<'],
  ['\u001b[?1016h', 'SGR_PIXELS', '\u001b[<']
] as const) {
  test(`restores ${encoding} wheel reports after terminal remount`, async () => {
    const original = new xterm.Terminal() as TestTerminal
    const serializer = new SerializeAddon()
    original.loadAddon(serializer)
    await write(original, `\u001b[?1049h\u001b[?1002h${mode}`)
    assert.equal(original._core.coreMouseService.activeEncoding, encoding)

    const restored = new xterm.Terminal() as TestTerminal
    await write(restored, serializeTerminalSnapshot(original, serializer))
    assert.equal(restored.buffer.active.type, 'alternate')
    assert.equal(restored.modes.mouseTrackingMode, 'drag')
    assert.equal(restored._core.coreMouseService.activeEncoding, encoding)

    let report = ''
    const listener = restored.onData((data) => { report += data })
    assert.equal(restored._core.coreMouseService.triggerMouseEvent({
      col: 1, row: 1, x: 10, y: 10, button: 4, action: 0,
      ctrl: false, alt: false, shift: false
    }), true)
    assert.ok(report.startsWith(prefix), `unexpected wheel report ${JSON.stringify(report)}`)
    listener.dispose()
    restored.dispose()
    original.dispose()
  })
}

test('leaves default mouse encoding unchanged', async () => {
  const original = new xterm.Terminal() as TestTerminal
  const serializer = new SerializeAddon()
  original.loadAddon(serializer)
  await write(original, '\u001b[?1002h')
  const restored = new xterm.Terminal() as TestTerminal
  await write(restored, serializeTerminalSnapshot(original, serializer))
  assert.equal(restored._core.coreMouseService.activeEncoding, 'DEFAULT')
  restored.dispose()
  original.dispose()
})
