import type { Terminal } from '@xterm/xterm'
import type { SerializeAddon } from '@xterm/addon-serialize'

interface TerminalMouseInternals {
  _core?: { coreMouseService?: { activeEncoding?: string } }
}

/** xterm's serializer restores mouse tracking but omits the mouse encoding. */
export function serializeTerminalSnapshot(term: Terminal, serializer: SerializeAddon): string {
  const serialized = serializer.serialize({ scrollback: 5000 })
  const encoding = (term as unknown as TerminalMouseInternals)._core?.coreMouseService?.activeEncoding
  if (encoding === 'SGR') return `${serialized}\x1b[?1006h`
  if (encoding === 'SGR_PIXELS') return `${serialized}\x1b[?1016h`
  return serialized
}
