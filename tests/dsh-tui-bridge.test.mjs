import assert from 'node:assert/strict'
import test from 'node:test'
import { apply } from '../app/native/src/dsh_tui_bridge.mjs'

function harness(config = {}) {
  const handlers = new Map()
  const sections = []
  apply({ systemPrompt: { section: value => sections.push(value) }, on: (event, fn) => handlers.set(event, fn) }, { instructions: 'routing', ...config })
  return { sections, handlers, before: handlers.get('tools/pre-execute') }
}

test('contributes literal instructions without replacing prompts or handling approvals', async () => {
  const h = harness()
  assert.deepEqual(h.sections, [{ name: 'luna-mux-routing', order: 10000, text: 'routing', interpolate: false }])
  assert.deepEqual([...h.handlers.keys()], ['tools/pre-execute'])
  assert.equal(await h.before({ name: 'read' }, () => 'existing policy'), 'existing policy')
})

test('Browser preflight preserves host denial and otherwise delegates policy', async () => {
  const original = globalThis.fetch
  try {
    let invoked = 0
    globalThis.fetch = async (url, request) => {
      assert.equal(JSON.parse(request.body).tool_name, 'mcp__agent_browser__agent_browser_open')
      assert.deepEqual(JSON.parse(request.body).tool_input, { session: 'forbidden' })
      return { ok: true, json: async () => ({ hookSpecificOutput: { permissionDecision: 'deny', permissionDecisionReason: 'Use existing resource' } }) }
    }
    const before = harness().before
    const next = () => { invoked++; return { kind: 'ask' } }
    assert.deepEqual(await before({ name: 'mcp__agent_browser__agent_browser_open', arguments: { session: 'forbidden' } }, next), { kind: 'deny', reason: 'Use existing resource' })
    assert.equal(invoked, 0)
    globalThis.fetch = async () => ({ ok: true, json: async () => ({ sequence: 1 }) })
    assert.deepEqual(await before({ name: 'mcp__agent_browser__agent_browser_get_url' }, next), { kind: 'ask' })
    assert.equal(invoked, 1)
    globalThis.fetch = async () => ({ ok: false })
    assert.equal((await before({ name: 'mcp__agent_browser__agent_browser_get_url' }, next)).kind, 'deny')
    assert.equal(invoked, 1)
  } finally { globalThis.fetch = original }
})
