import { describe, expect, it } from 'vitest'
import { allowRepo, api, OWNER, repoPath, waitForLog, sha, upstash, useContract } from './harness'

useContract()

const key = `pop:${OWNER}/game`

describe('safe redis', () => {
  it('returns undefined on throw', async () => {
    allowRepo('game')
    upstash.down = true
    const res = await api(repoPath('game', '/popular'))
    // Access checks still pass through GitHub; only the Redis-backed answer fails.
    expect(res.status).toBe(503)
    expect(await res.json()).toEqual({ error: 'redisDown' })
    await waitForLog('redis.error')
  })

  it('returns undefined after 300 ms', async () => {
    allowRepo('game')
    upstash.seedZset(key, [{ member: sha(1), score: 1 }])
    upstash.delayMs = 50
    expect((await api(repoPath('game', '/popular'))).status).toBe(200)

    upstash.delayMs = 600
    const slow = await api(repoPath('game', '/popular'))
    expect(slow.status).toBe(503)
    expect(await slow.json()).toEqual({ error: 'redisDown' })
  })

  it('maps a miss to null and a hit to its string', async () => {
    allowRepo('game')
    expect(await (await api(repoPath('game', '/popular'))).json()).toEqual({ items: [], builtAt: null })
    upstash.seedZset(key, [{ member: sha(1), score: 1 }])
    upstash.seedString(`${key}:at`, '1700000000000')
    expect(await (await api(repoPath('game', '/popular'))).json()).toEqual({ items: [{ sha: sha(1), score: 1 }], builtAt: 1700000000000 })
  })

  it('reads sorted-set members with scores', async () => {
    allowRepo('game')
    upstash.seedZset(key, [
      { member: sha(2), score: 1.5 },
      { member: sha(1), score: 3 }
    ])
    const body = (await (await api(repoPath('game', '/popular'))).json()) as { items: unknown[] }
    expect(body.items).toEqual([
      { sha: sha(1), score: 3 },
      { sha: sha(2), score: 1.5 }
    ])
  })
})
