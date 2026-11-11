import { describe, expect, it } from 'vitest'
import { HandlerRegistry } from './handlers'
import type { HandlerDescriptor } from './parse'
import type { Asset } from '@shared/types'

const base = { labelKey: 'x', badge: 'X', previewKind: 'three', materialsFromPrefabs: false, usesGuids: true, facts: null, hidden: false, delivery: 'unitypackage', alsoShows: null, unlisted: null } as const
const asset = (type: string, sha: string, name = 'a.prefab'): Asset => ({ id: name, type, path: name, name, sha, lfs: false })

describe('HandlerRegistry', () => {
  const registry = new HandlerRegistry([
    { ...base, id: 'prefab' },
    { ...base, id: 'vfx', facts: 'effects', alsoShows: { kind: 'fact', fromType: 'prefab', fact: 'effects' } },
    { ...base, id: 'animation', alsoShows: { kind: 'animatedModel' }, unlisted: 'notRigged' }
  ] as HandlerDescriptor[])

  it('lists prefabs the scan found effects in, and only those', () => {
    const facts = { effects: new Set(['fx']) }
    expect(registry.get('vfx')?.alsoShows?.(asset('prefab', 'fx'), undefined, facts)).toBe(true)
    expect(registry.get('vfx')?.alsoShows?.(asset('prefab', 'plain'), undefined, facts)).toBe(false)
    expect(registry.get('vfx')?.alsoShows?.(asset('prefab', 'fx'), undefined, undefined)).toBe(false)
    expect(registry.get('prefab')?.alsoShows).toBeUndefined()
  })

  it('lists models with clips, or named like clips, among the animations', () => {
    const also = registry.get('animation')!.alsoShows!
    expect(also(asset('fbx', 'a', 'Run.fbx'), { clips: 2 } as never)).toBe(true)
    expect(also(asset('fbx', 'a', 'Model@Run.fbx'), undefined)).toBe(true)
    expect(also(asset('fbx', 'a', 'Model.fbx'), undefined)).toBe(false)
    expect(registry.get('animation')!.unlisted!({ rigged: false } as never)).toBe(true)
    expect(registry.get('animation')!.unlisted!(undefined)).toBe(false)
  })
})
