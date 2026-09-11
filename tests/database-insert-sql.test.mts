import assert from 'node:assert/strict'
import test from 'node:test'
import { DATABASE_INSERT_ROWS_PER_STATEMENT, databaseInsertSql, queryResultTarget, sqlIdentifier, sqlLiteral } from '../app/frontend/src/database-insert-sql.ts'

test('literals follow the value type', () => {
  assert.equal(sqlLiteral(null, 'sqlite'), 'NULL')
  assert.equal(sqlLiteral(undefined, 'sqlite'), 'NULL')
  assert.equal(sqlLiteral(12, 'sqlite'), '12')
  assert.equal(sqlLiteral(1.5, 'mysql'), '1.5')
  assert.equal(sqlLiteral(true, 'postgresql'), 'TRUE')
  assert.equal(sqlLiteral(false, 'postgresql'), 'FALSE')
  assert.equal(sqlLiteral(7n, 'sqlite'), '7')
  assert.equal(sqlLiteral(Number.NaN, 'sqlite'), 'NULL')
})

test('text is quoted and single quotes are doubled', () => {
  assert.equal(sqlLiteral("O'Brien", 'sqlite'), "'O''Brien'")
  assert.equal(sqlLiteral('say \'hi\'', 'postgresql'), "'say ''hi'''")
  // SQLite sends INTEGER as a string, so a numeric string stays quoted.
  assert.equal(sqlLiteral('16', 'sqlite'), "'16'")
})

test('mysql doubles backslashes, other drivers keep them literal', () => {
  assert.equal(sqlLiteral('C:\\tmp\\x', 'mysql'), "'C:\\\\tmp\\\\x'")
  assert.equal(sqlLiteral('C:\\tmp\\x', 'sqlite'), "'C:\\tmp\\x'")
})

test('blob-shaped objects cannot be restored and become NULL', () => {
  assert.equal(sqlLiteral({ bytes: 4 }, 'sqlite'), 'NULL')
})

test('identifiers are quoted per driver', () => {
  assert.equal(sqlIdentifier('users', 'sqlite'), '"users"')
  assert.equal(sqlIdentifier('users', 'postgresql'), '"users"')
  assert.equal(sqlIdentifier('users', 'mysql'), '`users`')
  assert.equal(sqlIdentifier('we"ird', 'sqlite'), '"we""ird"')
  assert.equal(sqlIdentifier('we`ird', 'mysql'), '`we``ird`')
})

test('one statement holds every selected row', () => {
  const sql = databaseInsertSql('users', ['id', 'name'], [['1', 'Alice'], ['2', "Bo'b"]], 'sqlite')
  assert.equal(sql, 'INSERT INTO "users" ("id", "name") VALUES\n  (\'1\', \'Alice\'),\n  (\'2\', \'Bo\'\'b\');')
})

test('short rows fill the missing columns with NULL', () => {
  assert.equal(databaseInsertSql('t', ['a', 'b'], [['x']], 'sqlite'), 'INSERT INTO "t" ("a", "b") VALUES\n  (\'x\', NULL);')
})

test('large selections are split into several statements', () => {
  const rows = Array.from({ length: DATABASE_INSERT_ROWS_PER_STATEMENT + 1 }, (_, index) => [String(index)])
  const sql = databaseInsertSql('t', ['a'], rows, 'sqlite')
  assert.equal(sql.split('INSERT INTO').length - 1, 2)
  assert.ok(sql.endsWith(';'))
  assert.equal(sql.split(';').length - 1, 2)
})

test('empty inputs produce no SQL', () => {
  assert.equal(databaseInsertSql('', ['a'], [['x']], 'sqlite'), '')
  assert.equal(databaseInsertSql('t', [], [['x']], 'sqlite'), '')
  assert.equal(databaseInsertSql('t', ['a'], [], 'sqlite'), '')
})

test('query result targets come from simple FROM clauses with a neutral fallback', () => {
  assert.equal(queryResultTarget('select id from users where id > 1'), 'users')
  assert.equal(queryResultTarget('select * from app.users'), 'users')
  assert.equal(queryResultTarget('/* from ignored */ select 1'), 'query_result')
  assert.equal(queryResultTarget('select 1'), 'query_result')
})
