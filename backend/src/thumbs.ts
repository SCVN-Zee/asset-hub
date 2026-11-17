import { B2Client, B2HttpError } from './b2'
import type { Env } from './env'
import { errorJson, json, readBody, readSchema } from './http'
import { decodePack, encodeEntryHeader, type PackEntry } from './pack'
import type { RepoHandler } from './route'
import { discard, release, reserve, type Blob } from './storage'

const NAME = /^[0-9a-f]{40}(\.fx)?$/
const CURSOR = /^(\d{1,16}):([0-9a-f]{40}(?:\.fx)?)$/
const MAX_LIST = 1000
const MAX_GET_NAMES = 40
// Subrequest budget (Workers Free allows 50 per request, and D1 calls may count): auth uses up to 6,
// so a PUT of 20 thumbnails spends 20 uploads + 20 deletes + 3 D1 calls, and a GET of 40 names
// spends 40 downloads + 1 D1 call. Raise these limits only together with the plan tier.
const MAX_PUT_ENTRIES = 20
const MAX_PUT_BYTES = 4 * 1024 * 1024
const MAX_THUMB_BYTES = 1024 * 1024
const MAX_GET_BODY_BYTES = 8 * 1024
/** B2 GETs in flight at once. Each holds at most one thumbnail (1 MB), so memory stays near 6 MB. */
const GET_CONCURRENCY = 6

const thumbKey = (slug: string, schema: number, name: string): string => `repos/${slug}/s${schema}/thumbs/${name}`

/**
 * `GET /thumbs?after=<rev>:<name>&limit=1000`: the index, ordered by `(rev, name)`. `rev` counts
 * commits per repo and schema, so a replaced thumbnail always gets a higher value.
 */
export const listThumbs: RepoHandler = async ({ req, env, slug }) => {
  const schema = readSchema(req)
  if (schema instanceof Response) return schema

  const url = new URL(req.url)
  const rawLimit = url.searchParams.get('limit')
  const limit = rawLimit === null ? MAX_LIST : Number(rawLimit)
  if (!Number.isInteger(limit) || limit < 1 || limit > MAX_LIST) return errorJson('badRequest', 400)

  const after = url.searchParams.get('after')
  const cursor = after === null ? null : CURSOR.exec(after)
  if (after !== null && !cursor) return errorJson('badRequest', 400)

  // One extra row tells whether another page follows.
  const base = 'SELECT name, size, rev FROM thumbs WHERE repo = ? AND schema = ?'
  const order = ' ORDER BY rev, name LIMIT ?'
  const statement = cursor
    ? env.DB.prepare(`${base} AND (rev, name) > (?, ?)${order}`).bind(slug, schema, Number(cursor[1]), cursor[2], limit + 1)
    : env.DB.prepare(`${base}${order}`).bind(slug, schema, limit + 1)
  const { results } = await statement.all<{ name: string; size: number; rev: number }>()

  const items = results.slice(0, limit)
  const last = items[items.length - 1]
  return json(results.length > limit ? { items, next: `${last.rev}:${last.name}` } : { items })
}

/** `POST /thumbs/get` `{ names }`: a binary pack of the thumbnails that exist. */
export const getThumbs: RepoHandler = async ({ req, env, slug }) => {
  const schema = readSchema(req)
  if (schema instanceof Response) return schema

  const body = await readBody(req, MAX_GET_BODY_BYTES)
  if (body instanceof Response) return errorJson('badRequest', 400)
  let names: unknown
  try {
    names = (JSON.parse(new TextDecoder().decode(body)) as { names?: unknown } | null)?.names
  } catch {
    return errorJson('badRequest', 400)
  }
  if (!Array.isArray(names) || names.length > MAX_GET_NAMES) return errorJson('badRequest', 400)
  if (!names.every((n): n is string => typeof n === 'string' && NAME.test(n))) return errorJson('badRequest', 400)

  const wanted = [...new Set(names)]
  // The index decides which version is served, so a stray newer upload is never returned.
  const versions = await indexedVersions(env, slug, schema, wanted)
  return new Response(packStream(env, slug, schema, wanted.filter((n) => versions.has(n)), versions), {
    headers: { 'content-type': 'application/octet-stream', 'cache-control': 'private, no-store' }
  })
}

