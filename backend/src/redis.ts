import { Redis } from '@upstash/redis'
import type { Env } from './env'

/** Every Redis call gives up after this long; callers then behave as if Redis were down. */
export const REDIS_TIMEOUT_MS = 300
/** The cron rebuild is not latency sensitive; it gets a longer budget per pipeline. */
export const REDIS_REBUILD_TIMEOUT_MS = 5000

/**
 * Redis for derived data only. Each method resolves to `undefined` on an error or a timeout,
 * so a Redis outage never fails a request that can work without it.
 */
export interface SafeRedis {
  /** `null` is a miss; `undefined` is a failure. */
  get(key: string): Promise<string | null | undefined>
  set(key: string, value: string, ttlSec: number): Promise<boolean | undefined>
  zincrby(key: string, increment: number, member: string): Promise<number | undefined>
  zrevrange(key: string, start: number, stop: number): Promise<Array<{ member: string; score: number }> | undefined>
  /** Top `count` members of a ranking plus the time it was built (`null` if unknown), in one request. */
  ranking(key: string, count: number): Promise<{ items: Ranked[]; builtAt: number | null } | undefined>
  /**
   * Replaces many rankings, each as DEL tmp, ZADD tmp, RENAME tmp key, EXPIRE key, SET key:at.
   * All groups share one pipeline; a new pipeline starts only above `MAX_PIPELINE_COMMANDS`
   * commands or `MAX_PIPELINE_BYTES` serialized bytes. Resolves to the number of failed
   * commands (0 when everything worked). Each ranking must have at least one member.
   */
  replaceRankings(rankings: Array<{ key: string; members: Ranked[] }>, builtAt: number, ttlSec: number): Promise<number>
}

export interface Ranked {
  member: string
  score: number
}

/** Redis commands per ranking in `replaceRankings` (see `popular.ts` for the monthly budget). */
export const REPLACE_RANKING_COMMANDS = 5
/** Upstash accepts large pipelines; stay well below its request size and command limits. */
export const MAX_PIPELINE_COMMANDS = 500
export const MAX_PIPELINE_BYTES = 900 * 1024

/** The part of the Upstash client that the wrapper uses. */
export type RedisClient = Pick<Redis, 'get' | 'set' | 'zincrby' | 'zrange' | 'pipeline'>

function pairs(flat: string[]): Ranked[] {
  const out: Ranked[] = []
  for (let i = 0; i + 1 < flat.length; i += 2) out.push({ member: String(flat[i]), score: Number(flat[i + 1]) })
  return out
}

