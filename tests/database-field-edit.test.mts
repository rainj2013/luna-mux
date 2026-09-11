import assert from 'node:assert/strict'
import test from 'node:test'
import { pasteAtCaret } from '../app/frontend/src/database-field-edit.ts'

test('text is inserted at the caret', () => {
  assert.deepEqual(pasteAtCaret('select 1', 8, 8, ' from t'), { value: 'select 1 from t', caret: 15 })
  assert.deepEqual(pasteAtCaret('', 0, 0, 'abc'), { value: 'abc', caret: 3 })
})

test('a selection is replaced', () => {
  assert.deepEqual(pasteAtCaret('select 1', 0, 6, 'update'), { value: 'update 1', caret: 6 })
  // Selecting backwards reports start after end.
  assert.deepEqual(pasteAtCaret('select 1', 6, 0, 'drop  '), { value: 'drop   1', caret: 6 })
})

test('offsets outside the value are clamped', () => {
  assert.deepEqual(pasteAtCaret('abc', 99, 99, 'd'), { value: 'abcd', caret: 4 })
  assert.deepEqual(pasteAtCaret('abc', -5, -1, 'x'), { value: 'xabc', caret: 1 })
  assert.deepEqual(pasteAtCaret('abc', Number.NaN, Number.NaN, 'x'), { value: 'abcx', caret: 4 })
})

test('an empty paste keeps the value and moves the caret', () => {
  assert.deepEqual(pasteAtCaret('abc', 1, 1, ''), { value: 'abc', caret: 1 })
})
