import assert from 'node:assert/strict'
import test from 'node:test'
import { agentAdapterId, agentImagePasteInput, codexControlMInput, codexPasteMode, codexPastePayload, handleCodexPasteEvent, routeTerminalPaste } from '../app/frontend/src/terminal-input.ts'

test('image paste uses each Agent adapter\'s native shortcut', () => {
  assert.equal(agentImagePasteInput('codex', 'win32'), '\x16')
  assert.equal(agentImagePasteInput('grok-build', 'win32'), '\x16')
  assert.equal(agentImagePasteInput('claude-code', 'win32'), '\x1bv')
  assert.equal(agentImagePasteInput('claude-code', 'darwin'), '\x16')
  assert.equal(agentImagePasteInput(undefined, 'linux'), '\x16')
})

test('Codex multiline paste normalizes LF, CRLF, and CR before bracketing', () => {
  assert.equal(
    codexPastePayload('first\nsecond\r\nthird\rfourth', 'bracketed'),
    '\x1b[200~first\rsecond\rthird\rfourth\x1b[201~'
  )
})

test('Codex newline-only and trailing-newline pastes remain paste events', () => {
  assert.equal(codexPastePayload('\n', 'bracketed'), '\x1b[200~\r\x1b[201~')
  assert.equal(codexPastePayload('command\r\n', 'bracketed'), '\x1b[200~command\r\x1b[201~')
})

test('single-line text keeps using xterm paste handling', () => {
  assert.equal(codexPastePayload('single line', 'bracketed'), undefined)
  assert.equal(codexPastePayload('single line', 'windows'), undefined)
})

test('only native Windows Codex uses Alt+Enter for pasted newlines', () => {
  assert.equal(codexPasteMode('codex', 'win32', 'local:powershell'), 'windows')
  assert.equal(codexPasteMode('codex', 'win32', 'local:powershell5'), 'windows')
  assert.equal(codexPasteMode('codex', 'win32', 'local:wsl:Ubuntu'), 'bracketed')
  assert.equal(codexPasteMode('codex', 'win32', 'ssh-bookmark:work'), 'bracketed')
  assert.equal(codexPasteMode('codex', 'darwin', 'local:macos-shell'), 'bracketed')
  assert.equal(codexPasteMode('claude-code', 'win32', 'local:powershell'), undefined)
  assert.equal(codexPastePayload('一\r\n\t二\r三\n四\u0085五\u2028六\u2029', 'windows'), '一\x1b\r    二\x1b\r三\x1b\r四\x1b\r五\x1b\r六\x1b\r')
})

test('pasted control characters cannot end the bracketed paste or become key presses', () => {
  assert.equal(codexPastePayload('first\x1b[201~\x04\rsecond\x7f', 'bracketed'), '\x1b[200~first[201~\rsecond\x1b[201~')
  assert.equal(codexPastePayload('first\x1b[201~\x04\rsecond\x7f', 'windows'), 'first[201~\x1b\rsecond')
  assert.equal(codexPastePayload('first\tsecond', 'windows'), 'first    second')
})

test('Codex Ctrl+M inserts a newline without submitting', () => {
  assert.equal(codexControlMInput(true), '\x1b\r')
  assert.equal(codexControlMInput(false), undefined)
})

test('managed Codex is known from its launch profile before the first hook event', () => {
  assert.equal(agentAdapterId(undefined, 'codex.default'), 'codex')
  assert.equal(agentAdapterId('claude-code', 'codex.default'), 'claude-code')
  assert.equal(agentAdapterId(undefined, 'claude-code.default'), 'claude-code')
  assert.equal(agentAdapterId(undefined, 'grok-build.default'), 'grok-build')
  assert.equal(agentAdapterId(undefined, 'dsh-tui.default'), 'dsh-tui')
  assert.equal(agentAdapterId(undefined, 'dsh-tui.auto'), 'dsh-tui')
  assert.equal(agentAdapterId(undefined, 'claude-code.auto'), 'claude-code')
})

test('clipboard shortcut routes Codex multiline text to one direct write', () => {
  const directWrites: string[] = []
  const terminalPastes: string[] = []
  assert.equal(routeTerminalPaste('first\nsecond', 'windows', (data) => directWrites.push(data), (data) => terminalPastes.push(data)), 'direct')
  assert.deepEqual(directWrites, ['first\x1b\rsecond'])
  assert.deepEqual(terminalPastes, [])
})

test('native Codex multiline paste blocks xterm handling and writes exactly once', () => {
  let prevented = 0
  let stopped = 0
  const directWrites: string[] = []
  assert.equal(handleCodexPasteEvent('first\r\nsecond', 'windows', () => { prevented += 1 }, () => { stopped += 1 }, (data) => directWrites.push(data)), true)
  assert.equal(prevented, 1)
  assert.equal(stopped, 1)
  assert.deepEqual(directWrites, ['first\x1b\rsecond'])
})

test('native single-line and non-Codex paste remain available to xterm', () => {
  let sideEffects = 0
  const sideEffect = (): void => { sideEffects += 1 }
  assert.equal(handleCodexPasteEvent('single line', 'bracketed', sideEffect, sideEffect, sideEffect), false)
  assert.equal(handleCodexPasteEvent('first\nsecond', undefined, sideEffect, sideEffect, sideEffect), false)
  assert.equal(sideEffects, 0)
})

test('clipboard text consisting only of controls is consumed without writing to Codex', () => {
  const unexpected = (): never => assert.fail('clipboard controls must not reach the terminal')
  const noop = (): void => {}
  assert.equal(routeTerminalPaste('\x04\x1b', 'windows', unexpected, unexpected), 'direct')
  assert.equal(handleCodexPasteEvent('\x04\x1b', 'bracketed', noop, noop, unexpected), true)
})

test('single-line Codex and non-Codex multiline paste each use xterm once', () => {
  for (const [text, mode] of [['single line', 'bracketed'], ['first\nsecond', undefined]] as const) {
    const directWrites: string[] = []
    const terminalPastes: string[] = []
    assert.equal(routeTerminalPaste(text, mode, (data) => directWrites.push(data), (data) => terminalPastes.push(data)), 'terminal')
    assert.deepEqual(directWrites, [])
    assert.deepEqual(terminalPastes, [text])
  }
})
