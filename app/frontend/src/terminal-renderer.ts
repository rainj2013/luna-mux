import type { Platform } from './types'

/**
 * WKWebView's WebGL path can leave xterm's glyph atlas or live context in a
 * corrupted state, especially on Intel Macs. The visible symptom is correct
 * cell geometry with unrelated glyphs until a resize forces a full redraw.
 * Keep macOS on xterm's DOM renderer until the upstream atlas invalidation and
 * context-release fixes are available in a stable, matched xterm release.
 */
export function shouldUseWebglRenderer(platform: Platform, focused: boolean): boolean {
  return focused && platform !== 'darwin'
}
