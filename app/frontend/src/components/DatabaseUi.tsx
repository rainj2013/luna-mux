import { memo, useEffect, useRef } from 'react'
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

export const DatabaseResultGrid = memo(function DatabaseResultGrid({ result, onCopy, emptyMessage, rowOffset = 0 }: { result: DatabaseQueryResult | null; onCopy(value: string): void; emptyMessage?: string; rowOffset?: number }): React.JSX.Element {
  const { t } = useI18n()
  const scroll = useRef<HTMLDivElement>(null)
  const rows = result?.rows ?? []
  const virtualizer = useVirtualizer({ count: result?.columns.length ? rows.length : 0, getScrollElement: () => scroll.current, estimateSize: () => 29, overscan: 10 })
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
    {visibleRows.map((virtualRow) => { const row = rows[virtualRow.index]!; return <tr key={virtualRow.key} data-index={virtualRow.index} ref={virtualizer.measureElement}><td className="database-row-number">{rowOffset + virtualRow.index + 1}</td>{row.map((value, j) => <td key={j}
    className={value === null ? 'database-null' : undefined} title={String(value ?? 'NULL')}
    onDoubleClick={() => onCopy(String(value ?? 'NULL'))}>{String(value ?? 'NULL')}</td>)}</tr> })}
    {paddingBottom > 0 && <tr className="database-virtual-spacer" aria-hidden="true"><td colSpan={columnCount} style={{ height: paddingBottom }} /></tr>}
  </tbody></table>
    {!result.rows.length && <p className="database-result-empty">{t('database.noRows')}</p>}
  </div>
})

// Export the visible result snapshot. Never rerun SQL (which may contain writes).
export function downloadDatabaseResult(result: DatabaseQueryResult, format: 'csv' | 'json'): void {
  const text = format === 'json'
    ? JSON.stringify(result.rows.map((row) => Object.fromEntries(result.columns.map((column, index) => [column, row[index] ?? null]))), null, 2)
    : '\uFEFF' + [result.columns, ...result.rows].map((row) => row.map((cell) => `"${String(cell ?? '').replaceAll('"', '""')}"`).join(',')).join('\r\n')
  const url = URL.createObjectURL(new Blob([text], { type: format === 'json' ? 'application/json' : 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url; link.download = `query-result.${format}`; link.click()
  // Allow the webview to consume the URL before releasing it.
  setTimeout(() => URL.revokeObjectURL(url), 1000)
}
