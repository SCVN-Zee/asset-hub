import { defineConfig, devices } from '@playwright/test'

const PORT = 5199

export default defineConfig({
  testDir: 'e2e',
  testMatch: '**/*.spec.ts',
  outputDir: 'e2e/test-results',
  workers: 1,
  timeout: 90_000,
  reporter: 'list',
  use: { ...devices['Desktop Chrome'], baseURL: `http://localhost:${PORT}`, headless: true, viewport: { width: 1400, height: 900 } },
  webServer: {
    command: 'bun x vite --config e2e/vite.config.ts',
    url: `http://localhost:${PORT}/e2e/harness.html`,
    reuseExistingServer: false,
    timeout: 120_000
  }
})
