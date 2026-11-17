import { describe, expect, it } from 'vitest'
import { api, denyRepo, repoPath, SCHEMA, upstash, useContract } from './harness'

useContract()

/** Every route under `/v1/repos/:owner/:repo`, with a concrete sample path. A new route must be added here. */
const ROUTES: Array<{ method: string; path: string }> = [
  { method: 'POST', path: '/events' },
  { method: 'GET', path: '/popular' },
  { method: 'GET', path: '/cache/catalog' },
  { method: 'PUT', path: '/cache/catalog' },
  { method: 'GET', path: '/thumbs' },
  { method: 'POST', path: '/thumbs/get' },
  { method: 'POST', path: '/thumbs/put' }
]

const label = (r: { method: string; path: string }): string => `${r.method} ${r.path}`

describe('route table', () => {
  it('has a sample path for every route', async () => {
    for (const route of ROUTES) {
      const res = await api(repoPath('secret', route.path), { method: route.method, token: null, body: route.method === 'GET' ? undefined : '{}' })
      expect(res.status, label(route)).not.toBe(404)
    }
    // A method or path outside the list is not routed at all.
    for (const stray of [
      { method: 'GET', path: '/events' },
      { method: 'DELETE', path: '/cache/catalog' },
      { method: 'POST', path: '/popular' },
      { method: 'PUT', path: '/thumbs/put' }
    ]) {
      const res = await api(repoPath('secret', stray.path), { method: stray.method, token: null })
      expect(res.status, label(stray)).toBe(404)
    }
  })

  it('every repo route calls assertRepoAccess', async () => {
    denyRepo('secret')
    for (const route of ROUTES) {
      const res = await api(repoPath('secret', route.path), {
        method: route.method,
        headers: { ...SCHEMA, 'if-none-match': '*' },
        body: route.method === 'GET' ? undefined : '{}'
      })
      expect(res.status, label(route)).toBe(403)
      expect(await res.json()).toEqual({ error: 'noAccess' })
    }
  })

  it('every repo route requires a token', async () => {
    for (const route of ROUTES) {
      const res = await api(repoPath('secret', route.path), {
        method: route.method,
        token: null,
        body: route.method === 'GET' ? undefined : '{}'
      })
      expect(res.status, label(route)).toBe(401)
    }
    expect(upstash.commandNames()).toEqual([])
  })
})
