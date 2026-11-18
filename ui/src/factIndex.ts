import type { Asset, AssetFacts, FactKind, StatsSummary } from '@shared/types'
import type { HandlerRegistry } from './handlers'
import { inSubCategory, SUB_CATEGORIES, subCategoriesOf, tagsOf, type AssetTag, type SubCategory } from './tags'

/** A SHA no real blob has: what the handlers are asked about to learn which asset types read facts. */
const PROBE_SHA = 'probe'

/** Facts that hold the probe SHA under every kind. */
const EVERY_FACT = new Proxy({}, { get: () => new Set([PROBE_SHA]) }) as AssetFacts

/**
 * The asset types whose tab membership or tags depend on what scans found: the ones a handler's `alsoShows` or
 * `tagsOf` answers differently once the SHA of the asset is a known fact. Found by asking them, so a new rule needs no entry here.
 */
export function typesReadingFacts(registry: HandlerRegistry): Set<string> {
  const types = new Set<string>()
  for (const { id } of registry.all()) {
    // Only the fields the rules read are set.
    const probe = { id: PROBE_SHA, type: id, sha: PROBE_SHA, name: PROBE_SHA, path: PROBE_SHA } as Asset
    const showsDifferently = registry.all().some((h) => !!h.alsoShows && h.alsoShows(probe, undefined, EVERY_FACT) !== h.alsoShows(probe, undefined, {}))
    if (showsDifferently || tagsOf(probe, undefined, EVERY_FACT).length !== tagsOf(probe, undefined, {}).length) types.add(id)
  }
  return types
}

/** Adds SHAs to the facts in place. Returns the ones that were new (each once). */
export function rememberFacts(facts: Partial<Record<FactKind, Set<string>>>, kind: FactKind, shas: readonly string[]): string[] {
  const known = (facts[kind] ??= new Set())
  const fresh: string[] = []
  for (const sha of shas) {
    if (known.has(sha)) continue
    known.add(sha)
    fresh.push(sha)
  }
  return fresh
}

export interface FactIndexOptions {
  registry: HandlerRegistry
  /** The facts the page keeps: only `add` and `rememberFacts` change them. */
  facts: Partial<Record<FactKind, Set<string>>>
  /** Whether the asset is in the tab, judged with the given facts. */
  inTab(asset: Asset, tab: string, facts: AssetFacts): boolean
  statsOf(sha: string): StatsSummary | undefined
}

/**
 * What the sidebar and the cards take from the facts scans find: the count of each tab, the count of each
 * sub-category (`Other` and `Static` included) and the tags of the cards in the view. The counts are built once over a list
 * and do not depend on the open tab; `setView` sets which assets the tags are kept for, so a tab change costs the size of its list. When a
 * scan finds SHAs, `add` corrects the figures for the assets that have those SHAs and leaves the rest alone, so the
 * cost follows what was found and not the size of the list. Both ways count with the same `tally`, so they agree.
 */
export class FactIndex {
  /** Assets by type for the tabs, plus what tabs that list other types add. */
  readonly typeCounts = new Map<string, number>()
  /** For each tab that has sub-categories: the assets of the tab that belong to each one. */
  readonly subCounts = new Map<string, Map<SubCategory, number>>()
  /** The tags of each asset of the view that has any, by asset id. Changed in place by `add`. */
  readonly tags = new Map<string, AssetTag[]>()
  /** The assets whose type reads facts, by SHA (most SHAs have one asset). */
  private readonly bySha = new Map<string, Asset | Asset[]>()
  private inView: (asset: Asset, facts: AssetFacts) => boolean = () => false
  private readonly alsoShowing: string[]
  private readonly subTabs: Array<{ tab: string; subs: SubCategory[] }>

  constructor(
    source: readonly Asset[],
    private readonly options: FactIndexOptions
  ) {
    const { registry, facts } = options
    this.alsoShowing = registry.all().filter((h) => h.alsoShows).map((h) => h.id)
    this.subTabs = Object.keys(SUB_CATEGORIES).map((tab) => ({ tab, subs: subCategoriesOf(tab) }))
    for (const id of this.alsoShowing) this.typeCounts.set(id, 0)
    for (const { tab, subs } of this.subTabs) this.subCounts.set(tab, new Map(subs.map((sub) => [sub, 0])))
    const reading = typesReadingFacts(registry)
    for (const a of source) {
      this.typeCounts.set(a.type, (this.typeCounts.get(a.type) ?? 0) + 1)
      this.tally(a, facts, 1)
      if (reading.has(a.type)) {
        const had = this.bySha.get(a.sha)
        if (!had) this.bySha.set(a.sha, a)
        else if (Array.isArray(had)) had.push(a)
        else this.bySha.set(a.sha, [had, a])
      }
    }
  }

  /** Keeps the tags for the assets `inView` accepts, which `list` holds (the open view's list): the tags of any earlier view are dropped. */
  setView(inView: (asset: Asset, facts: AssetFacts) => boolean, list: readonly Asset[]) {
    this.inView = inView
    this.tags.clear()
    for (const a of list) this.retag(a, this.options.facts)
  }

  /**
   * Takes in SHAs a scan found. Repeated SHAs, and ones already known, change nothing. Returns whether the list of the
   * open view changed (an asset joined or left it), which needs a new list; the counts and tags are already right.
   * `changed` is false when no asset has any of the new SHAs.
   */
  add(kind: FactKind, shas: readonly string[]): { changed: boolean; viewChanged: boolean } {
    const { facts } = this.options
    const known = facts[kind]
    const fresh = [...new Set(shas)].filter((sha) => !known?.has(sha))
    const affected: Asset[] = []
    for (const sha of fresh) {
      const found = this.bySha.get(sha)
      if (Array.isArray(found)) affected.push(...found)
      else if (found) affected.push(found)
    }
    if (!affected.length) {
      rememberFacts(facts, kind, fresh)
      return { changed: false, viewChanged: false }
    }
    const inViewBefore = affected.map((a) => this.inView(a, facts))
    for (const a of affected) this.tally(a, facts, -1)
    rememberFacts(facts, kind, fresh)
    let viewChanged = false
    affected.forEach((a, i) => {
      this.tally(a, facts, 1)
      this.retag(a, facts)
      if (this.inView(a, facts) !== inViewBefore[i]) viewChanged = true
    })
    return { changed: true, viewChanged }
  }

  /** Counts the asset in (`sign` 1) or out (-1) of what depends on facts: the tabs that list other types and the sub-categories. */
  private tally(a: Asset, facts: AssetFacts, sign: 1 | -1) {
    const { inTab, statsOf } = this.options
    for (const id of this.alsoShowing) if (a.type !== id && inTab(a, id, facts)) this.typeCounts.set(id, (this.typeCounts.get(id) ?? 0) + sign)
    for (const { tab, subs } of this.subTabs) {
      if (!inTab(a, tab, facts)) continue
      const tags = tagsOf(a, statsOf(a.sha), facts)
      const counts = this.subCounts.get(tab)!
      for (const sub of subs) if (inSubCategory(tab, sub, tags)) counts.set(sub, (counts.get(sub) ?? 0) + sign)
    }
  }

  /** Sets or clears the tags of the asset by what is known now. */
  private retag(a: Asset, facts: AssetFacts) {
    const tags = this.inView(a, facts) ? tagsOf(a, this.options.statsOf(a.sha), facts) : []
    if (tags.length) this.tags.set(a.id, tags)
    else this.tags.delete(a.id)
  }
}
