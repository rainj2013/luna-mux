import { memo, useEffect, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import type { ReactNode } from 'react'
import { useVirtualizer } from '@tanstack/react-virtual'
import type { DatabaseQueryResult } from '../types'
import { useI18n } from '../i18n'

export interface DatabaseMenuItem {
  label: string
  icon: ReactNode
  disabled?: boolean
  danger?: boolean
  action(): void
}

// Rows picked in the result grid, plus the cell under the pointer, handed to the
// pane so it can build its own context menu.
export interface DatabaseResultSelection {
  rows: unknown[][]
  columns: string[]
  cell: string
}

export function DatabaseMenu({ x, y, items, onClose, anchor }: { x: number; y: number; items: DatabaseMenuItem[]; onClose(): void; anchor: HTMLElement }): React.JSX.Element {
  const menu = useRef<HTMLDivElement>(null)
  const restoreFocus = useRef(true)
  useEffect(() => {
    const node = menu.current
    node?.querySelector<HTMLButtonElement>('button:enabled')?.focus()
    const outside = (event: PointerEvent): void => { if (!node?.contains(event.target as Node)) { restoreFocus.current = false; onClose() } }
    const escape = (event: KeyboardEvent): void => { if (event.key === 'Escape') { event.preventDefault(); event.stopPropagation(); onClose() } }
    window.addEventListener('pointerdown', outside)
    window.addEventListener('keydown', escape, true)
    window.addEventListener('resize', onClose)
    return () => {
      window.removeEventListener('pointerdown', outside)
      window.removeEventListener('keydown', escape, true)
      window.removeEventListener('resize', onClose)
      if (restoreFocus.current && anchor.isConnected) anchor.focus({ preventScroll: true })
    }
  }, [])
  return createPortal(<div ref={menu} role="menu" className="sidebar-context-menu database-context-menu"
    style={{ left: Math.max(8, Math.min(x, window.innerWidth - 228)), top: Math.max(8, Math.min(y, window.innerHeight - items.length * 32 - 18)) }}
    onKeyDown={(event) => {
      if (!['ArrowDown', 'ArrowUp', 'Home', 'End', 'Tab'].includes(event.key)) return
      event.preventDefault()
      if (event.key === 'Tab') { onClose(); return }
      const buttons = Array.from(menu.current?.querySelectorAll<HTMLButtonElement>('button:enabled') ?? [])
      const current = buttons.indexOf(document.activeElement as HTMLButtonElement)
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : (current + (event.key === 'ArrowDown' ? 1 : -1) + buttons.length) % buttons.length
      buttons[next]?.focus()
    }}>
    {items.map((item) => <button key={item.label} role="menuitem" disabled={item.disabled} className={item.danger ? 'danger' : undefined}
      onClick={() => { onClose(); item.action() }}>{item.icon}{item.label}</button>)}
  </div>, anchor.closest('.app-shell') ?? document.body)
}

export const DatabaseResultGrid = memo(function DatabaseResultGrid({ result, onCopy, onRowMenu, emptyMessage, rowOffset = 0 }: { result: DatabaseQueryResult | null; onCopy(value: string): void; onRowMenu?(event: React.MouseEvent<HTMLElement>, selection: DatabaseResultSelection): void; emptyMessage?: string; rowOffset?: number }): React.JSX.Element {
  const { t } = useI18n()
  const scroll = useRef<HTMLDivElement>(null)
  const rows = result?.rows ?? []
  const [selectedRows, setSelectedRows] = useState<ReadonlySet<number>>(() => new Set())
  const anchorRow = useRef(-1)
  // Ends an in-flight drag selection; kept in a ref so unmounting also removes its listeners.
  const stopDrag = useRef<() => void>(() => {})
  // A drag that covered several rows must not let the trailing click collapse the selection.
  const suppressClick = useRef(false)
  // A new result replaces the rows, so the old indexes must not stay selected.
  useEffect(() => { setSelectedRows(new Set()); anchorRow.current = -1 }, [result])
  useEffect(() => () => stopDrag.current(), [])
  const virtualizer = useVirtualizer({ count: result?.columns.length ? rows.length : 0, getScrollElement: () => scroll.current, estimateSize: () => 29, overscan: 10 })
  const rowRange = (from: number, to: number): Set<number> => {
    const range = new Set<number>()
    for (let row = Math.min(from, to); row <= Math.max(from, to); row += 1) range.add(row)
    return range
  }
  const selectRow = (index: number, event: React.MouseEvent): void => {
    setSelectedRows((current) => {
      const next = new Set(current)
      if (event.shiftKey && anchorRow.current >= 0) {
        for (const row of rowRange(anchorRow.current, index)) next.add(row)
        return next
      }
      if (event.metaKey || event.ctrlKey) {
        if (next.has(index)) next.delete(index); else next.add(index)
        anchorRow.current = index
        return next
      }
      anchorRow.current = index
      return new Set([index])
    })
  }
  // Press and drag spans a contiguous range; the press row stays the anchor and the listeners
  // live on the window so the gesture keeps working outside the grid.
  const startRowDrag = (event: React.PointerEvent<HTMLElement>, index: number): void => {
    if (event.button !== 0 || event.shiftKey || event.metaKey || event.ctrlKey) return
    suppressClick.current = false
    anchorRow.current = index
    setSelectedRows(new Set([index]))
    // Applied synchronously so the browser never starts a text selection mid-gesture.
    const node = scroll.current
    node?.classList.add('selecting')
    let moved = false
    const move = (pointer: PointerEvent): void => {
      if (node) {
        // Dragging past an edge scrolls, so one gesture can reach rows off screen.
        const rect = node.getBoundingClientRect()
        if (pointer.clientY < rect.top + 24) node.scrollTop -= 24
        else if (pointer.clientY > rect.bottom - 24) node.scrollTop += 24
        const selection = window.getSelection()
        if (selection && !selection.isCollapsed) selection.removeAllRanges()
      }
      const value = document.elementFromPoint(pointer.clientX, pointer.clientY)?.closest<HTMLElement>('tr[data-index]')?.dataset.index
      if (value === undefined) return
      const over = Number(value)
      if (over === index && !moved) return
      moved = true
      setSelectedRows(rowRange(anchorRow.current, over))
    }
    const end = (): void => {
      stopDrag.current = () => {}
      suppressClick.current = moved
      node?.classList.remove('selecting')
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', end)
      window.removeEventListener('pointercancel', end)
    }
    stopDrag.current = end
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', end)
    window.addEventListener('pointercancel', end)
  }
  const openRowMenu = (event: React.MouseEvent<HTMLElement>, index: number, row: unknown[]): void => {
    if (!onRowMenu) return
    event.preventDefault()
    // Right-clicking outside the selection makes that row the selection; inside it
    // keeps the rows the user picked.
    const indices = selectedRows.has(index) ? [...selectedRows].sort((a, b) => a - b) : [index]
    if (!selectedRows.has(index)) { anchorRow.current = index; setSelectedRows(new Set([index])) }
    const cell = (event.target as HTMLElement).closest('td')
    const column = cell ? cell.cellIndex - 1 : -1
    onRowMenu(event, {
      rows: indices.map((selected) => rows[selected]).filter((value): value is unknown[] => Array.isArray(value)),
      columns: result?.columns ?? [],
      cell: column >= 0 ? String(row[column] ?? 'NULL') : '',
    })
  }
  if (!result) return <p className="database-result-empty">{emptyMessage ?? t('database.queryHint')}</p>
  if (!result.columns.length) return <p className="database-result-empty">{t('database.affectedRows', { count: result.affectedRows })}</p>
  const visibleRows = virtualizer.getVirtualItems().filter((virtualRow) => virtualRow.index < rows.length)
  const paddingTop = visibleRows.length ? visibleRows[0]!.start : 0
  const paddingBottom = visibleRows.length ? Math.max(0, virtualizer.getTotalSize() - visibleRows[visibleRows.length - 1]!.end) : 0
  const columnCount = result.columns.length + 1
  return <div ref={scroll} className="database-result" tabIndex={0} aria-label={t('database.results')}><table><thead><tr>
    <th className="database-row-number" aria-label={t('database.rowNumber')}>#</th>
    {result.columns.map((column, index) => <th key={index}>{column}</th>)}
  </tr></thead><tbody>
    {paddingTop > 0 && <tr className="database-virtual-spacer" aria-hidden="true"><td colSpan={columnCount} style={{ height: paddingTop }} /></tr>}
    {visibleRows.map((virtualRow) => { const row = rows[virtualRow.index]!; return <tr key={virtualRow.key} data-index={virtualRow.index} ref={virtualizer.measureElement} className={selectedRows.has(virtualRow.index) ? 'selected' : undefined} onClick={(event) => { if (suppressClick.current) { suppressClick.current = false; return } selectRow(virtualRow.index, event) }} onPointerDown={(event) => startRowDrag(event, virtualRow.index)} onContextMenu={(event) => openRowMenu(event, virtualRow.index, row)}><td className="database-row-number">{rowOffset + virtualRow.index + 1}</td>{row.map((value, j) => <td key={j}
    className={value === null ? 'database-null' : undefined} title={String(value ?? 'NULL')}
    onDoubleClick={() => onCopy(String(value ?? 'NULL'))}>{String(value ?? 'NULL')}</td>)}</tr> })}
    {paddingBottom > 0 && <tr className="database-virtual-spacer" aria-hidden="true"><td colSpan={columnCount} style={{ height: paddingBottom }} /></tr>}
  </tbody></table>
    {!result.rows.length && <p className="database-result-empty">{t('database.noRows')}</p>}
  </div>
})
