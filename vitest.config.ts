import { resolve } from 'path'
import { defineConfig } from 'vitest/config'

export default defineConfig({
  resolve: {
    alias: {
      '@shared': resolve('ui/shared')
    }
  },
  test: {
    include: ['ui/**/*.test.ts', 'scripts/*.test.mjs']
  }
})
