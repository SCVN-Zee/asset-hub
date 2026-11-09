import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { captureConsole } from '../test/capture-console'
import { chunkRankings, createSafeRedis, MAX_PIPELINE_BYTES, rankingCommands, type Ranked, type RedisClient } from './redis'

const asClient = (partial: Record<string, unknown>) => partial as unknown as RedisClient

describe('safe redis', () => {
  let output: ReturnType<typeof captureConsole>
  beforeEach(() => {
    output = captureConsole()
  })
  afterEach(() => {
    output.restore()
    vi.useRealTimers()
  })

  it('returns undefined on throw', async () => {
    const safe = createSafeRedis(() =>
      asClient({
        get: () => Promise.reject(new Error('boom')),
        set: () => {
          throw new Error('boom')
        }
      })
    )
    expect(await safe.get('k')).toBeUndefined()
    expect(await safe.set('k', 'v', 10)).toBeUndefined()
    expect(output.calls).toContainEqual(['redis.error', 'get'])
  })

  it('returns undefined after 300 ms', async () => {
    vi.useFakeTimers()
    const safe = createSafeRedis(() => asClient({ get: () => new Promise(() => {}) }))
    const pending = safe.get('k')
    await vi.advanceTimersByTimeAsync(299)
    let settled = false
    void pending.then(() => (settled = true))
    await Promise.resolve()
    expect(settled).toBe(false)
    await vi.advanceTimersByTimeAsync(1)
    expect(await pending).toBeUndefined()
  })

  it('maps a miss to null and a hit to its string', async () => {
    const values: Record<string, unknown> = { hit: '{"a":1}' }
    const safe = createSafeRedis(() => asClient({ get: async (k: string) => values[k] ?? null }))
    expect(await safe.get('hit')).toBe('{"a":1}')
    expect(await safe.get('miss')).toBeNull()
  })

  it('reads sorted-set members with scores', async () => {
    const safe = createSafeRedis(() => asClient({ zrange: async () => ['a', '3', 'b', '1.5'] }))
    expect(await safe.zrevrange('z', 0, 1)).toEqual([
      { member: 'a', score: 3 },
      { member: 'b', score: 1.5 }
    ])
  })

  describe('replaceRankings', () => {
    /** A client whose pipeline records the commands of each `exec`. */
    function pipelineClient(errorOnCommand = -1) {
      const execs: string[][] = []
      const client = {
        pipeline() {
          const commands: string[] = []
          const chain: Record<string, unknown> = {}
          for (const name of ['del', 'zadd', 'rename', 'expire', 'set']) {
            chain[name] = () => {
              commands.push(name)
              return chain
            }
          }
          chain.exec = async () => {
            execs.push(commands)
            return commands.map((_, i) => (i === errorOnCommand ? { error: 'ERR' } : { result: 'OK' }))
          }
          return chain
        }
      }
      return { client: asClient(client), execs }
    }
    const rankings = (n: number) =>
      Array.from({ length: n }, (_, i) => ({ key: `pop:o/r${i}`, members: [{ member: 'a'.repeat(40), score: 1 } satisfies Ranked] }))

    it('keeps 500 commands in one pipeline', async () => {
      const { client, execs } = pipelineClient()
      expect(await createSafeRedis(() => client).replaceRankings(rankings(100), 1, 60)).toBe(0)
      expect(execs).toHaveLength(1)
      expect(execs[0]).toHaveLength(500)
      expect(execs[0].slice(0, 5)).toEqual(['del', 'zadd', 'rename', 'expire', 'set'])
    })

    it('splits pipeline above 500 commands', async () => {
      const { client, execs } = pipelineClient()
      await createSafeRedis(() => client).replaceRankings(rankings(101), 1, 60)
      expect(execs.map((e) => e.length)).toEqual([500, 5])
    })

    const wide = (key: string, members: number, memberLength: number) => ({
      key,
      // Multi-byte characters: the byte length is double the character count.
      members: Array.from({ length: members }, (_, j) => ({ member: `${'é'.repeat(memberLength)}${j}`, score: 0.123456789 + j }))
    })
    const serialized = (chunk: Array<{ key: string; members: Ranked[] }>) =>
      new TextEncoder().encode(JSON.stringify(chunk.flatMap((r) => rankingCommands(r, 1, 60)))).length

    it('pipelines stay under 900 KiB by serialized size', async () => {
      // Each group is about 300 KiB serialized, though it has only 200 members.
      const groups = Array.from({ length: 7 }, (_, i) => wide(`pop:o/w${i}`, 200, 700))
      const { chunks, oversized } = chunkRankings(groups, 1, 60)
      expect(oversized).toBe(0)
      expect(chunks.length).toBeGreaterThan(1)
      for (const chunk of chunks) expect(serialized(chunk)).toBeLessThanOrEqual(MAX_PIPELINE_BYTES)
      expect(chunks.flat()).toEqual(groups)
    })

    it('oversized single group sent alone', async () => {
      const output = captureConsole()
      const { client, execs } = pipelineClient()
      const huge = wide('pop:o/huge', 200, 3000) // about 1.2 MB
      const rest = rankings(3)
      let failed: number
      try {
        failed = await createSafeRedis(() => client).replaceRankings([rest[0], huge, rest[1], rest[2]], 1, 60)
      } finally {
        output.restore()
      }
      expect(failed).toBe(0)
      expect(serialized([huge])).toBeGreaterThan(MAX_PIPELINE_BYTES)
      expect(execs.map((e) => e.length)).toEqual([5, 5, 10])
      expect(output.calls).toContainEqual(['redis.pipeline.oversized', 1])
    })

    it('counts failed commands without throwing', async () => {
      const { client } = pipelineClient(2)
      expect(await createSafeRedis(() => client).replaceRankings(rankings(2), 1, 60)).toBe(1)
    })

    it('counts a whole failed pipeline', async () => {
      const failing = asClient({
        pipeline: () => {
          throw new Error('boom')
        }
      })
      expect(await createSafeRedis(() => failing).replaceRankings(rankings(3), 1, 60)).toBe(15)
    })
  })
})
