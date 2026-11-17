import { readFileSync } from 'node:fs'
import { describe, expect, it } from 'vitest'
import type { Asset, AssetFacts } from '@shared/types'
import { ASSET_TAGS, isAnimatedModel, tagsOf } from './tags'

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
})
