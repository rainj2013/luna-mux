import { DEFAULT_TERMINAL_SETTINGS, type TerminalBackgroundFit, type TerminalSettings } from '../shared/types'

const colorPattern = /^#[0-9a-fA-F]{6}$/
const backgroundFits: TerminalBackgroundFit[] = ['cover', 'contain', 'stretch', 'tile']

function finiteNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

export function sanitizeTerminalSettings(value: unknown): TerminalSettings {
  const input = value && typeof value === 'object' ? value as Partial<TerminalSettings> : {}
  const fontFamily = typeof input.fontFamily === 'string' ? input.fontFamily.trim().slice(0, 200) : ''
  const foregroundColor = typeof input.foregroundColor === 'string' && colorPattern.test(input.foregroundColor) ? input.foregroundColor : DEFAULT_TERMINAL_SETTINGS.foregroundColor
  const backgroundColor = typeof input.backgroundColor === 'string' && colorPattern.test(input.backgroundColor) ? input.backgroundColor : DEFAULT_TERMINAL_SETTINGS.backgroundColor
  const backgroundImagePath = typeof input.backgroundImagePath === 'string' ? input.backgroundImagePath.trim().slice(0, 4096) : ''
  return {
    fontFamily: fontFamily || DEFAULT_TERMINAL_SETTINGS.fontFamily,
    fontSize: Math.round(Math.min(30, Math.max(10, finiteNumber(input.fontSize, DEFAULT_TERMINAL_SETTINGS.fontSize)))),
    foregroundColor,
    backgroundColor,
    backgroundOpacity: Math.min(1, Math.max(0, finiteNumber(input.backgroundOpacity, DEFAULT_TERMINAL_SETTINGS.backgroundOpacity))),
    backgroundImagePath,
    backgroundImageFit: backgroundFits.includes(input.backgroundImageFit as TerminalBackgroundFit) ? input.backgroundImageFit as TerminalBackgroundFit : DEFAULT_TERMINAL_SETTINGS.backgroundImageFit
  }
}
