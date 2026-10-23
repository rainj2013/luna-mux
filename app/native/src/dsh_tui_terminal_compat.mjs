// Node preload for dsh-TUI in Luna Mux's Windows xterm.js host. ConPTY turns
// VT arrows into individual synthetic character records after DECSET 9001;
// the published 0.16.1 build lacks the newer environment opt-out.
// Rewrite only that mode's enable to disable, preserving byte order, write
// boundaries, callbacks and backpressure. No terminal identity spoofing.
import { Buffer } from 'node:buffer'

export function installTerminalCompatibility(stream) {
  if (!stream.isTTY) return () => {}
  const enable = Buffer.from('\x1b[?9001h')
  const originalWrite = stream.write
  let matched = 0
  function write(chunk, encoding, callback) {
    const bytes = typeof chunk === 'string'
      ? Buffer.from(chunk, typeof encoding === 'string' ? encoding : 'utf8')
      : Buffer.from(chunk.buffer, chunk.byteOffset, chunk.byteLength)
    let replacement
    for (let index = 0; index < bytes.length; index++) {
      const byte = bytes[index]
      matched = byte === enable[matched] ? matched + 1 : byte === enable[0] ? 1 : 0
      if (matched === enable.length) {
        replacement ??= Buffer.from(bytes)
        replacement[index] = 0x6c // DECRST 9001: final 'l', same byte length.
        matched = 0
      }
    }
    return originalWrite.call(this, replacement ?? chunk, encoding, callback)
  }
  stream.write = write
  return () => { if (stream.write === write) stream.write = originalWrite }
}

if (process.platform === 'win32') installTerminalCompatibility(process.stdout)
