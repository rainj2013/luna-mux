export type DatabaseWriteKind = 'dml' | 'ddl'

export interface DatabaseWriteKinds {
  dml: boolean
  ddl: boolean
}

const DML_KEYWORDS = new Set(['insert', 'update', 'delete', 'replace'])
const DDL_KEYWORDS = new Set(['create', 'drop', 'alter', 'truncate', 'attach', 'vacuum', 'reindex'])
// These start a statement but hand control to a nested write statement.
const NESTED_KEYWORDS = new Set(['with', 'explain'])

// Replace comments and quoted content with spaces of the same shape, keeping the
// statement's first keyword intact. A ";" or a keyword inside a comment or a
// string must neither split a statement nor fake a write.
function masked(sql: string): string {
  let out = ''
  let index = 0
  while (index < sql.length) {
    const char = sql[index]!
    const next = sql[index + 1]
    if (char === '-' && next === '-') {
      const lineEnd = sql.indexOf('\n', index)
      out += ' '.repeat((lineEnd === -1 ? sql.length : lineEnd) - index)
      index = lineEnd === -1 ? sql.length : lineEnd
      continue
    }
    // MySQL/MariaDB line comment. Not a comment elsewhere, where skipping it
    // still only reveals the statement that follows.
    if (char === '#') {
      const lineEnd = sql.indexOf('\n', index)
      out += ' '.repeat((lineEnd === -1 ? sql.length : lineEnd) - index)
      index = lineEnd === -1 ? sql.length : lineEnd
      continue
    }
    if (char === '/' && next === '*') {
      const blockEnd = sql.indexOf('*/', index + 2)
      const end = blockEnd === -1 ? sql.length : blockEnd + 2
      out += ' '.repeat(end - index)
      index = end
      continue
    }
    if (char === "'" || char === '"' || char === '`') {
      const quote = char
      out += quote
      index += 1
      while (index < sql.length) {
        const inner = sql[index]!
        // A doubled quote is an escaped quote, not the end of the literal.
        if (inner === quote && sql[index + 1] === quote) {
          out += '  '
          index += 2
          continue
        }
        if (inner === '\\' && quote !== '`' && index + 1 < sql.length) {
          out += '  '
          index += 2
          continue
        }
        out += inner === quote ? quote : ' '
        index += 1
        if (inner === quote) break
      }
      continue
    }
    out += char
    index += 1
  }
  return out
}

function firstWord(statement: string): string {
  const match = /^[a-z_][a-z0-9_]*/i.exec(statement.trimStart())
  return match ? match[0].toLowerCase() : ''
}

// PostgreSQL runs simple-query batches, so every statement in the text matters,
// not just the leading one.
export function databaseWriteKinds(sql: string): DatabaseWriteKinds {
  const kinds: DatabaseWriteKinds = { dml: false, ddl: false }
  for (const statement of masked(sql).split(';')) {
    const keyword = firstWord(statement)
    if (DML_KEYWORDS.has(keyword)) { kinds.dml = true; continue }
    if (DDL_KEYWORDS.has(keyword)) { kinds.ddl = true; continue }
    // "WITH ... INSERT" and "EXPLAIN ANALYZE DELETE" write through a nested
    // statement. Masking already removed strings and comments, so scanning the
    // remaining words cannot match text the database would treat as data.
    if (!NESTED_KEYWORDS.has(keyword)) continue
    for (const word of statement.toLowerCase().match(/[a-z0-9_]+/g) ?? []) {
      if (DML_KEYWORDS.has(word)) kinds.dml = true
      else if (DDL_KEYWORDS.has(word)) kinds.ddl = true
    }
  }
  return kinds
}
