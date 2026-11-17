// Serves a bundled Rust Worker on a fixed local port, with in-memory D1 databases and a fake GitHub that
// grants push access to every token. It lets scripts/cpu-probe.mjs run end to end with no real service.
//
// Build a bundle without deploying:
//   npx wrangler deploy --dry-run -c wrangler.rust.toml --outdir <dir>
// Run (stops on SIGTERM or Ctrl-C):
//   node scripts/serve-local-worker.mjs <dir> [--port 8799]
// Then:
//   ASSETHUB_PROBE_TOKEN=local node scripts/cpu-probe.mjs --base http://127.0.0.1:8799 --allow-thumbs --routes health,thumbs-put,thumbs-get
//
// The cache routes need B2 and the popular route needs Upstash, so a probe of those routes fails here.
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import process, { argv, exit } from 'node:process'
import { fileURLToPath } from 'node:url'
import { Miniflare } from 'miniflare'

const backendDir = fileURLToPath(new URL('../', import.meta.url))
const dir = argv[2]
const portAt = argv.indexOf('--port')
const port = portAt >= 0 ? Number(argv[portAt + 1]) : 8799
if (!dir || dir.startsWith('--') || !Number.isInteger(port)) {
  console.error('usage: node scripts/serve-local-worker.mjs <bundle dir> [--port 8799]')
  exit(2)
}

const sqlOf = (folder) =>
  readdirSync(join(backendDir, folder))
    .filter((f) => f.endsWith('.sql'))
    .sort()
    .flatMap((f) =>
      readFileSync(join(backendDir, folder, f), 'utf8')
        .split(/;\s*(?:\n|$)/)
        .map((s) => s.trim())
        .filter(Boolean)
    )

const files = readdirSync(dir)
const entry = ['shim.mjs', 'shim.js', 'index.js'].find((f) => files.includes(f))
if (!entry) {
  console.error(`no entry module in ${dir}`)
  exit(2)
}
const modules = [entry, ...files.filter((f) => f !== entry && /\.(m?js|wasm)$/.test(f))].map((f) =>
  f.endsWith('.wasm')
    ? { type: 'CompiledWasm', path: join(dir, f), contents: readFileSync(join(dir, f)) }
    : { type: 'ESModule', path: join(dir, f), contents: readFileSync(join(dir, f), 'utf8') }
)

const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const mf = new Miniflare({
  modules,
  modulesRoot: dir,
  compatibilityDate: '2026-03-01',
  host: '127.0.0.1',
  port,
  d1Databases: { DB: 'local-db', ...Object.fromEntries(Array.from({ length: 8 }, (_, i) => [`THUMBS_${i}`, `local-thumbs-${i}`])) },
  bindings: {
    ALLOWED_OWNERS: 'supercent-io',
    B2_ENDPOINT: 'https://b2.invalid',
    B2_REGION: 'r',
    B2_BUCKET: 'b',
    B2_KEY_ID: 'k',
    B2_APP_KEY: 'k',
    UPSTASH_REDIS_REST_URL: 'https://upstash.invalid',
    UPSTASH_REDIS_REST_TOKEN: 't'
  },
  outboundService: (req) => (new URL(req.url).pathname === '/user' ? json({ login: 'dev' }) : json({ permissions: { pull: true, push: true } }))
})

try {
  await mf.ready
  const main = await mf.getD1Database('DB')
  for (const sql of sqlOf('migrations')) await main.prepare(sql).run()
  for (let i = 0; i < 8; i++) {
    const shard = await mf.getD1Database(`THUMBS_${i}`)
    for (const sql of sqlOf('migrations-thumbs')) await shard.prepare(sql).run()
  }
} catch (err) {
  console.error('start failed:', err instanceof Error ? err.message : err)
  await mf.dispose()
  exit(1)
}
console.log(`ready http://127.0.0.1:${port}`)
const stop = async () => {
  await mf.dispose()
  exit(0)
}
process.on('SIGTERM', stop)
process.on('SIGINT', stop)
setInterval(() => {}, 1 << 30)
