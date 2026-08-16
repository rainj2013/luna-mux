import type { HelpSection } from './help-content'

export interface LocaleMeta {
  code: string
  label: string
  order: number
}

export interface LocaleDefinition {
  meta: LocaleMeta
  messages: Record<string, string>
  helpSections(commandKey: string): HelpSection[]
}

export function defineLocale<const T extends LocaleDefinition>(locale: T): T {
  return locale
}
