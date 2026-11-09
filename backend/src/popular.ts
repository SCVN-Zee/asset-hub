import type { Env } from './env'
import { errorJson, json } from './http'
import { REPLACE_RANKING_COMMANDS, redis } from './redis'
import type { RepoHandler } from './route'

const DAY_MS = 24 * 60 * 60 * 1000
const TOP_N = 200
const WINDOW_DAYS = 60
const RETENTION_DAYS = 90
const HALF_LIFE_DAYS = 14
const WEIGHT = { send: 3, view: 1 } as const
const RANKING_TTL_SEC = 7 * 24 * 60 * 60

/**
 * Redis budget (Upstash free tier: 500K commands a month). A rebuild runs 8 times a day and sends
 * `REPLACE_RANKING_COMMANDS` commands per repo, all in one pipeline request (so a single
 * subrequest, well inside Workers Free's 50), so 51 repos cost about
 * 5 x 51 x 8 x 30 = 61K commands a month. D1 reads are the event count of the last 60 days, 8 times a day.
 */
export function monthlyRedisCommands(repos: number, rebuildsPerDay = 8, days = 30): number {
  return REPLACE_RANKING_COMMANDS * repos * rebuildsPerDay * days
}

export const rankingKey = (slug: string): string => `pop:${slug}`

/** `GET /popular`: one Redis request. With Redis down the answer is 503, never a D1 scan. */
export const getPopular: RepoHandler = async ({ env, slug }) => {
  const ranking = await redis(env).ranking(rankingKey(slug), TOP_N)
  if (!ranking) return errorJson('redisDown', 503)
  return json({ items: ranking.items.map((r) => ({ sha: r.member, score: r.score })), builtAt: ranking.builtAt })
}

interface EventRow {
  repo: string
  sha: string
  kind: 'view' | 'send'
  ts: number
}

/** Score = sum of weight x 0.5^(ageDays / 14) per sha, per repo. */
export function scoreEvents(rows: EventRow[], now: number): Map<string, Map<string, number>> {
  const byRepo = new Map<string, Map<string, number>>()
  for (const row of rows) {
    const ageDays = Math.max(0, now - row.ts) / DAY_MS
    const score = WEIGHT[row.kind] * 0.5 ** (ageDays / HALF_LIFE_DAYS)
    const shas = byRepo.get(row.repo) ?? new Map<string, number>()
    shas.set(row.sha, (shas.get(row.sha) ?? 0) + score)
    byRepo.set(row.repo, shas)
  }
  return byRepo
}

/** Cron: rebuild every repo's ranking from D1, then drop events older than 90 days. */
export async function rebuildPopular(env: Env): Promise<void> {
  const now = Date.now()
  const { results } = await env.DB.prepare('SELECT repo, sha, kind, ts FROM events WHERE ts > ?')
    .bind(now - WINDOW_DAYS * DAY_MS)
    .all<EventRow>()

  const rankings = [...scoreEvents(results, now).entries()].map(([repo, shas]) => ({
    key: rankingKey(repo),
    members: [...shas.entries()]
      .map(([member, score]) => ({ member, score }))
      .sort((a, b) => b.score - a.score || a.member.localeCompare(b.member))
      .slice(0, TOP_N)
  }))

  if (rankings.length > 0) {
    const failed = await redis(env).replaceRankings(rankings, now, RANKING_TTL_SEC)
    if (failed > 0) console.error('popular.rebuild.failed', failed)
  }

  await env.DB.prepare('DELETE FROM events WHERE ts < ?')
    .bind(now - RETENTION_DAYS * DAY_MS)
    .run()
}
