import { B2Client } from './b2'
import type { Env } from './env'

/** Hard cap on bytes the Worker may hold in B2. The free tier ends at 10 GB. */
export const STORAGE_CAP_BYTES = 8 * 1024 * 1024 * 1024

/** A stored B2 version and the bytes it charges against the cap. */
export interface Blob {
  key: string
  versionId: string
  size: number
}

/**
 * Charges `bytes` against the cap before an upload. One conditional statement, so writers at the
 * same time cannot pass the cap together. `false` means the cap would be exceeded.
 */
export async function reserve(db: D1Database, bytes: number): Promise<boolean> {
  const result = await db
    .prepare('UPDATE storage SET reserved_bytes = reserved_bytes + ?1 WHERE id = 1 AND reserved_bytes + ?1 <= ?2')
    .bind(bytes, STORAGE_CAP_BYTES)
    .run()
  return result.meta.changes > 0
}

/**
 * Gives back bytes that are known to be free: a confirmed delete or a known-failed upload. When
 * the outcome is unknown, do not call this; the bytes stay charged and the cap stays honest.
 */
export async function release(db: D1Database, bytes: number): Promise<void> {
  if (bytes <= 0) return
  // The read and the write run in one batch, so the value read is the one the write subtracts from.
  const [before] = await db.batch<{ reserved_bytes: number }>([
    db.prepare('SELECT reserved_bytes FROM storage WHERE id = 1'),
    db.prepare('UPDATE storage SET reserved_bytes = MAX(0, reserved_bytes - ?) WHERE id = 1').bind(bytes)
  ])
  // Clamping hides a double release, so say so.
  if ((before.results[0]?.reserved_bytes ?? 0) < bytes) console.error('releaseUnderflow')
}

/**
 * Deletes versions and releases the bytes of those this call deleted. A failed delete is logged
 * and its bytes stay charged. A version B2 no longer has frees nothing now, so it releases nothing.
 * Call it only for versions D1 does not reference. Never throws.
 */
export async function discard(env: Env, blobs: readonly Blob[]): Promise<void> {
  if (blobs.length === 0) return
  const b2 = new B2Client(env)
  const outcomes = await Promise.allSettled(blobs.map((b) => b2.delete(b.key, b.versionId)))
  let freed = 0
  outcomes.forEach((outcome, i) => {
    if (outcome.status === 'fulfilled') {
      if (outcome.value) freed += blobs[i].size
    } else console.error('b2DeleteFailed', outcome.reason instanceof Error ? outcome.reason.message : 'unknown')
  })
  try {
    await release(env.DB, freed)
  } catch (err) {
    console.error('releaseFailed', err instanceof Error ? err.name : 'unknown')
  }
}
