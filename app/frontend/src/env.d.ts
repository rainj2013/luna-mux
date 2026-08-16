import type { AppApi } from './types'

declare global {
  interface Window {
    api: AppApi
  }
}
export {}
