// Acceptance runner for a Worker: sends n sequential requests per route, checks every response, and exits
// non-zero when any check fails or any route is skipped. With --tail it also matches the CPU time of each
// request in a `wrangler tail` or Workers Logs export.
//
// Run:  ASSETHUB_PROBE_TOKEN="$(gh auth token -u SCVN-Zee)" \
//         node scripts/cpu-probe.mjs --base https://asset-hub-api.asset-hub-team.workers.dev \
//         --routes health,events,popular --n 20
//       node scripts/cpu-probe.mjs --base <url> --routes <list> --dry     (prints the plan, sends nothing)
//
// Options:
//   --base <url>          Worker URL. Plain http is allowed for localhost only, so the token never travels in clear.
//   --routes <a,b,...>    health, events, popular, cache-get-200, cache-get-304, cache-put-small, cache-put-large,
//                         thumbs-list, thumbs-put, thumbs-put-new, thumbs-put-max, thumbs-get, thumbs-get-max,
//                         thumbs-put-1s, thumbs-put-new-1s, thumbs-put-big-1s, thumbs-get-1s, thumbs-put-1s-8x
//                         (default: everything except the thumbs-put and thumbs-get routes).
//   --n <count>           Requests per route (default 20).
//   --delay <ms>          Pause between requests (default 150).
//   --read-schema <n>     Schema header for thumbs-list only (default: the probe schema). Pass 1 to list the real index.
//   --allow-thumbs        Required for the thumbs-put and thumbs-get routes. A preview of a Worker that shares the production
//                         databases must not run them, because their index rows would hold generation ids that the live
//                         TypeScript Worker cannot serve.
//   --run-id <id>         Run id (1 to 24 letters, digits or dashes). Default: 8 random hex digits.
//   --tail <file>         After the run, read a `wrangler tail --format json` file (or a Workers Logs export) and print, per
//                         route, how many requests were sent, completed and observed, how many are missing, and the cpuTime
//                         median, p95 and max of the observed ones. Requests match by the header x-probe-run.
//   --tail-wait <ms>      Wait before reading the tail file, so late events arrive (default 5000, only with --tail).
//   --dry                 Print the planned requests and exit.
//
// The token comes from the environment variable ASSETHUB_PROBE_TOKEN. It is never printed.
//
// Every request carries the header `x-probe-run: <run id>.<sequence>`. The value holds no secret. Setup and read-back
// requests carry it too, with the route label `setup` or `verify`, so a tail record never goes unmatched.
//
// Checks (any failure gives exit code 1, a skipped route too):
//   health, events, popular   status 200 and the JSON shape.
//   cache-get-200             200, a gzip body, and the ETag the setup learned.
//   cache-get-304             304 with no body.
//   cache-put-*               200 and an ETag in header and body that agree. The object is not read back, because a read costs
//                             a B2 Class B call and Backblaze without a card caps them.
//   thumbs-list               200, the JSON shape, and the size of every probe name that the reply lists.
//   thumbs-put*               200 with {"stored":20}, then one read-back of the last request: every name, in order, with every byte.
//   thumbs-get*               200, the pack holds the requested names in order, and every byte equals what the probe wrote.
//
// Thumbnail payloads. typical: 3 KiB to 9 KiB, 6 KiB on average. max: the same mix plus one 62 KiB entry (the live maximum)
// and one 256 KiB entry (the limit of this Worker). The bytes derive from the name, so a read-back can compare them.
//   thumbs-put       20 typical entries, names 0..19. Replaces rows that the setup wrote.
//   thumbs-put-new   20 typical entries with fresh names for every request: 20 new rows per request stay in schema 998.
//   thumbs-put-max   the max mix in 20 entries, names 100..119, about 420 KiB. Replaces rows that the setup wrote.
//   thumbs-get       40 typical names, 0..39.
//   thumbs-get-max   the 20 names of thumbs-put-max.
//
// One-shard routes. They model the client policy that every thumbs/put and thumbs/get request touches exactly one D1
// shard (shard = first hex digit of the name modulo 8, `shard_of` in crates/pack). Names are picked by searching sha1 of
// a fixed text for ones on shard ONE_SHARD, so the choice is deterministic. The plan lists the shards of each request.
//   thumbs-put-1s       8 typical entries (about 48 KiB), one shard. Replaces rows that the setup wrote.
//   thumbs-put-new-1s   8 typical entries with fresh names for every request, one shard. New rows stay in schema 998.
//   thumbs-put-big-1s   one 256 KiB entry alone, one shard.
//   thumbs-get-1s       21 typical names (about 128 KiB), one shard. The first 8 are the names of thumbs-put-1s.
//   thumbs-put-1s-8x    the sizes of thumbs-put-1s with one name on each of the 8 shards. It isolates the per-shard cost.
// A one-shard pack that holds a 256 KiB entry reads back as one reply of about 256 KiB. The read-back is not counted in n.
//
// Safety:
//   - The only repo this script may touch is supercent-io/hb_creative_bulldozer. It is hard-coded.
//   - Every write uses schema 998 (PROBE_SCHEMA), a cache namespace no desktop build uses, so the real catalog,
//     stats and facts (schema 1) stay untouched. Cache objects are facts-metaguids, facts-tmpfont and facts-prefabfx3
//     under that schema.
//   - Thumbnail names must be sha40, so they cannot carry a "probe" prefix. They are sha1("assethub-cpu-probe-thumb-<i>")
//     or, for thumbs-put-new, sha1("assethub-cpu-probe-thumb-new-<run id>-<n>"), which no real git blob hash equals, and they
//     live in schema 998 only.
//   - events sends one valid event for a fixed probe sha (sha1("assethub-cpu-probe-event")) with a fresh uuid each time.
//     No asset has that sha, but the rows stay in the events table until the Worker prunes them.
//
// B2 cost per route (Backblaze without a card caps at 2,500 Class B calls per day):
//   cache-get-200      n B2 GETs. This is the only route that reads B2 on every request.
//   cache-get-304      0. A matching If-None-Match answers from D1.
//   cache-put-small    0 GETs. n PUTs and n DELETEs of the replaced version (Class A and C).
//   cache-put-large    0 GETs. n PUTs of about 15 MB and n DELETEs. Total upload is about n x 15 MB.
//   health, events, popular, thumbs-*   0. Thumbnails live in D1 shards.
//   Setup (not counted in n): a cache object is looked up with `If-None-Match: *`, which answers 304 without a B2 read,
//   and created with one small PUT when it is missing. Setup never issues a B2 GET.
//
// Pacing: requests run one at a time. The run stops when more than 3 requests in a row answer 429 or 5xx.
import { createHash, randomBytes, randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { gzipSync } from 'node:zlib'
import { argv, env, exit } from 'node:process'

const ALLOWED_REPO = 'supercent-io/hb_creative_bulldozer'
const PROBE_SCHEMA = '998'
const LARGE_BYTES = 15 * 1024 * 1024
const MAX_BAD_STREAK = 3
const LIVE_MAX_THUMB = 62 * 1024
const WORKER_MAX_THUMB = 256 * 1024
const PUT_ENTRIES = 20
const GET_NAMES = 40
const SHARDS = 8
const ONE_SHARD = 3
const ONE_SHARD_PUT = 8
const ONE_SHARD_GET = 21
const DEFAULT_ROUTES = ['health', 'events', 'popular', 'cache-get-200', 'cache-get-304', 'cache-put-small', 'cache-put-large', 'thumbs-list']
const THUMB_ROUTES = ['thumbs-put', 'thumbs-put-new', 'thumbs-put-max', 'thumbs-get', 'thumbs-get-max', 'thumbs-put-1s', 'thumbs-put-new-1s', 'thumbs-put-big-1s', 'thumbs-get-1s', 'thumbs-put-1s-8x']
const ALL_ROUTES = [...DEFAULT_ROUTES, ...THUMB_ROUTES]
const EXIT_FAILED = 1
const EXIT_USAGE = 2
const EXIT_NO_TAIL_MATCH = 3

const sha1 = (text) => createHash('sha1').update(text).digest('hex')
const PROBE_EVENT_SHA = sha1('assethub-cpu-probe-event')
const thumbName = (i) => sha1(`assethub-cpu-probe-thumb-${i}`)

// Port of `shard_of` in crates/pack: the first hex digit of the name modulo 8, or null when the name starts with no hex digit.
const shardOf = (name) => {
  const digit = /^[0-9a-fA-F]/.test(name) ? parseInt(name[0], 16) : NaN
  return Number.isNaN(digit) ? null : digit % SHARDS
}

// The pinned values of the unit test in crates/pack/src/lib.rs. The probe aborts when its mapping disagrees.
function checkShardMapping() {
  const name = (digit) => `${digit}${'0'.repeat(39)}`
  const pinned = [['0', 0], ['7', 7], ['8', 0], ['9', 1], ['a', 2], ['c', 4], ['f', 7]]
  const bad = []
  for (const [digit, shard] of pinned) {
    if (shardOf(name(digit)) !== shard) bad.push(digit)
    if (shardOf(`${name(digit)}.fx`) !== shard) bad.push(`${digit}.fx`)
  }
  for (const text of ['', 'zz', '.fx']) if (shardOf(text) !== null) bad.push(JSON.stringify(text))
  if (bad.length) fail(`the JS shard mapping disagrees with crates/pack on: ${bad.join(', ')}`)
}
checkShardMapping()

// The first `count` names sha1(`<prefix>-<j>`), j = 0, 1, ..., that land on `shard`.
function namesOnShard(prefix, shard, count) {
  const names = []
  for (let j = 0; names.length < count; j++) {
    const name = sha1(`${prefix}-${j}`)
    if (shardOf(name) === shard) names.push(name)
  }
  return names
}

function fail(message) {
  console.error(message)
  exit(EXIT_USAGE)
}

function parseArgs(args) {
  const opts = {
    base: '',
    routes: DEFAULT_ROUTES,
    n: 20,
    delay: 150,
    readSchema: PROBE_SCHEMA,
    allowThumbs: false,
    dry: false,
    runId: randomBytes(4).toString('hex'),
    tail: '',
    tailWait: 5000
  }
  const value = (i, flag) => {
    if (i + 1 >= args.length || args[i + 1].startsWith('--')) fail(`${flag} needs a value`)
    return args[i + 1]
  }
  for (let i = 0; i < args.length; i++) {
    const flag = args[i]
    if (flag === '--base') opts.base = value(i++, flag)
    else if (flag === '--routes') opts.routes = value(i++, flag).split(',').map((r) => r.trim()).filter(Boolean)
    else if (flag === '--n') opts.n = Number(value(i++, flag))
    else if (flag === '--delay') opts.delay = Number(value(i++, flag))
    else if (flag === '--read-schema') opts.readSchema = value(i++, flag)
    else if (flag === '--run-id') opts.runId = value(i++, flag)
    else if (flag === '--tail') opts.tail = value(i++, flag)
    else if (flag === '--tail-wait') opts.tailWait = Number(value(i++, flag))
    else if (flag === '--allow-thumbs') opts.allowThumbs = true
    else if (flag === '--dry') opts.dry = true
    else fail(`unknown option ${flag}`)
  }
  if (!opts.base) fail('usage: node scripts/cpu-probe.mjs --base <url> --routes <comma list> --n <count> [--dry] [--allow-thumbs] [--tail <file>]')
  if (!Number.isInteger(opts.n) || opts.n < 1 || opts.n > 1000) fail('--n must be an integer from 1 to 1000')
  if (!Number.isFinite(opts.delay) || opts.delay < 0) fail('--delay must be 0 or more')
  if (!Number.isFinite(opts.tailWait) || opts.tailWait < 0) fail('--tail-wait must be 0 or more')
  if (!/^[0-9]{1,6}$/.test(opts.readSchema)) fail('--read-schema must be 1 to 6 digits')
  if (!/^[A-Za-z0-9-]{1,24}$/.test(opts.runId)) fail('--run-id must be 1 to 24 letters, digits or dashes')
  if (opts.routes.length === 0) fail('--routes names no route')
  for (const route of opts.routes) if (!ALL_ROUTES.includes(route)) fail(`unknown route ${route}. Known: ${ALL_ROUTES.join(', ')}`)
  if (opts.routes.some((r) => THUMB_ROUTES.includes(r)) && !opts.allowThumbs) fail('the thumbs-put and thumbs-get routes need --allow-thumbs (not for a preview URL)')
  return opts
}

function parseBase(text) {
  let url
  try {
    url = new URL(text)
  } catch {
    fail('--base is not a valid URL')
  }
  const local = url.hostname === 'localhost' || url.hostname === '127.0.0.1' || url.hostname === '[::1]'
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && local)) fail('--base must be https, or http on localhost')
  return url.origin
}

