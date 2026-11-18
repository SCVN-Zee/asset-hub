import { describe, expect, it } from 'vitest'
import type { Asset, AssetFacts, FactKind } from '@shared/types'
import { FactIndex, rememberFacts, typesReadingFacts } from './factIndex'
import { HandlerRegistry } from './handlers'
import type { HandlerDescriptor } from './parse'
import { inSubCategory, SUB_CATEGORIES, subCategoriesOf, tagsOf } from './tags'

const base = { labelKey: 'x', badge: 'X', previewKind: 'three', materialsFromPrefabs: false, usesGuids: true, facts: null, hidden: false, delivery: 'unitypackage', alsoShows: null, unlisted: null, resolver: null } as const
const registry = new HandlerRegistry([
  { ...base, id: 'fbx' },
  { ...base, id: 'prefab' },
  { ...base, id: 'texture' },
  { ...base, id: 'vfx', alsoShows: { kind: 'fact', fromType: 'prefab', fact: 'effects' } },
  { ...base, id: 'sprite', alsoShows: { kind: 'fact', fromType: 'texture', fact: 'sprites' } },
  { ...base, id: 'animation', alsoShows: { kind: 'animatedModel' } }
] as HandlerDescriptor[])

const asset = (type: string, id: string, sha: string, name = id): Asset => ({ id, type, path: id, name, sha, lfs: false })
// Twelve prefabs (two pairs share a blob), six textures and some models.
const source: Asset[] = [
  ...Array.from({ length: 12 }, (_, i) => asset('prefab', `p${i}.prefab`, `p${i % 10}`)),
  ...Array.from({ length: 6 }, (_, i) => asset('texture', `t${i}.png`, `t${i}`)),
  asset('fbx', 'Hero@Run.fbx', 'm0'),
  asset('fbx', 'Hero.fbx', 'm1')
]

/** What the page computed before the figures were kept up to date: every list filtered again from the facts. */
function reference(facts: AssetFacts, inTab: (a: Asset, tab: string) => boolean, view: string) {
  const typeCounts = new Map<string, number>()
  for (const a of source) typeCounts.set(a.type, (typeCounts.get(a.type) ?? 0) + 1)
  for (const h of registry.all()) if (h.alsoShows) typeCounts.set(h.id, (typeCounts.get(h.id) ?? 0) + source.filter((a) => a.type !== h.id && inTab(a, h.id)).length)
  const subCounts = new Map<string, Map<string, number>>()
  for (const tab of Object.keys(SUB_CATEGORIES)) {
    const tagged = source.filter((a) => inTab(a, tab)).map((a) => tagsOf(a, undefined, facts))
    subCounts.set(tab, new Map(subCategoriesOf(tab).map((sub) => [sub, tagged.filter((tags) => inSubCategory(tab, sub, tags)).length])))
  }
  const tags = new Map<string, string[]>()
  for (const a of source.filter((a) => inTab(a, view))) {
    const own = tagsOf(a, undefined, facts)
    if (own.length) tags.set(a.id, own)
  }
  return { typeCounts, subCounts, tags }
}

function setup(view: string) {
  const facts: Partial<Record<FactKind, Set<string>>> = {}
  const inTab = (a: Asset, tab: string, f: AssetFacts) => (a.type === tab && tab !== 'all') || !!registry.get(tab)?.alsoShows?.(a, undefined, f)
  const inView = (a: Asset, f: AssetFacts) => inTab(a, view, f)
  const index = new FactIndex(source, { registry, facts, inTab, statsOf: () => undefined })
  index.setView(inView, source.filter((a) => inView(a, facts)))
  const expectSameAsReference = () => {
    const expected = reference(facts, (a, tab) => inTab(a, tab, facts), view)
    expect(index.typeCounts).toEqual(expected.typeCounts)
    expect(index.subCounts).toEqual(expected.subCounts)
    expect(index.tags).toEqual(expected.tags)
    // A new index over the same facts agrees as well.
    const fresh = new FactIndex(source, { registry, facts, inTab, statsOf: () => undefined })
    fresh.setView(inView, source.filter((a) => inView(a, facts)))
    expect(index.typeCounts).toEqual(fresh.typeCounts)
    expect(index.subCounts).toEqual(fresh.subCounts)
    expect(index.tags).toEqual(fresh.tags)
  }
  return { facts, index, expectSameAsReference }
}

