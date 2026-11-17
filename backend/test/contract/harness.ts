import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import type { Readable } from 'node:stream'
import { Miniflare, type WorkerOptions } from 'miniflare'
import { afterAll, afterEach, beforeAll, beforeEach, expect, inject, vi } from 'vitest'
import { backendDir } from './build-under-test'
import { B2_APP_KEY, B2_BUCKET, B2_ENDPOINT, B2_HOST, B2_KEY_ID, B2_REGION, FakeB2 } from './fake-b2'
import { FakeGithub } from './fake-github'
import { FakeUpstash, UPSTASH_TOKEN } from './fake-upstash'
import { FakeNetwork } from './http-server'

export const OWNER = 'supercent-io'
export const SCHEMA = { 'x-asset-hub-schema': '1' }
export const sha = (n: number): string => n.toString(16).padStart(40, '0')

const GITHUB_HOST = 'api.github.com'
const UPSTASH_HOST = 'redis.fake.test'
export const THUMB_SHARDS = 8

export const github = new FakeGithub()
export const upstash = new FakeUpstash()
export const b2 = new FakeB2()

let mf: Miniflare
let network: FakeNetwork
let stdio = ''
let tokenCounter = 0
let currentToken = 'tok-unset'
let hasThumbShards = false
let thumbShardSql: string[] = []

/** Each test gets its own token, so GitHub answers set up by one test never reach another. */
export const testToken = (): string => currentToken

export const repoPath = (repo: string, rest: string): string => `/v1/repos/${OWNER}/${repo}${rest}`

/** Sends a request to the Worker over HTTP. Pass `token: null` to send no credentials. */
export async function api(
  path: string,
  init: { method?: string; headers?: Record<string, string>; body?: BodyInit; token?: string | null } = {}
): Promise<Response> {
  const { headers = {}, token = currentToken, ...rest } = init
  const res = await mf.dispatchFetch(`http://worker.test${path}`, {
    ...rest,
    // Node requires this flag for a streamed request body.
    ...(rest.body instanceof ReadableStream ? { duplex: 'half' } : {}),
    headers: { ...(token ? { authorization: `Bearer ${token}` } : {}), ...headers }
  } as never)
  // Miniflare answers with undici's Response, which has the same surface the tests use.
  return res as unknown as Response
}

/** GitHub says the token is valid and `repo` is readable (and writable when `push`). */
export function allowRepo(repo: string, push = false): void {
  github.user(currentToken)
  github.repo(currentToken, OWNER, repo, 200, { permissions: { pull: true, push } })
}

/** GitHub says the token is valid but `repo` is not readable. */
export function denyRepo(repo: string): void {
  github.user(currentToken)
  github.repo(currentToken, OWNER, repo, 404, { message: 'Not Found' })
}

export function db(): Promise<D1Database> {
  return mf.getD1Database('DB') as unknown as Promise<D1Database>
}

/** One thumbnail shard database. The Worker writes thumbnails to them. */
export function shardDb(shard: number): Promise<D1Database> {
  return mf.getD1Database(`THUMBS_${shard}`) as unknown as Promise<D1Database>
}

/** Creates `table` of a shard again from its migration, after a test dropped it to break the shard. */
export async function restoreShardTable(shard: number, table: string): Promise<void> {
  const ddl = thumbShardSql.find((sql) => new RegExp(`CREATE TABLE ${table}\\b`).test(sql))
  if (!ddl) throw new Error(`no migration creates ${table}`)
  await (await shardDb(shard)).prepare(ddl).run()
}

/** Runs the statements of one migration file of the main database, as `wrangler d1 migrations apply` would. */
export async function applyMainMigration(fileName: string): Promise<void> {
  const statements = readFileSync(join(backendDir, 'migrations', fileName), 'utf8')
    .split(/;\s*(?:\n|$)/)
    .map((sql) => sql.replace(/^--.*$/gm, '').trim())
    .filter(Boolean)
  await applyMigrations(await db(), statements)
}

/** Bytes the Worker counts against its storage cap. */
export async function reservedBytes(): Promise<number> {
  return (await (await db()).prepare('SELECT reserved_bytes AS n FROM storage WHERE id = 1').first<{ n: number }>())!.n
}

/** Runs the cron trigger and waits for it to finish. The Worker picks the shard to sweep from `scheduledTime`. */
export async function runScheduled(scheduledTime?: Date): Promise<void> {
  const worker = await mf.getWorker()
  const result = await worker.scheduled({ cron: '17 */3 * * *', ...(scheduledTime ? { scheduledTime } : {}) })
  if (result.outcome !== 'ok') throw new Error(`scheduled run ended with ${result.outcome}`)
}

/**
 * Waits until the Worker's stdout or stderr contains `needle`, then returns everything written so
 * far. The runtime flushes its pipes after the request ends, so a test that asserts something is
 * absent from the log first waits for a line that must be there. Lines arrive in order.
 */
export async function waitForLog(needle: string): Promise<string> {
  await vi.waitFor(() => expect(stdio).toContain(needle), { timeout: 10_000, interval: 20 })
  return stdio
}

/** A body of `chunks` chunks of `chunkBytes` bytes with no known length. `pulled()` says how many were read. */
export function unsizedBody(
  chunks: number,
  chunkBytes: number,
  first?: Uint8Array,
  pauseMs = 0
): { body: ReadableStream<Uint8Array>; pulled: () => number } {
  let n = 0
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      if (n >= chunks) return controller.close()
      if (pauseMs > 0) await new Promise((resolve) => setTimeout(resolve, pauseMs))
      const chunk = new Uint8Array(chunkBytes)
      if (n === 0 && first) chunk.set(first)
      n += 1
      controller.enqueue(chunk)
    }
  })
  return { body, pulled: () => n }
}