const repoPath = (suffix) => `/v1/repos/${ALLOWED_REPO}${suffix}`

// A tiny length-prefixed pack, the wire format of crates/pack: [u16 nameLen][name][u32 byteLen][bytes], big-endian.
function encodePack(entries) {
  const parts = []
  for (const { name, bytes } of entries) {
    const nameBytes = Buffer.from(name, 'utf8')
    const head = Buffer.alloc(2 + nameBytes.length + 4)
    head.writeUInt16BE(nameBytes.length, 0)
    nameBytes.copy(head, 2)
    head.writeUInt32BE(bytes.length, 2 + nameBytes.length)
    parts.push(head, bytes)
  }
  return Buffer.concat(parts)
}

// The entries of a pack, or null when it is truncated or has trailing bytes.
function decodePack(buf) {
  const entries = []
  let at = 0
  while (at < buf.length) {
    if (at + 2 > buf.length) return null
    const nameLen = buf.readUInt16BE(at)
    if (at + 2 + nameLen + 4 > buf.length) return null
    const name = buf.toString('utf8', at + 2, at + 2 + nameLen)
    const byteLen = buf.readUInt32BE(at + 2 + nameLen)
    const start = at + 6 + nameLen
    if (start + byteLen > buf.length) return null
    entries.push({ name, bytes: buf.subarray(start, start + byteLen) })
    at = start + byteLen
  }
  return entries
}

