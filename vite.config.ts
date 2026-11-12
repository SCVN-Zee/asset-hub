import { cpSync, createReadStream, existsSync, statSync } from 'fs'
import { join, normalize, resolve } from 'path'
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

/**
 * The Luna runtime files live in `<repo>/vendor/luna` and are loaded by the Luna host page only. They are never bundled:
 * Vite serves them at `/luna/<version>/` in dev and copies the folder into `dist/luna/` at build.
 */
function lunaRuntime(): Plugin {
  const source = resolve('vendor/luna')
  let outDir = resolve('dist')
  return {
    name: 'luna-runtime',
    configResolved(config) {
      outDir = resolve(config.root, config.build.outDir)
    },
    configureServer(server) {
      server.middlewares.use('/luna/', (req, res, next) => {
        const rel = normalize(decodeURIComponent((req.url ?? '/').split('?')[0]))
        const file = join(source, rel)
        if (!file.startsWith(source + '/') || !existsSync(file) || !statSync(file).isFile()) return next()
        res.setHeader('Content-Type', file.endsWith('.json') ? 'application/json' : 'text/javascript')
        createReadStream(file).pipe(res)
      })
    },
    closeBundle() {
      if (existsSync(source)) cpSync(source, join(outDir, 'luna'), { recursive: true })
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
  plugins: [react(), dropWoff(), lunaRuntime()],
  clearScreen: false,
  server: { port: 5173, strictPort: true, host: 'localhost' },
  build: {
    outDir: resolve('dist'),
    emptyOutDir: true,
    target: 'safari16',
    // The second page runs the Luna runtime in an iframe, so its globals stay out of the app window.
    rollupOptions: { input: { main: resolve('ui/index.html'), 'luna-host': resolve('ui/luna-host.html') } }
  }
})
