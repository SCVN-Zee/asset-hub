import type { Env } from './env'
import { errorJson } from './http'
import { redis } from './redis'

/** How long a verified token or repo permission is trusted. Fixed from the lookup, never extended. */
export const ACCESS_TTL_SEC = 300

const GITHUB_API = 'https://api.github.com'
const GITHUB_TIMEOUT_MS = 5000

export interface AuthContext {
  login: string
  /** `hex(sha256(token))`: the only form of the token that is ever used as a key. */
  tokenKey: string
  /** The caller's token, kept in memory for this request only. Never stored or logged. */
  token: string
  /** Set by `assertRepoAccess`: whether the caller has push access to the repo. */
  canPush: boolean
}

interface GithubReply {
  status: number
  body: unknown
  rateLimited: boolean
}

async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, '0')).join('')
}

/** `undefined` when GitHub could not be reached. */
async function github(path: string, token: string): Promise<GithubReply | undefined> {
  try {
    const res = await fetch(`${GITHUB_API}${path}`, {
      headers: {
        authorization: `Bearer ${token}`,
        'user-agent': 'asset-hub-api',
        accept: 'application/vnd.github+json'
      },
      signal: AbortSignal.timeout(GITHUB_TIMEOUT_MS)
    })
    const rateLimited = res.status === 429 || (res.status === 403 && res.headers.get('x-ratelimit-remaining') === '0')
    let body: unknown = null
    if (res.ok) body = await res.json().catch(() => null)
    else await res.body?.cancel()
    return { status: res.status, body, rateLimited }
  } catch {
    return undefined
  }
}

const upstream = () => errorJson('upstream', 503)

function parseCached<T>(raw: string | null | undefined): T | undefined {
  if (!raw) return undefined
  try {
    return JSON.parse(raw) as T
  } catch {
    return undefined
  }
}

export function bearerToken(req: Request): string | undefined {
  const match = /^Bearer\s+(\S+)$/i.exec(req.headers.get('authorization') ?? '')
  return match?.[1]
}

export async function authenticate(req: Request, env: Env): Promise<AuthContext | Response> {
  const token = bearerToken(req)
  if (!token) return errorJson('auth', 401)

  const tokenKey = await sha256Hex(token)
  const cache = redis(env)
  const cacheKey = `auth:${tokenKey}`

  const cached = parseCached<{ login: string }>(await cache.get(cacheKey))
  if (cached?.login) return { login: cached.login, tokenKey, token, canPush: false }

  const reply = await github('/user', token)
  if (!reply || reply.rateLimited || reply.status >= 500) return upstream()
  const login = (reply.body as { login?: unknown } | null)?.login
  if (reply.status !== 200 || typeof login !== 'string') return errorJson('auth', 401)

  await cache.set(cacheKey, JSON.stringify({ login }), ACCESS_TTL_SEC)
  return { login, tokenKey, token, canPush: false }
}

function ownerAllowed(owner: string, env: Env): boolean {
  return env.ALLOWED_OWNERS.split(',')
    .map((o) => o.trim().toLowerCase())
    .filter(Boolean)
    .includes(owner.toLowerCase())
}

/** Proves the caller can read `owner/repo` and records whether they can push. */
export async function assertRepoAccess(
  ctx: AuthContext,
  owner: string,
  repo: string,
  env: Env
): Promise<true | Response> {
  if (!ownerAllowed(owner, env)) return errorJson('noAccess', 403)

  const slug = `${owner}/${repo}`.toLowerCase()
  const cache = redis(env)
  const cacheKey = `acl:${ctx.tokenKey}:${slug}`

  const cached = parseCached<{ pull: boolean; push: boolean }>(await cache.get(cacheKey))
  if (cached?.pull) {
    ctx.canPush = cached.push === true
    return true
  }

  const reply = await github(`/repos/${encodeURIComponent(owner)}/${encodeURIComponent(repo)}`, ctx.token)
  if (!reply || reply.rateLimited || reply.status >= 500) return upstream()
  const permissions = (reply.body as { permissions?: { pull?: unknown; push?: unknown } } | null)?.permissions
  if (reply.status !== 200 || permissions?.pull !== true) return errorJson('noAccess', 403)

  const access = { pull: true, push: permissions.push === true }
  await cache.set(cacheKey, JSON.stringify(access), ACCESS_TTL_SEC)
  ctx.canPush = access.push
  return true
}
