import { createExecutionContext, env, waitOnExecutionContext } from 'cloudflare:test'
import { describe, expect, it } from 'vitest'
import worker from './index'

async function call(path: string, init?: RequestInit): Promise<Response> {
  const ctx = createExecutionContext()
  const res = await worker.fetch(new Request(`https://api.test${path}`, init), env, ctx)
  await waitOnExecutionContext(ctx)
  return res
}

describe('router', () => {
  it('health returns 200 without auth', async () => {
    const res = await call('/v1/health')
    expect(res.status).toBe(200)
    const body = (await res.json()) as { ok: boolean; version: string }
    expect(body.ok).toBe(true)
    expect(typeof body.version).toBe('string')
  })

  it('unknown path returns 404 json', async () => {
    const res = await call('/nope')
    expect(res.status).toBe(404)
    expect(res.headers.get('content-type')).toContain('application/json')
    expect(await res.json()).toEqual({ error: 'notFound' })
  })

  it('unknown repo route returns 404 json', async () => {
    const res = await call('/v1/repos/supercent-io/game/unknown')
    expect(res.status).toBe(404)
    expect(await res.json()).toEqual({ error: 'notFound' })
  })

  it('sets no CORS headers', async () => {
    const res = await call('/v1/health')
    expect(res.headers.get('access-control-allow-origin')).toBeNull()
  })
})