/** `name -> version id` for the names the index knows. Rows from before versions were recorded are left out. */
async function indexedVersions(env: Env, slug: string, schema: number, names: string[]): Promise<Map<string, string>> {
  if (names.length === 0) return new Map()
  const marks = names.map(() => '?').join(',')
  const { results } = await env.DB.prepare(`SELECT name, version_id FROM thumbs WHERE repo = ? AND schema = ? AND name IN (${marks})`)
    .bind(slug, schema, ...names)
    .all<{ name: string; version_id: string }>()
  return new Map(results.filter((row) => row.version_id !== '').map((row) => [row.name, row.version_id]))
}

type Fetched = { name: string; bytes: Uint8Array } | { name: string; missing: true } | { name: string; error: unknown }

/**
 * Streams the pack in request order while up to `GET_CONCURRENCY` B2 GETs run at once. The B2
 * bucket is far from the Worker, so one GET after another would take seconds. At most
 * `GET_CONCURRENCY` bodies are held: the window refills only after the head entry is written.
 * Objects missing from B2 are skipped. Cancelling aborts the GETs still in flight.
 */
function packStream(env: Env, slug: string, schema: number, names: string[], versions: Map<string, string>): ReadableStream<Uint8Array> {
  const b2 = new B2Client(env)
  const abort = new AbortController()
  const window: Promise<Fetched>[] = []
  let next = 0

  const fetchOne = async (name: string): Promise<Fetched> => {
    try {
      const res = await b2.get(thumbKey(slug, schema, name), abort.signal, versions.get(name))
      return res ? { name, bytes: new Uint8Array(await res.arrayBuffer()) } : { name, missing: true }
    } catch (error) {
      return { name, error }
    }
  }

  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      for (;;) {
        while (window.length < GET_CONCURRENCY && next < names.length) window.push(fetchOne(names[next++]))
        const head = window.shift()
        if (!head) return controller.close()
        const entry = await head
        if ('missing' in entry) continue
        if ('error' in entry) {
          // Stop the sibling downloads; an error does not call `cancel`.
          const wasAborted = abort.signal.aborted
          abort.abort()
          if (!wasAborted) controller.error(entry.error)
          return
        }
        controller.enqueue(encodeEntryHeader(entry.name, entry.bytes.length))
        controller.enqueue(entry.bytes)
        return
      }
    },
    cancel() {
      abort.abort()
    }
  })
}

/**
 * `POST /thumbs/put` with a binary pack: needs push access. Reserves the bytes, uploads to B2,
 * commits the D1 index in one batch, then deletes the versions the batch replaced.
 */
export const putThumbs: RepoHandler = async ({ req, env, slug, caller }) => {
  const schema = readSchema(req)
  if (schema instanceof Response) return schema
  if (!caller.canPush) return errorJson('readOnly', 403)

  const body = await readBody(req, MAX_PUT_BYTES)
  if (body instanceof Response) return body
  let decoded
  try {
    decoded = decodePack(body)
  } catch {
    return errorJson('badPack', 400)
  }
  if (decoded.length < 1 || decoded.length > MAX_PUT_ENTRIES) return errorJson('badRequest', 400)
  if (!decoded.every((e) => NAME.test(e.name))) return errorJson('badRequest', 400)
  if (decoded.some((e) => e.bytes.length > MAX_THUMB_BYTES)) return errorJson('tooLarge', 413)
  // A name sent twice keeps its last bytes; uploading both would leave a version nothing points at.
  const entries = [...new Map(decoded.map((e) => [e.name, e])).values()]

  const total = entries.reduce((sum, e) => sum + e.bytes.length, 0)
  if (!(await reserve(env.DB, total))) return errorJson('storageFull', 507)

  const uploaded = await uploadAll(env, slug, schema, entries)
  let replaced: Blob[]
  try {
    replaced = await commitIndex(env, slug, schema, entries, uploaded)
  } catch (err) {
    // The batch failed, but it may have committed. Delete only versions the index does not name;
    // if the index cannot be read, delete nothing and keep the bytes charged.
    const named = await indexedVersions(env, slug, schema, entries.map((e) => e.name)).catch(() => null)
    if (named) {
      const orphans = entries.flatMap((e, i) => (named.get(e.name) === uploaded[i] ? [] : [{ key: thumbKey(slug, schema, e.name), versionId: uploaded[i], size: e.bytes.length }]))
      await discard(env, orphans)
    }
    throw err
  }
  await discard(env, replaced)
  return json({ stored: entries.length })
}

