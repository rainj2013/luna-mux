import type { DatabaseDriver } from './types'

// A single statement with too many value tuples can hit engine limits (SQLite's
// compound-select term count, MySQL's max_allowed_packet), so a large copy is split
// into several INSERT statements.
export const DATABASE_INSERT_ROWS_PER_STATEMENT = 200

// Identifiers are quoted for the driver: MySQL only accepts backticks unless
// ANSI_QUOTES is enabled, and double quotes are what SQLite and PostgreSQL accept.
export function sqlIdentifier(name: string, driver: DatabaseDriver): string {
  return driver === 'mysql' ? `\`${name.replaceAll('`', '``')}\`` : `"${name.replaceAll('"', '""')}"`
}

export function sqlTextLiteral(value: string, driver: DatabaseDriver): string {
  // MySQL treats a backslash as an escape character unless NO_BACKSLASH_ESCAPES is set,
  // so a Windows path or JSON payload would otherwise lose characters.
  const escaped = driver === 'mysql' ? value.replaceAll('\\', '\\\\').replaceAll("'", "''") : value.replaceAll("'", "''")
  return `'${escaped}'`
}

// Values reach the frontend as JSON. SQLite sends INTEGER as a string (it may exceed
// Number.MAX_SAFE_INTEGER) and BLOB as { bytes: n } without its content, so a blob can
// only be restored as NULL.
export function sqlLiteral(value: unknown, driver: DatabaseDriver): string {
  if (value === null || value === undefined) return 'NULL'
  if (typeof value === 'boolean') return value ? 'TRUE' : 'FALSE'
  if (typeof value === 'bigint') return value.toString()
  if (typeof value === 'number') return Number.isFinite(value) ? String(value) : 'NULL'
  if (typeof value === 'object') return 'NULL'
  return sqlTextLiteral(String(value), driver)
}

export function databaseInsertSql(table: string, columns: string[], rows: unknown[][], driver: DatabaseDriver): string {
  if (!table || !columns.length || !rows.length) return ''
  const target = `${sqlIdentifier(table, driver)} (${columns.map((column) => sqlIdentifier(column, driver)).join(', ')})`
  const statements: string[] = []
  for (let start = 0; start < rows.length; start += DATABASE_INSERT_ROWS_PER_STATEMENT) {
    const tuples = rows.slice(start, start + DATABASE_INSERT_ROWS_PER_STATEMENT).map((row) => `(${columns.map((_, index) => sqlLiteral(row[index] ?? null, driver)).join(', ')})`)
    statements.push(`INSERT INTO ${target} VALUES\n  ${tuples.join(',\n  ')};`)
  }
  return statements.join('\n')
}

// A result set has no write target of its own. For simple SELECT ... FROM table queries, keep the
// source table as the INSERT target; expressions, joins, and SELECTs without FROM use a neutral
// name that the user can replace in the generated SQL before importing it.
export function queryResultTarget(statement: string): string {
  const withoutComments = statement.replace(/--[^\r\n]*/g, ' ').replace(/\/\*[\s\S]*?\*\//g, ' ')
  const match = withoutComments.match(/\bfrom\s+(?:(?:"([^"]+)"|`([^`]+)`|([A-Za-z_][A-Za-z0-9_$]*))(?:\s*\.\s*(?:"([^"]+)"|`([^`]+)`|([A-Za-z_][A-Za-z0-9_$]*)))?)/i)
  const candidate = match ? (match[4] ?? match[5] ?? match[6] ?? match[1] ?? match[2] ?? match[3]) : undefined
  return candidate?.trim() || 'query_result'
}
