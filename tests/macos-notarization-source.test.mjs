import assert from 'node:assert/strict'
import { execFile } from 'node:child_process'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { promisify } from 'node:util'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const exec = promisify(execFile)
const source = fileURLToPath(new URL('../scripts/check-notarization-resume.py', import.meta.url))
const releaseTest = process.platform === 'win32' ? test.skip : test
const sha = '0123456789abcdef0123456789abcdef01234567'
const fixture = {
  head_repository: { full_name: 'test-owner/test-repo' },
  event: 'push', head_branch: 'v0.6.3', path: '.github/workflows/release.yml',
  status: 'completed', head_sha: sha
}
const program = `
import importlib.util,json,sys
spec=importlib.util.spec_from_file_location('resume_source',sys.argv[1])
module=importlib.util.module_from_spec(spec); spec.loader.exec_module(module)
run=json.loads(sys.argv[2]); tag_sha=sys.argv[3]; run_id=sys.argv[4]; tag=sys.argv[5]
def api(route):
    if '/actions/runs/' in route: return run
    if '/git/ref/tags/' in route: return {'object':{'type':'tag','sha':'a'*40}}
    if '/git/tags/' in route: return {'object':{'type':'commit','sha':tag_sha}}
    raise AssertionError('unexpected API route')
module.api=api
sys.argv=['check-notarization-resume.py',run_id,tag]
module.main()
`

async function verify(t, change = {}, tagSha = sha, runId = '123', tag = 'v0.6.3') {
  const directory = await mkdtemp(path.join(os.tmpdir(), 'luna-resume-source-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const output = path.join(directory, 'outputs')
  await exec('python3', ['-c', program, source, JSON.stringify({ ...fixture, ...change }), tagSha, runId, tag], {
    env: { ...process.env, PYTHONDONTWRITEBYTECODE: '1', GITHUB_REPOSITORY: 'test-owner/test-repo', GITHUB_OUTPUT: output }
  })
  return readFile(output, 'utf8')
}

releaseTest('recovery accepts an original tag-push build and dereferences its annotated tag', async (t) => {
  const output = await verify(t)
  assert.match(output, new RegExp(`source_sha=${sha}`))
  assert.match(output, /source_run_id=123\nrelease_tag=v0.6.3/)
})

for (const [name, change, tagSha] of [
  ['a fork', { head_repository: { full_name: 'someone-else/fork' } }],
  ['a pull request', { event: 'pull_request' }],
  ['a different tag', { head_branch: 'v0.6.2' }],
  ['a different workflow', { path: '.github/workflows/other.yml' }],
  ['a still-running source build', { status: 'in_progress' }],
  ['a moved tag', {}, 'f'.repeat(40)]
]) {
  releaseTest(`recovery refuses ${name}`, async (t) => {
    await assert.rejects(verify(t, change, tagSha))
  })
}

releaseTest('recovery rejects unvalidated run IDs and tag names', async (t) => {
  await assert.rejects(verify(t, {}, sha, '../123'))
  await assert.rejects(verify(t, {}, sha, '123', 'master'))
})
