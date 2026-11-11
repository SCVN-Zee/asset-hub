import { PLAYABLE_BUDGET as BINDING_PLAYABLE_BUDGET, TIGHT_BUDGET as BINDING_TIGHT_BUDGET } from './bindings'
import type { BudgetIssue, BudgetPreset, ModelStats, PlayableBudget, StatsSummary } from '@shared/types'

export const BUDGET_KEYS = ['triangles', 'textureSize', 'textureBytes', 'fileBytes', 'bones', 'audioBytes', 'audioSeconds', 'fontBytes', 'particles'] as const satisfies ReadonlyArray<keyof PlayableBudget>

/** For networks taking a 5 MB playable (Google, AppLovin, ironSource, Unity…): the default. Generated from the Rust limits. */
export const PLAYABLE_BUDGET: PlayableBudget = BINDING_PLAYABLE_BUDGET

/** For networks capping the whole playable at 2 MB (Meta): every asset has to be lighter. */
export const TIGHT_BUDGET: PlayableBudget = BINDING_TIGHT_BUDGET

/** `off`: assets are not judged for playables at all (not everything is made for one). */
export const BUDGET_PRESETS = ['off', 'standard', 'tight', 'custom'] as const
/** Until someone picks a preset, nothing is judged against a playable budget. */
export const DEFAULT_BUDGET_PRESET: BudgetPreset = 'off'

/** The budget a preset stands for; `custom` fills what it does not set from the default. */
export function budgetFor(preset: BudgetPreset | undefined, custom?: Partial<PlayableBudget>): PlayableBudget {
  if (preset === 'tight') return TIGHT_BUDGET
  if (preset === 'custom') return { ...PLAYABLE_BUDGET, ...custom }
  return PLAYABLE_BUDGET
}

export function checkPlayableBudget(stats: ModelStats, budget = PLAYABLE_BUDGET): BudgetIssue[] {
  return summaryIssues(summarizeStats(stats, 'viewer'), budget)
}

const MAX_CLIP_NAMES = 30

export function summarizeStats(stats: ModelStats, source: StatsSummary['source']): StatsSummary {
  return {
    triangles: stats.triangles,
    vertices: stats.vertices,
    bones: stats.bones,
    clips: stats.animations.length,
    ...(stats.animations.length ? { clipNames: stats.animations.slice(0, MAX_CLIP_NAMES) } : {}),
    materials: stats.materials,
    fileBytes: stats.fileBytes,
    textureBytes: stats.textures.reduce((s, t) => s + t.bytes, 0),
    maxTextureSize: Math.max(0, ...stats.textures.map((t) => Math.max(t.width ?? 0, t.height ?? 0))),
    source
  }
}

export function summaryIssues(s: StatsSummary, budget = PLAYABLE_BUDGET): BudgetIssue[] {
  const issues: BudgetIssue[] = []
  if (s.triangles > budget.triangles) issues.push({ key: 'triangles', value: s.triangles, limit: budget.triangles })
  if (s.maxTextureSize > budget.textureSize) issues.push({ key: 'textureSize', value: s.maxTextureSize, limit: budget.textureSize })
  if (s.textureBytes > budget.textureBytes) issues.push({ key: 'textureBytes', value: s.textureBytes, limit: budget.textureBytes })
  // Audio clips and fonts are judged by their own budgets below.
  if (s.glyphs !== undefined) {
    if (s.fileBytes > budget.fontBytes) issues.push({ key: 'fontBytes', value: s.fileBytes, limit: budget.fontBytes })
  } else if (s.duration === undefined && s.fileBytes > budget.fileBytes) issues.push({ key: 'fileBytes', value: s.fileBytes, limit: budget.fileBytes })
  if (s.particles !== undefined && s.particles > budget.particles) issues.push({ key: 'particles', value: s.particles, limit: budget.particles })
  if (s.bones > budget.bones) issues.push({ key: 'bones', value: s.bones, limit: budget.bones })
  if (s.duration !== undefined) {
    if (s.fileBytes > budget.audioBytes) issues.push({ key: 'audioBytes', value: s.fileBytes, limit: budget.audioBytes })
    if (s.duration > budget.audioSeconds) issues.push({ key: 'audioLength', value: Math.round(s.duration), limit: budget.audioSeconds })
  }
  return issues
}

/** How a result list is ordered. `relevance` keeps the search order. */
export type SortMode = 'relevance' | 'playable' | 'smallest' | 'triangles' | 'used'

/**
 * Orders results for reuse in playables. Models not measured yet keep their
 * relative order after the measured ones (`used` needs no measurements).
 */
export function rankForPlayable<T>(
  items: readonly T[],
  mode: SortMode,
  statsOf: (item: T) => StatsSummary | undefined,
  usedOf: (item: T) => number = () => 0,
  /** Search match strength: a closer match always ranks above a lighter but looser one. */
  matchOf: (item: T) => number = () => 0,
  budget: PlayableBudget = PLAYABLE_BUDGET
): T[] {
  if (mode === 'relevance') return [...items]
  const keyed = items.map((item, index) => {
    const s = statsOf(item)
    const size = s ? s.fileBytes + s.textureBytes : Infinity
    const keys: number[] =
      mode === 'used'
        ? [-usedOf(item)]
        : !s
          ? [Infinity]
          : mode === 'playable'
            ? [summaryIssues(s, budget).length, size, -usedOf(item)]
            : mode === 'smallest'
              ? [size, s.triangles]
              : [s.width || s.textureCount !== undefined || s.duration !== undefined || s.clipLength !== undefined ? 1 : 0, s.triangles, size] // textures, materials, audio and clips (no triangles) after models
    return { item, index, keys: [-matchOf(item), ...keys] }
  })
  keyed.sort((a, b) => {
    for (let i = 0; i < Math.max(a.keys.length, b.keys.length); i++) {
      const d = (a.keys[i] ?? 0) - (b.keys[i] ?? 0)
      if (d && !Number.isNaN(d)) return d
      if (a.keys[i] === Infinity && b.keys[i] !== Infinity) return 1
      if (b.keys[i] === Infinity && a.keys[i] !== Infinity) return -1
    }
    return a.index - b.index
  })
  return keyed.map((k) => k.item)
}