export function createSafeRedis(getClient: () => RedisClient): SafeRedis {
  async function guard<T>(
    command: string,
    run: (client: RedisClient) => Promise<T>,
    timeoutMs = REDIS_TIMEOUT_MS
  ): Promise<T | undefined> {
    let timer: ReturnType<typeof setTimeout> | undefined
    try {
      const timeout = new Promise<undefined>((resolve) => {
        timer = setTimeout(() => resolve(undefined), timeoutMs)
      })
      const result = await Promise.race([run(getClient()), timeout])
      if (result === undefined) console.error('redis.error', command, 'timeout')
      return result
    } catch {
      // The command name only: keys can hold hashes of tokens.
      console.error('redis.error', command)
      return undefined
    } finally {
      if (timer !== undefined) clearTimeout(timer)
    }
  }

  return {
    get: (key) =>
      guard('get', async (c) => {
        const value = await c.get<string>(key)
        return value === null || value === undefined ? null : String(value)
      }),
    set: (key, value, ttlSec) => guard('set', async (c) => (await c.set(key, value, { ex: ttlSec })) === 'OK'),
    zincrby: (key, increment, member) => guard('zincrby', (c) => c.zincrby(key, increment, member)),
    zrevrange: (key, start, stop) =>
      guard('zrevrange', async (c) => pairs(await c.zrange<string[]>(key, start, stop, { rev: true, withScores: true }))),
    ranking: (key, count) =>
      guard('ranking', async (c) => {
        const [flat, builtAt] = await c
          .pipeline()
          .zrange(key, 0, count - 1, { rev: true, withScores: true })
          .get(`${key}:at`)
          .exec<[string[], string | null]>()
        const at = builtAt === null ? Number.NaN : Number(builtAt)
        return { items: pairs(flat), builtAt: Number.isFinite(at) ? at : null }
      }),
    replaceRankings: async (rankings, builtAt, ttlSec) => {
      let failed = 0
      const { chunks, oversized } = chunkRankings(rankings, builtAt, ttlSec)
      if (oversized > 0) console.error('redis.pipeline.oversized', oversized)
      for (const chunk of chunks) {
        const result = await guard(
          'replaceRankings',
          async (c) => {
            const pipeline = c.pipeline()
            for (const { key, members } of chunk) {
              const tmp = `${key}:new`
              const [first, ...rest] = members.map((m) => ({ score: m.score, member: m.member }))
              pipeline
                .del(tmp)
                .zadd(tmp, first, ...rest)
                .rename(tmp, key)
                .expire(key, ttlSec)
                .set(`${key}:at`, String(builtAt), { ex: ttlSec })
            }
            const replies = await pipeline.exec({ keepErrors: true })
            return replies.filter((r) => r.error).length
          },
          REDIS_REBUILD_TIMEOUT_MS
        )
        failed += result ?? chunk.length * REPLACE_RANKING_COMMANDS
      }
      return failed
    }
  }
}

type RankingInput = { key: string; members: Ranked[] }

/** The commands of one ranking, as they are serialized into the pipeline request body. */
export function rankingCommands(
  { key, members }: RankingInput,
  builtAt: number,
  ttlSec: number
): Array<Array<string | number>> {
  const tmp = `${key}:new`
  return [
    ['DEL', tmp],
    ['ZADD', tmp, ...members.flatMap((m) => [m.score, m.member])],
    ['RENAME', tmp, key],
    ['EXPIRE', key, ttlSec],
    ['SET', `${key}:at`, String(builtAt), 'EX', ttlSec]
  ]
}

const utf8Bytes = (text: string): number => new TextEncoder().encode(text).length

/**
 * Splits rankings into groups that each fit one pipeline, measuring the real serialized size
 * (UTF-8 bytes of the JSON command array). A ranking is never split: one that alone exceeds
 * the limit is sent alone and counted in `oversized`.
 */
export function chunkRankings(
  rankings: RankingInput[],
  builtAt: number,
  ttlSec: number
): { chunks: RankingInput[][]; oversized: number } {
  const chunks: RankingInput[][] = []
  let current: RankingInput[] = []
  let bytes = 2 // the enclosing "[" and "]"
  let oversized = 0
  for (const ranking of rankings) {
    const size = utf8Bytes(JSON.stringify(rankingCommands(ranking, builtAt, ttlSec))) - 1 // minus the outer brackets, plus a comma
    if (size > MAX_PIPELINE_BYTES) oversized += 1
    const full =
      (current.length + 1) * REPLACE_RANKING_COMMANDS > MAX_PIPELINE_COMMANDS || bytes + size > MAX_PIPELINE_BYTES
    if (current.length > 0 && full) {
      chunks.push(current)
      current = []
      bytes = 2
    }
    current.push(ranking)
    bytes += size
  }
  if (current.length > 0) chunks.push(current)
  return { chunks, oversized }
}

export function redis(env: Env): SafeRedis {
  let client: Redis | undefined
  return createSafeRedis(() => {
    // Raw strings only (a 40-hex SHA of digits must not turn into a number), and no retries:
    // the 300 ms budget covers one attempt.
    client ??= new Redis({
      url: env.UPSTASH_REDIS_REST_URL,
      token: env.UPSTASH_REDIS_REST_TOKEN,
      automaticDeserialization: false,
      retry: false
    })
    return client
  })
}