const smallGzip = (label) => gzipSync(Buffer.from(JSON.stringify({ probe: label, note: 'asset-hub cpu probe, safe to delete' })))
let largeBody // built once, only when cache-put-large runs
const largeGzip = () => (largeBody ??= gzipSync(randomBytes(LARGE_BYTES), { level: 0 }))
const isGzip = (buf) => buf.length >= 2 && buf[0] === 0x1f && buf[1] === 0x8b

// Thumbnail bytes derive from the name and the size, so a read-back knows what to expect.
const thumbBytesCache = new Map()
function thumbBytes(name, size) {
  const key = `${name}:${size}`
  let bytes = thumbBytesCache.get(key)
  if (!bytes) {
    const blocks = []
    for (let n = 0; blocks.length * 32 < size; n++) blocks.push(createHash('sha256').update(`${name}:${n}`).digest())
    bytes = Buffer.concat(blocks).subarray(0, size)
    thumbBytesCache.set(key, bytes)
  }
  return bytes
}

// 3 KiB to 9 KiB, 6 KiB on average.
const typicalSize = (i) => 3072 + ((i * 937) % 6144)
const typicalEntry = (name, i) => ({ name, bytes: thumbBytes(name, typicalSize(i)) })
const typicalEntries = (from, count) => Array.from({ length: count }, (_, k) => typicalEntry(thumbName(from + k), from + k))
const MAX_FROM = 100
const MAX_LIVE_AT = 3
const MAX_WORKER_AT = 11
const maxEntries = () =>
  Array.from({ length: PUT_ENTRIES }, (_, k) => {
    const name = thumbName(MAX_FROM + k)
    const size = k === MAX_LIVE_AT ? LIVE_MAX_THUMB : k === MAX_WORKER_AT ? WORKER_MAX_THUMB : typicalSize(MAX_FROM + k)
    return { name, bytes: thumbBytes(name, size) }
  })
