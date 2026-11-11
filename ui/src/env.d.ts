/// <reference types="vite/client" />
import type { api } from './api'

declare global {
  /** package.json "version", filled in at build time. */
  const __APP_VERSION__: string
  interface Window {
    api: typeof api
  }
}
