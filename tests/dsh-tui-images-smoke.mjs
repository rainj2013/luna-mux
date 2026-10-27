import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { createServer } from 'node:http'
import { Writable, PassThrough } from 'node:stream'
import { dirname, join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

// Exercise the published DSH renderer through the browser fixture's real
// xterm parser/canvas. Only synthetic pixels; no profile, model or Agent boot.
// node tests/dsh-tui-images-smoke.mjs <isolated-upstream-install> [port=1422]
// Then open the Vite terminal-images.html fixture with ?upstream=1422.
assert(process.argv[2], 'Provide an upstream installation directory')
const requireUpstream = createRequire(join(resolve(process.argv[2]), 'package.json'))
const React = requireUpstream('react')
const upstreamRoot = dirname(requireUpstream.resolve('@deepseek-harness-tui/dsh-tui/package.json'))
const { render, AlternateScreen, Box, Text, Image } = await import(pathToFileURL(join(upstreamRoot, 'lib/types/ui.js')).href)
const cols = 80, rows = 20
let chunks = []
class Output extends Writable {
  isTTY = true; columns = cols; rows = rows
  _write(chunk, _encoding, callback) { chunks.push(chunk.toString()); callback() }
}
class Input extends PassThrough {
  isTTY = true
  setRawMode() { return this }
  ref() { return this }
  unref() { return this }
}
const stdout = new Output(), stdin = new Input()
const stderr = new Writable({ write(_chunk, _encoding, callback) { callback() } })
const data = new Uint8Array(64 * 48 * 4)
for (let y = 0; y < 48; y++) for (let x = 0; x < 64; x++) {
  const i = (y * 64 + x) * 4
  data.set([x * 4, y * 5, ((x >> 3) + (y >> 3)) % 2 ? 230 : 40, 255], i)
}
let app
const h = React.createElement
const tree = (visible, large = false) => h(AlternateScreen, null,
  h(Box, { width: cols, height: rows, flexDirection: 'column' },
    h(Text, null, 'Published dst image preview (synthetic fixture)'),
    visible ? h(Image, { source: { data, width: 64, height: 48 },
      width: large ? 48 : 32, height: large ? 15 : 12,
      alt: 'NO_GRAPHICS_FALLBACK', presentation: 'preview' }) : h(Text, null, 'Preview closed'),
    h(Text, null, 'Footer must stay readable')))
const server = createServer(async (request, response) => {
  response.setHeader('Access-Control-Allow-Origin', 'http://127.0.0.1:1421')
  response.setHeader('Access-Control-Allow-Headers', 'Content-Type')
  response.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS')
  response.setHeader('Content-Type', 'application/json')
  if (request.method === 'OPTIONS') { response.end(); return }
  try {
    if (request.method === 'GET' && request.url === '/output') {
      const output = chunks.join(''); chunks = []
      response.end(JSON.stringify({ output })); return
    }
    let body = ''
    for await (const chunk of request) {
      body += chunk
      if (body.length > 65536) throw new Error('fixture input too large')
    }
    if (request.method !== 'POST') { response.writeHead(405).end('{}'); return }
    if (request.url === '/input') stdin.write(JSON.parse(body).data)
    else if (request.url === '/start') {
      if (app) throw new Error('fixture already started')
      app = await render(tree(true), { stdout, stdin, stderr, patchConsole: false, exitOnCtrlC: false, terminalImages: true })
    } else if (request.url === '/hide') app.rerender(tree(false))
    else if (request.url === '/show') app.rerender(tree(true))
    else if (request.url === '/zoom') app.rerender(tree(true, true))
    else { response.writeHead(404).end('{}'); return }
    response.end('{}')
  } catch (error) {
    response.writeHead(500).end(JSON.stringify({ error: String(error) }))
  }
})
server.listen(Number(process.argv[3] ?? 1422), '127.0.0.1', () => {
  console.log(`Published dst image fixture listening at http://127.0.0.1:${server.address().port}`)
})
process.on('SIGINT', async () => {
  await app?.detachForShutdown()
  app?.cleanup()
  server.close()
  process.exit(0)
})