const newEntries = (ctx) => {
  const run = ctx.newPacks++
  return Array.from({ length: PUT_ENTRIES }, (_, k) => typicalEntry(sha1(`assethub-cpu-probe-thumb-new-${ctx.runId}-${run}-${k}`), k))
}

const oneShardEntries = (count) => namesOnShard('assethub-cpu-probe-thumb-1s', ONE_SHARD, count).map((name, k) => ({ name, bytes: thumbBytes(name, typicalSize(k)) }))
const oneShardNewEntries = (ctx) => {
  const run = ctx.newPacks++
  return namesOnShard(`assethub-cpu-probe-thumb-new-1s-${ctx.runId}-${run}`, ONE_SHARD, ONE_SHARD_PUT).map((name, k) => ({ name, bytes: thumbBytes(name, typicalSize(k)) }))
}
const bigEntry = () => {
  const [name] = namesOnShard('assethub-cpu-probe-thumb-big-1s', ONE_SHARD, 1)
  return [{ name, bytes: thumbBytes(name, WORKER_MAX_THUMB) }]
}
const spreadEntries = () =>
  Array.from({ length: ONE_SHARD_PUT }, (_, k) => {
    const [name] = namesOnShard(`assethub-cpu-probe-thumb-8x-${k}`, k % SHARDS, 1)
    return { name, bytes: thumbBytes(name, typicalSize(k)) }
  })

const OBJECT = { get: 'facts-metaguids', small: 'facts-tmpfont', large: 'facts-prefabfx3' }
const SCHEMA_HEADER = { 'x-asset-hub-schema': PROBE_SCHEMA }

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

// A check returns a list of problems, empty when the response is right.
const json = (result) => {
  try {
    return JSON.parse(result.buf.toString('utf8'))
  } catch {
    return undefined
  }
}
const statusIs = (result, want) => (result.status === want ? [] : [`status ${result.status || result.error}, expected ${want}`])
const jsonProblems = (result, test, what) => {
  const body = json(result)
  return body !== undefined && test(body) ? [] : [`body is not ${what}`]
}

function checkPack(result, wanted) {
  const problems = statusIs(result, 200)
  if (problems.length) return problems
  if (result.res.headers.get('content-type') !== 'application/octet-stream') problems.push(`content-type ${result.res.headers.get('content-type')}`)
  const got = decodePack(result.buf)
  if (!got) return [...problems, 'the pack is truncated']
  if (got.length !== wanted.length) return [...problems, `pack holds ${got.length} entries, expected ${wanted.length}`]
  wanted.forEach((want, i) => {
    if (got[i].name !== want.name) problems.push(`entry ${i} is ${got[i].name.slice(0, 8)}, expected ${want.name.slice(0, 8)}`)
    else if (!got[i].bytes.equals(want.bytes)) problems.push(`entry ${i} (${want.name.slice(0, 8)}) holds ${got[i].bytes.length} bytes that differ from the ${want.bytes.length} written`)
  })
  return problems
}

const getRequest = (entries) => ({
  method: 'POST',
  path: repoPath('/thumbs/get'),
  headers: { ...SCHEMA_HEADER, 'content-type': 'application/json' },
  body: JSON.stringify({ names: entries.map((e) => e.name) }),
  check: (result) => checkPack(result, entries),
  expect: entries
})

