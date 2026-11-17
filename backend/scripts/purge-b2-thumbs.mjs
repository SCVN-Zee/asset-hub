// One-off: deletes the thumbnail objects that the B2 build of the Worker left in the bucket, and
// gives their bytes back to the `storage.reserved_bytes` counter.
//
// Input: the index rows of the B2 era, exported after the Rust Worker is deployed and before
// migration 0003 deletes them (the Rust Worker already treats these rows as absent):
//   wrangler d1 execute asset-hub --remote -c wrangler.rust.toml --json \
//     --command "<the SELECT in the header of migrations/0003_thumbs_d1.sql>" > thumbs-export.json
// A bare JSON array of rows works too.
//
// Run:  node scripts/purge-b2-thumbs.mjs thumbs-export.json            (lists what it would delete)
//       node scripts/purge-b2-thumbs.mjs thumbs-export.json --execute  (deletes, then releases bytes)
// Options: --config <file> picks the wrangler config for the release (default wrangler.rust.toml,
//          resolved in the backend folder).
// Secrets come from the environment: B2_KEY_ID and B2_APP_KEY. B2_ENDPOINT, B2_REGION and B2_BUCKET
// default to the values in wrangler.toml.
//
// Reruns are safe. The script appends every version it handled to `<export>.progress.jsonl` and skips
// those versions next time. Only a version that this script deleted (not one that B2 already reports
// gone, 404) releases bytes, so no byte is released twice. A release that failed is retried with the
// bytes still pending. Over-counting the reserved bytes is the safe direction, so a 404 releases nothing.
import { execFileSync } from 'node:child_process'
import { appendFileSync, existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { env, exit } from 'node:process'
import { fileURLToPath } from 'node:url'
import { AwsClient } from 'aws4fetch'

const CONCURRENCY = 8
const backendDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')

const [file, ...flags] = process.argv.slice(2)
const execute = flags.includes('--execute')
const configAt = flags.indexOf('--config')
const config = configAt >= 0 && flags[configAt + 1] ? flags[configAt + 1] : 'wrangler.rust.toml'
if (!file) {
  console.error('usage: node scripts/purge-b2-thumbs.mjs <export.json> [--execute] [--config <wrangler file>]')
  exit(2)
}
const progressFile = `${file}.progress.jsonl`

const parsed = JSON.parse(readFileSync(file, 'utf8'))
const rows = Array.isArray(parsed) && parsed.length > 0 && Array.isArray(parsed[0]?.results) ? parsed.flatMap((part) => part.results) : parsed
if (!Array.isArray(rows)) {
  console.error('the export is neither a row array nor wrangler --json output')
  exit(2)
}

const valid = rows.filter(
  (r) => r && typeof r.repo === 'string' && Number.isInteger(r.schema) && typeof r.name === 'string' && typeof r.version_id === 'string' && r.version_id !== '' && Number.isInteger(r.size) && r.size >= 0
)

/** What earlier runs did: versions handled, bytes of the versions this script deleted, bytes already released. */
function readProgress() {
  const done = new Set()
  let deletedBytes = 0
  let releasedBytes = 0
  if (existsSync(progressFile)) {
    for (const line of readFileSync(progressFile, 'utf8').split('\n')) {
      if (!line.trim()) continue
      const entry = JSON.parse(line)
      if (typeof entry.version === 'string') {
        done.add(entry.version)
        if (entry.deleted) deletedBytes += entry.size
      } else if (Number.isInteger(entry.released)) releasedBytes += entry.released
    }
  }
  return { done, deletedBytes, releasedBytes }
}

const progress = readProgress()
const todo = valid.filter((r) => !progress.done.has(r.version_id))
console.log(`${rows.length} rows, ${valid.length} with a B2 version, ${valid.reduce((sum, r) => sum + r.size, 0)} bytes`)
console.log(`${valid.length - todo.length} versions done in earlier runs, ${todo.length} left, ${progress.deletedBytes - progress.releasedBytes} bytes waiting for release`)
if (!execute) {
  console.log('dry run: nothing deleted. Pass --execute to delete.')
  exit(0)
}

const { B2_KEY_ID, B2_APP_KEY } = env
if (!B2_KEY_ID || !B2_APP_KEY) {
  console.error('set B2_KEY_ID and B2_APP_KEY')
  exit(2)
}
const endpoint = env.B2_ENDPOINT ?? 'https://s3.us-west-004.backblazeb2.com'
const region = env.B2_REGION ?? 'us-west-004'
const bucket = env.B2_BUCKET ?? 'asset-hub-cache'
const client = new AwsClient({ accessKeyId: B2_KEY_ID, secretAccessKey: B2_APP_KEY, service: 's3', region })

const keyOf = (r) => `repos/${r.repo}/s${r.schema}/thumbs/${r.name}`
const record = (entry) => appendFileSync(progressFile, `${JSON.stringify(entry)}\n`)

/** `deleted`: this call removed the version. `gone`: B2 had none. `failed`: still there, try again later. Never prints a secret. */
async function purge(r) {
  const url = `${endpoint}/${bucket}/${keyOf(r).split('/').map(encodeURIComponent).join('/')}?versionId=${encodeURIComponent(r.version_id)}`
  try {
    const res = await client.fetch(url, { method: 'DELETE' })
    await res.arrayBuffer()
    if (res.ok) return 'deleted'
    if (res.status === 404) return 'gone'
    console.error(`delete failed: ${res.status} ${keyOf(r)}`)
  } catch (error) {
    console.error(`delete failed: ${error?.name ?? 'error'} ${keyOf(r)}`)
  }
  return 'failed'
}

const counts = { deleted: 0, gone: 0, failed: 0 }
let next = 0
await Promise.all(
  Array.from({ length: CONCURRENCY }, async () => {
    while (next < todo.length) {
      const r = todo[next++]
      const outcome = await purge(r)
      counts[outcome] += 1
      if (outcome !== 'failed') record({ version: r.version_id, size: r.size, deleted: outcome === 'deleted' })
    }
  })
)
console.log(`deleted ${counts.deleted} versions, ${counts.gone} were already gone (nothing released for those), failed ${counts.failed}.`)

const pending = readProgress()
const toRelease = pending.deletedBytes - pending.releasedBytes
if (toRelease > 0) {
  const sql = `UPDATE storage SET reserved_bytes = MAX(0, reserved_bytes - ${toRelease}) WHERE id = 1`
  console.log(`releasing ${toRelease} bytes: ${sql}`)
  try {
    execFileSync('npx', ['wrangler', 'd1', 'execute', 'asset-hub', '--remote', '-c', config, '--command', sql], { cwd: backendDir, stdio: 'inherit' })
    record({ released: toRelease })
  } catch {
    console.error(`the release failed. Run the script again, or run the SQL above by hand and add {"released":${toRelease}} to ${progressFile}.`)
    exit(1)
  }
}
exit(counts.failed > 0 ? 1 : 0)
