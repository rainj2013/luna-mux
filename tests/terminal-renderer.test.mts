import assert from 'node:assert/strict'
import test from 'node:test'
import { shouldUseWebglRenderer } from '../app/frontend/src/terminal-renderer.ts'

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
