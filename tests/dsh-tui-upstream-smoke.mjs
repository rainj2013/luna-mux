import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { writeFileSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { pathToFileURL } from 'node:url'
import { createRequire } from 'node:module'

// Optional integration check against an isolated, pinned upstream installation.
// Usage: node tests/dsh-tui-upstream-smoke.mjs <installation> <exported-fixture>
assert(process.argv[2] && process.argv[3], 'Provide upstream installation and Rust-exported fixture directories')
const installation = resolve(process.argv[2])
const requireUpstream = createRequire(join(installation, 'package.json'))
const upstream = name => import(pathToFileURL(requireUpstream.resolve(name)).href)
const { boot, loadOverlayPatches } = await upstream('@deepseek-ai/dsh-app-boot')
const { default: SystemPrompt } = await upstream('@deepseek-ai/dsh-system-prompt')
const { default: Tools } = await upstream('@deepseek-ai/dsh-tools')
const fixture = resolve(process.argv[3])
let calls = 0
const server = createServer(async (request, response) => {
  assert.equal(request.headers.authorization, 'Bearer smoke-only')
  if (request.method !== 'POST') { response.writeHead(405).end(); return }
  let text = ''
  for await (const chunk of request) text += chunk
  const body = JSON.parse(text)
  if (body.id === undefined) { response.writeHead(202).end(); return }
  let result
  switch (body.method) {
    case 'initialize': result = { protocolVersion: body.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: 'fixture', version: '1' } }; break
    case 'tools/list': result = { tools: [{ name: 'echo', description: 'Smoke echo', inputSchema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] } }] }; break
    case 'tools/call': calls++; result = { content: [{ type: 'text', text: body.params.arguments.text }] }; break
    default: response.writeHead(400).end(); return
  }
  response.writeHead(200, { 'Content-Type': 'application/json' }).end(JSON.stringify({ jsonrpc: '2.0', id: body.id, result }))
})
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve))
process.env.LUNA_MUX_MCP_ENDPOINT = `http://127.0.0.1:${server.address().port}/mcp`
process.env.LUNA_MUX_MCP_AUTHORIZATION = 'smoke-only'
delete process.env.LUNA_MUX_BROWSER_CDP_PORT
delete process.env.LUNA_MUX_BROWSER_BRIDGE_PORT
const root = join(fixture, 'cordis.yml')
writeFileSync(root, '[]\n')
let ctx
try {
  const patches = loadOverlayPatches('smoke', join(fixture, 'dsh-tui.patch.yml'))
  assert.equal(patches[0].insert.length, 3)
  ctx = await boot('smoke', root, patches, async host => {
    await host.plugin(SystemPrompt)
    await host.plugin(Tools)
  }, pathToFileURL(installation).href + '/')
  const entries = [...ctx.loader.entries()]
  assert(entries.some(entry => entry.options.id === 'luna-mux-routing' && entry.fiber))
  const assembly = await ctx.systemPrompt.assemble()
  assert(assembly.sections.some(section => section.name === 'luna-mux-routing' && section.text.includes('Luna Mux tool routing contract')))
  assert(assembly.tools.some(tool => tool.name === 'mcp__luna_mux__echo'))
  const result = await ctx.tools.execute({ callId: 'smoke', name: 'mcp__luna_mux__echo', arguments: { text: 'actual MCP call' }, signal: AbortSignal.timeout(5000) })
  assert.equal(calls, 1)
  assert(JSON.stringify(result).includes('actual MCP call'))
  console.log('PASS: generated overlay + real Cordis loader + routing plugin + HTTP MCP discovery and invocation')
} finally {
  await ctx?.fiber.dispose()
  await new Promise(resolve => server.close(resolve))
}
