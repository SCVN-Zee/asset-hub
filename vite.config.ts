import { resolve } from 'path'
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import type { Plugin } from 'vite'
import { version } from './package.json'

/** Fonts come as .woff2 and .woff; the system web views all take the .woff2, so the .woff files are left out. */
function dropWoff(): Plugin {
  return {
    name: 'drop-woff',
    apply: 'build',
    generateBundle(_options, bundle) {
      for (const name of Object.keys(bundle)) if (name.endsWith('.woff')) delete bundle[name]
    }
  }
}

// The page of the Tauri shell.
export default defineConfig({
  root: resolve('ui'),
  resolve: {
    alias: {
      '@shared': resolve('ui/shared'),
      '@renderer': resolve('ui/src')
    }
  },
  define: { __APP_VERSION__: JSON.stringify(version) },
  plugins: [react(), dropWoff()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, host: 'localhost' },
  build: { outDir: resolve('dist'), emptyOutDir: true, target: 'safari16' }
})
