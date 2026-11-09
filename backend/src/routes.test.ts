import { describe, expect, it, vi } from 'vitest'
import { fakeRedis } from '../test/fake-redis'
import { api, denyRepo, repoPath, SCHEMA, useApiHarness } from '../test/helpers'
import { repoRoutes } from './index'

vi.mock('./redis', async (importOriginal) => {
  const { fakeRedis: fake } = await import('../test/fake-redis')
  return { ...(await importOriginal<typeof import('./redis')>()), redis: () => fake }
})

useApiHarness()

/** A concrete path for each route pattern. A new route must be added here, or the first test fails. */
const SAMPLE_PATHS: Record<string, string> = {
  '^\\/events$': '/events',
  '^\\/popular$': '/popular',
  '^\\/cache\\/([\\w-]+)$': '/cache/catalog',
  '^\\/thumbs$': '/thumbs',
  '^\\/thumbs\\/get$': '/thumbs/get',
  '^\\/thumbs\\/put$': '/thumbs/put'
}

describe('route table', () => {
  it('has a sample path for every route', () => {
    for (const route of repoRoutes) expect(SAMPLE_PATHS[route.pattern.source], route.pattern.source).toBeDefined()
  })

  it('every repo route calls assertRepoAccess', async () => {
    denyRepo('secret')
    for (const route of repoRoutes) {
      const res = await api(repoPath('secret', SAMPLE_PATHS[route.pattern.source]), {
        method: route.method,
        headers: { ...SCHEMA, 'if-none-match': '*' },
        body: route.method === 'GET' ? undefined : '{}'
      })
      expect(res.status, `${route.method} ${route.pattern.source}`).toBe(403)
      expect(await res.json()).toEqual({ error: 'noAccess' })
    }
  })

  it('every repo route requires a token', async () => {
    for (const route of repoRoutes) {
      const res = await api(repoPath('secret', SAMPLE_PATHS[route.pattern.source]), {
        method: route.method,
        token: null,
        body: route.method === 'GET' ? undefined : '{}'
      })
      expect(res.status, `${route.method} ${route.pattern.source}`).toBe(401)
    }
    expect(fakeRedis.calls).toEqual([])
  })
})
