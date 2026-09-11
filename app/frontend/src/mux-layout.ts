import type { MuxPane, MuxSplitNode } from './types'

export type LayoutPreset = 'horizontal' | 'vertical' | 'twoColumns'

export function layoutFromPanes(layout: MuxSplitNode | undefined, panes: Pick<MuxPane, 'id'>[]): MuxSplitNode | undefined {
  const paneIds = new Set(panes.map((pane) => pane.id))
  if (!paneIds.size) return undefined
  const placed = new Set<string>()
  const prune = (node: MuxSplitNode | undefined): MuxSplitNode | undefined => {
    if (!node) return undefined
    if (node.type === 'pane') {
      if (!paneIds.has(node.paneId) || placed.has(node.paneId)) return undefined
      placed.add(node.paneId)
      return node
    }
    const first = prune(node.first)
    const second = prune(node.second)
    if (!first) return second
    if (!second) return first
    return { ...node, first, second }
  }
  const normalized = prune(layout)
  // Preserve complete user layouts; missing panes need the same default as new panes.
  return placed.size === paneIds.size ? normalized : layoutForPreset([...paneIds], 'twoColumns')
}

export function paneIdsInLayout(layout: MuxSplitNode): string[] {
  return layout.type === 'pane' ? [layout.paneId] : [...paneIdsInLayout(layout.first), ...paneIdsInLayout(layout.second)]
}

// A visible split path skips branches containing only minimized panes.
// Update its original node so resizing never drops those panes from saved layout.
export function resizeVisibleSplit(layout: MuxSplitNode, visiblePaneIds: Set<string>, path: string, ratio: number): MuxSplitNode {
  if (layout.type === 'pane') return layout
  const firstVisible = paneIdsInLayout(layout.first).some((id) => visiblePaneIds.has(id))
  const secondVisible = paneIdsInLayout(layout.second).some((id) => visiblePaneIds.has(id))
  if (!firstVisible && !secondVisible) return layout
  if (!firstVisible) return { ...layout, second: resizeVisibleSplit(layout.second, visiblePaneIds, path, ratio) }
  if (!secondVisible) return { ...layout, first: resizeVisibleSplit(layout.first, visiblePaneIds, path, ratio) }
  if (!path) return { ...layout, ratio }
  return path[0] === '0'
    ? { ...layout, first: resizeVisibleSplit(layout.first, visiblePaneIds, path.slice(1), ratio) }
    : { ...layout, second: resizeVisibleSplit(layout.second, visiblePaneIds, path.slice(1), ratio) }
}

function balancedLayout(nodes: MuxSplitNode[], direction: 'horizontal' | 'vertical'): MuxSplitNode {
  if (nodes.length === 1) return nodes[0]!
  const midpoint = Math.ceil(nodes.length / 2)
  return {
    type: 'split',
    direction,
    ratio: midpoint / nodes.length,
    first: balancedLayout(nodes.slice(0, midpoint), direction),
    second: balancedLayout(nodes.slice(midpoint), direction)
  }
}

export function layoutForPreset(paneIds: string[], preset: LayoutPreset): MuxSplitNode {
  const leaves = paneIds.map((paneId): MuxSplitNode => ({ type: 'pane', paneId }))
  if (preset !== 'twoColumns') return balancedLayout(leaves, preset)
  const rows: MuxSplitNode[] = []
  for (let index = 0; index < leaves.length; index += 2) {
    const row = leaves.slice(index, index + 2)
    rows.push(row.length === 1 ? row[0]! : balancedLayout(row, 'horizontal'))
  }
  return balancedLayout(rows, 'vertical')
}
