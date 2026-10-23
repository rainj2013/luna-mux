import assert from 'node:assert/strict'
import test from 'node:test'
import { Writable } from 'node:stream'
import { once } from 'node:events'
import { installTerminalCompatibility } from '../app/native/src/dsh_tui_terminal_compat.mjs'

function output(isTTY = true) {
  const chunks = []
  const callbacks = []
  const stream = { isTTY, write(chunk, encoding, callback) {
    assert.equal(this, stream)
    chunks.push(typeof chunk === 'string' ? Buffer.from(chunk, typeof encoding === 'string' ? encoding : 'utf8') : Buffer.from(chunk))
    callbacks.push(typeof encoding === 'function' ? encoding : callback)
    return false
  } }
  return { stream, chunks, callbacks, text: () => Buffer.concat(chunks).toString('utf8') }
}

test('disables Win32 record input across every write boundary without buffering', () => {
  const enable = '\x1b[?9001h'
  for (let split = 0; split <= enable.length; split++) {
    const h = output()
    installTerminalCompatibility(h.stream)
    const first = Buffer.from(enable.slice(0, split))
    const second = new Uint8Array(Buffer.from(enable.slice(split)))
    const callback = () => {}
    assert.equal(h.stream.write(first, callback), false)
    assert.equal(h.chunks[0].length, first.length)
    assert.equal(h.stream.write(second, callback), false)
    assert.equal(h.text(), '\x1b[?9001l')
    assert.deepEqual(h.callbacks, [callback, callback])
    // Callers retain ownership of their buffers.
    assert.equal(Buffer.concat([first, second]).toString(), enable)
  }
})

test('preserves real Writable callbacks, drain and backpressure', async () => {
  const chunks = []
  const stream = new Writable({ highWaterMark: 1, write(chunk, encoding, callback) {
    chunks.push(Buffer.from(chunk))
    setImmediate(callback)
  } })
  stream.isTTY = true
  installTerminalCompatibility(stream)
  let callbacks = 0
  for (const chunk of ['\x1b[?90', '01h', Buffer.from('\x1b[?9001h')]) {
    const drained = once(stream, 'drain')
    assert.equal(stream.write(chunk, () => { callbacks++ }), false)
    await drained
  }
  assert.equal(callbacks, 3)
  assert.equal(Buffer.concat(chunks).toString(), '\x1b[?9001l\x1b[?9001l')
  stream.end()
})

test('preserves text, other modes, encodings, original receiver and pipe output', () => {
  const h = output()
  const original = h.stream.write
  const restore = installTerminalCompatibility(h.stream)
  const text = '中文\x1b[?1h\x1b[?2004h\x1b[?9001l\x1b[?9001H'
  h.stream.write(text, 'utf8')
  h.stream.write('\x1b[?9001h\x1b[?9001h')
  assert.equal(h.text(), text + '\x1b[?9001l\x1b[?9001l')
  restore()
  assert.equal(h.stream.write, original)
  const pipe = output(false)
  const originalPipeWrite = pipe.stream.write
  installTerminalCompatibility(pipe.stream)
  assert.equal(pipe.stream.write, originalPipeWrite)
  pipe.stream.write('\x1b[?9001h')
  assert.equal(pipe.text(), '\x1b[?9001h')
})
