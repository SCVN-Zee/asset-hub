import { describe, expect, it } from 'vitest'
import { buildTree, checkState, filterOf, matching, ownerPrefix, partOf, pickSet, selectionOf, targetOf, toggleLeaves, toPicks } from './scopeTree'

describe('buildTree', () => {
  it('nests branches by every slash and lists plain names first', () => {
    const tree = buildTree(['main', 'feature/ui/x', 'feature/ui/y', 'feature/z', 'dev'])
    expect(tree.map((n) => n.label)).toEqual(['main', 'dev', 'feature'])
    const feature = tree[2]
    expect(feature.leaves).toHaveLength(3)
    expect(feature.children.map((n) => n.label)).toEqual(['z', 'ui'])
    expect(feature.children[1].children.map((n) => n.value)).toEqual(['feature/ui/x', 'feature/ui/y'])
  })

  it('groups repos by owner only, one level deep', () => {
    const tree = buildTree(['luna/farm', 'luna/zoo', 'solo'], 1)
    expect(tree.map((n) => [n.label, n.value])).toEqual([
      ['solo', 'solo'],
      ['luna', undefined]
    ])
    expect(tree[1].children.map((n) => n.label)).toEqual(['farm', 'zoo'])
  })

  it('keeps a name that is both a branch and a prefix as two rows', () => {
    const tree = buildTree(['feature', 'feature/x'])
    expect(tree.map((n) => n.id)).toEqual(['v:feature', 'g:feature/'])
  })
})

describe('matching', () => {
  it('ignores case and keeps all values for an empty text', () => {
    expect(matching(['Main', 'dev'], ' MA ')).toEqual(['Main'])
    expect(matching(['a', 'b'], '')).toEqual(['a', 'b'])
  })
})

describe('tri-state', () => {
  const leaves = ['a/1', 'a/2']
  it('is off, mixed or on by how many leaves are checked', () => {
    expect(checkState(leaves, new Set())).toBe('off')
    expect(checkState(leaves, new Set(['a/1']))).toBe('mixed')
    expect(checkState(leaves, new Set(leaves))).toBe('on')
  })

  it('checks a mixed or empty row and unchecks a full one', () => {
    expect([...toggleLeaves(new Set(['a/1']), leaves)].sort()).toEqual(leaves)
    expect([...toggleLeaves(new Set(), leaves)].sort()).toEqual(leaves)
    expect([...toggleLeaves(new Set([...leaves, 'b']), leaves)]).toEqual(['b'])
  })
})

describe('picks', () => {
  const all = ['main', 'dev', 'x']
  it('turns every value checked into all, and drops values that are gone', () => {
    expect(toPicks(new Set(all), all)).toBe('all')
    expect(toPicks(new Set(['dev', 'gone']), all)).toEqual(['dev'])
    expect(toPicks(new Set(), all)).toEqual([])
    expect(toPicks(new Set(), [])).toEqual([])
  })

  it('reads all as every value', () => {
    expect([...pickSet('all', all)]).toEqual(all)
    expect([...pickSet(['dev'], all)]).toEqual(['dev'])
  })
})

describe('targetOf', () => {
  const repos = ['o/a', 'o/b']
  it('opens the branch view for one repo and one branch', () => {
    expect(targetOf({ repos: ['o/a'], branches: ['main'] }, repos)).toEqual({ scope: 'branch', repo: 'o/a', branch: 'main', empty: false })
    expect(targetOf({ repos: 'all', branches: ['main'] }, ['o/a'])).toMatchObject({ scope: 'branch', repo: 'o/a' })
  })

  it('searches the catalog of one repo, or of every repo', () => {
    expect(targetOf({ repos: ['o/a'], branches: 'all' }, repos)).toEqual({ scope: 'repo', repo: 'o/a', empty: false })
    expect(targetOf({ repos: ['o/a'], branches: ['main', 'dev'] }, repos).scope).toBe('repo')
    expect(targetOf({ repos: ['o/a', 'o/b'], branches: ['main'] }, repos).scope).toBe('all')
    expect(targetOf({ repos: 'all', branches: 'all' }, repos).scope).toBe('all')
  })

  it('is empty when a column has nothing checked', () => {
    expect(targetOf({ repos: [], branches: 'all' }, repos).empty).toBe(true)
    expect(targetOf({ repos: 'all', branches: [] }, repos).empty).toBe(true)
  })
})

describe('filters and labels', () => {
  it('sends no limit for all', () => {
    expect(filterOf({ repos: 'all', branches: ['main'] })).toEqual({ repos: [], branches: ['main'] })
  })

  it('selects one open branch', () => {
    expect(selectionOf('o/a', 'main')).toEqual({ repos: ['o/a'], branches: ['main'] })
    expect(selectionOf('o/a', '')).toEqual({ repos: ['o/a'], branches: [] })
  })

  it('names a column by what is checked', () => {
    expect(partOf('all')).toEqual({ kind: 'all' })
    expect(partOf([])).toEqual({ kind: 'none' })
    expect(partOf(['x'])).toEqual({ kind: 'one', value: 'x' })
    expect(partOf(['x', 'y'])).toEqual({ kind: 'some', count: 2 })
  })
})

describe('ownerPrefix', () => {
  it('is the owner with its slash, or empty', () => {
    expect(ownerPrefix('luna/farm')).toBe('luna/')
    expect(ownerPrefix('solo')).toBe('')
  })
})
