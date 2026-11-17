import { describe, expect, it } from 'vitest'
import { prefabScanTargets } from './catalogScan'

const prefab = (...at: [string, string[]][]) => ({ type: 'prefab', locations: at.map(([repo, branches]) => ({ repo, path: 'A.prefab', branches })) })

describe('prefabScanTargets', () => {
  it('picks the fullest branch first, then only branches that still add a prefab', () => {
    const list = [prefab(['r', ['main', 'dev']]), prefab(['r', ['main', 'dev']]), prefab(['r', ['dev']]), prefab(['r', ['feature']])]
    expect(prefabScanTargets(list, () => false)).toEqual([
      { repo: 'r', branch: 'dev' },
      { repo: 'r', branch: 'feature' }
    ])
  })

  it('skips prefabs held by a branch that is scanned already, and other types', () => {
    const list = [prefab(['r', ['main', 'dev']]), prefab(['r', ['feature']]), { type: 'fbx', locations: [{ repo: 'r', path: 'M.fbx', branches: ['x'] }] }]
    expect(prefabScanTargets(list, (repo, branch) => repo === 'r' && branch === 'dev')).toEqual([{ repo: 'r', branch: 'feature' }])
  })

  it('keeps repos apart and reads nothing when all is scanned', () => {
    const list = [prefab(['a', ['main']]), prefab(['b', ['main']])]
    expect(prefabScanTargets(list, () => false)).toHaveLength(2)
    expect(prefabScanTargets(list, () => true)).toEqual([])
  })
})
