import assert from 'node:assert/strict'
import test from 'node:test'
import { fileDialogExtension, fileDialogJoin, fileDialogSegments, fileDialogSeparator, fileDialogWithExtension } from '../app/frontend/src/file-dialog-path.ts'

test('posix paths split into root and nested crumbs', () => {
  assert.deepEqual(fileDialogSegments('/'), [{ name: '/', path: '/' }])
  assert.deepEqual(fileDialogSegments('/Users/me'), [
    { name: '/', path: '/' },
    { name: 'Users', path: '/Users' },
    { name: 'me', path: '/Users/me' }
  ])
  // Trailing separators only trim for display; the last crumb stays the directory itself.
  assert.deepEqual(fileDialogSegments('/Users/me/'), [
    { name: '/', path: '/' },
    { name: 'Users', path: '/Users' },
    { name: 'me', path: '/Users/me' }
  ])
  assert.deepEqual(fileDialogSegments(''), [])
})

test('windows drives and UNC shares keep a usable root crumb', () => {
  assert.deepEqual(fileDialogSegments('C:\\'), [{ name: 'C:\\', path: 'C:\\' }])
  assert.deepEqual(fileDialogSegments('C:\\Users\\me'), [
    { name: 'C:\\', path: 'C:\\' },
    { name: 'Users', path: 'C:\\Users' },
    { name: 'me', path: 'C:\\Users\\me' }
  ])
  assert.deepEqual(fileDialogSegments('\\\\server\\share\\dir'), [
    { name: '\\\\server\\share\\', path: '\\\\server\\share\\' },
    { name: 'dir', path: '\\\\server\\share\\dir' }
  ])
})

test('a relative path falls back to plain segments', () => {
  assert.deepEqual(fileDialogSegments('a/b'), [{ name: 'a', path: 'a' }, { name: 'b', path: 'a/b' }])
})

test('joining uses the separator the directory already uses', () => {
  assert.equal(fileDialogSeparator('/Users/me'), '/')
  assert.equal(fileDialogSeparator('C:\\Users'), '\\')
  assert.equal(fileDialogJoin('/Users/me', 'dump.sql'), '/Users/me/dump.sql')
  assert.equal(fileDialogJoin('/', 'dump.sql'), '/dump.sql')
  assert.equal(fileDialogJoin('C:\\Users', 'dump.sql'), 'C:\\Users\\dump.sql')
  assert.equal(fileDialogJoin('C:\\', 'dump.sql'), 'C:\\dump.sql')
  assert.equal(fileDialogJoin('', 'dump.sql'), 'dump.sql')
})

test('extensions are read case-insensitively and dotfiles have none', () => {
  assert.equal(fileDialogExtension('dump.sql'), 'sql')
  assert.equal(fileDialogExtension('DUMP.SQL'), 'sql')
  assert.equal(fileDialogExtension('dump'), '')
  assert.equal(fileDialogExtension('.bashrc'), '')
  assert.equal(fileDialogExtension('archive.tar.gz'), 'gz')
})

test('a saved name gains the filter extension only when it has none', () => {
  assert.equal(fileDialogWithExtension('dump', ['sql']), 'dump.sql')
  assert.equal(fileDialogWithExtension('dump.', ['sql']), 'dump.sql')
  assert.equal(fileDialogWithExtension('dump.sql', ['sql']), 'dump.sql')
  assert.equal(fileDialogWithExtension('my.dump', ['sql']), 'my.dump')
  assert.equal(fileDialogWithExtension('dump', []), 'dump')
})
