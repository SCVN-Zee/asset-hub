import type { PlayableBudget, StatsSummary } from '@shared/types'
import { useBudget } from '../budget'

/**
 * How much of the playable budget an asset uses: its heaviest measure as a
 * share of that limit (1 = right at the limit). Mirrors `summaryIssues`.
 */
export function budgetLoad(s: StatsSummary, budget: PlayableBudget): number {
  const shares = [s.triangles / budget.triangles, s.maxTextureSize / budget.textureSize, s.textureBytes / budget.textureBytes, s.bones / budget.bones]
  if (s.glyphs !== undefined) shares.push(s.fileBytes / budget.fontBytes)
  else if (s.duration !== undefined) shares.push(s.fileBytes / budget.audioBytes, s.duration / budget.audioSeconds)
  else shares.push(s.fileBytes / budget.fileBytes)
  if (s.particles !== undefined) shares.push(s.particles / budget.particles)
  return Math.max(0, ...shares.filter(Number.isFinite))
}

/** The asset's "weight" against the budget: a quiet bar that turns red past the limit. */
export function WeightBar({ stats }: { stats: StatsSummary }) {
  const budget = useBudget()
  // No weight to show when assets are not judged for playables.
  if (!budget) return null
  const load = budgetLoad(stats, budget)
  return (
    <span className="weight" data-over={load > 1} aria-hidden="true">
      <span className="weight-fill" style={{ width: `${Math.min(load, 1) * 100}%` }} />
    </span>
  )
}
