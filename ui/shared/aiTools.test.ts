import { describe, expect, it } from 'vitest'
import { AI_TOOL_NAMES, aiToolSpecs, assetRef, matchRepos, parseAssetRef } from './aiTools'

describe('asset refs', () => {
  it('round-trips repo, branch (with slashes) and path', () => {
    const ref = assetRef('supercent-io/luna_pizza', 'VN/DEV/playable003', 'Assets/FX/Fire Effect.prefab')
    expect(parseAssetRef(ref)).toEqual({ repo: 'supercent-io/luna_pizza', branch: 'VN/DEV/playable003', path: 'Assets/FX/Fire Effect.prefab' })
  })
  it('rejects anything else', () => {
    expect(parseAssetRef('Assets/FX/Fire.prefab')).toBeUndefined()
    expect(parseAssetRef(42)).toBeUndefined()
  })
  it('defines exactly the named tools', () => {
    expect(aiToolSpecs(['vfx']).map((t) => t.name)).toEqual([...AI_TOOL_NAMES])
  })
})

describe('matchRepos', () => {
  const repos = ['supercent-io/luna_pizza', 'supercent-io/luna_pizzasimulator', 'supercent-io/luna_crablife', 'supercent-io/hb_creative_bulldozer']
  it('prefers a whole word of the name over a part of one', () => {
    expect(matchRepos(repos, 'pizza')).toEqual(['supercent-io/luna_pizza'])
    expect(matchRepos(repos, 'Bulldozer')).toEqual(['supercent-io/hb_creative_bulldozer'])
  })
  it('takes the whole name, with or without the owner', () => {
    expect(matchRepos(repos, 'luna_pizzasimulator')).toEqual(['supercent-io/luna_pizzasimulator'])
    expect(matchRepos(repos, 'supercent-io/luna_pizza')).toEqual(['supercent-io/luna_pizza'])
  })
  it('falls back to any part, and returns every equal match', () => {
    expect(matchRepos(repos, 'crab')).toEqual(['supercent-io/luna_crablife'])
    expect(matchRepos(repos, 'luna')).toEqual(['supercent-io/luna_pizza', 'supercent-io/luna_pizzasimulator', 'supercent-io/luna_crablife'])
    expect(matchRepos(repos, 'tractor')).toEqual([])
  })
})
