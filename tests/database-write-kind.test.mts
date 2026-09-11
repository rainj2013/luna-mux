import assert from 'node:assert/strict'
import test from 'node:test'
import { databaseWriteKinds } from '../app/frontend/src/database-write-kind.ts'

const none = { dml: false, ddl: false }
const dml = { dml: true, ddl: false }
const ddl = { dml: false, ddl: true }
const both = { dml: true, ddl: true }

test('data statements are dml and read-only statements require nothing', () => {
  for (const sql of ['INSERT INTO t VALUES (1)', 'insert into t values (1)', 'UPDATE t SET a = 1', 'DELETE FROM t', 'REPLACE INTO t VALUES (1)']) {
    assert.deepEqual(databaseWriteKinds(sql), dml, sql)
  }
  for (const sql of ['SELECT 1', 'select * from users', 'WITH x AS (SELECT 1) SELECT * FROM x', 'PRAGMA table_info(t)', '']) {
    assert.deepEqual(databaseWriteKinds(sql), none, sql)
  }
})

test('schema statements are ddl', () => {
  const statements = ['CREATE TABLE t (a INTEGER)', 'create index i on t(a)', 'DROP TABLE t', 'ALTER TABLE t ADD COLUMN b TEXT', 'TRUNCATE TABLE t', 'ATTACH DATABASE \'x.db\' AS x', 'VACUUM', 'REINDEX']
  for (const sql of statements) assert.deepEqual(databaseWriteKinds(sql), ddl, sql)
})

test('every statement in a batch counts, so a leading SELECT cannot hide a write', () => {
  assert.deepEqual(databaseWriteKinds('SELECT 1; DROP TABLE users'), ddl)
  assert.deepEqual(databaseWriteKinds('SELECT 1; UPDATE users SET id = 2;'), dml)
  assert.deepEqual(databaseWriteKinds('INSERT INTO t VALUES (1); CREATE INDEX i ON t(a)'), both)
  assert.deepEqual(databaseWriteKinds('SELECT 1; SELECT 2'), none)
  assert.deepEqual(databaseWriteKinds('SELECT 1; /* ; */ DROP TABLE users'), ddl)
  assert.deepEqual(databaseWriteKinds('INSERT INTO t VALUES (1); /* x ; */ DROP TABLE u'), both)
})

test('a semicolon inside a comment or a literal does not split a statement', () => {
  assert.deepEqual(databaseWriteKinds('/* ; */ DROP TABLE users'), ddl)
  assert.deepEqual(databaseWriteKinds('SELECT 1; /* ; */ DELETE FROM users'), dml)
  assert.deepEqual(databaseWriteKinds("SELECT 'a;b'"), none)
  assert.deepEqual(databaseWriteKinds("INSERT INTO t VALUES ('it''s; fine')"), dml)
  assert.deepEqual(databaseWriteKinds('SELECT 1; -- ;\nDELETE FROM t'), dml)
})

test('leading comments and whitespace do not hide the write keyword', () => {
  assert.deepEqual(databaseWriteKinds('  \n\t-- drop everything later\nDELETE FROM t'), dml)
  assert.deepEqual(databaseWriteKinds('/* schema change */ DROP TABLE t'), ddl)
  assert.deepEqual(databaseWriteKinds('# c;omment\nDROP TABLE users'), ddl)
  assert.deepEqual(databaseWriteKinds('-- a\n-- b\n  create table t (a int)'), ddl)
  assert.deepEqual(databaseWriteKinds('/* unterminated'), none)
  assert.deepEqual(databaseWriteKinds('-- only a comment'), none)
})

test('statements that write through a nested statement are detected', () => {
  assert.deepEqual(databaseWriteKinds('SELECT 1; WITH x AS (SELECT 1) INSERT INTO t SELECT * FROM x'), dml)
  assert.deepEqual(databaseWriteKinds('WITH x AS (SELECT 1) DELETE FROM t WHERE id IN (SELECT * FROM x)'), dml)
  assert.deepEqual(databaseWriteKinds('EXPLAIN ANALYZE DELETE FROM t'), dml)
  assert.deepEqual(databaseWriteKinds('WITH deleted AS (SELECT 1) SELECT * FROM deleted'), none)
  assert.deepEqual(databaseWriteKinds("WITH x AS (SELECT 'delete' AS s) SELECT * FROM x"), none)
})

test('keywords are matched as whole words', () => {
  assert.deepEqual(databaseWriteKinds('deletee FROM t'), none)
  assert.deepEqual(databaseWriteKinds('inserted into t'), none)
  assert.deepEqual(databaseWriteKinds('creates table t'), none)
  assert.deepEqual(databaseWriteKinds('WITH x AS (SELECT 1) SELECT * FROM update_log'), none)
})
