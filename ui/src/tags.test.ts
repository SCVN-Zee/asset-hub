import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { Asset, AssetFacts } from '@shared/types'
import { ASSET_TAGS, inSubCategory, isAnimatedModel, subCategoriesOf, tagsOf } from './tags'

const asset = (type: string, name: string, sha = 'a'): Asset => ({ id: `${type}:${name}`, type, name, path: name, sha }) as Asset
const facts = (f: Record<string, string[]>): AssetFacts => Object.fromEntries(Object.entries(f).map(([k, v]) => [k, new Set(v)]))

describe('asset tags', () => {
  it('lists the same tags as the Rust enum', () => {
    const rust = readFileSync(new URL('../../crates/core/src/tags.rs', import.meta.url), 'utf8')
    const ids = [...rust.matchAll(/Self::\w+ => "(\w+)"/g)].map((m) => m[1])
    expect(ids).toEqual([...ASSET_TAGS])
  })

  it('tags a model as animated by its clips or by an @ in its name', () => {
    expect(isAnimatedModel(asset('fbx', 'Run.fbx'), { clips: 2 })).toBe(true)
    expect(isAnimatedModel(asset('fbx', 'Hero@Run.fbx'), undefined)).toBe(true)
    expect(isAnimatedModel(asset('fbx', 'Hero.glb'), { clips: 0 })).toBe(false)
    expect(isAnimatedModel(asset('texture', 'a@b.png'), { clips: 3 })).toBe(false)
  })

  it('tags a prefab by what the scans found in it', () => {
    const both = facts({ uiPrefabs: ['a'], effects: ['a'] })
    expect(tagsOf(asset('prefab', 'Hud.prefab'), undefined, both)).toEqual(['ui', 'particleSystem'])
    expect(tagsOf(asset('prefab', 'Hud.prefab'), undefined, facts({ effects: ['a'] }))).toEqual(['particleSystem'])
    expect(tagsOf(asset('prefab', 'Hud.prefab', 'b'), undefined, both)).toEqual([])
    expect(tagsOf(asset('texture', 'Hud.png'), undefined, both)).toEqual([])
  })

  it('lists the sub-categories of the Model and Prefab tabs only', () => {
    expect(subCategoriesOf('fbx')).toEqual(['all', 'animation', 'static'])
    expect(subCategoriesOf('prefab')).toEqual(['all', 'ui', 'particleSystem', 'other'])
    expect(subCategoriesOf('texture')).toEqual([])
  })

  it('puts models without the Animation tag under Static', () => {
    expect(inSubCategory('fbx', 'animation', ['animation'])).toBe(true)
    expect(inSubCategory('fbx', 'static', ['animation'])).toBe(false)
    expect(inSubCategory('fbx', 'static', undefined)).toBe(true)
    expect(inSubCategory('fbx', 'animation', undefined)).toBe(false)
    expect(inSubCategory('fbx', 'all', ['animation'])).toBe(true)
  })

  it('puts prefabs with neither tag (or no scan result yet) under Other, and one with both under both tags', () => {
    expect(inSubCategory('prefab', 'other', undefined)).toBe(true)
    expect(inSubCategory('prefab', 'other', [])).toBe(true)
    expect(inSubCategory('prefab', 'other', ['ui'])).toBe(false)
    expect(inSubCategory('prefab', 'other', ['particleSystem'])).toBe(false)
    const both = ['ui', 'particleSystem'] as const
    expect(inSubCategory('prefab', 'ui', both)).toBe(true)
    expect(inSubCategory('prefab', 'particleSystem', both)).toBe(true)
    expect(inSubCategory('prefab', 'other', both)).toBe(false)
  })

  it('does not filter a tab without sub-categories', () => {
    expect(inSubCategory('texture', 'other', ['animation'])).toBe(true)
  })

  it('tags catalog-shaped prefabs (id is the blob SHA, other branches) from facts read by SHA', () => {
    const catalog = ['s1', 's2', 's3'].map((sha) => ({ ...asset('prefab', `${sha}.prefab`, sha), id: sha }))
    const known = facts({ effects: ['s1'], uiPrefabs: ['s2'] })
    const tags = catalog.map((a) => tagsOf(a, undefined, known))
    expect(tags).toEqual([['particleSystem'], ['ui'], []])
    const count = (sub: 'ui' | 'particleSystem' | 'other') => tags.filter((t) => inSubCategory('prefab', sub, t)).length
    expect([count('ui'), count('particleSystem'), count('other')]).toEqual([1, 1, 1])
    // Before any scan has read them, every prefab is Other.
    expect(catalog.filter((a) => inSubCategory('prefab', 'other', tagsOf(a, undefined, {}))).length).toBe(3)
  })
})
