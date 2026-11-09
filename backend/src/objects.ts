import { errorJson, json, readBody, readSchema } from './http'
import { CACHE_OBJECT_NAMES } from './kinds'
import type { RepoHandler, RepoRequest } from './route'

const MAX_OBJECT_BYTES = 20 * 1024 * 1024

const hasBody = (o: R2Object | R2ObjectBody): o is R2ObjectBody => 'body' in o

/** Key of a cache object, or the 404/400 `Response` for an unknown name or a bad schema header. */
function objectKey({ req, slug, params }: RepoRequest): string | Response {
  const name = params[0]
  if (!CACHE_OBJECT_NAMES.includes(name)) return errorJson('notFound', 404)
  const schema = readSchema(req)
  if (schema instanceof Response) return schema
  return `repos/${slug}/s${schema}/${name}.json.gz`
}

/** `GET /cache/:name`: the stored gzip body, untouched. `If-None-Match` gives 304. */
export const getObject: RepoHandler = async (r) => {
  const key = objectKey(r)
  if (key instanceof Response) return key

  const ifNoneMatch = r.req.headers.get('if-none-match')
  const object = await r.env.CACHE.get(key, ifNoneMatch ? { onlyIf: new Headers({ 'if-none-match': ifNoneMatch }) } : undefined)
  if (!object) return errorJson('notFound', 404)
  // A conditional read that fails its condition returns the object without a body.
  const etag = object.httpEtag
  if (!hasBody(object)) return new Response(null, { status: 304, headers: { etag } })
  return new Response(object.body, {
    headers: { etag, 'content-type': 'application/gzip', 'cache-control': 'private, no-store' }
  })
}

/**
 * `PUT /cache/:name`: needs push access, a gzip body, and `If-Match: <etag>` (replace) or
 * `If-None-Match: *` (create). A lost race gives 412 and the client merges again.
 */
export const putObject: RepoHandler = async (r) => {
  const key = objectKey(r)
  if (key instanceof Response) return key
  if (!r.caller.canPush) return errorJson('readOnly', 403)

  const ifMatch = r.req.headers.get('if-match')
  const ifNoneMatch = r.req.headers.get('if-none-match')
  let onlyIf: Headers
  // A concrete ETag proves the writer saw the current version; `*` would not.
  if (ifMatch?.trim() === '*') return errorJson('preconditionRequired', 428)
  if (ifMatch) onlyIf = new Headers({ 'if-match': ifMatch })
  else if (ifNoneMatch === '*') onlyIf = new Headers({ 'if-none-match': '*' })
  else return errorJson('preconditionRequired', 428)

  const body = await readBody(r.req, MAX_OBJECT_BYTES)
  if (body instanceof Response) return body
  if (body.length < 2 || body[0] !== 0x1f || body[1] !== 0x8b) return errorJson('notGzip', 400)

  const stored = await r.env.CACHE.put(key, body, {
    onlyIf,
    httpMetadata: { contentType: 'application/gzip' }
  })
  if (!stored) return errorJson('preconditionFailed', 412)
  return json({ etag: stored.httpEtag }, 200, { etag: stored.httpEtag })
}
