/** JSON response with a stable shape. Failures use `{ error: <code> }`. */
export function json(body: unknown, status = 200, headers?: Record<string, string>): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8', ...headers }
  })
}

export function errorJson(error: string, status: number, extra?: Record<string, unknown>): Response {
  return json({ error, ...extra }, status)
}

/** The cache namespace number the desktop sends. Missing or malformed gives a 400 `Response`. */
export function readSchema(req: Request): number | Response {
  const raw = req.headers.get('x-asset-hub-schema')
  if (raw === null || !/^\d{1,6}$/.test(raw)) return errorJson('badSchema', 400)
  return Number(raw)
}

/**
 * Reads a request body of at most `maxBytes`. Returns a 413 `Response` when it is larger. An
 * oversized `Content-Length` exits early; otherwise the stream is counted chunk by chunk and
 * cancelled as soon as the limit is passed, so an oversized body is never buffered.
 */
export async function readBody(req: Request, maxBytes: number): Promise<Uint8Array | Response> {
  const declared = Number(req.headers.get('content-length') ?? Number.NaN)
  if (Number.isFinite(declared) && declared > maxBytes) return errorJson('tooLarge', 413)
  if (!req.body) return new Uint8Array()

  const reader = req.body.getReader()
  const chunks: Uint8Array[] = []
  let total = 0
  for (;;) {
    const { done, value } = await reader.read()
    if (done) break
    total += value.length
    if (total > maxBytes) {
      await reader.cancel().catch(() => {})
      return errorJson('tooLarge', 413)
    }
    chunks.push(value)
  }
  const out = new Uint8Array(total)
  let at = 0
  for (const chunk of chunks) {
    out.set(chunk, at)
    at += chunk.length
  }
  return out
}
