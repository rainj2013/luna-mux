// Runtime-scoped Cordis contribution. No dsh internals or user-profile writes.
import { spawn } from 'node:child_process'
import { mkdtempSync, openSync, closeSync, statSync, readFileSync, rmSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
export const name = 'luna-mux-dsh-tui'
export const inject = ['systemPrompt']

export function apply(ctx, config) {
  ctx.systemPrompt.section({
    name: 'luna-mux-routing', order: 10000,
    text: config.instructions, interpolate: false,
  })
  // Keep Browser Resource ownership and lazy startup identical to other adapters.
  // This observer never answers an approval request.
  ctx.on('tools/pre-execute', async (exec, next) => {
    if (!exec.name.startsWith('mcp__agent_browser__') && typeof exec.arguments?.command !== 'string') return next()
    const payload = { hook_event_name: 'PreToolUse', tool_name: exec.name, tool_input: exec.arguments }
    let result
    if (config.hookCommand) {
      // WSL loopback is not Windows loopback. Use the existing native forwarder.
      result = await forwardNative(config.hookCommand, payload, exec.signal)
    } else {
      const response = await fetch(process.env.LUNA_MUX_HOOK_ENDPOINT, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          Authorization: `Bearer ${process.env.LUNA_MUX_HOOK_AUTHORIZATION}`,
          'X-Luna-Mux-Agent-Adapter': 'dsh-tui',
          'X-Luna-Mux-Agent-Process-Id': process.env.LUNA_MUX_AGENT_PROCESS_ID,
        },
        body: JSON.stringify(payload),
        signal: exec.signal
          ? AbortSignal.any([exec.signal, AbortSignal.timeout(25000)])
          : AbortSignal.timeout(25000),
      })
      if (!response.ok) return { kind: 'deny', reason: 'Luna Mux Browser integration is unavailable' }
      result = await response.json()
    }
    if (result.hookSpecificOutput?.permissionDecision === 'deny') {
      return { kind: 'deny', reason: result.hookSpecificOutput.permissionDecisionReason ?? 'Luna Mux Browser operation refused' }
    }
    return next()
  })
}

async function forwardNative(command, payload, signal) {
  const directory = mkdtempSync(join(tmpdir(), 'luna-mux-dsh-hook-'))
  const path = join(directory, 'response.json')
  const fd = openSync(path, 'wx', 0o600)
  try {
    const child = spawn(command[0], command.slice(1), { stdio: ['pipe', fd, 'ignore'], windowsHide: true })
    child.stdin.on('error', () => {})
    child.stdin.end(JSON.stringify(payload) + '\n')
    await new Promise((resolve, reject) => {
      const abort = () => { child.kill(); reject(new Error('Luna Mux Browser hook cancelled')) }
      const timer = setTimeout(abort, 25000)
      signal?.addEventListener('abort', abort, { once: true })
      const cleanup = () => { clearTimeout(timer); signal?.removeEventListener('abort', abort) }
      child.once('error', () => { cleanup(); reject(new Error('Luna Mux Browser hook unavailable')) })
      child.once('exit', code => { cleanup(); code === 0 ? resolve() : reject(new Error('Luna Mux Browser hook failed')) })
      if (signal?.aborted) abort()
    })
    // A grandchild cannot hold an inherited stdout pipe open: output is a file.
    if (statSync(path).size > 65536) throw new Error('Luna Mux Browser hook response too large')
    const text = readFileSync(path, 'utf8').trim()
    return text ? JSON.parse(text) : {}
  } finally {
    closeSync(fd)
    rmSync(directory, { recursive: true, force: true })
  }
}