const putRequest = (entries) => ({
  method: 'POST',
  path: repoPath('/thumbs/put'),
  headers: { ...SCHEMA_HEADER, 'content-type': 'application/octet-stream' },
  body: encodePack(entries),
  check: (result) => [...statusIs(result, 200), ...(result.status === 200 ? jsonProblems(result, (b) => b.stored === entries.length, `{"stored":${entries.length}}`) : [])],
  readBack: entries
})

function cachePut(ctx, name, body) {
  const etag = ctx.etags.get(name)
  return {
    method: 'PUT',
    path: repoPath(`/cache/${name}`),
    headers: { ...SCHEMA_HEADER, 'content-type': 'application/gzip', ...(etag ? { 'if-match': etag } : { 'if-none-match': '*' }) },
    body,
    check: (result) => {
      const problems = statusIs(result, 200)
      if (problems.length) return problems
      const header = result.res.headers.get('etag')
      const body = json(result)
      if (!header || typeof body?.etag !== 'string' || body.etag !== header) problems.push('etag missing, or header and body disagree')
      return problems
    }
  }
}

// Each route builds a request from `ctx`: { runId, etags, readSchema, newPacks, expectedSizes }.
// `seed` names the thumbnail set that the route needs to exist before its first request.
const ROUTES = {
  health: { request: () => ({ method: 'GET', path: '/v1/health', auth: false, check: (r) => [...statusIs(r, 200), ...(r.status === 200 ? jsonProblems(r, (b) => b.ok === true, '{"ok":true}') : [])] }) },
  events: {
    request: () => ({
      method: 'POST',
      path: repoPath('/events'),
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ events: [{ id: randomUUID(), sha: PROBE_EVENT_SHA, kind: 'view', ts: Date.now() }] }),
      check: (r) => [...statusIs(r, 200), ...(r.status === 200 ? jsonProblems(r, (b) => Number.isInteger(b.accepted) && Array.isArray(b.rejected) && b.accepted + b.rejected.length === 1, 'one accepted or rejected event') : [])]
    })
  },
  popular: { request: () => ({ method: 'GET', path: repoPath('/popular'), check: (r) => [...statusIs(r, 200), ...(r.status === 200 ? jsonProblems(r, (b) => Array.isArray(b.items), '{"items":[...]}') : [])] }) },
  'cache-get-200': {
    setup: [OBJECT.get],
    request: (ctx) => ({
      method: 'GET',
      path: repoPath(`/cache/${OBJECT.get}`),
      headers: SCHEMA_HEADER,
      check: (r) => {
        const problems = statusIs(r, 200)
        if (problems.length) return problems
        if (!isGzip(r.buf)) problems.push('body is not gzip')
        const etag = ctx.etags.get(OBJECT.get)
        if (etag && r.res.headers.get('etag') !== etag) problems.push('etag differs from the one the setup learned')
        return problems
      }
    })
  },
  'cache-get-304': {
    setup: [OBJECT.get],
    request: (ctx) => ({
      method: 'GET',
      path: repoPath(`/cache/${OBJECT.get}`),
      headers: { ...SCHEMA_HEADER, 'if-none-match': ctx.etags.get(OBJECT.get) ?? '"unknown"' },
      check: (r) => [...statusIs(r, 304), ...(r.buf.length === 0 ? [] : ['a 304 carries a body'])]
    })
  },
  'cache-put-small': { setup: [OBJECT.small], request: (ctx) => cachePut(ctx, OBJECT.small, smallGzip('small')) },
  'cache-put-large': { setup: [OBJECT.large], request: (ctx) => cachePut(ctx, OBJECT.large, largeGzip()), timeoutMs: 180_000 },
  'thumbs-list': {
    request: (ctx) => ({
      method: 'GET',
      path: repoPath('/thumbs'),
      headers: { 'x-asset-hub-schema': ctx.readSchema },
      check: (r) => {
        const problems = statusIs(r, 200)
        if (problems.length) return problems
        const body = json(r)
        if (!body || !Array.isArray(body.items) || !Number.isInteger(body.epoch)) return ['body is not {"items":[...],"epoch":n}']
        for (const item of body.items) {
          if (typeof item.name !== 'string' || !Number.isInteger(item.size) || !Number.isInteger(item.rev)) return ['an item has no name, size or rev']
          const want = ctx.expectedSizes.get(item.name)
          if (ctx.readSchema === PROBE_SCHEMA && want !== undefined && item.size !== want) problems.push(`${item.name.slice(0, 8)} lists size ${item.size}, expected ${want}`)
        }
        return problems
      }
    })
  },
  'thumbs-put': { seed: 'typical', request: () => putRequest(typicalEntries(0, PUT_ENTRIES)) },
  'thumbs-put-new': { request: (ctx) => putRequest(newEntries(ctx)) },
  'thumbs-put-max': { seed: 'max', request: () => putRequest(maxEntries()) },
  'thumbs-get': { seed: 'typical', request: () => getRequest(typicalEntries(0, GET_NAMES)) },
  'thumbs-get-max': { seed: 'max', request: () => getRequest(maxEntries()) },
  'thumbs-put-1s': { seed: 'oneShard', request: () => putRequest(oneShardEntries(ONE_SHARD_PUT)) },
  'thumbs-put-new-1s': { request: (ctx) => putRequest(oneShardNewEntries(ctx)) },
  'thumbs-put-big-1s': { request: () => putRequest(bigEntry()) },
  'thumbs-get-1s': { seed: 'oneShard', request: () => getRequest(oneShardEntries(ONE_SHARD_GET)) },
  'thumbs-put-1s-8x': { seed: 'spread', request: () => putRequest(spreadEntries()) }
}

