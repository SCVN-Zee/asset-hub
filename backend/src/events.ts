import { errorJson, json, readBody } from './http'
import type { RepoHandler } from './route'

const MAX_BODY_BYTES = 32 * 1024
const MAX_EVENTS = 100
const MAX_AGE_MS = 7 * 24 * 60 * 60 * 1000
const MAX_AHEAD_MS = 5 * 60 * 1000
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i
const SHA = /^[0-9a-f]{40}$/

interface UsageEvent {
  id: string
  sha: string
  kind: 'view' | 'send'
  ts: number
}

function validEvent(value: unknown, now: number): value is UsageEvent {
  if (typeof value !== 'object' || value === null) return false
  const e = value as Record<string, unknown>
  return (
    typeof e.id === 'string' &&
    UUID.test(e.id) &&
    typeof e.sha === 'string' &&
    SHA.test(e.sha) &&
    (e.kind === 'view' || e.kind === 'send') &&
    typeof e.ts === 'number' &&
    Number.isSafeInteger(e.ts) &&
    e.ts >= now - MAX_AGE_MS &&
    e.ts <= now + MAX_AHEAD_MS
  )
}

/** `POST /events`: anonymous, de-duplicated by id. Nothing here touches Redis. */
export const postEvents: RepoHandler = async ({ req, env, slug }) => {
  const body = await readBody(req, MAX_BODY_BYTES)
  if (body instanceof Response) return errorJson('badRequest', 400)

  let parsed: unknown
  try {
    parsed = JSON.parse(new TextDecoder().decode(body))
  } catch {
    return errorJson('badRequest', 400)
  }
  const list = (parsed as { events?: unknown } | null)?.events
  if (!Array.isArray(list) || list.length < 1 || list.length > MAX_EVENTS) return errorJson('badRequest', 400)

  const now = Date.now()
  const valid: UsageEvent[] = []
  const rejected: string[] = []
  for (const item of list) {
    if (validEvent(item, now)) valid.push(item)
    else if (typeof (item as { id?: unknown } | null)?.id === 'string') rejected.push((item as { id: string }).id.slice(0, 64))
  }

  if (valid.length > 0) {
    const insert = env.DB.prepare('INSERT OR IGNORE INTO events (id, repo, sha, kind, ts) VALUES (?, ?, ?, ?, ?)')
    await env.DB.batch(valid.map((e) => insert.bind(e.id.toLowerCase(), slug, e.sha, e.kind, e.ts)))
  }
  return json({ accepted: valid.length, rejected })
}
