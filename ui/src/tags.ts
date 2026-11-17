// The tags an asset can carry: chips on cards and in the inspector, the sub-categories of the Model and Prefab tabs, and words that
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

/** The sub-category that lists every asset of a tab. */
export const SUB_ALL = 'all'
/** The sub-category of the assets that have none of the tab's tags. */
export type SubRest = 'static' | 'other'
export type SubCategory = typeof SUB_ALL | AssetTag | SubRest

/**
 * The sub-categories of each tab (by handler id), as the tags that form them and the name of the remainder
 * (assets with none of those tags). A tab not listed here has no sub-categories. `tags.<id>` is the label of each.
 */
export const SUB_CATEGORIES: Readonly<Record<string, { tags: readonly AssetTag[]; rest: SubRest }>> = {
  fbx: { tags: ['animation'], rest: 'static' },
  prefab: { tags: ['ui', 'particleSystem'], rest: 'other' }
}

/** The sub-categories the tab shows, in order (`all` first), or none when the tab has none. */
export function subCategoriesOf(tab: string): SubCategory[] {
  const sub = SUB_CATEGORIES[tab]
  return sub ? [SUB_ALL, ...sub.tags, sub.rest] : []
}

/** Whether an asset with these tags belongs to a sub-category of the tab. An asset can belong to several tags' sub-categories. */
export function inSubCategory(tab: string, sub: SubCategory, tags: readonly AssetTag[] | undefined): boolean {
  const rule = SUB_CATEGORIES[tab]
  if (!rule || sub === SUB_ALL) return true
  const own = (tags ?? []).filter((tag) => rule.tags.includes(tag))
  return sub === rule.rest ? own.length === 0 : own.includes(sub as AssetTag)
}
