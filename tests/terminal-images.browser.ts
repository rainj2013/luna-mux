// Browser checks use the real canvas and both xterm renderers. No Tauri,
// user profile, model connection or local image-file access is needed.
import { Terminal } from '@xterm/xterm'
import { WebglAddon } from '@xterm/addon-webgl'
import '@xterm/xterm/css/xterm.css'
import { configureTerminalImages } from '../app/frontend/src/terminal-images'
import desktopConfig from '../app/native/tauri.conf.json'

// Exercise decoding under the desktop's actual script policy. An unrestricted
// Vite page otherwise hides WebAssembly compilation failures in the WebView.
const desktopScriptPolicy = desktopConfig.app.security.csp.split(';').find(policy => policy.trim().startsWith('script-src '))!
const scriptPolicy = document.createElement('meta')
scriptPolicy.httpEquiv = 'Content-Security-Policy'
scriptPolicy.content = desktopScriptPolicy
document.head.appendChild(scriptPolicy)

// A background Browser Resource can suspend animation frames altogether.
// Drive the same render callbacks with timers in this test page only; canvas,
// layout, decoding and renderer implementations remain real.
if (document.hidden) {
  window.requestAnimationFrame = callback => window.setTimeout(() => callback(performance.now()), 16)
  window.cancelAnimationFrame = frame => window.clearTimeout(frame)
}

const host = document.querySelector<HTMLDivElement>('#terminal')!
const results: { name: string; passed: boolean; error?: string }[] = []
Object.assign(window, { imageTestResults: results })
const term = new Terminal({ cols: 80, rows: 20, allowProposedApi: true, fontSize: 16 })
term.open(host)
const images = configureTerminalImages(term)
let replies = ''
term.onData(data => { replies += data })
const write = (text: string) => new Promise<void>(resolve => term.write(text, resolve))
const repaint = async () => {
  term.refresh(0, term.rows - 1)
  await new Promise(resolve => setTimeout(resolve, 80))
}
function assert(condition: unknown, message: string): asserts condition {
  if (!condition) throw new Error(message)
}
async function check(name: string, run: () => Promise<void>) {
  try {
    await run()
    results.push({ name, passed: true })
  } catch (error) {
    results.push({ name, passed: false, error: String(error) })
  }
  const row = document.createElement('li')
  const result = results.at(-1)!
  row.className = result.passed ? 'pass' : 'fail'
  row.textContent = `${result.passed ? 'PASS' : 'FAIL'} ${name}${result.error ? ': ' + result.error : ''}`
  document.querySelector('#results')!.appendChild(row)
}
// Two bands, red then green, encoded as real SIXEL (12 x 12 pixels).
const sixel = '\x1bP0;1;0q"1;1;12;12#1;2;100;0;0#1!12~-#2;2;0;100;0#2!12~\x1b\\'
const image = () => images.getImageAtBufferCell(0, 0)
const layerHasPixels = () => {
  const layer = host.querySelector<HTMLCanvasElement>('.xterm-image-layer')
  return layer && layer.getContext('2d')!.getImageData(0, 0, layer.width, layer.height).data.some((v, i) => i % 4 === 3 && v > 0)
}

