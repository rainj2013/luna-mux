export interface FieldEdit {
  value: string
  caret: number
}

function clamp(offset: number, max: number): number {
  return Number.isFinite(offset) ? Math.min(Math.max(offset, 0), max) : max
}

// Pasted text replaces the current selection and the caret lands after it. The
// offsets come from a field that may have lost focus, so they are clamped
// instead of trusted.
export function pasteAtCaret(value: string, start: number, end: number, text: string): FieldEdit {
  const from = clamp(Math.min(start, end), value.length)
  const to = clamp(Math.max(start, end), value.length)
  return { value: value.slice(0, from) + text + value.slice(to), caret: from + text.length }
}
