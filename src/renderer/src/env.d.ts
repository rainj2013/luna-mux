import type { AppApi } from '../../shared/types'

declare global { interface Window { api: AppApi } }
export {}
