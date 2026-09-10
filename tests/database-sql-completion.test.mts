import test from 'node:test'
import assert from 'node:assert/strict'
import { databaseSqlCompletionContext, databaseSqlCompletionItems } from '../app/frontend/src/database-sql-completion.ts'

test('suggests table names and SQL keywords from the current token', () => {
  const sql = 'select * from us'
  const context = databaseSqlCompletionContext(sql, sql.length, ['users', 'orders'])
  const items = databaseSqlCompletionItems(context, ['users', 'orders'], {})
  assert.deepEqual(items.map((item) => item.label), ['users'])
  assert.equal(items[0]?.kind, 'table')
})

test('resolves aliases and suggests columns for a qualified token', () => {
  const sql = 'select u.na from users u'
  const cursor = sql.indexOf('na') + 2
  const context = databaseSqlCompletionContext(sql, cursor, ['users'], 'users')
  const items = databaseSqlCompletionItems(context, ['users'], { users: ['id', 'name', 'nickname'] })
  assert.equal(context?.table, 'users')
  assert.deepEqual(items.map((item) => item.label), ['name'])
  assert.equal(items[0]?.kind, 'column')
})

test('uses the selected table for unqualified column completion', () => {
  const sql = 'select em'
  const context = databaseSqlCompletionContext(sql, sql.length, ['users'], 'users')
  const items = databaseSqlCompletionItems(context, ['users'], { users: ['email', 'name'] })
  assert.deepEqual(items.map((item) => item.label), ['email'])
})
