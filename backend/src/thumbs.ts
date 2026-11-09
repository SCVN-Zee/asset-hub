import { errorJson, json, readBody, readSchema } from './http'
import { decodePack, encodeEntryHeader } from './pack'
import type { RepoHandler } from './route'

const NAME = /^[0-9a-f]{40}(\.fx)?$/
const CURSOR = /^(\d{1,16}):([0-9a-f]{40}(?:\.fx)?)$/
const MAX_LIST = 1000
const MAX_GET_NAMES = 40
const MAX_PUT_ENTRIES = 20
const MAX_PUT_BYTES = 4 * 1024 * 1024
const MAX_THUMB_BYTES = 1024 * 1024
const MAX_GET_BODY_BYTES = 8 * 1024

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

  return new Response(packStream(env.CACHE, slug, schema, [...new Set(names)]), {
    headers: { 'content-type': 'application/octet-stream', 'cache-control': 'private, no-store' }
  })
}

/**
 * Streams the pack one object at a time: header, then the R2 body as it arrives. Nothing is
 * buffered beyond the current chunk, and objects missing from R2 are skipped.
 */
function packStream(bucket: R2Bucket, slug: string, schema: number, names: string[]): ReadableStream<Uint8Array> {
  let next = 0
  let body: ReadableStreamDefaultReader<Uint8Array> | null = null
  let cancelled = false
  async function step(controller: ReadableStreamDefaultController<Uint8Array>): Promise<void> {
    for (;;) {
      if (cancelled) return
      if (body) {
        const { done, value } = await body.read()
        if (cancelled) return
        if (done) {
          body = null
          continue
        }
        controller.enqueue(value)
        return
      }
      if (next >= names.length) {
        controller.close()
        return
      }
      const name = names[next++]
      const object = await bucket.get(thumbKey(slug, schema, name))
      if (cancelled) {
        // The client left during the lookup: nothing will read this body.
        await object?.body.cancel().catch(() => {})
        return
      }
      if (!object) continue
      controller.enqueue(encodeEntryHeader(name, object.size))
      body = object.body.getReader()
      return
    }
  }

  return new ReadableStream<Uint8Array>({
    async pull(controller) {
      try {
        await step(controller)
      } catch (err) {
        // A read that fails because the client left needs no report; anything else ends the stream.
        if (!cancelled) controller.error(err)
      }
    },
    async cancel(reason) {
      cancelled = true
      await body?.cancel(reason).catch(() => {})
    }
  })
}

/** `POST /thumbs/put` with a binary pack: needs push access. Writes R2, then the D1 index. */
export const putThumbs: RepoHandler = async ({ req, env, slug, caller }) => {
  const schema = readSchema(req)
  if (schema instanceof Response) return schema
  if (!caller.canPush) return errorJson('readOnly', 403)

  const body = await readBody(req, MAX_PUT_BYTES)
  if (body instanceof Response) return body
  let entries
  try {
    entries = decodePack(body)
  } catch {
    return errorJson('badPack', 400)
  }
  if (entries.length < 1 || entries.length > MAX_PUT_ENTRIES) return errorJson('badRequest', 400)
  if (!entries.every((e) => NAME.test(e.name))) return errorJson('badRequest', 400)
  if (entries.some((e) => e.bytes.length > MAX_THUMB_BYTES)) return errorJson('tooLarge', 413)

  await Promise.all(entries.map((e) => env.CACHE.put(thumbKey(slug, schema, e.name), e.bytes)))

  // One commit per call: the revision moves up by one and every entry carries it. The batch is
  // atomic and runs in order, so concurrent writers get distinct revisions and a reader that
  // saw revision N never misses a later commit.
  const bump = env.DB.prepare(
    'INSERT INTO thumb_rev (repo, schema, rev) VALUES (?, ?, 1) ON CONFLICT (repo, schema) DO UPDATE SET rev = rev + 1'
  ).bind(slug, schema)
  const upsert = env.DB.prepare(
    'INSERT INTO thumbs (repo, schema, name, size, rev) VALUES (?, ?, ?, ?, (SELECT rev FROM thumb_rev WHERE repo = ? AND schema = ?)) ' +
      'ON CONFLICT (repo, schema, name) DO UPDATE SET size = excluded.size, rev = excluded.rev'
  )
  await env.DB.batch([bump, ...entries.map((e) => upsert.bind(slug, schema, e.name, e.bytes.length, slug, schema))])
  return json({ stored: entries.length })
}
