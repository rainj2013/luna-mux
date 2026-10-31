import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rm, symlink, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import test from 'node:test'

const exec = promisify(execFile)
const script = new URL('../scripts/release-macos.sh', import.meta.url)
const releaseTest = process.platform === 'win32' ? test.skip : test

// Stand in for macOS tools, never using real certificates, a keychain, or Apple.
const mock = `#!/usr/bin/env node
import fs from 'node:fs'
import path from 'node:path'
const command = path.basename(process.argv[1])
const args = process.argv.slice(2)
const record = (value) => fs.appendFileSync(process.env.MOCK_EVENTS, value + '\\n')
const sub = args[0]
if (process.env.MOCK_FAIL_TOOL === command ||
    (process.env.MOCK_FAIL_TOOL === 'stapler' && command === 'xcrun' && sub === 'stapler')) process.exit(14)
if (command === 'uname') console.log('Darwin')
if (command === 'security') {
  record('security ' + sub)
  if (sub === 'list-keychains' && !args.includes('-s')) console.log('    "/tmp/Original Keychain.keychain-db"')
  if (sub === 'import') {
    const file = args[1]
    if ((fs.statSync(file).mode & 0o077) !== 0) process.exit(8)
    if (process.env.MOCK_IMPORT_FAIL === '1') process.exit(9)
  }
  if (sub === 'find-identity') console.log('1) FAKE "' + process.env.APPLE_SIGNING_IDENTITY + '"')
}
if (command === 'npm') {
  record('bundle')
  for (const name of ['APPLE_CERTIFICATE', 'APPLE_CERTIFICATE_PASSWORD', 'APPLE_API_PRIVATE_KEY']) {
    if (process.env[name]) process.exit(10)
  }
  const root = 'app/native/target/x86_64-apple-darwin/release/bundle'
  const bin = root + '/macos/Luna Mux.app/Contents/MacOS'
  fs.mkdirSync(bin, { recursive: true })
  fs.mkdirSync(root + '/dmg', { recursive: true })
  fs.writeFileSync(bin + '/agent-browser', '#!/bin/bash\\necho agent-browser-test\\n', { mode: 0o755 })
  fs.writeFileSync(root + '/dmg/Luna Mux.dmg', 'signed installer fixture')
  // Tauri removes the intermediate .app when only the DMG was requested.
  if (!args[args.indexOf('--bundles') + 1].split(',').includes('app')) {
    fs.rmSync(root + '/macos', { recursive: true })
  }
}
if (command === 'codesign') {
  record('codesign ' + sub)
  if (sub === '-dv') {
    console.error('Authority=' + process.env.APPLE_SIGNING_IDENTITY)
    console.error('flags=0x10000(runtime)')
  }
}
if (command === 'xcrun') {
  record('xcrun ' + args.slice(0, 2).join(' '))
  if (sub === 'notarytool') {
    if (args.includes('--issuer')) process.exit(11)
    const key = args[args.indexOf('--key') + 1]
    if ((fs.statSync(key).mode & 0o077) !== 0) process.exit(12)
    if (process.env.MOCK_NOTARY_BLOCK === '1') {
      record('notarization-wait')
      // Model GitHub cancelling the shell and its active notarytool child.
      setTimeout(() => {
        process.kill(process.ppid, 'SIGTERM')
        process.exit(143)
      }, 100)
    }
    console.log(JSON.stringify({status: process.env.MOCK_NOTARY_STATUS || 'Accepted', id: 'fake-submission'}))
    if (process.env.MOCK_NOTARY_TIMEOUT === '1') process.exit(13)
  }
}
if (['spctl', 'hdiutil'].includes(command)) record(command)
`

