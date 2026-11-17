import { describe, expect, it } from 'vitest'
import { api, b2, build, github, OWNER, repoPath, waitForLog, SCHEMA, upstash, useContract } from './harness'
import { expectNoCors, JSON_TYPE } from './raw'

useContract()

const TOKEN = 'tok-secret-9f3a7c1d'
const GZIP = new Uint8Array([0x1f, 0x8b, 8, 0, 1, 2, 3])

/** An authenticated read that touches nothing but the access checks: an empty thumbnail index. */
const probe = (repo: string, token: string | null = TOKEN, owner = OWNER): Promise<Response> =>
  api(`/v1/repos/${owner}/${repo}/thumbs`, { headers: SCHEMA, token })

/** A write that needs push access. */
const write = (repo: string, name = 'catalog'): Promise<Response> =>
  api(repoPath(repo, `/cache/${name}`), { method: 'PUT', body: GZIP, headers: { ...SCHEMA, 'if-none-match': '*' }, token: TOKEN })

const readable = (push = false) => ({ permissions: { pull: true, push } })

describe('authentication and repo access', () => {
  it('rejects missing token', async () => {
    const res = await probe('game', null)
    expect(res.status).toBe(401)
    expect(await res.json()).toEqual({ error: 'auth' })
  })

  it('rejects owner outside allowlist', async () => {
    github.user(TOKEN)
    const res = await probe('game', TOKEN, 'someone-else')
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'noAccess' })
  })

  it('denies repo B when only repo A is readable', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    github.repo(TOKEN, OWNER, 'b', 404, { message: 'Not Found' })
    expect((await probe('a')).status).toBe(200)
    const denied = await probe('b')
    expect(denied.status).toBe(403)
    expect(await denied.json()).toEqual({ error: 'noAccess' })
  })

  it('revoked token denied after auth cache expiry', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    expect((await probe('a')).status).toBe(200)

    upstash.advance(301_000)
    github.user(TOKEN, 401)
    const res = await probe('a')
    expect(res.status).toBe(401)
    expect(await res.json()).toEqual({ error: 'auth' })
  })

  it('revoked repo access denied after ACL cache expiry', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    expect((await probe('a')).status).toBe(200)
    const before = github.calls.length

    // Within the TTL the cached answer is used: GitHub is not called again.
    upstash.advance(299_000)
    expect((await probe('a')).status).toBe(200)
    expect(github.calls).toHaveLength(before)

    upstash.advance(2_000)
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 404, { message: 'Not Found' })
    const res = await probe('a')
    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'noAccess' })
  })

  it('rate-limited 403 returns 503, not noAccess', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 403, { message: 'rate limit' }, { 'x-ratelimit-remaining': '0' })
    const res = await probe('a')
    expect(res.status).toBe(503)
    expect(await res.json()).toEqual({ error: 'upstream' })
  })

  it('429 and 5xx return 503 and are not cached', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 429)
    expect((await probe('a')).status).toBe(503)
    github.repo(TOKEN, OWNER, 'a', 502)
    expect((await probe('a')).status).toBe(503)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    expect((await probe('a')).status).toBe(200)
  })

  it('a plain 403 is noAccess and is never cached', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 403, { message: 'forbidden' })
    expect((await probe('a')).status).toBe(403)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    expect((await probe('a')).status).toBe(200)
  })

  it('owner and repo are case-insensitive', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'game', 200, readable())
    expect((await probe('game')).status).toBe(200)
    const before = github.calls.length
    // Same cache entry: GitHub is not called again.
    expect((await probe('GAME', TOKEN, 'SuperCent-IO')).status).toBe(200)
    expect(github.calls).toHaveLength(before)
  })

  it('push flag follows permissions.push', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'rw', 200, readable(true))
    github.repo(TOKEN, OWNER, 'ro', 200, readable(false))
    expect((await write('rw')).status).toBe(200)
    const readOnly = await write('ro')
    expect(readOnly.status).toBe(403)
    expect(await readOnly.json()).toMatchObject({ error: 'readOnly' })
    // From the cache: GitHub is not called again.
    const before = github.calls.length
    expect((await write('rw', 'stats')).status).toBe(200)
    expect(github.calls).toHaveLength(before)
  })

  it('works when Redis is down', async () => {
    upstash.down = true
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    const res = await probe('a')
    expect(res.status).toBe(200)
    expect(await res.json()).toEqual(build === 'rust' ? { items: [], epoch: 2 } : { items: [] })
  })

  it('never logs the token', async () => {
    const secrets = [TOKEN, 'other-token-value']
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    await probe('a')
    github.user('other-token-value', 401)
    await probe('a', 'other-token-value')
    upstash.down = true
    github.user(TOKEN, 500)
    await probe('a')
    // GitHub answers an error whose body and headers carry the token.
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'leaky', 500, { message: `bad credentials ${TOKEN}` }, { 'x-echo': `Bearer ${TOKEN}` })
    await probe('leaky')
    // GitHub loses the connection.
    github.drop('dropped-token-value')
    github.user('dropped-token-value')
    await probe('a', 'dropped-token-value')
    secrets.push('dropped-token-value')
    // A refused upload makes the Worker answer 500 and log the failure. The refusal echoes the token.
    github.repo(TOKEN, OWNER, 'a', 200, readable(true))
    github.user(TOKEN)
    b2.failWith = (call) => (call.method === 'PUT' ? 503 : null)
    b2.failReply = { body: `denied for ${TOKEN}`, headers: { 'x-echo': `Bearer ${TOKEN}` } }
    const boom = await write('a')
    expect(boom.status).toBe(500)
    expect(await boom.text()).toBe('{"error":"internal"}')

    // The 500 above logs `internal`; earlier lines come before it.
    const logged = await waitForLog('internal')
    for (const secret of secrets) expect(logged).not.toContain(secret)
  })

  it('never sends the raw token to Redis', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable(true))
    await probe('a')
    expect((await write('a')).status).toBe(200)
    expect(upstash.requests.length).toBeGreaterThan(0)
    expect(JSON.stringify(upstash.requests)).not.toContain(TOKEN)
    expect(upstash.keys().join('\n')).not.toContain(TOKEN)
  })
})

describe('authentication raw bytes and headers', () => {
  it('auth, noAccess and upstream errors are exact', async () => {
    const missing = await probe('game', null)
    expect(await missing.text()).toBe('{"error":"auth"}')
    expect(missing.headers.get('content-type')).toBe(JSON_TYPE)
    expectNoCors(missing)

    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 404, { message: 'Not Found' })
    const denied = await probe('a')
    expect(await denied.text()).toBe('{"error":"noAccess"}')
    expect(denied.headers.get('content-type')).toBe(JSON_TYPE)

    github.repo(TOKEN, OWNER, 'b', 502)
    const upstream = await probe('b')
    expect(await upstream.text()).toBe('{"error":"upstream"}')
    expect(upstream.headers.get('content-type')).toBe(JSON_TYPE)
  })

  it('Upstash requests use a reply encoding the fake knows', async () => {
    github.user(TOKEN)
    github.repo(TOKEN, OWNER, 'a', 200, readable())
    await probe('a')
    expect(upstash.requests.length).toBeGreaterThan(0)
    expect(upstash.encodings.every((e) => e === null || e === 'base64')).toBe(true)
    // An access check is one GET then one SET for the token, and again for the repo.
    expect(upstash.commandNames()).toEqual(['GET', 'SET', 'GET', 'SET'])
  })
})
