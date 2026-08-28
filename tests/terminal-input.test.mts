import assert from 'node:assert/strict'
import test from 'node:test'
import { agentAdapterId, codexMultilinePastePayload, handleCodexMultilinePasteEvent, routeTerminalPaste } from '../app/frontend/src/terminal-input.ts'

test('Codex multiline paste normalizes LF, CRLF, and CR before bracketing', () => {
  assert.equal(
    codexMultilinePastePayload('first\nsecond\r\nthird\rfourth'),
    '\x1b[200~first\rsecond\rthird\rfourth\x1b[201~'
  )
})

test('Codex newline-only and trailing-newline pastes remain paste events', () => {
  assert.equal(codexMultilinePastePayload('\n'), '\x1b[200~\r\x1b[201~')
  assert.equal(codexMultilinePastePayload('command\r\n'), '\x1b[200~command\r\x1b[201~')
})

test('single-line text keeps using xterm paste handling', () => {
  assert.equal(codexMultilinePastePayload('single line'), undefined)
})

test('managed Codex is known from its launch profile before the first hook event', () => {
  assert.equal(agentAdapterId(undefined, 'codex.default'), 'codex')
  assert.equal(agentAdapterId('claude-code', 'codex.default'), 'claude-code')
  assert.equal(agentAdapterId(undefined, 'claude-code.default'), undefined)
})

test('clipboard shortcut routes Codex multiline text to one direct write', () => {
  const directWrites: string[] = []
  const terminalPastes: string[] = []
  assert.equal(routeTerminalPaste('first\nsecond', true, (data) => directWrites.push(data), (data) => terminalPastes.push(data)), 'direct')
  assert.deepEqual(directWrites, ['\x1b[200~first\rsecond\x1b[201~'])
  assert.deepEqual(terminalPastes, [])
})

test('native Codex multiline paste blocks xterm handling and writes exactly once', () => {
  let prevented = 0
  let stopped = 0
  const directWrites: string[] = []
  assert.equal(handleCodexMultilinePasteEvent('first\r\nsecond', true, () => { prevented += 1 }, () => { stopped += 1 }, (data) => directWrites.push(data)), true)
  assert.equal(prevented, 1)
  assert.equal(stopped, 1)
  assert.deepEqual(directWrites, ['\x1b[200~first\rsecond\x1b[201~'])
})

test('native single-line and non-Codex paste remain available to xterm', () => {
  let sideEffects = 0
  const sideEffect = (): void => { sideEffects += 1 }
  assert.equal(handleCodexMultilinePasteEvent('single line', true, sideEffect, sideEffect, sideEffect), false)
  assert.equal(handleCodexMultilinePasteEvent('first\nsecond', false, sideEffect, sideEffect, sideEffect), false)
  assert.equal(sideEffects, 0)
})

test('single-line Codex and non-Codex multiline paste each use xterm once', () => {
  for (const [text, codexTui] of [['single line', true], ['first\nsecond', false]] as const) {
    const directWrites: string[] = []
    const terminalPastes: string[] = []
    assert.equal(routeTerminalPaste(text, codexTui, (data) => directWrites.push(data), (data) => terminalPastes.push(data)), 'terminal')
    assert.deepEqual(directWrites, [])
    assert.deepEqual(terminalPastes, [text])
  }
})
