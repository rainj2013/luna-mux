import type { Terminal } from '@xterm/xterm'
import { UnicodeGraphemesAddon } from '@xterm/addon-unicode-graphemes'

/** Configure cell widths before parsing output, including restored snapshots. */
export function configureTerminalUnicode(term: Terminal): void {
  term.loadAddon(new UnicodeGraphemesAddon())
}
