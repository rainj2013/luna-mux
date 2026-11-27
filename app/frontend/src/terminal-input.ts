const bracketedPasteStart = '\x1b[200~'
const bracketedPasteEnd = '\x1b[201~'
export const codexInsertNewline = '\x1b\r'
const launchProfileAdapters: Record<string, string> = {
  'dsh-tui.default': 'dsh-tui',
  'dsh-tui.auto': 'dsh-tui',
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

export type CodexPasteMode = 'windows' | 'bracketed'

export function codexPasteMode(adapterId: string | undefined, platform: string, targetId: string): CodexPasteMode | undefined {
  if (adapterId !== 'codex') return undefined
  return platform === 'win32' && (targetId === 'local:powershell' || targetId === 'local:powershell5') ? 'windows' : 'bracketed'
}

export function codexPastePayload(text: string, mode: CodexPasteMode): string | undefined {
  const sanitized = text.replace(/\r\n|[\r\n\u0085\u2028\u2029]/g, '\n').replace(/[\x00-\x08\x0b\x0c\x0e-\x1f\x7f-\x9f]/g, '')
  if (mode === 'windows') {
    const payload = sanitized.replace(/\n/g, codexInsertNewline).replace(/\t/g, '    ')
    return payload === text ? undefined : payload
  }
  if (!sanitized.includes('\n')) return sanitized === text ? undefined : sanitized
  return `${bracketedPasteStart}${sanitized.replace(/\n/g, '\r')}${bracketedPasteEnd}`
}

/**
 * Codex treats Ctrl+M as an editor newline, while a terminal normally emits
 * the same carriage return as Enter.  ESC+CR is the portable Alt+Enter form
 * that Codex accepts without submitting the draft.
 */
export function codexControlMInput(codexTui: boolean): string | undefined {
  return codexTui ? codexInsertNewline : undefined
}

export function routeTerminalPaste(
  text: string,
  mode: CodexPasteMode | undefined,
  writeDirect: (payload: string) => void,
  pasteWithTerminal: (payload: string) => void
): 'direct' | 'terminal' {
  const codexPayload = mode ? codexPastePayload(text, mode) : undefined
  if (codexPayload !== undefined) {
    if (codexPayload) writeDirect(codexPayload)
    return 'direct'
  }
  pasteWithTerminal(text)
  return 'terminal'
}

export function handleCodexPasteEvent(
  text: string | undefined,
  mode: CodexPasteMode | undefined,
  preventDefault: () => void,
  stopImmediatePropagation: () => void,
  writeDirect: (payload: string) => void
): boolean {
  const payload = text !== undefined && mode ? codexPastePayload(text, mode) : undefined
  if (payload === undefined) return false
  preventDefault()
  stopImmediatePropagation()
  if (payload) writeDirect(payload)
  return true
}
