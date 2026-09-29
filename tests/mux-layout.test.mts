import assert from 'node:assert/strict'
import test from 'node:test'
import { layoutFromPanes, layoutForPreset, layoutForVisiblePreset, paneIdsInLayout, resizeVisibleSplit } from '../app/frontend/src/mux-layout.ts'
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

test('layout presets rearrange only visible panes and retain a hidden pane beside its neighbor', () => {
  const fourPanes = [...panes, { id: 'fourth' }]
  const original: MuxSplitNode = {
    type: 'split', direction: 'vertical', ratio: 0.6,
    first: { type: 'split', direction: 'horizontal', ratio: 0.3, first: leaf('ssh'), second: leaf('powershell') },
    second: { type: 'split', direction: 'horizontal', ratio: 0.4, first: leaf('db'), second: leaf('fourth') }
  }
  const visible = new Set(['ssh', 'db', 'fourth'])
  for (const preset of ['horizontal', 'vertical', 'twoColumns'] as const) {
    const arranged = layoutForVisiblePreset(original, visible, preset)
    assert.deepEqual(paneIdsInLayout(arranged).sort(), fourPanes.map((pane) => pane.id).sort())
    assert.deepEqual(layoutFromPanes(arranged, fourPanes.filter((pane) => visible.has(pane.id))), layoutForPreset(['ssh', 'db', 'fourth'], preset))
    assert.deepEqual(layoutFromPanes(arranged, panes.slice(0, 2)), original.type === 'split' ? original.first : undefined)
  }
})

test('multiple hidden panes retain their own grouping when visible panes are rearranged', () => {
  const hidden: MuxSplitNode = { type: 'split', direction: 'vertical', ratio: 0.35, first: leaf('hidden-a'), second: leaf('hidden-b') }
  const original: MuxSplitNode = { type: 'split', direction: 'horizontal', ratio: 0.25, first: hidden, second: grid }
  const visible = new Set(['ssh', 'powershell', 'db'])
  const arranged = layoutForVisiblePreset(original, visible, 'vertical')
  assert.deepEqual(layoutFromPanes(arranged, panes), layoutForPreset(['ssh', 'powershell', 'db'], 'vertical'))
  assert.deepEqual(paneIdsInLayout(arranged), ['hidden-a', 'hidden-b', 'ssh', 'powershell', 'db'])
  assert.deepEqual(layoutFromPanes(arranged, [{ id: 'hidden-a' }, { id: 'hidden-b' }, { id: 'ssh' }]), { type: 'split', direction: 'horizontal', ratio: 0.25, first: hidden, second: leaf('ssh') })
})

test('all-visible and no-visible preset requests keep the expected complete layout', () => {
  assert.deepEqual(layoutForVisiblePreset(grid, new Set(['ssh', 'powershell', 'db']), 'horizontal'), layoutForPreset(['ssh', 'powershell', 'db'], 'horizontal'))
  assert.deepEqual(layoutForVisiblePreset(grid, new Set(), 'horizontal'), grid)
  assert.deepEqual(layoutForVisiblePreset(grid, new Set(['db']), 'horizontal'), grid)
})
