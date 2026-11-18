import { describe, expect, it } from 'vitest'
import { NO_EXCLUSIONS, allState, setAll, branchesOf, groupByOrg, isBranchOff, isRepoOff, orgState, repoState, search, setBranch, pruneAbsent, setOrg, setRepo, toScopeRepo, totalBytes, type Exclusions, type ScopeRepo } from './cacheScope'

const repo = (fullName: string, branches: string[], bytes = 0): ScopeRepo => ({ fullName, org: fullName.split('/')[0], defaultBranch: 'main', branches, bytes })
const a1 = repo('a/one', ['main', 'dev', 'wip'], 100)
const a2 = repo('a/two', ['main'], 50)
const b1 = repo('b/one', [], 0)
const aRepos = [a1, a2]
const ex = (over: Partial<Exclusions>): Exclusions => ({ ...NO_EXCLUSIONS, ...over })

describe('states', () => {
  it('reads a repo and a branch as off inside an excluded org or repo', () => {
    expect(isRepoOff(ex({ orgs: ['a'] }), a1)).toBe(true)
    expect(isRepoOff(ex({ repos: ['a/one'] }), a2)).toBe(false)
    expect(isBranchOff(ex({ repos: ['a/one'] }), a1, 'main')).toBe(true)
    expect(isBranchOff(ex({ branches: ['a/one@dev'] }), a1, 'dev')).toBe(true)
    expect(isBranchOff(ex({ branches: ['a/one@dev'] }), a1, 'main')).toBe(false)
  })

  it('is mixed when some branches or some repos are left out', () => {
    expect(repoState(NO_EXCLUSIONS, a1)).toBe('on')
    expect(repoState(ex({ branches: ['a/one@dev'] }), a1)).toBe('mixed')
    expect(repoState(ex({ repos: ['a/one'] }), a1)).toBe('off')
    expect(orgState(ex({ repos: ['a/two'] }), aRepos)).toBe('mixed')
    expect(orgState(ex({ repos: ['a/one', 'a/two'] }), aRepos)).toBe('off')
    expect(orgState(ex({ orgs: ['a'] }), aRepos)).toBe('off')
    expect(orgState(ex({ branches: ['a/two@main'] }), aRepos)).toBe('mixed')
  })

  it('lists the branches left out that the catalog no longer knows', () => {
    expect(branchesOf(ex({ branches: ['a/two@gone', 'a/one@dev'] }), a2)).toEqual(['main', 'gone'])
  })
})

describe('toggling', () => {
  it('unchecks and checks an org, clearing what is inside it', () => {
    const off = setOrg(ex({ repos: ['a/one', 'b/x'], branches: ['a/two@main', 'b/one@y'] }), 'a', false)
    expect(off).toEqual({ orgs: ['a'], repos: ['b/x'], branches: ['b/one@y'] })
    expect(setOrg(off, 'a', true)).toEqual({ orgs: [], repos: ['b/x'], branches: ['b/one@y'] })
    expect(setOrg(ex({ repos: ['a/one'], branches: ['a/two@main'] }), 'a', true)).toEqual(NO_EXCLUSIONS)
  })

  it('unchecks a repo and checks it again with its branches', () => {
    const off = setRepo(ex({ branches: ['a/one@dev', 'a/two@main'] }), a1, aRepos, false)
    expect(off).toEqual({ orgs: [], repos: ['a/one'], branches: ['a/two@main'] })
    expect(setRepo(off, a1, aRepos, true)).toEqual({ orgs: [], repos: [], branches: ['a/two@main'] })
  })

  it('checking a repo of an unchecked org leaves its siblings unchecked one by one', () => {
    expect(setRepo(ex({ orgs: ['a'] }), a1, aRepos, true)).toEqual({ orgs: [], repos: ['a/two'], branches: [] })
  })

  it('checking a branch of an unchecked repo caches the repo with only that branch', () => {
    expect(setBranch(ex({ repos: ['a/one'] }), a1, aRepos, 'dev', true)).toEqual({ orgs: [], repos: [], branches: ['a/one@main', 'a/one@wip'] })
    expect(setBranch(ex({ orgs: ['a'] }), a1, aRepos, 'main', true)).toEqual({ orgs: [], repos: ['a/two'], branches: ['a/one@dev', 'a/one@wip'] })
  })

  it('checks and unchecks a branch of a cached repo', () => {
    const off = setBranch(NO_EXCLUSIONS, a1, aRepos, 'dev', false)
    expect(off).toEqual(ex({ branches: ['a/one@dev'] }))
    expect(setBranch(off, a1, aRepos, 'dev', true)).toEqual(NO_EXCLUSIONS)
  })

  it('unchecking the last checked branch unchecks the repo', () => {
    const next = setBranch(ex({ branches: ['a/one@dev', 'a/one@wip'] }), a1, aRepos, 'main', false)
    expect(next).toEqual(ex({ repos: ['a/one'] }))
  })

  it('maps back to the same exclusions after a round trip', () => {
    let e = setBranch(ex({ repos: ['a/one'] }), a1, aRepos, 'dev', true)
    e = setBranch(e, a1, aRepos, 'main', true)
    e = setBranch(e, a1, aRepos, 'wip', true)
    expect(e).toEqual(NO_EXCLUSIONS)
  })
})