/**
 * Uploads every entry and returns the version ids in entry order. If any upload fails, the
 * versions that did land are deleted, bytes of uploads that B2 refused are released, and the
 * first error is thrown. Bytes of an upload with an unknown outcome stay charged.
 */
async function uploadAll(env: Env, slug: string, schema: number, entries: PackEntry[]): Promise<string[]> {
  const b2 = new B2Client(env)
  const outcomes = await Promise.allSettled(entries.map((e) => b2.put(thumbKey(slug, schema, e.name), e.bytes, 'application/octet-stream')))
  const failed = outcomes.find((o): o is PromiseRejectedResult => o.status === 'rejected')
  if (!failed) return outcomes.map((o) => (o as PromiseFulfilledResult<string>).value)

  const landed: Blob[] = []
  let refused = 0
  outcomes.forEach((o, i) => {
    const size = entries[i].bytes.length
    if (o.status === 'fulfilled') landed.push({ key: thumbKey(slug, schema, entries[i].name), versionId: o.value, size })
    else if (o.reason instanceof B2HttpError) refused += size
  })
  await release(env.DB, refused).catch(() => {})
  await discard(env, landed)
  throw failed.reason
}

/**
 * One commit per call: the revision moves up by one and every entry carries it. The batch is
 * atomic and runs in order, so concurrent writers get distinct revisions and a reader that saw
 * revision N never misses a later commit. Its first statement reads the rows the upserts then
 * replace, so the returned versions are exactly the ones this commit made obsolete.
 */
async function commitIndex(env: Env, slug: string, schema: number, entries: PackEntry[], versionIds: string[]): Promise<Blob[]> {
  const marks = entries.map(() => '?').join(',')
  const previous = env.DB.prepare(`SELECT name, size, version_id FROM thumbs WHERE repo = ? AND schema = ? AND name IN (${marks})`).bind(
    slug, schema, ...entries.map((e) => e.name)
  )
  const bump = env.DB.prepare(
    'INSERT INTO thumb_rev (repo, schema, rev) VALUES (?, ?, 1) ON CONFLICT (repo, schema) DO UPDATE SET rev = rev + 1'
  ).bind(slug, schema)
  const upsert = env.DB.prepare(
    'INSERT INTO thumbs (repo, schema, name, size, rev, version_id) VALUES (?, ?, ?, ?, (SELECT rev FROM thumb_rev WHERE repo = ? AND schema = ?), ?) ' +
      'ON CONFLICT (repo, schema, name) DO UPDATE SET size = excluded.size, rev = excluded.rev, version_id = excluded.version_id'
  )
  const results = await env.DB.batch([
    previous,
    bump,
    ...entries.map((e, i) => upsert.bind(slug, schema, e.name, e.bytes.length, slug, schema, versionIds[i]))
  ])
  const rows = (results[0] as D1Result<{ name: string; size: number; version_id: string }>).results
  // Rows written before versions were recorded have no id to delete.
  return rows.filter((row) => row.version_id !== '').map((row) => ({ key: thumbKey(slug, schema, row.name), versionId: row.version_id, size: row.size }))
}
