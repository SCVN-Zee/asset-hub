import { B2Client, B2HttpError } from './b2'
import { errorJson, json, readBody, readSchema } from './http'
import { CACHE_OBJECT_NAMES } from './kinds'
import type { RepoHandler, RepoRequest } from './route'
import { discard, release, reserve } from './storage'

const MAX_OBJECT_BYTES = 20 * 1024 * 1024
const CONTENT_TYPE = 'application/gzip'

interface Head {
  etag: string
  key: string
  version_id: string
  size: number
}

/** Name and schema of a cache object, or the 404/400 `Response` for an unknown name or a bad schema header. */
function objectId({ req, params }: RepoRequest): { name: string; schema: number } | Response {
  const name = params[0]
  if (!CACHE_OBJECT_NAMES.includes(name)) return errorJson('notFound', 404)
  const schema = readSchema(req)
  if (schema instanceof Response) return schema
  return { name, schema }
}

const readHead = ({ env, slug }: RepoRequest, schema: number, name: string): Promise<Head | null> =>
  env.DB.prepare('SELECT etag, key, version_id, size FROM objects WHERE repo = ? AND schema = ? AND name = ?').bind(slug, schema, name).first<Head>()

/** True when an `If-None-Match` value (a list of ETags or `*`) names `etag`, ignoring weak markers. */
function etagListed(header: string, etag: string): boolean {
  return header.split(',').some((part) => {
    const tag = part.trim().replace(/^W\//, '')
    return tag === '*' || tag === etag
  })
}

/**
 * `GET /cache/:name`: the stored gzip body, untouched. `If-None-Match` gives 304 without a B2
 * call. A replace can delete the version between the head read and the body read, so a missing
 * body re-reads the head once; a second miss gives 503.
 */
export const getObject: RepoHandler = async (r) => {
  const id = objectId(r)
  if (id instanceof Response) return id
  const ifNoneMatch = r.req.headers.get('if-none-match')
  const b2 = new B2Client(r.env)

  for (let attempt = 0; attempt < 2; attempt++) {
    const head = await readHead(r, id.schema, id.name)
    if (!head) return errorJson('notFound', 404)
    if (ifNoneMatch && etagListed(ifNoneMatch, head.etag)) return new Response(null, { status: 304, headers: { etag: head.etag } })
    const res = await b2.get(head.key)
    if (res) {
      return new Response(res.body, { headers: { etag: head.etag, 'content-type': CONTENT_TYPE, 'cache-control': 'private, no-store' } })
    }
  }
  return errorJson('storageUnavailable', 503)
}

/** The precondition of a PUT: replace the head with this ETag, or create when none exists. */
type Precondition = { replace: string } | { create: true }

function readPrecondition(req: Request): Precondition | Response {
  const ifMatch = req.headers.get('if-match')
  // A concrete ETag proves the writer saw the current version; `*` would not.
  if (ifMatch?.trim() === '*') return errorJson('preconditionRequired', 428)
  if (ifMatch) return { replace: ifMatch.trim() }
  if (req.headers.get('if-none-match') === '*') return { create: true }
  return errorJson('preconditionRequired', 428)
}

/**
 * Uploads a new version. A failed upload releases its bytes only when B2 answered with an error;
 * a lost response leaves the outcome unknown, so those bytes stay charged.
 */
async function upload({ env }: RepoRequest, key: string, body: Uint8Array): Promise<string> {
  try {
    return await new B2Client(env).put(key, body, CONTENT_TYPE)
  } catch (err) {
    if (err instanceof B2HttpError) await release(env.DB, body.length).catch(() => {})
    throw err
  }
}

/** Whether the head is the version with `etag`; `null` when D1 cannot answer. */
async function isHead({ env, slug }: RepoRequest, schema: number, name: string, etag: string): Promise<boolean | null> {
  try {
    const row = await env.DB.prepare('SELECT 1 AS present FROM objects WHERE repo = ? AND schema = ? AND name = ? AND etag = ?').bind(slug, schema, name, etag).first()
    return row !== null
  } catch {
    return null
  }
}

/** Moves the head to the new version only if it still is the version the writer saw. */
async function switchHead({ env, slug }: RepoRequest, schema: number, name: string, pre: Precondition, next: Head): Promise<boolean> {
  const statement =
    'replace' in pre
      ? env.DB.prepare('UPDATE objects SET etag = ?, key = ?, version_id = ?, size = ? WHERE repo = ? AND schema = ? AND name = ? AND etag = ?').bind(
          next.etag, next.key, next.version_id, next.size, slug, schema, name, pre.replace
        )
      : env.DB.prepare('INSERT INTO objects (etag, key, version_id, size, repo, schema, name) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (repo, schema, name) DO NOTHING').bind(
          next.etag, next.key, next.version_id, next.size, slug, schema, name
        )
  return (await statement.run()).meta.changes > 0
}

/**
 * `PUT /cache/:name`: needs push access, a gzip body, and `If-Match: <etag>` (replace) or
 * `If-None-Match: *` (create). A lost race gives 412 and the client merges again. D1 decides the
 * winner; B2 only holds versioned keys, so a loser never overwrites the current object.
 */
export const putObject: RepoHandler = async (r) => {
  const id = objectId(r)
  if (id instanceof Response) return id
  if (!r.caller.canPush) return errorJson('readOnly', 403)
  const pre = readPrecondition(r.req)
  if (pre instanceof Response) return pre

  const body = await readBody(r.req, MAX_OBJECT_BYTES)
  if (body instanceof Response) return body
  if (body.length < 2 || body[0] !== 0x1f || body[1] !== 0x8b) return errorJson('notGzip', 400)

  // A cheap early exit. The conditional statement below stays the authority. For a replace, the
  // head with the expected ETag is the one that statement replaces, because ETags never repeat.
  const head = await readHead(r, id.schema, id.name)
  if ('replace' in pre ? head?.etag !== pre.replace : head !== null) return errorJson('preconditionFailed', 412)

  if (!(await reserve(r.env.DB, body.length))) return errorJson('storageFull', 507)

  const uuid = crypto.randomUUID()
  const key = `repos/${r.slug}/s${id.schema}/${id.name}/${uuid}.json.gz`
  const versionId = await upload(r, key, body)
  const next: Head = { etag: `"${uuid}"`, key, version_id: versionId, size: body.length }
  const fresh = { key, versionId, size: body.length }

  let switched: boolean
  try {
    switched = await switchHead(r, id.schema, id.name, pre, next)
  } catch (err) {
    // The call failed, but the commit may have happened. Only D1 can say which.
    const committed = await isHead(r, id.schema, id.name, next.etag)
    if (committed === false) await discard(r.env, [fresh])
    // Unknown: delete nothing and keep the bytes charged.
    if (committed !== true) throw err
    switched = true
  }
  if (!switched) {
    await discard(r.env, [fresh])
    return errorJson('preconditionFailed', 412)
  }
  if (head) await discard(r.env, [{ key: head.key, versionId: head.version_id, size: head.size }])
  // Cloudflare compresses JSON at the edge and then weakens the ETag header (`W/"…"`), which can
  // never satisfy `If-Match`. `no-transform` keeps the header strong; the body carries it as well.
  return json({ etag: next.etag }, 200, { etag: next.etag, 'cache-control': 'no-transform' })
}
