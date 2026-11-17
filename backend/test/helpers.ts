import { createExecutionContext, env, fetchMock, waitOnExecutionContext } from 'cloudflare:test'
import { beforeEach, afterEach, vi, type MockInstance } from 'vitest'
import worker from '../src/index'
import { fakeB2 } from './fake-b2'
import { fakeRedis } from './fake-redis'

export { fakeB2 }

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
  await env.DB.batch([
    env.DB.prepare('DELETE FROM events'),
    env.DB.prepare('DELETE FROM thumbs'),
    env.DB.prepare('DELETE FROM thumb_rev'),
    env.DB.prepare('DELETE FROM objects'),
    env.DB.prepare('UPDATE storage SET reserved_bytes = 0')
  ])
  fakeB2.reset()
}

/** Bytes the Worker counts against its storage cap. */
export async function reservedBytes(): Promise<number> {
  return (await env.DB.prepare('SELECT reserved_bytes AS n FROM storage WHERE id = 1').first<{ n: number }>())!.n
}

let d1Spies: MockInstance[] = []

/**
 * Makes D1 statements whose SQL matches `sql` throw when they run. `after` runs the statement first,
 * so the write is committed but the caller sees a failure, as with a lost response. `before` throws
 * without running it. The harness restores D1 after every test.
 */
export function breakD1(...rules: Array<{ sql: RegExp; mode: 'before' | 'after' }>): void {
  const real = env.DB.prepare.bind(env.DB)
  const lost = new Error('d1 lost')
  d1Spies.push(
    vi.spyOn(env.DB, 'prepare').mockImplementation((query: string) => {
      const rule = rules.find((r) => r.sql.test(query))
      const statement = real(query)
      if (!rule) return statement
      const wrap = (bound: D1PreparedStatement): D1PreparedStatement =>
        new Proxy(bound, {
          get(target, prop) {
            if (prop === 'bind') return (...values: unknown[]) => wrap(target.bind(...values))
            if (prop === 'run' || prop === 'first' || prop === 'all') {
              return async (...args: unknown[]) => {
                if (rule.mode === 'after') await (target[prop] as (...a: unknown[]) => Promise<unknown>)(...args)
                throw lost
              }
            }
            const value = Reflect.get(target, prop)
            return typeof value === 'function' ? value.bind(target) : value
          }
        })
      return wrap(statement)
    })
  )
}

/** Makes the next `env.DB.batch` call throw, after committing (`after`) or without running (`before`). Later calls run normally. */
export function breakD1Batch(mode: 'before' | 'after'): void {
  const real = env.DB.batch.bind(env.DB)
  const spy = vi.spyOn(env.DB, 'batch')
  d1Spies.push(spy)
  spy.mockImplementation((async (statements: D1PreparedStatement[]) => {
      spy.mockImplementation(real as never)
      if (mode === 'after') await real(statements)
      throw new Error('d1 lost')
    }) as never)
}

/** Clears D1, the fake B2 and Redis, and resets GitHub mocks, before every test of the calling file. */
export function useApiHarness(): void {
  beforeEach(async () => {
    tokenCounter += 1
    currentToken = `tok-test-${tokenCounter}`
    await clearStorage()
    fakeRedis.reset()
    fetchMock.activate()
    fetchMock.disableNetConnect()
    fakeB2.install()
  })
  afterEach(() => {
    fakeB2.uninstall()
    for (const spy of d1Spies) spy.mockRestore()
    d1Spies = []
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