await check('desktop script policy permits WebAssembly while blocking JavaScript eval', async () => {
  await WebAssembly.compile(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]))
  let evalBlocked = false
  try { new Function('return 1')() } catch (error) { evalBlocked = error instanceof EvalError }
  assert(evalBlocked, 'JavaScript eval must remain blocked')
})
await check('capability and pixel metrics queries select Sixel without claiming Kitty', async () => {
  await repaint()
  await write('\x1b[c\x1b[16t\x1b[14t\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\')
  assert(replies.includes('\x1b[?62;4;9;22c'), 'missing SIXEL capability')
  assert(/\x1b\[6;[1-9]\d*;[1-9]\d*t/.test(replies), 'missing cell pixel dimensions')
  assert(/\x1b\[4;[1-9]\d*;[1-9]\d*t/.test(replies), 'missing window pixel dimensions')
  assert(!replies.includes('OK'), 'Kitty must not be advertised')
})
await check('split image sequences produce the expected canvas pixels on the DOM renderer', async () => {
  await write('\x1b[?1049h\x1b[H')
  for (let start = 0; start < sixel.length; start += 3) await write(sixel.slice(start, start + 3))
  await repaint()
  const canvas = image()
  assert(canvas?.width === 12 && canvas.height === 12, 'decoded dimensions differ')
  const pixel = (y: number) => [...canvas.getContext('2d')!.getImageData(1, y, 1, 1).data]
  assert(pixel(1).join() === '255,0,0,255', 'first band must be red')
  assert(pixel(7).join() === '0,255,0,255', 'second band must be green')
  assert(layerHasPixels(), 'image was decoded but not displayed')
})
await check('font changes, resize and hiding a pane preserve a visible image', async () => {
  term.options.fontSize = 18
  term.resize(84, 22)
  host.hidden = true
  await repaint()
  host.hidden = false
  await repaint()
  assert(image() && layerHasPixels(), 'image disappeared after geometry/visibility changes')
})
await check('WebGL activation and return to DOM redraw the image layer', async () => {
  const webgl = new WebglAddon()
  term.loadAddon(webgl)
  await repaint()
  assert(layerHasPixels(), 'missing image with WebGL renderer')
  webgl.dispose()
  await repaint()
  assert(layerHasPixels(), 'missing image after returning to DOM renderer')
})
await check('overwriting image cells clears the preview pixels', async () => {
  await write('\x1b[H\x1b[2K')
  await repaint()
  assert(!image() && !layerHasPixels(), 'stale image remains over text')
})
await check('leaving the alternate screen releases its images', async () => {
  await write('\x1b[H' + sixel)
  await repaint()
  assert(image(), 'image not created before buffer switch')
  await write('\x1b[?1049l')
  await repaint()
  assert(images.storageUsage === 0 && !layerHasPixels(), 'alternate images retained')
})
await check('oversized rasters are rejected and text parsing recovers', async () => {
  await write('\x1b[?1049h\x1b[H\x1bP0;1;0q"1;1;4096;4096#1;2;100;0;0#1~\x1b\\OK')
  await repaint()
  assert(!image(), 'oversized image was accepted')
  assert(term.buffer.active.getLine(0)?.translateToString(true) === 'OK', 'parser did not recover')
})
await check('repeated images and font enlargement respect the retained canvas budget', async () => {
  term.options.fontSize = 10
  await repaint()
  const large = '\x1bP0;1;0q"1;1;2048;1024#1;2;100;0;0#1' + Array(171).fill('!2048~-').join('') + '\x1b\\'
  for (let i = 0; i < 6; i++) {
    await write('\x1b[H' + large)
    await repaint()
    assert(images.storageUsage > 0, 'large fixture was rejected instead of exercising eviction')
    assert(images.storageUsage <= images.storageLimit, 'cache grew past its insertion budget')
  }
  term.options.fontSize = 30
  await repaint()
  assert(images.storageUsage <= images.storageLimit, 'scaled canvases bypassed the storage budget')
  term.options.fontSize = 16
  await write('\x1b[2J\x1b[H')
})
await check('terminal disposal removes the image canvas', async () => {
  await write('\x1b[H' + sixel)
  await repaint()
  term.dispose()
  assert(!host.querySelector('.xterm-image-layer'), 'image layer leaked')
})
Object.assign(window, { imageTestsComplete: true })

// Leave a visible, scalable pixel fixture for screenshot inspection.
const preview = new Terminal({ cols: 80, rows: 20, fontSize: 16, allowProposedApi: true })
preview.open(host)
const previewImages = configureTerminalImages(preview)
const upstreamPort = new URLSearchParams(location.search).get('upstream')
if (upstreamPort) {
  assert(/^\d+$/.test(upstreamPort), 'invalid upstream fixture port')
  const endpoint = `http://127.0.0.1:${upstreamPort}`
  const post = async (path: string, body = {}) => {
    const response = await fetch(endpoint + path, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) })
    assert(response.ok, await response.text())
  }
  preview.onData(data => { void post('/input', { data }) })
  let polling = false
  setInterval(async () => {
    if (polling) return
    polling = true
    try {
      const { output } = await fetch(endpoint + '/output').then(response => response.json())
      if (output) await new Promise<void>(resolve => preview.write(output, resolve))
    } finally { polling = false }
  }, 100)
  Object.assign(window, { dshImageSmoke: {
    control: (action: string) => post('/' + action),
    state: () => {
      let imageCells = 0
      for (let y = 0; y < preview.rows; y++) for (let x = 0; x < preview.cols; x++) {
        if (previewImages.getImageAtBufferCell(x, y)) imageCells++
      }
      return { imageCells, storage: previewImages.storageUsage, displayed: layerHasPixels(),
        text: Array.from({ length: preview.rows }, (_, y) => preview.buffer.active.getLine(y)?.translateToString(true)).join('\n') }
    }
  } })
  await post('/start')
} else {
  preview.write('SIXEL preview: red / green bands\r\n\x1bP0;1;0q"1;1;240;120#1;2;100;0;0#1' +
    Array(10).fill('!240~-').join('') + '#2;2;0;100;0#2' + Array(10).fill('!240~-').join('') + '\x1b\\')
}
