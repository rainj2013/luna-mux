import messages from './zh-CN.messages'
import { createChineseHelpSections } from './help-content'
import { defineLocale } from './types'

export default defineLocale({
  meta: { code: 'zh-CN', label: '简体中文', order: 10 },
  messages,
  helpSections: createChineseHelpSections
})
