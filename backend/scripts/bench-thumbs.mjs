// Local micro-benchmark of the thumbnail routes: runs a bundled Rust Worker in Miniflare with in-memory
// D1 databases and a fake GitHub, and times many sequential requests.
//
// This is a rough proxy only. It measures wall time on this machine, not the CPU time that Workers Free
// limits, and Miniflare's D1 does not copy bytes through the production binding. Use it to compare two
// builds on one machine, never to predict production CPU.
//
// Build a bundle without deploying:
//   npx wrangler deploy --dry-run -c wrangler.rust.toml --outdir <dir>
// Run:
//   node scripts/bench-thumbs.mjs <dir> [--n 60] [--warmup 10]
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { argv, exit } from 'node:process'
import { fileURLToPath } from 'node:url'
import { Miniflare } from 'miniflare'

const backendDir = fileURLToPath(new URL('../', import.meta.url))
const args = argv.slice(2)
const dir = args.find((a) => !a.startsWith('--') && !/^\d+$/.test(a))
const flag = (name, fallback) => {
  const at = args.indexOf(name)
  return at >= 0 ? Number(args[at + 1]) : fallback
}
if (!dir) {
  console.error('usage: node scripts/bench-thumbs.mjs <bundle dir> [--n 60] [--warmup 10]')
  exit(2)
}
const N = flag('--n', 60)
const WARMUP = flag('--warmup', 10)
const SHARDS = 8
const TOKEN = 'bench-token'
const REPO = '/v1/repos/supercent-io/game'

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

function modules(folder) {
  const files = readdirSync(folder)
  const entry = ['index.js', 'shim.mjs', 'shim.js'].find((f) => files.includes(f))
  if (!entry) throw new Error(`no entry module in ${folder}`)
  return [entry, ...files.filter((f) => f !== entry && /\.(m?js|wasm)$/.test(f))].map((f) =>
    f.endsWith('.wasm')
      ? { type: 'CompiledWasm', path: join(folder, f), contents: readFileSync(join(folder, f)) }
      : { type: 'ESModule', path: join(folder, f), contents: readFileSync(join(folder, f), 'utf8') }
  )
}

const json = (body, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
const github = (req) => {
  const { pathname } = new URL(req.url)
  return pathname === '/user' ? json({ login: 'dev' }) : json({ permissions: { pull: true, push: true } })
}

const mf = new Miniflare({
  modules: modules(dir),
  compatibilityDate: '2026-03-01',
  d1Databases: { DB: 'bench-db', ...Object.fromEntries(Array.from({ length: SHARDS }, (_, i) => [`THUMBS_${i}`, `bench-thumbs-${i}`])) },
  bindings: { ALLOWED_OWNERS: 'supercent-io', B2_ENDPOINT: 'https://b2.invalid', B2_REGION: 'r', B2_BUCKET: 'b', B2_KEY_ID: 'k', B2_APP_KEY: 'k', UPSTASH_REDIS_REST_URL: 'https://u.invalid', UPSTASH_REDIS_REST_TOKEN: 't' },
  modulesRoot: dir,
  outboundService: github
})

async function migrate() {
  const main = await mf.getD1Database('DB')
  for (const sql of sqlOf('migrations')) await main.prepare(sql).run()
  for (let i = 0; i < SHARDS; i++) {
    const shard = await mf.getD1Database(`THUMBS_${i}`)
    for (const sql of sqlOf('migrations-thumbs')) await shard.prepare(sql).run()
  }
}

const hex = (i) => (i + 4096).toString(16).padStart(40, '0')
const nameOf = (i) => `${(i % 16).toString(16)}${hex(i).slice(1)}`
const content = (i, size) => Uint8Array.from({ length: size }, (_, k) => (k * 31 + i) % 256)

function encodePack(entries) {
  const parts = entries.flatMap(({ name, bytes }) => {
    const head = Buffer.alloc(6 + name.length)
    head.writeUInt16BE(name.length, 0)
    head.write(name, 2, 'utf8')
    head.writeUInt32BE(bytes.length, 2 + name.length)
    return [head, Buffer.from(bytes)]
  })
  return Buffer.concat(parts)
}

const typicalSize = (i) => 3000 + ((i * 937) % 6000)
const typical = (count) => Array.from({ length: count }, (_, i) => ({ name: nameOf(i), bytes: content(i, typicalSize(i)) }))
const maxProfile = () => Array.from({ length: 20 }, (_, i) => ({ name: nameOf(100 + i), bytes: content(i, i === 3 ? 62 * 1024 : i === 11 ? 256 * 1024 : typicalSize(i)) }))

const call = (path, body, headers) => mf.dispatchFetch(`http://worker.test${REPO}${path}`, { method: 'POST', body, headers: { authorization: `Bearer ${TOKEN}`, 'x-asset-hub-schema': '998', ...headers } })

const put = (entries) => async () => {
  const res = await call('/thumbs/put', encodePack(entries), { 'content-type': 'application/octet-stream' })
  if (res.status !== 200) throw new Error(`put gave ${res.status}`)
  await res.arrayBuffer()
}
const get = (names, bytesExpected) => async () => {
  const res = await call('/thumbs/get', JSON.stringify({ names }), { 'content-type': 'application/json' })
  if (res.status !== 200) throw new Error(`get gave ${res.status}`)
  const size = (await res.arrayBuffer()).byteLength
  if (size < bytesExpected) throw new Error(`get gave ${size} bytes`)
}

const stat = (times) => {
  const sorted = [...times].sort((a, b) => a - b)
  const at = (p) => sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))]
  return `median ${at(0.5).toFixed(1)} ms  p95 ${at(0.95).toFixed(1)} ms  max ${sorted.at(-1).toFixed(1)} ms`
}

async function bench(label, action) {
  for (let i = 0; i < WARMUP; i++) await action()
  const times = []
  for (let i = 0; i < N; i++) {
    const started = performance.now()
    await action()
    times.push(performance.now() - started)
  }
  console.log(`${label.padEnd(34)} n=${N}  ${stat(times)}`)
}

try {
  await migrate()
  const small = typical(20)
  const seeded = typical(40)
  const sum = (list) => list.reduce((a, e) => a + e.bytes.length, 0)
  const big = maxProfile()
  await put(seeded.slice(0, 20))()
  await put(seeded.slice(20))()
  await put(big)()
  console.log(`bundle ${dir}  (wall time in Miniflare, a rough proxy for CPU)`)
  await bench(`thumbs/put 20 x ~6 KB (${(sum(small) / 1024).toFixed(0)} KiB)`, put(small))
  await bench(`thumbs/put max mix (${(sum(big) / 1024).toFixed(0)} KiB)`, put(big))
  await bench(`thumbs/get 40 x ~6 KB (${(sum(seeded) / 1024).toFixed(0)} KiB)`, get(seeded.map((e) => e.name), sum(seeded)))
  await bench(`thumbs/get max mix (${(sum(big) / 1024).toFixed(0)} KiB)`, get(big.map((e) => e.name), sum(big)))
} finally {
  await mf.dispose()
}
