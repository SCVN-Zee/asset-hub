// The asset types, as the Rust side declares them (`handler_descriptors`): which tabs exist, how each
// is delivered, and the two declarative rules the page applies to what it already holds (what a tab
// also lists, what lists leave out). Classifying files into assets happens in Rust (`browse_assets`).

import type { Asset, AssetFacts, FactKind, StatsSummary } from '@shared/types'
import type { HandlerDescriptor } from './parse'

export interface AssetTypeHandler {
  id: string
  /** i18n key for tabs and badges. */
  labelKey: string
  /** Short badge text shown on cards, e.g. 'FBX'. */
  badge: string
  preview: { kind: string }
  /** No tab and not under All: only listed by other tabs, once a scan has found them. */
  hidden: boolean
  /** Its dependencies can be resolved (a texture or font resolves to itself). */
  resolvesDependencies: boolean
  /** False when dependency resolution never looks a GUID up (a texture is just itself and its .meta). */
  usesGuids: boolean
  /** How a user takes it: a `.unitypackage` with its dependencies, or the file itself (a texture). */
  delivery: 'unitypackage' | 'file'
  /** The tab needs this scan of the branch to know what it lists (started when the tab is opened). */
  facts?: FactKind
  /** Models whose materials may be set on prefabs: offers the prefab material search. */
  materialsFromPrefabs: boolean
  /** Assets of other types this tab also lists, judged by what is known about them. */
  alsoShows?(asset: Asset, stats: StatsSummary | undefined, facts?: AssetFacts): boolean
  /** Left out of lists once measured to be of no use to look at; the grid offers to show them anyway. */
  unlisted?(stats: StatsSummary | undefined): boolean
}

function alsoShowsRule(rule: NonNullable<HandlerDescriptor['alsoShows']>): NonNullable<AssetTypeHandler['alsoShows']> {
  if (rule.kind === 'fact') return (asset, _stats, facts) => asset.type === rule.fromType && !!facts?.[rule.fact]?.has(asset.sha)
  // Models holding clips (measured), or named like animation files (Model@Run.fbx), are animations too.
  return (asset, stats) => asset.type === 'fbx' && ((stats?.clips ?? 0) > 0 || asset.name.includes('@'))
}

/** The handlers in the order the tabs show them. */
export class HandlerRegistry {
  private readonly handlers: AssetTypeHandler[]

  constructor(descriptors: readonly HandlerDescriptor[]) {
    this.handlers = descriptors.map((d) => ({
      id: d.id,
      labelKey: d.labelKey,
      badge: d.badge,
      preview: { kind: d.previewKind },
      hidden: d.hidden,
      resolvesDependencies: d.resolver !== null,
      usesGuids: d.usesGuids,
      delivery: d.delivery,
      ...(d.facts ? { facts: d.facts } : {}),
      materialsFromPrefabs: d.materialsFromPrefabs,
      ...(d.alsoShows ? { alsoShows: alsoShowsRule(d.alsoShows) } : {}),
      // Its card found no model to pose it on: curves alone say little.
      ...(d.unlisted === 'notRigged' ? { unlisted: (stats: StatsSummary | undefined) => stats?.rigged === false } : {})
    }))
  }

  get(id: string): AssetTypeHandler | undefined {
    return this.handlers.find((h) => h.id === id)
  }

  all(): readonly AssetTypeHandler[] {
    return this.handlers
  }
}