describe('listing', () => {
  it('groups repos by org, A to Z', () => {
    expect(groupByOrg([b1, a2, a1]).map((g) => [g.org, g.repos.map((r) => r.fullName)])).toEqual([
      ['a', ['a/one', 'a/two']],
      ['b', ['b/one']]
    ])
  })

  it('finds repos by name, keeping all branches, and by branch, keeping the matching ones', () => {
    expect(search(NO_EXCLUSIONS, [a1, a2], '  ').map((s) => s.repo.fullName)).toEqual(['a/one', 'a/two'])
    expect(search(NO_EXCLUSIONS, [a1, a2], 'ONE')).toEqual([{ repo: a1, branches: ['main', 'dev', 'wip'] }])
    expect(search(NO_EXCLUSIONS, [a1, a2], 'wi')).toEqual([{ repo: a1, branches: ['wip'] }])
  })

  it('sums bytes and reads a missing size as zero', () => {
    expect(totalBytes(aRepos)).toBe(150)
    expect(toScopeRepo({ fullName: 'a/x', org: 'a', defaultBranch: 'main', branches: [], bytes: null }).bytes).toBe(0)
  })
})

describe('pruneAbsent', () => {
  it('drops orgs, repos and branches that have no row', () => {
    const e = ex({ orgs: ['a', 'gone'], repos: ['a/one', 'gone/x', 'a/removed'], branches: ['a/one@dev', 'gone/x@y', 'a/removed@z'] })
    expect(pruneAbsent(e, [a1, a2])).toEqual({ orgs: ['a'], repos: ['a/one'], branches: ['a/one@dev'] })
    expect(pruneAbsent(e, [])).toEqual(NO_EXCLUSIONS)
  })
})

describe('all', () => {
  const everything = [a1, a2, b1]
  it('is on, off or mixed over the repos shown', () => {
    expect(allState(NO_EXCLUSIONS, everything)).toBe('on')
    expect(allState(ex({ orgs: ['a', 'b'] }), everything)).toBe('off')
    expect(allState(ex({ orgs: ['a'], repos: ['b/one'] }), everything)).toBe('off')
    expect(allState(ex({ orgs: ['a'] }), everything)).toBe('mixed')
    expect(allState(ex({ branches: ['a/one@dev'] }), everything)).toBe('mixed')
    expect(allState(NO_EXCLUSIONS, [])).toBe('off')
  })

  it('checks all as no exclusions and unchecks all as every org', () => {
    expect(setAll(ex({ orgs: ['a'], branches: ['b/one@x'] }), everything, everything, true)).toEqual(NO_EXCLUSIONS)
    expect(setAll(ex({ repos: ['a/one'] }), everything, everything, false)).toEqual({ orgs: ['a', 'b'], repos: [], branches: [] })
  })

  it('acts only on the repos a search shows', () => {
    const off = setAll(NO_EXCLUSIONS, [a1], everything, false)
    expect(off).toEqual(ex({ repos: ['a/one'] }))
    expect(allState(off, [a1])).toBe('off')
    expect(allState(off, everything)).toBe('mixed')
    expect(setAll(off, [a1], everything, true)).toEqual(NO_EXCLUSIONS)
    // Checking a shown repo of an unchecked org leaves its hidden siblings unchecked.
    expect(setAll(ex({ orgs: ['a'] }), [a1], everything, true)).toEqual(ex({ repos: ['a/two'] }))
  })
})
