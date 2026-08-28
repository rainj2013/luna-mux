const bracketedPasteStart = '\x1b[200~'
const bracketedPasteEnd = '\x1b[201~'
const defaultCodexLaunchProfileId = 'codex.default'

export function agentAdapterId(adapterId: string | undefined, launchProfileId: string): string | undefined {
  return adapterId ?? (launchProfileId === defaultCodexLaunchProfileId ? 'codex' : undefined)
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
