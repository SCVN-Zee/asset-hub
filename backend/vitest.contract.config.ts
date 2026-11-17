import { defineConfig } from 'vitest/config'

// Black-box HTTP suite. It runs the bundled Worker in Miniflare, so it needs no Workers pool.
export default defineConfig({
  test: {
    environment: 'node',
    include: ['test/contract/**/*.test.ts'],
    globalSetup: ['./test/contract/global-setup.ts'],
    fileParallelism: false,
    testTimeout: 30_000,
    hookTimeout: 60_000
  }
})