async function scenario(t, overrides = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'luna-release-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const bin = path.join(directory, 'bin')
  const runner = path.join(directory, 'runner')
  await mkdir(bin)
  await mkdir(runner)
  await writeFile(path.join(directory, 'package.json'), '{"type":"module"}')
  const mockPath = path.join(bin, 'mock.mjs')
  await writeFile(mockPath, mock)
  await chmod(mockPath, 0o755)
  for (const name of ['uname', 'security', 'npm', 'codesign', 'xcrun', 'spctl', 'hdiutil']) {
    await symlink(mockPath, path.join(bin, name))
  }
  const localScript = path.join(directory, 'release-macos.sh')
  await copyFile(script, localScript)
  const events = path.join(directory, 'events')
  const env = {
    ...process.env,
    PATH: `${bin}${path.delimiter}${process.env.PATH}`,
    RUNNER_TEMP: runner,
    MOCK_EVENTS: events,
    APPLE_CERTIFICATE: Buffer.from('FAKE_CERTIFICATE_SENTINEL').toString('base64'),
    APPLE_CERTIFICATE_PASSWORD: 'FAKE_PASSWORD_SENTINEL',
    APPLE_SIGNING_IDENTITY: 'Developer ID Application: Test (FAKE_TEAM)',
    APPLE_API_KEY: 'FAKE_KEY_ID',
    APPLE_API_PRIVATE_KEY: 'FAKE_PRIVATE_KEY_SENTINEL',
    ...overrides
  }
  let error
  let output
  try {
    output = await exec('/bin/bash', [localScript, 'x86_64-apple-darwin', 'x64'], { cwd: directory, env, timeout: 15000 })
  } catch (caught) {
    error = caught
    output = caught
  }
  const log = `${output.stdout}\n${output.stderr}`
  for (const secret of [env.APPLE_CERTIFICATE, env.APPLE_CERTIFICATE_PASSWORD, env.APPLE_API_PRIVATE_KEY, 'FAKE_CERTIFICATE_SENTINEL']) {
    if (secret) assert.ok(!log.includes(secret), 'credentials must not appear in output')
  }
  assert.deepEqual(await readdir(runner), [], 'temporary credentials must be removed')
  return { directory, error, log, events: await readFile(events, 'utf8').catch(() => '') }
}

releaseTest('accepted notarization verifies and collects only the DMG, then removes credentials', async (t) => {
  const result = await scenario(t)
  assert.ifError(result.error)
  assert.deepEqual(await readdir(path.join(result.directory, 'release-assets')), ['Luna-Mux_macos-x64.dmg'])
  assert.match(result.events, /xcrun stapler staple/)
  assert.match(result.events, /spctl/)
  assert.match(result.events, /hdiutil/)
  assert.match(result.events, /security delete-keychain/)
})

for (const [name, overrides] of [
  ['missing credentials', { APPLE_API_PRIVATE_KEY: '' }],
  ['failed certificate import', { MOCK_IMPORT_FAIL: '1' }],
  ['rejected notarization', { MOCK_NOTARY_STATUS: 'Invalid' }],
  ['notarization timeout', { MOCK_NOTARY_STATUS: 'In Progress', MOCK_NOTARY_TIMEOUT: '1' }],
  ['cancellation during notarization', { MOCK_NOTARY_BLOCK: '1' }],
  ['signature verification failure', { MOCK_FAIL_TOOL: 'codesign' }],
  ['ticket validation failure', { MOCK_FAIL_TOOL: 'stapler' }],
  ['Gatekeeper failure', { MOCK_FAIL_TOOL: 'spctl' }],
  ['DMG verification failure', { MOCK_FAIL_TOOL: 'hdiutil' }]
]) {
  releaseTest(`${name} stops artifact collection without logging credentials`, async (t) => {
    const result = await scenario(t, overrides)
    assert.ok(result.error)
    await assert.rejects(readFile(path.join(result.directory, 'release-assets/Luna-Mux_macos-x64.dmg')), { code: 'ENOENT' })
    if (name !== 'missing credentials') assert.match(result.events, /security delete-keychain/)
  })
}
