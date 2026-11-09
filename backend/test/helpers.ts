import { createExecutionContext, env, fetchMock, waitOnExecutionContext } from 'cloudflare:test'
import { beforeEach, afterEach } from 'vitest'
import worker from '../src/index'
import { fakeRedis } from './fake-redis'

/**
 * Each test gets its own token. GitHub mocks match on it, so a mock that persists from an earlier
 * test can never answer a later test.
 */
let tokenCounter = 0
let currentToken = 'tok-unset'
export const testToken = (): string => currentToken
export const OWNER = 'supercent-io'

/** Calls the Worker handler directly. Pass `token: null` to send no credentials. */
export async function api(
  path: string,
  init: { method?: string; headers?: Record<string, string>; body?: BodyInit; token?: string | null } = {}
): Promise<Response> {
  const { headers = {}, ...rest } = init
  const token = 'token' in init ? init.token : currentToken
  const ctx = createExecutionContext()
  const res = await worker.fetch(
    new Request(`https://api.test${path}`, { ...rest, ...(rest.body instanceof ReadableStream ? { duplex: 'half' } : {}), headers: { ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers } }),
    env,
    ctx
  )
  await waitOnExecutionContext(ctx)
  return res
}

export const repoPath = (repo: string, rest: string): string => `/v1/repos/${OWNER}/${repo}${rest}`

const github = () => fetchMock.get('https://api.github.com')
const asCaller = () => ({ authorization: `Bearer ${currentToken}` })

/** GitHub says the token is valid and `repo` is readable (and writable when `push`). */
export function allowRepo(repo: string, push = false): void {
  github().intercept({ path: '/user', headers: asCaller() }).reply(200, { login: 'dev' }).persist()
  github()
    .intercept({ path: `/repos/${OWNER}/${repo}`, headers: asCaller() })
    .reply(200, { permissions: { pull: true, push } })
    .persist()
}

/** GitHub says the token is valid but `repo` is not readable. */
export function denyRepo(repo: string): void {
  github().intercept({ path: '/user', headers: asCaller() }).reply(200, { login: 'dev' }).persist()
  github()
    .intercept({ path: `/repos/${OWNER}/${repo}`, headers: asCaller() })
    .reply(404, { message: 'Not Found' })
    .persist()
}

async function clearStorage(): Promise<void> {
  await env.DB.batch([env.DB.prepare('DELETE FROM events'), env.DB.prepare('DELETE FROM thumbs'), env.DB.prepare('DELETE FROM thumb_rev')])
  let cursor: string | undefined
  do {
    const page = await env.CACHE.list({ cursor })
    if (page.objects.length > 0) await env.CACHE.delete(page.objects.map((o) => o.key))
    cursor = page.truncated ? page.cursor : undefined
  } while (cursor)
}

/** Clears D1, R2 and Redis, and resets GitHub mocks, before every test of the calling file. */
export function useApiHarness(): void {
  beforeEach(async () => {
    tokenCounter += 1
    currentToken = `tok-test-${tokenCounter}`
    await clearStorage()
    fakeRedis.reset()
    fetchMock.activate()
    fetchMock.disableNetConnect()
  })
  afterEach(() => {
    fetchMock.deactivate()
  })
}

/** A body of `chunks` chunks of `chunkBytes` bytes with no known length. `pulled()` says how many were read. */
export function unsizedBody(chunks: number, chunkBytes: number, first?: Uint8Array): { body: ReadableStream<Uint8Array>; pulled: () => number } {
  let n = 0
  const body = new ReadableStream<Uint8Array>({
    pull(controller) {
      if (n >= chunks) return controller.close()
      const chunk = new Uint8Array(chunkBytes)
      if (n === 0 && first) chunk.set(first)
      n += 1
      controller.enqueue(chunk)
    }
  })
  return { body, pulled: () => n }
}

export const SCHEMA = { 'x-asset-hub-schema': '1' }

export const sha = (n: number): string => n.toString(16).padStart(40, '0')
