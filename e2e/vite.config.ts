import { resolve } from 'path'
import { defineConfig, mergeConfig } from 'vite'
import base from '../vite.config'

// The app's own config, served from the repo root so the harness page can sit in e2e/.
// Its own port and cache folder keep it apart from a running `tauri dev`.
export default mergeConfig(
  base,
  defineConfig({
    root: resolve('.'),
    cacheDir: resolve('e2e/.vite-cache'),
    server: { port: 5199, strictPort: true, host: 'localhost' },
    optimizeDeps: { entries: ['e2e/harness.html'] }
  })
)