const BATCHES: Array<[FactKind, string[]]> = [
  ['effects', ['p0', 'p1']],
  ['effects', ['p1', 'p1', 'p2']], // repeated within a batch and across batches
  ['uiPrefabs', ['p2', 'p3']], // p2 now has both tags, so it is not Other and counts once under each
  ['uiPrefabs', []],
  ['effects', ['nothing-has-this']],
  ['sprites', ['t0', 't0', 't5']],
  ['effects', ['p0', 'p3', 'p9']], // p0 repeats, p3 gains its second tag
  ['uiPrefabs', ['p0', 'p1', 'p2', 'p3', 'p4', 'p5', 'p6', 'p7', 'p8', 'p9']]
]

describe('FactIndex', () => {
  it.each(['prefab', 'vfx', 'sprite', 'texture'])('counts and tags match a full recount after every batch (view %s)', (view) => {
    const { index, expectSameAsReference } = setup(view)
    expectSameAsReference()
    for (const [kind, shas] of BATCHES) {
      index.add(kind, shas)
      expectSameAsReference()
    }
  })

  it('moves a prefab out of Other when either tag appears, and counts both assets of a shared blob', () => {
    const { index } = setup('prefab')
    const other = () => index.subCounts.get('prefab')!.get('other')
    expect(other()).toBe(12)
    index.add('effects', ['p0']) // p0 is held by two assets
    expect(other()).toBe(10)
    index.add('uiPrefabs', ['p0']) // already out of Other: no change
    expect(other()).toBe(10)
    expect(index.subCounts.get('prefab')!.get('particleSystem')).toBe(2)
    expect(index.subCounts.get('prefab')!.get('ui')).toBe(2)
    expect(index.tags.get('p0.prefab')).toEqual(['ui', 'particleSystem'])
  })

  it('reports a change only when an asset has a new SHA, and a view change only when the open list gained or lost an asset', () => {
    const prefabTab = setup('prefab')
    expect(prefabTab.index.add('effects', ['nothing-has-this'])).toEqual({ changed: false, viewChanged: false })
    expect(prefabTab.index.add('effects', ['p4'])).toEqual({ changed: true, viewChanged: false })
    expect(prefabTab.index.add('effects', ['p4'])).toEqual({ changed: false, viewChanged: false })
    const vfxTab = setup('vfx')
    expect(vfxTab.index.add('effects', ['p4'])).toEqual({ changed: true, viewChanged: true })
    expect(vfxTab.index.add('uiPrefabs', ['p4'])).toEqual({ changed: true, viewChanged: false })
    expect(vfxTab.index.typeCounts.get('vfx')).toBe(1)
  })

  it('changes the view without touching the counts, and keeps its tags right as facts arrive', () => {
    const { index } = setup('prefab')
    index.add('effects', ['p0'])
    const counts = structuredClone([...index.subCounts].map(([tab, m]) => [tab, [...m]]))
    index.setView((a) => a.type === 'texture', source.filter((a) => a.type === 'texture'))
    expect(index.tags.size).toBe(0)
    expect([...index.subCounts].map(([tab, m]) => [tab, [...m]])).toEqual(counts)
    index.add('sprites', ['t1'])
    expect(index.add('sprites', ['t1'])).toEqual({ changed: false, viewChanged: false })
  })

  it('keeps the facts of SHAs no asset has, for a list that holds them later', () => {
    const { facts, index } = setup('prefab')
    index.add('effects', ['later'])
    expect(facts.effects?.has('later')).toBe(true)
  })

  it('finds the asset types that read facts by asking the handlers', () => {
    expect([...typesReadingFacts(registry)].sort()).toEqual(['prefab', 'texture'])
  })

  it('adds only the new SHAs to the facts', () => {
    const facts: Partial<Record<FactKind, Set<string>>> = {}
    expect(rememberFacts(facts, 'effects', ['a', 'b', 'a'])).toEqual(['a', 'b'])
    expect(rememberFacts(facts, 'effects', ['b', 'c'])).toEqual(['c'])
  })
})
