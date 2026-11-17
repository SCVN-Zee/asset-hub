export const UPSTASH_TOKEN = 'test-redis-token'

export interface RankedMember {
  member: string
  score: number
}

type Entry = { kind: 'string'; value: string; expiresAt: number | null } | { kind: 'zset'; members: Map<string, number>; expiresAt: number | null }

/** One HTTP request to the fake: `/` carries one command, `/pipeline` carries a list. */
export interface UpstashRequest {
  path: string
  commands: string[][]
}

class CommandError extends Error {}

/**
 * Stand-in for the Upstash Redis REST API. It supports the commands the Worker uses (GET, SET with
 * EX, DEL, ZADD, RENAME, EXPIRE, ZRANGE with REV and WITHSCORES) and records every request. A
 * command it does not know goes to `unknown`, which fails the test in `afterEach`.
 *
 * Expiry follows the fake's own clock: `advance(ms)` moves it. The Worker's clock is not involved.
 */
export class FakeUpstash {
  requests: UpstashRequest[] = []
  unknown: string[] = []
  /** `true` makes every request answer 500. */
  down = false
  /** Delay before every answer, in milliseconds. */
  delayMs = 0
  /** Encodings the Worker asked for with `upstash-encoding`. */
  encodings: Array<string | null> = []
  private clock = 0
  private store = new Map<string, Entry>()

  reset(): void {
    this.requests = []
    this.unknown = []
    this.down = false
    this.delayMs = 0
    this.encodings = []
    this.clock = 0
    this.store.clear()
  }

  advance(ms: number): void {
    this.clock += ms
  }

  /** Upper-case command names received, in order, across every request. */
  commandNames(): string[] {
    return this.requests.flatMap((r) => r.commands.map((c) => String(c[0]).toUpperCase()))
  }

  /** Stores a string, as the Worker would after a `SET`. */
  seedString(key: string, value: string, ttlSec?: number): void {
    this.store.set(key, { kind: 'string', value, expiresAt: ttlSec === undefined ? null : this.clock + ttlSec * 1000 })
  }

  seedZset(key: string, members: RankedMember[]): void {
    this.store.set(key, { kind: 'zset', members: new Map(members.map((m) => [m.member, m.score])), expiresAt: null })
  }

  /** The members of a sorted set, best score first (ties: larger member first, as `ZRANGE REV`). */
  zset(key: string): RankedMember[] {
    const entry = this.live(key)
    if (entry?.kind !== 'zset') return []
    return sortedDesc(entry.members)
  }

  keys(): string[] {
    return [...this.store.keys()].filter((k) => this.live(k)).sort()
  }

  async handle(req: Request): Promise<Response> {
    const { pathname } = new URL(req.url)
    const encoding = req.headers.get('upstash-encoding')
    this.encodings.push(encoding)
    const reply = await this.answer(req, pathname, encoding === 'base64')
    // The command has run by now; only the answer is late, so a slow test cannot change a later one.
    if (this.delayMs > 0) await new Promise((resolve) => setTimeout(resolve, this.delayMs))
    return reply
  }

  private async answer(req: Request, pathname: string, base64: boolean): Promise<Response> {
    if (req.headers.get('authorization') !== `Bearer ${UPSTASH_TOKEN}`) {
      this.unknown.push('wrong token')
      return new Response('{"error":"Unauthorized"}', { status: 401 })
    }
    if (this.down) return new Response('down', { status: 500 })

    const parsed: unknown = await req.json().catch(() => null)
    if (pathname === '/pipeline' && Array.isArray(parsed)) {
      const commands = parsed as string[][]
      this.requests.push({ path: pathname, commands })
      return json(commands.map((c) => this.run(c, base64)))
    }
    if (pathname === '/' && Array.isArray(parsed)) {
      const commands = [parsed as string[]]
      this.requests.push({ path: pathname, commands })
      const [reply] = commands.map((c) => this.run(c, base64))
      return json(reply, 'error' in reply ? 400 : 200)
    }
    this.unknown.push(`request shape ${req.method} ${pathname}`)
    return json({ error: 'ERR unsupported request' }, 400)
  }