const SEEDS = {
  typical: () => [typicalEntries(0, PUT_ENTRIES), typicalEntries(PUT_ENTRIES, GET_NAMES - PUT_ENTRIES)],
  max: () => [maxEntries()],
  // Chunks of the put size, so the setup follows the same one-shard policy as the measured requests.
  oneShard: () => {
    const all = oneShardEntries(ONE_SHARD_GET)
    return Array.from({ length: Math.ceil(all.length / ONE_SHARD_PUT) }, (_, c) => all.slice(c * ONE_SHARD_PUT, (c + 1) * ONE_SHARD_PUT))
  },
  spread: () => [spreadEntries()]
}

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b)
  const mid = sorted.length >> 1
  return sorted.length % 2 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2
}
const percentile = (values, p) => {
  const sorted = [...values].sort((a, b) => a - b)
  return sorted[Math.max(0, Math.ceil(sorted.length * p) - 1)]
}

// Sends one request. The token is added here and nowhere else. An error message never holds the token.
// `label` names the route in the tail summary: a route name, `setup` or `verify`.
async function send(run, req, label, timeoutMs = 60_000) {
  const seq = ++run.seq
  const headers = { ...(req.headers ?? {}), 'x-probe-run': `${run.id}.${seq}` }
  if (req.auth !== false) headers.authorization = `Bearer ${run.token}`
  const started = performance.now()
  let result
  try {
    const res = await fetch(run.base + req.path, { method: req.method, headers, body: req.body, signal: AbortSignal.timeout(timeoutMs) })
    const buf = Buffer.from(await res.arrayBuffer())
    result = { status: res.status, ms: performance.now() - started, res, buf }
  } catch (err) {
    result = { status: 0, ms: performance.now() - started, error: err instanceof Error ? err.name : 'error' }
  }
  run.sent.push({ seq, label, completed: result.status !== 0 })
  return result
}

// Problems of a response, or one problem for a transport error.
function problemsOf(req, result) {
  if (result.status === 0) return [`no response (${result.error})`]
  return req.check ? req.check(result) : []
}

function etagOf(result) {
  const header = result.res?.headers.get('etag')
  if (header) return header
  const body = result.buf ? json(result) : undefined
  return body?.etag ?? null
}

// Learns the etag of a cache object with `If-None-Match: *` (304, no B2 read), or creates it when missing.
async function ensureObject(run, name) {
  const { ctx } = run
  if (ctx.etags.has(name)) return true
  const look = await send(run, { method: 'GET', path: repoPath(`/cache/${name}`), headers: { ...SCHEMA_HEADER, 'if-none-match': '*' } }, 'setup')
  console.log(`setup  lookup ${name}  ${look.status}  ${look.ms.toFixed(0)} ms`)
  if (look.status === 304) {
    const etag = etagOf(look)
    if (etag) ctx.etags.set(name, etag)
    return Boolean(etag)
  }
  if (look.status !== 404) return false
  const body = name === OBJECT.large ? largeGzip() : smallGzip(name)
  const req = { ...cachePut(ctx, name, body), headers: { ...SCHEMA_HEADER, 'content-type': 'application/gzip', 'if-none-match': '*' } }
  const made = await send(run, req, 'setup', 180_000)
  console.log(`setup  create ${name}  ${made.status}  ${made.ms.toFixed(0)} ms`)
  if (made.status !== 200) return false
  const etag = etagOf(made)
  if (etag) ctx.etags.set(name, etag)
  return Boolean(etag)
}

async function seedThumbs(run, set) {
  if (run.ctx.seeded.has(set)) return true
  for (const entries of SEEDS[set]()) {
    const req = putRequest(entries)
    const res = await send(run, req, 'setup')
    const problems = problemsOf(req, res)
    console.log(`setup  thumbs/put ${set} ${entries.length} entries  ${res.status}  ${res.ms.toFixed(0)} ms${problems.length ? `  FAIL ${problems.join('; ')}` : ''}`)
    if (problems.length) return false
    for (const e of entries) run.ctx.expectedSizes.set(e.name, e.bytes.length)
  }
  run.ctx.seeded.add(set)
  return true
}

