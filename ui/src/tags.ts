// The tags an asset can carry: chips on cards and in the inspector, filter chips above the grid, and words that
// search finds. `crates/core/src/tags.rs` lists the same tags (a test compares the two lists) and holds the words
// search matches. A tag follows from what is known about an asset: its type and name, what was measured (animation
// clips) and what the scans of the branch found (prefabs with particle systems or UI objects).

import type { Asset, AssetFacts, StatsSummary } from '@shared/types'

/** The tags, in the order the chips show them. `tags.<id>` is the label of each. */
export const ASSET_TAGS = ['animation', 'ui', 'particleSystem'] as const
export type AssetTag = (typeof ASSET_TAGS)[number]

/**
 * A model that holds animation clips (measured), or is named like an animation file (Model@Run.fbx).
 * The one rule behind the Animation tag and the animation tab's list of models (`isAnimatedModel` in tags.rs).
 */
export function isAnimatedModel(asset: Pick<Asset, 'type' | 'name'>, stats: Pick<StatsSummary, 'clips'> | undefined): boolean {
  return asset.type === 'fbx' && ((stats?.clips ?? 0) > 0 || asset.name.includes('@'))
}

/** The tags of an asset, from what is known about it now. */
export function tagsOf(asset: Asset, stats: Pick<StatsSummary, 'clips'> | undefined, facts: AssetFacts): AssetTag[] {
  const tags: AssetTag[] = []
  if (isAnimatedModel(asset, stats)) tags.push('animation')
  if (asset.type === 'prefab') {
    if (facts.uiPrefabs?.has(asset.sha)) tags.push('ui')
    if (facts.effects?.has(asset.sha)) tags.push('particleSystem')
  }
  return tags
}
