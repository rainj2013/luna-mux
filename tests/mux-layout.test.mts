import assert from 'node:assert/strict'
import test from 'node:test'
import { layoutFromPanes, layoutForPreset, paneIdsInLayout, resizeVisibleSplit } from '../app/frontend/src/mux-layout.ts'
import type { MuxSplitNode } from '../app/frontend/src/types.ts'

const panes = [{ id: 'ssh', kind: 'terminal' }, { id: 'powershell', kind: 'terminal' }, { id: 'db', kind: 'database' }]
const leaf = (paneId: string): MuxSplitNode => ({ type: 'pane', paneId })
const grid: MuxSplitNode = {
  type: 'split', direction: 'vertical', ratio: 0.5,
  first: { type: 'split', direction: 'horizontal', ratio: 0.5, first: leaf('ssh'), second: leaf('powershell') },
  second: leaf('db')
}

test('mixed historical panes without a layout open as two columns', () => {
  assert.deepEqual(layoutFromPanes(undefined, panes), grid)
})

test('DB omitted from an old layout joins a second row rather than a third column', () => {
  assert.deepEqual(layoutFromPanes(grid.type === 'split' ? grid.first : undefined, panes), grid)
})

test('a complete custom layout retains its directions and resized ratios', () => {
  const custom: MuxSplitNode = { type: 'split', direction: 'horizontal', ratio: 0.7, first: leaf('db'), second: { type: 'split', direction: 'vertical', ratio: 0.3, first: leaf('powershell'), second: leaf('ssh') } }
  assert.deepEqual(layoutFromPanes(custom, panes), custom)
})

test('minimizing and restoring a pane preserves its saved place', () => {
  assert.deepEqual(layoutFromPanes(grid, panes.slice(0, 2)), grid.type === 'split' ? grid.first : undefined)
  assert.deepEqual(layoutFromPanes(grid, panes), grid)
})

test('resize while DB is minimized retains its place and the adjusted 70/30 ratio on restore', () => {
  const resized = resizeVisibleSplit(grid, new Set(['ssh', 'powershell']), '', 0.7)
  assert.deepEqual(paneIdsInLayout(resized), ['ssh', 'powershell', 'db'])
  assert.equal(resized.type === 'split' && resized.first.type === 'split' && resized.first.ratio, 0.7)
  assert.deepEqual(layoutFromPanes(resized, panes), resized)
})

test('visible resize paths skip hidden branches without modifying their ratios', () => {
  const hiddenBranch: MuxSplitNode = { type: 'split', direction: 'horizontal', ratio: 0.2, first: leaf('hidden'), second: grid }
  const resized = resizeVisibleSplit(hiddenBranch, new Set(['ssh', 'powershell', 'db']), '0', 0.7)
  assert.equal(resized.type === 'split' && resized.ratio, 0.2)
  assert.equal(resized.type === 'split' && resized.second.type === 'split' && resized.second.first.type === 'split' && resized.second.first.ratio, 0.7)
  assert.deepEqual(paneIdsInLayout(resized), ['hidden', 'ssh', 'powershell', 'db'])
})

test('empty, single, stale, and duplicate pane references recover without losing panes', () => {
  assert.equal(layoutFromPanes(grid, []), undefined)
  assert.deepEqual(layoutFromPanes(undefined, [panes[2]!]), leaf('db'))
  const stale: MuxSplitNode = { type: 'split', direction: 'horizontal', ratio: 0.5, first: leaf('deleted'), second: leaf('ssh') }
  assert.deepEqual(layoutFromPanes(stale, panes), grid)
  const duplicate: MuxSplitNode = { type: 'split', direction: 'horizontal', ratio: 0.5, first: leaf('ssh'), second: leaf('ssh') }
  assert.deepEqual(paneIdsInLayout(layoutFromPanes(duplicate, panes)!), ['ssh', 'powershell', 'db'])
})

test('four panes use equal rows and keep sidebar order', () => {
  const layout = layoutFromPanes(undefined, [...panes, { id: 'fourth' }])!
  assert.equal(layout.type, 'split')
  if (layout.type !== 'split') return
  assert.equal(layout.direction, 'vertical')
  assert.equal(layout.ratio, 0.5)
  assert.deepEqual(paneIdsInLayout(layout.first), ['ssh', 'powershell'])
  assert.deepEqual(paneIdsInLayout(layout.second), ['db', 'fourth'])
  assert.deepEqual(layoutForPreset(paneIdsInLayout(layout), 'twoColumns'), layout)
})
