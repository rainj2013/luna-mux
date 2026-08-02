import type { CSSProperties } from 'react'
import type { TerminalSettings } from './types'

export function colorWithOpacity(color: string, opacity: number): string {
  const red = Number.parseInt(color.slice(1, 3), 16)
  const green = Number.parseInt(color.slice(3, 5), 16)
  const blue = Number.parseInt(color.slice(5, 7), 16)
  return `rgba(${red}, ${green}, ${blue}, ${opacity})`
}

export function terminalBackgroundStyle(settings: TerminalSettings, imageDataUrl: string): CSSProperties {
  if (!settings.backgroundImagePath || !imageDataUrl) return {}
  if (settings.backgroundImageFit === 'tile') {
    return { backgroundImage: `url("${imageDataUrl}")`, backgroundPosition: 'left top', backgroundRepeat: 'repeat', backgroundSize: 'auto' }
  }
  return {
    backgroundImage: `url("${imageDataUrl}")`,
    backgroundPosition: 'center',
    backgroundRepeat: 'no-repeat',
    backgroundSize: settings.backgroundImageFit === 'stretch' ? '100% 100%' : settings.backgroundImageFit
  }
}
