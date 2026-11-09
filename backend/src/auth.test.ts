import { createExecutionContext, env, fetchMock, waitOnExecutionContext } from 'cloudflare:test'
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'
import { captureConsole } from '../test/capture-console'
import { fakeRedis } from '../test/fake-redis'
import worker, { repoRoutes } from './index'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

const TOKEN = 'tok-secret-9f3a7c1d'
const OWNER = 'supercent-io'

let testRoutes: typeof repoRoutes

beforeAll(() => {
  fetchMock.activate()
  fetchMock.disableNetConnect()
  testRoutes = [
    {
      method: 'GET',
      pattern: /^\/ping$/,
      handler: async ({ caller }) => Response.json({ login: caller.login, canPush: caller.canPush })
    },
    {
      method: 'GET',
      pattern: /^\/boom$/,
      handler: async () => {
        throw new Error(`failure that mentions ${TOKEN}`)
      }
    }
  ]
  repoRoutes.push(...testRoutes)
})

afterAll(() => {
  for (const route of testRoutes) repoRoutes.splice(repoRoutes.indexOf(route), 1)
})

beforeEach(() => {
  fakeRedis.reset()
  vi.useFakeTimers({ toFake: ['Date'] })
  vi.setSystemTime(new Date('2026-11-09T00:00:00Z'))
})

afterEach(() => {
  vi.useRealTimers()
  vi.restoreAllMocks()
  fetchMock.assertNoPendingInterceptors()
})

async function ping(path: string, token: string | null = TOKEN): Promise<Response> {
  const ctx = createExecutionContext()
  const res = await worker.fetch(
    new Request(`https://api.test${path}`, { headers: token ? { authorization: `Bearer ${token}` } : {} }),
    env,
    ctx
  )
  await waitOnExecutionContext(ctx)
  return res
}

const github = () => fetchMock.get('https://api.github.com')
const user = (status = 200, token = TOKEN) =>
  github()
    .intercept({ path: '/user', headers: { authorization: `Bearer ${token}` } })
    .reply(status, status === 200 ? { login: 'dev' } : { message: 'x' })
const repoReply = (
  repo: string,
  status: number,
  body: unknown = {},
  headers: Record<string, string> = {}
) =>
  github()
    .intercept({ path: `/repos/${OWNER}/${repo}`, headers: { authorization: `Bearer ${TOKEN}` } })
    .reply(status, body as object, { headers })
const readable = (push = false) => ({ permissions: { pull: true, push } })

describe('authentication and repo access', () => {
  it('rejects missing token', async () => {
    const res = await ping(`/v1/repos/${OWNER}/game/ping`, null)
    expect(res.status).toBe(401)
    expect(await res.json()).toEqual({ error: 'auth' })
  })

  it('rejects owner outside allowlist', async () => {
    user()
    const res = await ping('/v1/repos/someone-else/game/ping')
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'noAccess' })
  })

  it('denies repo B when only repo A is readable', async () => {
    user()
    repoReply('a', 200, readable())
    repoReply('b', 404, { message: 'Not Found' })
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)
    const denied = await ping(`/v1/repos/${OWNER}/b/ping`)
    expect(denied.status).toBe(403)
    expect(await denied.json()).toEqual({ error: 'noAccess' })
  })

  it('revoked token denied after auth cache expiry', async () => {
    user()
    repoReply('a', 200, readable())
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)

    vi.setSystemTime(Date.now() + 301_000)
    user(401)
    const res = await ping(`/v1/repos/${OWNER}/a/ping`)
    expect(res.status).toBe(401)
    expect(await res.json()).toEqual({ error: 'auth' })
  })

  it('revoked repo access denied after ACL cache expiry', async () => {
    user()
    repoReply('a', 200, readable())
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)

    // Within the TTL the cached answer is used: no GitHub call, so no pending interceptor.
    vi.setSystemTime(Date.now() + 299_000)
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)

    vi.setSystemTime(Date.now() + 2_000)
    user()
    repoReply('a', 404, { message: 'Not Found' })
    const res = await ping(`/v1/repos/${OWNER}/a/ping`)
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'noAccess' })
  })

  it('rate-limited 403 returns 503, not noAccess', async () => {
    user()
    repoReply('a', 403, { message: 'rate limit' }, { 'x-ratelimit-remaining': '0' })
    const res = await ping(`/v1/repos/${OWNER}/a/ping`)
    expect(res.status).toBe(503)
    expect(await res.json()).toEqual({ error: 'upstream' })
  })

  it('429 and 5xx return 503 and are not cached', async () => {
    user()
    repoReply('a', 429)
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(503)
    repoReply('a', 502)
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(503)
    repoReply('a', 200, readable())
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)
  })

  it('a plain 403 is noAccess and is never cached', async () => {
    user()
    repoReply('a', 403, { message: 'forbidden' })
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(403)
    repoReply('a', 200, readable())
    expect((await ping(`/v1/repos/${OWNER}/a/ping`)).status).toBe(200)
  })

  it('owner and repo are case-insensitive', async () => {
    user()
    repoReply('game', 200, readable())
    expect((await ping(`/v1/repos/${OWNER}/game/ping`)).status).toBe(200)
    // Same cache entry: no second GitHub call is mocked.
    expect((await ping('/v1/repos/SuperCent-IO/GAME/ping')).status).toBe(200)
  })

  it('push flag follows permissions.push', async () => {
    user()
    repoReply('rw', 200, readable(true))
    repoReply('ro', 200, readable(false))
    expect(await (await ping(`/v1/repos/${OWNER}/rw/ping`)).json()).toMatchObject({ canPush: true })
    expect(await (await ping(`/v1/repos/${OWNER}/ro/ping`)).json()).toMatchObject({ canPush: false })
    // From the cache.
    expect(await (await ping(`/v1/repos/${OWNER}/rw/ping`)).json()).toMatchObject({ canPush: true })
  })

  it('works when Redis is down', async () => {
    fakeRedis.down = true
    user()
    repoReply('a', 200, readable())
    const res = await ping(`/v1/repos/${OWNER}/a/ping`)
    expect(res.status).toBe(200)
    expect(await res.json()).toEqual({ login: 'dev', canPush: false })
  })

  it('never logs the token', async () => {
    const output = captureConsole()
    user()
    repoReply('a', 200, readable())
    await ping(`/v1/repos/${OWNER}/a/ping`)
    user(401, 'other-token-value')
    await ping(`/v1/repos/${OWNER}/a/ping`, 'other-token-value')
    fakeRedis.down = true
    user(500)
    await ping(`/v1/repos/${OWNER}/a/ping`)
    repoReply('a', 200, readable())
    user()
    const boom = await ping(`/v1/repos/${OWNER}/a/boom`)
    expect(boom.status).toBe(500)
    expect(await boom.text()).toBe('{"error":"internal"}')

    output.restore()
    const logged = JSON.stringify(output.calls)
    expect(output.calls.length).toBeGreaterThan(0)
    expect(logged).not.toContain(TOKEN)
    expect(logged).not.toContain('other-token-value')
  })
})
