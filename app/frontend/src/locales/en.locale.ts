import enMessages from './en.messages'
import zhCNMessages from './zh-CN.messages'
import { createEnglishHelpSections } from './help-content'
import { defineLocale } from './types'

const messages: Record<keyof typeof zhCNMessages, string> = enMessages

export default defineLocale({
  meta: { code: 'en', label: 'English', order: 20 },
  messages,
  helpSections: createEnglishHelpSections
})
