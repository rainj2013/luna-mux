import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { Writable, PassThrough } from 'node:stream'
import { resolve, join, dirname } from 'node:path'
import { pathToFileURL } from 'node:url'
import { configureTerminalUnicode } from '../app/frontend/src/terminal-unicode.ts'

// Real published renderer, synthetic history, no profile or model/API access.
// node --experimental-strip-types tests/dsh-tui-render-smoke.mjs tmp/dsh-validation
assert(process.argv[2], 'Provide an isolated upstream installation directory')
const requireUpstream = createRequire(join(resolve(process.argv[2]), 'package.json'))
const React = requireUpstream('react')
const { Terminal } = createRequire(import.meta.url)('@xterm/xterm')
const upstreamRoot = dirname(requireUpstream.resolve('@deepseek-harness-tui/dsh-tui/package.json'))
const upstream = path => import(pathToFileURL(join(upstreamRoot, 'lib/types', path)).href)
const { render, AlternateScreen, Box, Text, ScrollBox } = await upstream('ui.js')
const { default: instances } = await upstream('ink/instances.js')
const { cellAt } = await upstream('ink/screen.js')
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms))
const cols = 90, rows = 40
const term = new Terminal({ cols, rows, allowProposedApi: true })
configureTerminalUnicode(term)
class Output extends Writable {
  isTTY = true; columns = cols; rows = rows
  _write(chunk, encoding, callback) { term.write(chunk, callback) }
}
class Input extends PassThrough {
  isTTY = true
  setRawMode() { return this }
  ref() { return this }
  unref() { return this }
}
const stdout = new Output(), stdin = new Input()
term.onData(data => stdin.write(data))
const scroll = React.createRef()
const h = React.createElement
const history = Array.from({ length: 180 }, (_, i) => h(Text, {
  key: i, color: i % 2 ? 'cyan' : 'white',
}, `ROW_${String(i).padStart(3, '0')} ` +
  (i % 3 ? '中文 | Windows 11 | memory 32GB ' : '🌐 🖥️ 💾 🎮 ') +
  'abcdef'.repeat(i % 9) + ' END'))
const tree = h(AlternateScreen, null, h(Box, { flexDirection: 'column', height: rows, width: cols },
  h(Text, null, 'HEADER'),
  h(ScrollBox, { ref: scroll, height: rows - 4, flexShrink: 0, flexDirection: 'column' }, history),
  h(Text, null, 'FOOTER')))
const stderr = new Writable({ write(chunk, encoding, callback) { callback() } })
const app = await render(tree, { stdout, stdin, stderr, patchConsole: false, exitOnCtrlC: false, terminalImages: false })
let checks = 0
const compare = () => {
  const screen = instances.get(stdout).frontFrame.screen
  for (let y = 0; y < rows; y++) for (let x = 0; x < cols; x++) {
    const cell = cellAt(screen, x, y)
    if (cell?.width >= 2) continue // upstream's explicit wide-cell spacers
    assert.equal(term.buffer.active.getLine(y)?.getCell(x)?.getChars() || ' ', cell?.char || ' ', `frame ${checks}, cell ${x},${y}`)
  }
  checks++
}
try {
  await sleep(400)
  compare()
  // Alternates direction, wraps long rows, and repaints text of unequal width.
  for (let index = 0; index < 30; index++) {
    scroll.current.scrollTo((index * 7) % 130)
    await sleep(60)
    compare()
  }
  console.log(`Published dst renderer: ${checks} frames match xterm cell-for-cell`)
} finally {
  await app.detachForShutdown()
  app.cleanup()
  term.dispose()
}