// Reads the entries of a put back and compares every byte. Not counted in n.
async function verifyPut(run, route, entries) {
  const names = entries.slice(0, GET_NAMES)
  const req = getRequest(names)
  const res = await send(run, req, 'verify')
  const problems = problemsOf(req, res)
  console.log(`verify ${route}  read back ${names.length} entries  ${res.status}  ${problems.length ? `FAIL ${problems.join('; ')}` : 'ok'}`)
  return problems
}

function plan(opts) {
  console.log(`base ${opts.base}  repo ${ALLOWED_REPO}  schema ${PROBE_SCHEMA}  n ${opts.n}  delay ${opts.delay} ms  run ${opts.runId}`)
  const ctx = { runId: opts.runId, etags: new Map(), readSchema: opts.readSchema, newPacks: 0, expectedSizes: new Map(), seeded: new Set() }
  for (const name of opts.routes) {
    const route = ROUTES[name]
    for (const object of route.setup ?? []) console.log(`${name}: setup  GET /cache/${object} (If-None-Match: *), then PUT (If-None-Match: *) only if 404`)
    if (route.seed) console.log(`${name}: setup  POST /thumbs/put x${SEEDS[route.seed]().length} (${route.seed} set)`)
    const req = route.request(ctx)
    const size = req.body ? `${req.body.length} bytes` : 'no body'
    const entries = req.readBack ?? req.expect
    const shards = entries ? [...new Set(entries.map((e) => shardOf(e.name)))].sort((a, b) => a - b) : []
    const total = entries ? entries.reduce((sum, e) => sum + e.bytes.length, 0) : 0
    const mix = entries
      ? `  ${entries.length} entries, ${Math.min(...entries.map((e) => e.bytes.length))} to ${Math.max(...entries.map((e) => e.bytes.length))} bytes each, ${total} bytes of thumbnails, ${shards.length} shard${shards.length === 1 ? '' : 's'} (${shards.join(',')})`
      : ''
    console.log(`${name}: ${opts.n} x ${req.method} ${req.path}  ${size}${mix}  headers ${Object.keys(req.headers ?? {}).join(',') || '-'}`)
    if (req.readBack) console.log(`${name}: verify  POST /thumbs/get of the last request, every byte compared`)
  }
  console.log('every request: header x-probe-run: <run id>.<sequence>')
}

async function run(opts, base, token) {
  const ctx = { runId: opts.runId, etags: new Map(), readSchema: opts.readSchema, newPacks: 0, expectedSizes: new Map(), seeded: new Set() }
  const state = { id: opts.runId, base, token, ctx, seq: 0, sent: [] }
  const summaries = []
  const failures = []
  let badStreak = 0
  let aborted = false

  for (const name of opts.routes) {
    if (aborted) {
      summaries.push({ name, skipped: true })
      continue
    }
    const route = ROUTES[name]
    let ready = true
    for (const object of route.setup ?? []) ready = (await ensureObject(state, object)) && ready
    if (route.seed) ready = (await seedThumbs(state, route.seed)) && ready
    if (!ready) {
      console.error(`${name}: setup failed, route skipped`)
      summaries.push({ name, skipped: true })
      continue
    }

    const stats = { name, statuses: new Map(), times: [], failed: 0 }
    let last
    for (let i = 1; i <= opts.n; i++) {
      const req = route.request(ctx)
      const result = await send(state, req, name, route.timeoutMs)
      const key = result.status === 0 ? `err:${result.error}` : String(result.status)
      stats.statuses.set(key, (stats.statuses.get(key) ?? 0) + 1)
      stats.times.push(result.ms)
      last = req

      const problems = problemsOf(req, result)
      if (req.method === 'PUT' && req.path.includes('/cache/')) {
        const object = req.path.split('/').pop()
        if (result.status === 200) {
          const etag = etagOf(result)
          if (etag) ctx.etags.set(object, etag)
        } else if (result.status === 412) {
          ctx.etags.delete(object)
          await ensureObject(state, object)
        }
      }
      if (problems.length) {
        stats.failed++
        failures.push(`${name} #${i}: ${problems.join('; ')}`)
      }
      console.log(`${name}  #${i}  ${key}  ${result.ms.toFixed(0)} ms${problems.length ? `  FAIL ${problems.join('; ')}` : ''}`)

      badStreak = result.status === 429 || result.status >= 500 || result.status === 0 ? badStreak + 1 : 0
      if (badStreak > MAX_BAD_STREAK) {
        console.error(`abort: more than ${MAX_BAD_STREAK} requests in a row failed with 429, 5xx or a network error`)
        failures.push(`${name}: aborted after repeated 429, 5xx or network errors`)
        aborted = true
        break
      }
      if (i < opts.n && opts.delay > 0) await sleep(opts.delay)
    }
    if (last?.readBack && !aborted) {
      const problems = await verifyPut(state, name, last.readBack)
      if (problems.length) {
        stats.failed++
        failures.push(`${name} read-back: ${problems.join('; ')}`)
      } else for (const e of last.readBack) ctx.expectedSizes.set(e.name, e.bytes.length)
    }
    summaries.push(stats)
    if (!aborted && opts.delay > 0) await sleep(opts.delay)
  }

  console.log(`\nSummary (wall time, run ${opts.runId}; CPU is in the tail)`)
  for (const s of summaries) {
    if (s.skipped) {
      console.log(`${s.name}: SKIPPED`)
      failures.push(`${s.name}: skipped`)
      continue
    }
    const counts = [...s.statuses].map(([status, count]) => `${status} x${count}`).join(', ')
    console.log(`${s.name}: ${s.times.length} requests  ${counts}  failed ${s.failed}  median ${median(s.times).toFixed(0)} ms  max ${Math.max(...s.times).toFixed(0)} ms`)
  }
  return { state, failures }
}

