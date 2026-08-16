import { createContext, useContext, useEffect, useMemo, type ReactNode } from 'react'
import type { AppLanguage, NativeMenuLabels } from './types'
import zhCNMessages from './locales/zh-CN.messages'
import type { LocaleDefinition } from './locales/types'
import type { HelpSection } from './locales/help-content'
import { PRODUCT_INFO } from './product-info'

export type MessageKey = keyof typeof zhCNMessages
export type MessageParams = Record<string, string | number>

type LoadedLocale = LocaleDefinition & { messages: Record<MessageKey, string> }
type LocaleModule = { default: LoadedLocale }

const modules = import.meta.glob<LocaleModule>('./locales/*.locale.ts', { eager: true })
const localeMap = new Map<string, LoadedLocale>()
for (const module of Object.values(modules)) localeMap.set(module.default.meta.code, module.default)

const fallbackLanguage = 'zh-CN'
const fallbackLocale: LoadedLocale = (() => {
  const locale = localeMap.get(fallbackLanguage)
  if (!locale) throw new Error(`Missing fallback locale: ${fallbackLanguage}`)
  return locale
})()

export const availableLanguages = [...localeMap.values()]
  .map((locale) => locale.meta)
  .sort((left, right) => left.order - right.order)

export function resolveLanguage(language: string): AppLanguage {
  const normalized = language === 'zhCn' ? 'zh-CN' : language
  return localeMap.has(normalized) ? normalized : fallbackLanguage
}

export function getLocale(language: AppLanguage): LoadedLocale {
  return localeMap.get(resolveLanguage(language)) ?? fallbackLocale
}

function interpolate(message: string, params?: MessageParams): string {
  if (!params) return message
  return message.replace(/\{\{\s*([A-Za-z0-9_]+)\s*\}\}/g, (placeholder, key: string) => {
    const value = params[key]
    return value === undefined ? placeholder : String(value)
  })
}

export function translate(language: AppLanguage, key: MessageKey, params?: MessageParams): string {
  const locale = getLocale(language)
  return interpolate(locale.messages[key] ?? fallbackLocale.messages[key] ?? key, params)
}

export function getNativeMenuLabels(language: AppLanguage): NativeMenuLabels {
  const t = (key: MessageKey, params?: MessageParams): string => translate(language, key, params)
  const productName = { productName: PRODUCT_INFO.displayName }
  return {
    settings: t('native.settings'), newConnection: t('native.newConnection'), importOpenSshConfig: t('native.importOpenSshConfig'),
    newSession: t('native.newSession'), closeTab: t('native.closeTab'), terminal: t('native.terminal'), files: t('native.files'),
    toggleSidebar: t('native.toggleSidebar'), helpItem: t('native.helpItem', productName), about: t('native.about', productName), services: t('native.services'),
    hide: t('native.hide', productName), hideOthers: t('native.hideOthers'), showAll: t('native.showAll'), quit: t('native.quit', productName),
    connectionMenu: t('native.connectionMenu'), editMenu: t('native.editMenu'), undo: t('native.undo'), redo: t('native.redo'),
    cut: t('native.cut'), copy: t('native.copy'), paste: t('native.paste'), selectAll: t('native.selectAll'), viewMenu: t('native.viewMenu'),
    fullscreen: t('native.fullscreen'), windowMenu: t('native.windowMenu'), minimize: t('native.minimize'), zoom: t('native.zoom'),
    bringAllToFront: t('native.bringAllToFront'), helpMenu: t('native.helpMenu')
  }
}

interface I18nValue {
  language: AppLanguage
  setLanguage(language: AppLanguage): void
  t(key: MessageKey, params?: MessageParams): string
  helpSections(commandKey: string): HelpSection[]
}

const I18nContext = createContext<I18nValue | null>(null)

export function I18nProvider({ language, setLanguage, children }: { language: AppLanguage; setLanguage(language: AppLanguage): void; children: ReactNode }): React.JSX.Element {
  const resolvedLanguage = resolveLanguage(language)
  useEffect(() => {
    document.documentElement.lang = resolvedLanguage
  }, [resolvedLanguage])
  const value = useMemo<I18nValue>(() => ({
    language: resolvedLanguage,
    setLanguage,
    t: (key, params) => translate(resolvedLanguage, key, params),
    helpSections: getLocale(resolvedLanguage).helpSections
  }), [resolvedLanguage, setLanguage])
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>
}

export function useI18n(): I18nValue {
  const value = useContext(I18nContext)
  if (!value) throw new Error('useI18n must be used within I18nProvider')
  return value
}
