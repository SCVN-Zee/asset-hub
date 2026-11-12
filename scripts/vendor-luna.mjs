#!/usr/bin/env node
// Copies the Luna runtime subset that the Luna preview host loads from a Luna package into
// vendor/luna/<version>/, unchanged, and writes VERSION and SHA256SUMS.
// Usage: bun scripts/vendor-luna.mjs <luna-package-path>
import { createHash } from 'node:crypto'
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/** Files copied from the package, relative to the package root. Keep in step with ui/src/luna/host/boot.ts. */
const VENDOR_FILES = [
  'config.json',
  'pipeline/templates/html/resources/js/pi.js',
  ...[
    'manifest.json',
    'script-1.js',
    'script1.js',
    'script3.js',
    'particle-system-1.js',
    'urp-1-stub.js',
    'physics3d-0-stub.js',
    'physics3d-1-stub.js',
    'physics2d-1-stub.js'
  ].map((f) => `engine/luna/${f}`),
  ...['bridge.js', 'Bridge.Locales.js', 'UnityEngine.js', 'UnityEngine.UI.js', 'UnityEngine.UniversalRenderPipeline.js', 'TextMeshPro.js'].map(
    (f) => `pipeline/templates/LunaCompiler/bin/${f}`
  )
]

/** Optional files: copied when the package has them. */
const OPTIONAL_FILES = ['pipeline/templates/LunaCompiler/bin/manifest.json']

const sha256 = (file) => createHash('sha256').update(readFileSync(file)).digest('hex')

const source = process.argv[2]
if (!source) {
  console.error('Usage: vendor-luna.mjs <luna-package-path>')
  process.exit(2)
}
const pkg = resolve(source)
const configPath = join(pkg, 'config.json')
if (!existsSync(configPath)) {
  console.error(`Not a Luna package: ${configPath} is missing.`)
  process.exit(1)
}
const version = JSON.parse(readFileSync(configPath, 'utf8')).version
if (typeof version !== 'string' || !/^\d+\.\d+\.\d+$/.test(version)) {
  console.error(`config.json has no usable "version" (got ${JSON.stringify(version)}).`)
  process.exit(1)
}
const files = [...VENDOR_FILES, ...OPTIONAL_FILES.filter((f) => existsSync(join(pkg, f)))]
const missing = files.filter((f) => !existsSync(join(pkg, f)))
if (missing.length > 0) {
  console.error(`The package lacks files the preview needs:\n  ${missing.join('\n  ')}`)
  process.exit(1)
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const out = join(root, 'vendor', 'luna', version)
rmSync(out, { recursive: true, force: true })
for (const rel of files) {
  const to = join(out, rel)
  mkdirSync(dirname(to), { recursive: true })
  cpSync(join(pkg, rel), to)
}
const sums = files
  .slice()
  .sort()
  .map((rel) => `${sha256(join(out, rel))}  ${rel}`)
  .join('\n')
writeFileSync(join(out, 'SHA256SUMS'), sums + '\n')
writeFileSync(join(out, 'VERSION'), version + '\n')
const bytes = files.reduce((n, rel) => n + statSync(join(out, rel)).size, 0)
console.log(`Vendored Luna ${version}: ${files.length} files, ${(bytes / 1048576).toFixed(1)} MB -> ${out}`)
