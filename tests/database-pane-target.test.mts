import assert from 'node:assert/strict'
import test from 'node:test'
import { databasePaneTarget, parseDatabasePaneTarget } from '../app/frontend/src/database-pane-target.ts'

test('saved DB target restores profile identity and read-only mode', () => {
  for (const profileId of ['mysql-local', 'SQLite 本地', 'profile:?/#%&=']) {
    for (const readOnly of [false, true]) {
      assert.deepEqual(parseDatabasePaneTarget(databasePaneTarget(profileId, readOnly)), { profileId, readOnly })
    }
  }
})

test('legacy empty and non-database targets never resolve to a DB connection', () => {
  for (const target of ['', 'ssh-bookmark:legacy', 'local:powershell', 'database-profile:', 'database-profile:?readOnly=true']) {
    assert.equal(parseDatabasePaneTarget(target), undefined)
  }
})

test('malformed saved references do not crash workspace restoration', () => {
  assert.equal(parseDatabasePaneTarget('database-profile:%E0%A4%A'), undefined)
  assert.equal(parseDatabasePaneTarget('database-profile:%'), undefined)
  // A target saved before the flag existed restores read-only, the safe default.
  assert.deepEqual(parseDatabasePaneTarget('database-profile:old-id'), { profileId: 'old-id', readOnly: true })
  assert.deepEqual(parseDatabasePaneTarget('database-profile:old-id?readOnly=false'), { profileId: 'old-id', readOnly: false })
})
