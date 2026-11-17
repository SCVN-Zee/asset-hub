import { api, db, repoPath, SCHEMA, sha, shardDb, THUMB_SHARDS } from './harness'
import { encodePack, type PackEntry } from './pack'

export { decodePack, encodePack, type PackEntry } from './pack'

export const bytesFor = (n: number) => new Uint8Array([n % 256, (n >> 8) % 256, 7, 7])
export const entry = (n: number, suffix = ''): PackEntry => ({ name: `${sha(n)}${suffix}`, bytes: bytesFor(n) })

export const putPack = (entries: PackEntry[], headers: Record<string, string> = SCHEMA, repo = 'game') =>
  api(repoPath(repo, '/thumbs/put'), { method: 'POST', body: encodePack(entries), headers })

export const getPack = (names: unknown, headers: Record<string, string> = SCHEMA) =>
  api(repoPath('game', '/thumbs/get'), { method: 'POST', body: JSON.stringify({ names }), headers })

export interface ThumbList {
  items: Array<{ name: string; size: number; rev: number }>
  next?: string
}

export const list = async (query = '') => {
  const res = await api(repoPath('game', `/thumbs${query}`), { headers: SCHEMA })
  return { res, body: (await res.json()) as ThumbList }
}

/** A generation id as the Rust Worker writes it: a lower-case UUID. */
export const uuidFor = (n: number): string => `00000000-0000-4000-8000-${n.toString(16).padStart(12, '0')}`

/**
 * Writes index rows straight into D1, so a list test needs no upload. The one place that knows where
 * the index lives. A row names a generation of the Rust Worker unless `versionId` says otherwise
 * (`''` or a B2 version id make a row of the TypeScript Worker).
 */
export async function seedThumbs(rows: Array<{ repo: string; schema: number; name: string; rev: number; versionId?: string }>): Promise<void> {
  const database = await db()
  const insert = database.prepare('INSERT INTO thumbs (repo, schema, name, size, rev, version_id) VALUES (?, ?, ?, 4, ?, ?)')
  await database.batch(rows.map((r, i) => insert.bind(r.repo, r.schema, r.name, r.rev, r.versionId ?? uuidFor(i + 1))))
}

/** A name whose first hex digit is `digit`, so it lands in shard `digit % 8`. */
export const nameIn = (digit: number, n: number, suffix = ''): string => `${digit.toString(16)}${n.toString(16).padStart(39, '0')}${suffix}`
export const entryIn = (digit: number, n: number, suffix = ''): PackEntry => ({ name: nameIn(digit, n, suffix), bytes: bytesFor(digit * 16 + n) })
export const shardOf = (name: string): number => Number.parseInt(name[0], 16) % THUMB_SHARDS

export interface BlobRow {
  shard: number
  repo: string
  schema: number
  name: string
  gen: string
  size: number
}

/** Every thumbnail row in every shard. */
export async function blobRows(): Promise<BlobRow[]> {
  const rows: BlobRow[] = []
  for (let shard = 0; shard < THUMB_SHARDS; shard++) {
    const { results } = await (await shardDb(shard)).prepare('SELECT repo, schema, name, gen, length(bytes) AS size FROM blobs ORDER BY name, gen').all<Omit<BlobRow, 'shard'>>()
    rows.push(...results.map((r) => ({ ...r, shard })))
  }
  return rows
}

/** Bytes a shard counts against its cap. */
export async function shardUsage(shard: number): Promise<number> {
  return (await (await shardDb(shard)).prepare('SELECT bytes AS n FROM usage WHERE id = 1').first<{ n: number }>())!.n
}

export async function setShardUsage(shard: number, bytes: number): Promise<void> {
  await (await shardDb(shard)).prepare('UPDATE usage SET bytes = ? WHERE id = 1').bind(bytes).run()
}

export interface SeedBlob {
  repo: string
  schema: number
  name: string
  gen: string
  bytes: Uint8Array
  /** Insert time in milliseconds. Defaults to now. */
  created?: number
}

/** Writes shard rows straight into their shards. The one place that knows how a shard row looks. */
export async function seedBlobs(rows: SeedBlob[]): Promise<void> {
  for (let shard = 0; shard < THUMB_SHARDS; shard++) {
    const mine = rows.filter((r) => shardOf(r.name) === shard)
    if (mine.length === 0) continue
    const database = await shardDb(shard)
    const insert = database.prepare('INSERT INTO blobs (repo, schema, name, gen, bytes, created) VALUES (?, ?, ?, ?, ?, ?)')
    await database.batch(mine.map((r) => insert.bind(r.repo, r.schema, r.name, r.gen, Array.from(r.bytes), r.created ?? Date.now())))
  }
}

export const seedBlob = (row: SeedBlob): Promise<void> => seedBlobs([row])

/** Makes the rows of `name` look `ageMs` old. */
export async function ageBlobs(name: string, ageMs: number): Promise<void> {
  await (await shardDb(shardOf(name))).prepare('UPDATE blobs SET created = ? WHERE name = ?').bind(Date.now() - ageMs, name).run()
}

/** The generation the main index names for `name`, or null. */
export async function indexedGen(name: string): Promise<string | null> {
  const row = await (await db()).prepare('SELECT version_id FROM thumbs WHERE name = ?').bind(name).first<{ version_id: string }>()
  return row ? row.version_id : null
}