// The events of a `wrangler tail --format json` file (objects one after another, or a JSON array) or a Workers Logs export.
function readTailEvents(file) {
  const text = readFileSync(file, 'utf8').trim()
  if (!text) return []
  try {
    const value = JSON.parse(text)
    return Array.isArray(value) ? value : [value]
  } catch {
    return text.split(/\n(?=\{)/).flatMap((chunk) => {
      try {
        return [JSON.parse(chunk)]
      } catch {
        return []
      }
    })
  }
}

// Prints sent, completed, observed and missing per label and the cpuTime of the observed requests. Gives the number of
// requests that no tail event matched.
function summarizeTail(opts, state) {
  const wanted = new Map(state.sent.map((s) => [s.seq, s]))
  const seen = new Map()
  let foreign = 0
  for (const event of readTailEvents(opts.tail)) {
    const headers = event.event?.request?.headers ?? event.$workers?.event?.request?.headers
    const tag = headers?.['x-probe-run']
    const [id, seqText] = typeof tag === 'string' ? tag.split('.') : []
    if (id !== state.id || !wanted.has(Number(seqText))) {
      if (id === state.id) foreign++
      continue
    }
    const seq = Number(seqText)
    if (seen.has(seq)) continue
    seen.set(seq, { cpu: event.cpuTime ?? event.$workers?.cpuTimeMs, outcome: event.outcome ?? event.$workers?.outcome, exceptions: (event.exceptions ?? []).length })
  }

  const labels = [...new Set(state.sent.map((s) => s.label))]
  console.log(`\nTail ${opts.tail}  run ${state.id}`)
  console.log('label'.padEnd(18) + ['sent', 'done', 'seen', 'missing', 'cpu median', 'p95', 'max', 'not ok', 'exceptions'].map((h) => h.padStart(11)).join(''))
  let missingTotal = 0
  for (const label of labels) {
    const mine = state.sent.filter((s) => s.label === label)
    const observed = mine.filter((s) => seen.has(s.seq)).map((s) => seen.get(s.seq))
    const cpu = observed.map((o) => o.cpu).filter((v) => typeof v === 'number')
    const missing = mine.length - observed.length
    missingTotal += missing
    const stat = (f) => (cpu.length ? String(f(cpu)).padStart(11) : '-'.padStart(11))
    console.log(
      label.padEnd(18) +
        [mine.length, mine.filter((s) => s.completed).length, observed.length, missing].map((v) => String(v).padStart(11)).join('') +
        stat(median) + stat((v) => percentile(v, 0.95)) + stat((v) => Math.max(...v)) +
        String(observed.filter((o) => o.outcome !== 'ok').length).padStart(11) +
        String(observed.reduce((sum, o) => sum + o.exceptions, 0)).padStart(11)
    )
  }
  if (foreign > 0) console.log(`${foreign} tail events carry this run id with an unknown sequence number`)
  if (missingTotal > 0) console.log(`WARNING: ${missingTotal} requests have no tail event. Missing events are unknown, not successful observations.`)
  return missingTotal
}

const opts = parseArgs(argv.slice(2))
const base = parseBase(opts.base)
opts.base = base
if (opts.dry) {
  plan(opts)
  exit(0)
}
const token = env.ASSETHUB_PROBE_TOKEN
if (!token || /\s/.test(token)) fail('set ASSETHUB_PROBE_TOKEN to a GitHub token (no white space)')
const { state, failures } = await run(opts, base, token)
let code = 0
if (failures.length > 0) {
  console.error(`\nFAILED: ${failures.length} problem${failures.length === 1 ? '' : 's'}`)
  for (const line of failures.slice(0, 20)) console.error(`  ${line}`)
  if (failures.length > 20) console.error(`  ... and ${failures.length - 20} more`)
  code = EXIT_FAILED
} else {
  console.log('\nOK: every response passed its checks')
}
if (opts.tail) {
  if (opts.tailWait > 0) await sleep(opts.tailWait)
  if (summarizeTail(opts, state) > 0 && code === 0) code = EXIT_NO_TAIL_MATCH
}
exit(code)
