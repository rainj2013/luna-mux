const bracketedPasteStart = '\x1b[200~'
const bracketedPasteEnd = '\x1b[201~'
const launchProfileAdapters: Record<string, string> = {
  'codex.default': 'codex',
  'claude-code.default': 'claude-code',
  'grok-build.default': 'grok-build',
  'codex.auto': 'codex',
  'claude-code.auto': 'claude-code',
  'grok-build.auto': 'grok-build'
}

export type AgentImagePasteMode = 'control-v' | 'alt-v'

const defaultImagePasteMode: AgentImagePasteMode = 'control-v'
const imagePasteModes: Record<string, AgentImagePasteMode> = {
  codex: 'control-v',
  'claude-code': 'alt-v',
  'grok-build': 'control-v'
}

export function agentAdapterId(adapterId: string | undefined, launchProfileId: string): string | undefined {
  return adapterId ?? launchProfileAdapters[launchProfileId]
}

/** Return the terminal sequence expected by the active Agent for an image paste. */
export function agentImagePasteInput(adapterId: string | undefined, platform: string): string {
  const mode = imagePasteModes[adapterId ?? ''] ?? defaultImagePasteMode
  // Claude Code uses Alt+V on Windows; terminal emulators encode that as ESC + v.
  if (mode === 'alt-v' && platform === 'win32') return '\x1bv'
  return '\x16'
}

export function codexMultilinePastePayload(text: string): string | undefined {
  if (!/[\r\n]/.test(text)) return undefined
  const normalized = text.replace(/\r\n|\r|\n/g, '\r')
  return `${bracketedPasteStart}${normalized}${bracketedPasteEnd}`
}

export function routeTerminalPaste(
  text: string,
  codexTui: boolean,
  writeDirect: (payload: string) => void,
  pasteWithTerminal: (payload: string) => void
): 'direct' | 'terminal' {
  const codexPayload = codexTui ? codexMultilinePastePayload(text) : undefined
  if (codexPayload !== undefined) {
    writeDirect(codexPayload)
    return 'direct'
  }
  pasteWithTerminal(text)
  return 'terminal'
}

export function handleCodexMultilinePasteEvent(
  text: string | undefined,
  codexTui: boolean,
  preventDefault: () => void,
  stopImmediatePropagation: () => void,
  writeDirect: (payload: string) => void
): boolean {
  const payload = text !== undefined && codexTui ? codexMultilinePastePayload(text) : undefined
  if (payload === undefined) return false
  preventDefault()
  stopImmediatePropagation()
  writeDirect(payload)
  return true
}
