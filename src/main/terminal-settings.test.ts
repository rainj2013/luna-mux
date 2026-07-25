import { describe, expect, it } from 'vitest'
import { DEFAULT_TERMINAL_SETTINGS } from '../shared/types'
import { sanitizeTerminalSettings } from './terminal-settings'

describe('terminal settings', () => {
  it('accepts supported appearance values', () => {
    expect(sanitizeTerminalSettings({
      fontFamily: 'JetBrains Mono, monospace', fontSize: 18,
      foregroundColor: '#abcdef', backgroundColor: '#123456', backgroundOpacity: 0.45,
      backgroundImagePath: '/tmp/background.png', backgroundImageFit: 'contain'
    })).toEqual({
      fontFamily: 'JetBrains Mono, monospace', fontSize: 18,
      foregroundColor: '#abcdef', backgroundColor: '#123456', backgroundOpacity: 0.45,
      backgroundImagePath: '/tmp/background.png', backgroundImageFit: 'contain'
    })
  })

  it('bounds numbers and replaces malformed values', () => {
    expect(sanitizeTerminalSettings({
      fontFamily: '  ', fontSize: 99,
      foregroundColor: 'red', backgroundColor: '#123', backgroundOpacity: -2,
      backgroundImageFit: 'unknown'
    })).toMatchObject({
      fontFamily: DEFAULT_TERMINAL_SETTINGS.fontFamily,
      fontSize: 30,
      foregroundColor: DEFAULT_TERMINAL_SETTINGS.foregroundColor,
      backgroundColor: DEFAULT_TERMINAL_SETTINGS.backgroundColor,
      backgroundOpacity: 0,
      backgroundImageFit: DEFAULT_TERMINAL_SETTINGS.backgroundImageFit
    })
  })
})