  private live(key: string): Entry | undefined {
    const entry = this.store.get(key)
    if (!entry) return undefined
    if (entry.expiresAt !== null && entry.expiresAt <= this.clock) {
      this.store.delete(key)
      return undefined
    }
    return entry
  }

  private run(command: string[], base64: boolean): { result: unknown } | { error: string } {
    try {
      const result = this.exec(command)
      return { result: base64 ? encode(result) : result }
    } catch (err) {
      if (err instanceof CommandError) return { error: err.message }
      throw err
    }
  }

  private exec(command: string[]): unknown {
    const [rawName, key, ...args] = command
    const name = String(rawName).toUpperCase()
    switch (name) {
      case 'GET': {
        const entry = this.live(key)
        if (!entry) return null
        if (entry.kind !== 'string') throw new CommandError('WRONGTYPE')
        return entry.value
      }
      case 'SET': {
        const [value, option, seconds] = args
        if (option !== undefined && String(option).toUpperCase() !== 'EX') return this.unsupported(command)
        this.store.set(key, {
          kind: 'string',
          value: String(value),
          expiresAt: option === undefined ? null : this.clock + Number(seconds) * 1000
        })
        return 'OK'
      }
      case 'DEL': {
        let removed = 0
        for (const k of [key, ...args]) if (this.live(k) && this.store.delete(k)) removed += 1
        return removed
      }
      case 'ZADD': {
        const entry = this.live(key)
        if (entry && entry.kind !== 'zset') throw new CommandError('WRONGTYPE')
        const members = entry?.kind === 'zset' ? entry.members : new Map<string, number>()
        let added = 0
        for (let i = 0; i + 1 < args.length; i += 2) {
          const member = String(args[i + 1])
          if (!members.has(member)) added += 1
          members.set(member, Number(args[i]))
        }
        this.store.set(key, { kind: 'zset', members, expiresAt: entry?.expiresAt ?? null })
        return added
      }
      case 'RENAME': {
        const entry = this.live(key)
        if (!entry) throw new CommandError('ERR no such key')
        this.store.delete(key)
        this.store.set(String(args[0]), entry)
        return 'OK'
      }
      case 'EXPIRE': {
        const entry = this.live(key)
        if (!entry) return 0
        entry.expiresAt = this.clock + Number(args[0]) * 1000
        return 1
      }
      case 'ZRANGE': {
        const flags = args.slice(2).map((a) => String(a).toUpperCase())
        if (flags.some((f) => f !== 'REV' && f !== 'WITHSCORES')) return this.unsupported(command)
        const entry = this.live(key)
        if (!entry) return []
        if (entry.kind !== 'zset') throw new CommandError('WRONGTYPE')
        const ordered = sortedDesc(entry.members)
        if (!flags.includes('REV')) ordered.reverse()
        const start = Number(args[0])
        const stop = Number(args[1])
        const picked = ordered.slice(start, stop < 0 ? undefined : stop + 1)
        return flags.includes('WITHSCORES') ? picked.flatMap((m) => [m.member, String(m.score)]) : picked.map((m) => m.member)
      }
      default:
        return this.unsupported(command)
    }
  }

  private unsupported(command: string[]): never {
    this.unknown.push(`command ${String(command[0]).toUpperCase()}`)
    throw new CommandError('ERR unsupported command')
  }
}

const sortedDesc = (members: Map<string, number>): RankedMember[] =>
  [...members.entries()].map(([member, score]) => ({ member, score })).sort((a, b) => b.score - a.score || (a.member < b.member ? 1 : -1))

/** The `upstash-encoding: base64` reply form: every string is base64, except the `OK` status. */
function encode(value: unknown): unknown {
  if (typeof value === 'string') return value === 'OK' ? value : Buffer.from(value, 'utf8').toString('base64')
  if (Array.isArray(value)) return value.map(encode)
  return value
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'content-type': 'application/json' } })
}