function readSqlFiles(dir: string): string[] {
  try {
    return readdirSync(dir)
      .filter((f) => f.endsWith('.sql'))
      .sort()
      .flatMap((f) =>
        readFileSync(join(dir, f), 'utf8')
          .split(/;\s*(?:\n|$)/)
          .map((s) => s.trim())
          .filter(Boolean)
      )
  } catch {
    return []
  }
}

function moduleList(dir: string): WorkerOptions['modules'] {
  const files = readdirSync(dir)
  const entry = ['index.js', 'shim.mjs', 'shim.js'].find((f) => files.includes(f))
  if (!entry) throw new Error(`no entry module in ${dir}: ${files.join(', ')}`)
  const wanted = [entry, ...files.filter((f) => f !== entry && /\.(m?js|wasm)$/.test(f))]
  return wanted.map((f) =>
    f.endsWith('.wasm')
      ? { type: 'CompiledWasm' as const, path: join(dir, f), contents: readFileSync(join(dir, f)) }
      : { type: 'ESModule' as const, path: join(dir, f), contents: readFileSync(join(dir, f), 'utf8') }
  )
}

async function applyMigrations(database: D1Database, statements: string[]): Promise<void> {
  for (const sql of statements) await database.prepare(sql).run()
}

async function clearTables(database: D1Database): Promise<void> {
  const { results } = await database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite\\_%' ESCAPE '\\' AND name NOT LIKE '\\_cf\\_%' ESCAPE '\\' AND name NOT LIKE 'd1\\_%' ESCAPE '\\'").all<{ name: string }>()
  const names = results.map((r) => r.name)
  if (names.length === 0) return
  await database.batch(names.map((name) => database.prepare(`DELETE FROM "${name}"`)))
  if (names.includes('storage')) await database.prepare('INSERT INTO storage (id, reserved_bytes) VALUES (1, 0)').run()
  if (names.includes('usage')) await database.prepare('INSERT INTO usage (id, bytes) VALUES (1, 0)').run()
}

/**
 * Starts the bundled Worker in Miniflare with every outside service faked, and resets the faked
 * services, the D1 tables and the token before each test. Call it once at the top of a test file.
 */
export function useContract(): void {
  beforeAll(async () => {
    network = new FakeNetwork({
      [GITHUB_HOST]: (req) => github.handle(req),
      [UPSTASH_HOST]: (req) => upstash.handle(req),
      [B2_HOST]: (req) => b2.handle(req)
    })
    const port = await network.listen()
    const thumbShards = readSqlFiles(join(backendDir, 'migrations-thumbs'))
    thumbShardSql = thumbShards
    hasThumbShards = thumbShards.length > 0
    mf = new Miniflare({
      modules: moduleList(inject('bundleDir')),
      compatibilityDate: '2026-03-01',
      compatibilityFlags: [],
      d1Databases: {
        DB: 'contract-db',
        ...Object.fromEntries(Array.from({ length: THUMB_SHARDS }, (_, i) => [`THUMBS_${i}`, `contract-thumbs-${i}`]))
      },
      bindings: {
        ALLOWED_OWNERS: 'supercent-io,SCVN-Dan',
        UPSTASH_REDIS_REST_URL: `https://${UPSTASH_HOST}`,
        UPSTASH_REDIS_REST_TOKEN: UPSTASH_TOKEN,
        B2_ENDPOINT,
        B2_REGION,
        B2_BUCKET,
        B2_KEY_ID,
        B2_APP_KEY
      },
      outboundService: { external: { address: `127.0.0.1:${port}`, http: {} } },
      handleRuntimeStdio(stdout: Readable, stderr: Readable) {
        stdout.on('data', (d: Buffer) => (stdio += String(d)))
        stderr.on('data', (d: Buffer) => (stdio += String(d)))
      }
    })
    await applyMigrations(await db(), readSqlFiles(join(backendDir, 'migrations')))
    if (hasThumbShards) {
      for (let i = 0; i < THUMB_SHARDS; i++) {
        await applyMigrations((await mf.getD1Database(`THUMBS_${i}`)) as unknown as D1Database, thumbShards)
      }
    }
  })

  afterAll(async () => {
    await mf?.dispose()
    await network?.close()
  })

  beforeEach(async () => {
    tokenCounter += 1
    currentToken = `tok-test-${tokenCounter}`
    stdio = ''
    github.reset()
    upstash.reset()
    b2.reset()
    network.unknownHosts = []
    await clearTables(await db())
    if (hasThumbShards) {
      for (let i = 0; i < THUMB_SHARDS; i++) await clearTables((await mf.getD1Database(`THUMBS_${i}`)) as unknown as D1Database)
    }
  })

  afterEach(() => {
    const problems = [
      ...b2.badSignatures.map((p) => `bad B2 signature: ${p}`),
      ...github.unexpected.map((p) => `unexpected GitHub call: ${p}`),
      ...upstash.unknown.map((p) => `unknown Upstash request: ${p}`),
      ...network.unknownHosts.map((p) => `request to an unknown host: ${p}`)
    ]
    if (problems.length > 0) throw new Error(problems.join('\n'))
  })
}
