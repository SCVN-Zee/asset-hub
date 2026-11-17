import { describe, expect, it } from 'vitest'
import { api, useContract } from './harness'
import { expectNoCors } from './raw'

useContract()

describe('router', () => {
  it('health returns 200 without auth', async () => {
    const res = await api('/v1/health', { token: null })
    expect(res.status).toBe(200)
    const body = (await res.json()) as { ok: boolean; version: string }
    expect(body.ok).toBe(true)
    expect(typeof body.version).toBe('string')
  })

  it('unknown path returns 404 json', async () => {
    const res = await api('/nope', { token: null })
    expect(res.status).toBe(404)
    expect(res.headers.get('content-type')).toContain('application/json')
    expect(await res.json()).toEqual({ error: 'notFound' })
  })

  it('unknown repo route returns 404 json', async () => {
    const res = await api('/v1/repos/supercent-io/game/unknown', { token: null })
    expect(res.status).toBe(404)
    expect(await res.json()).toEqual({ error: 'notFound' })
  })

  it('sets no CORS headers', async () => {
    const res = await api('/v1/health', { token: null })
    expect(res.headers.get('access-control-allow-origin')).toBeNull()
  })
})

describe('router raw bytes and headers', () => {
  it('health body and headers are exact', async () => {
    const res = await api('/v1/health', { token: null })
    expect(await res.text()).toBe('{"ok":true,"version":"0.1.0"}')
    expect(res.headers.get('content-type')).toBe('application/json; charset=utf-8')
    expectNoCors(res)
  })

  it('not found body and headers are exact', async () => {
    const res = await api('/nope', { token: null })
    expect(await res.text()).toBe('{"error":"notFound"}')
    expect(res.headers.get('content-type')).toBe('application/json; charset=utf-8')
    expectNoCors(res)
  })
})
