export type DatabaseSqlCompletionKind = 'keyword' | 'table' | 'column'

export interface DatabaseSqlCompletionContext {
  start: number
  end: number
  prefix: string
  qualifier?: string
  table?: string
}

export interface DatabaseSqlCompletionItem {
  label: string
  kind: DatabaseSqlCompletionKind
  start: number
  end: number
}

const SQL_KEYWORDS = [
  'SELECT', 'FROM', 'WHERE', 'JOIN', 'LEFT JOIN', 'RIGHT JOIN', 'INNER JOIN',
  'GROUP BY', 'ORDER BY', 'HAVING', 'LIMIT', 'OFFSET', 'INSERT INTO', 'UPDATE',
  'DELETE FROM', 'VALUES', 'SET', 'AND', 'OR', 'AS', 'ON', 'DISTINCT', 'NULL'
]

const RESERVED_ALIASES = new Set([
  'where', 'join', 'left', 'right', 'inner', 'outer', 'cross', 'on', 'group',
  'order', 'having', 'limit', 'offset', 'union', 'set', 'values', 'returning'
])

function matchingTable(tables: string[], value: string | undefined): string | undefined {
  if (!value) return undefined
  return tables.find((table) => table.toLowerCase() === value.toLowerCase())
}

export function databaseSqlCompletionContext(sql: string, cursor: number, tables: string[], selectedTable = ''): DatabaseSqlCompletionContext | null {
  const before = sql.slice(0, Math.max(0, Math.min(cursor, sql.length)))
  const qualified = before.match(/([A-Za-z_$][\w$]*)\.([A-Za-z_$][\w$]*)?$/)
  const word = qualified ? undefined : before.match(/([A-Za-z_$][\w$]*)$/)
  if (!qualified && !word) return null

  const qualifier = qualified?.[1]
  const prefix = qualified?.[2] ?? word?.[1] ?? ''
  const references = new Map<string, string>()
  const referencePattern = /\b(?:from|join)\s+[`"]?([A-Za-z_$][\w$]*)[`"]?(?:\s+(?:as\s+)?([A-Za-z_$][\w$]*))?/gi
  for (const match of sql.matchAll(referencePattern)) {
    const table = matchingTable(tables, match[1])
    if (!table) continue
    references.set(table.toLowerCase(), table)
    const alias = match[2]?.toLowerCase()
    if (alias && !RESERVED_ALIASES.has(alias)) references.set(alias, table)
  }
  const table = qualifier
    ? references.get(qualifier.toLowerCase()) ?? matchingTable(tables, qualifier)
    : references.values().next().value ?? matchingTable(tables, selectedTable)
  return { start: cursor - prefix.length, end: cursor, prefix, qualifier, table }
}

export function databaseSqlCompletionItems(
  context: DatabaseSqlCompletionContext | null,
  tables: string[],
  columnsByTable: Record<string, string[]>
): DatabaseSqlCompletionItem[] {
  if (!context) return []
  const prefix = context.prefix.toLowerCase()
  const candidates: Array<{ label: string; kind: DatabaseSqlCompletionKind }> = []
  if (context.qualifier) {
    if (context.table) candidates.push(...(columnsByTable[context.table] ?? []).map((label) => ({ label, kind: 'column' as const })))
  } else {
    candidates.push(...SQL_KEYWORDS.map((label) => ({ label, kind: 'keyword' as const })))
    candidates.push(...tables.map((label) => ({ label, kind: 'table' as const })))
    if (context.table) candidates.push(...(columnsByTable[context.table] ?? []).map((label) => ({ label, kind: 'column' as const })))
  }
  const seen = new Set<string>()
  return candidates
    .filter(({ label }) => !prefix || label.toLowerCase().startsWith(prefix))
    .filter(({ label }) => {
      const key = label.toLowerCase()
      if (seen.has(key)) return false
      seen.add(key)
      return true
    })
    .slice(0, 12)
    .map(({ label, kind }) => ({ label, kind, start: context.start, end: context.end }))
}
