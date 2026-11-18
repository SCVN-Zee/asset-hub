import type { CacheExclusions, CacheScopeRepo } from './bindings'
import type { CheckState } from './scopeTree'

/** What the catalog index leaves out. Anything not listed, and not inside a listed org or repo, is cached. */
export type Exclusions = CacheExclusions

export const NO_EXCLUSIONS: Exclusions = { orgs: [], repos: [], branches: [] }

/** A repo of the tree, with the size of its stored index as a number. */
export interface ScopeRepo extends Omit<CacheScopeRepo, 'bytes'> {
  bytes: number
}

export const toScopeRepo = (repo: CacheScopeRepo): ScopeRepo => ({ ...repo, bytes: repo.bytes ?? 0 })

/** How a branch is written in the exclusions. */
export const branchEntry = (repo: string, branch: string) => `${repo}@${branch}`

export const isRepoOff = (ex: Exclusions, repo: ScopeRepo) => ex.orgs.includes(repo.org) || ex.repos.includes(repo.fullName)

export const isBranchOff = (ex: Exclusions, repo: ScopeRepo, branch: string) => isRepoOff(ex, repo) || ex.branches.includes(branchEntry(repo.fullName, branch))

/** The branches to show for a repo: those its catalog knows, then those left out that it no longer lists. */
export function branchesOf(ex: Exclusions, repo: ScopeRepo): string[] {
  const prefix = `${repo.fullName}@`
  const left = ex.branches.filter((e) => e.startsWith(prefix)).map((e) => e.slice(prefix.length))
  return [...new Set([...repo.branches, ...left])]
}

const stateOf = (on: number, total: number): CheckState => (on === total ? 'on' : on === 0 ? 'off' : 'mixed')

/** A repo with no known branches is checked or not as a whole. */
export function repoState(ex: Exclusions, repo: ScopeRepo): CheckState {
  if (isRepoOff(ex, repo)) return 'off'
  const branches = branchesOf(ex, repo)
  return stateOf(branches.filter((b) => !isBranchOff(ex, repo, b)).length, branches.length)
}

export function orgState(ex: Exclusions, repos: readonly ScopeRepo[]): CheckState {
  if (repos.length && ex.orgs.includes(repos[0].org)) return 'off'
  const states = repos.map((r) => repoState(ex, r))
  const on = states.filter((s) => s === 'on').length
  return states.some((s) => s === 'mixed') ? 'mixed' : stateOf(on, states.length)
}

const without = (list: readonly string[], drop: ReadonlySet<string>) => list.filter((v) => !drop.has(v))
const add = (list: readonly string[], ...values: string[]) => [...new Set([...list, ...values])]

/** Drops entries that an org or repo exclusion already covers. */
function tidy(ex: Exclusions): Exclusions {
  const orgOf = (repo: string) => repo.slice(0, repo.indexOf('/'))
  const repos = ex.repos.filter((r) => !ex.orgs.includes(orgOf(r)))
  const branches = ex.branches.filter((e) => {
    const repo = e.slice(0, e.indexOf('@'))
    return !repos.includes(repo) && !ex.orgs.includes(orgOf(repo))
  })
  return { orgs: ex.orgs, repos, branches }
}

/** Checks (everything under the org is cached) or unchecks the org. */
export function setOrg(ex: Exclusions, org: string, on: boolean): Exclusions {
  const under = (repo: string) => repo.startsWith(`${org}/`)
  if (!on) return tidy({ ...ex, orgs: add(ex.orgs, org) })
  return {
    orgs: ex.orgs.filter((o) => o !== org),
    repos: ex.repos.filter((r) => !under(r)),
    branches: ex.branches.filter((e) => !under(e))
  }
}

/**
 * Checks (every branch of it is cached) or unchecks the repo. `orgRepos` are the repos of its org:
 * checking a repo of an unchecked org lifts the org and leaves its other repos unchecked one by one.
 */
export function setRepo(ex: Exclusions, repo: ScopeRepo, orgRepos: readonly ScopeRepo[], on: boolean): Exclusions {
  if (!on) return tidy({ ...ex, repos: add(ex.repos, repo.fullName) })
  const orgOff = ex.orgs.includes(repo.org)
  const others = orgOff ? orgRepos.filter((r) => r.fullName !== repo.fullName).map((r) => r.fullName) : []
  const drop = new Set([repo.fullName])
  return tidy({
    orgs: ex.orgs.filter((o) => o !== repo.org),
    repos: add(without(ex.repos, drop), ...others),
    branches: ex.branches.filter((e) => !e.startsWith(`${repo.fullName}@`))
  })
}

/**
 * Checks or unchecks one branch. Checking a branch of an unchecked repo (or org) caches the repo and
 * leaves its other branches unchecked; unchecking the last checked branch unchecks the repo.
 */
export function setBranch(ex: Exclusions, repo: ScopeRepo, orgRepos: readonly ScopeRepo[], branch: string, on: boolean): Exclusions {
  const entry = branchEntry(repo.fullName, branch)
  const all = branchesOf(ex, repo)
  if (on) {
    if (!isRepoOff(ex, repo)) return { ...ex, branches: ex.branches.filter((e) => e !== entry) }
    const lifted = setRepo(ex, repo, orgRepos, true)
    return { ...lifted, branches: add(lifted.branches, ...all.filter((b) => b !== branch).map((b) => branchEntry(repo.fullName, b))) }
  }
  const next = { ...ex, branches: add(ex.branches, entry) }
  return all.every((b) => isBranchOff(next, repo, b)) ? setRepo(next, repo, orgRepos, false) : next
}

export interface OrgGroup {
  org: string
  repos: ScopeRepo[]
}

/** The repos grouped by org, both A to Z. */
export function groupByOrg(repos: readonly ScopeRepo[]): OrgGroup[] {
  const groups = new Map<string, ScopeRepo[]>()
  for (const repo of repos) groups.set(repo.org, [...(groups.get(repo.org) ?? []), repo])
  return [...groups]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([org, list]) => ({ org, repos: list.sort((a, b) => a.fullName.localeCompare(b.fullName)) }))
}

export interface Shown {
  repo: ScopeRepo
  /** The branches to list under it. */
  branches: string[]
}

/** The repos (with their branches) that contain the text, ignoring case; a repo whose own name matches keeps all its branches. */
export function search(ex: Exclusions, repos: readonly ScopeRepo[], query: string): Shown[] {
  const q = query.trim().toLowerCase()
  const shown: Shown[] = []
  for (const repo of repos) {
    const branches = branchesOf(ex, repo)
    if (!q || repo.fullName.toLowerCase().includes(q)) shown.push({ repo, branches })
    else {
      const hits = branches.filter((b) => b.toLowerCase().includes(q))
      if (hits.length) shown.push({ repo, branches: hits })
    }
  }
  return shown
}

/** Bytes the stored indexes of these repos take. */
export const totalBytes = (repos: readonly ScopeRepo[]) => repos.reduce((sum, r) => sum + r.bytes, 0)

/**
 * Drops the entries of orgs, repos and branches that have no row in `repos`: nothing in the tree
 * could check them again, so they would stay for good.
 */
export function pruneAbsent(ex: Exclusions, repos: readonly ScopeRepo[]): Exclusions {
  const names = new Set(repos.map((r) => r.fullName))
  const orgs = new Set(repos.map((r) => r.org))
  return {
    orgs: ex.orgs.filter((o) => orgs.has(o)),
    repos: ex.repos.filter((r) => names.has(r)),
    branches: ex.branches.filter((e) => names.has(e.slice(0, e.indexOf('@'))))
  }
}

/** The state of all the `shown` repos together: on when none is left out, off when every org shown is fully left out. */
export function allState(ex: Exclusions, shown: readonly ScopeRepo[]): CheckState {
  const states = groupByOrg(shown).map((g) => orgState(ex, g.repos))
  if (!states.length) return 'off'
  return states.every((s) => s === 'on') ? 'on' : states.every((s) => s === 'off') ? 'off' : 'mixed'
}

/**
 * Checks or unchecks every `shown` repo. When they are all the repos there are (no search), that is
 * no exclusions, or every org left out. Otherwise each shown repo is set on its own, so the repos
 * the search hides keep their state.
 */
export function setAll(ex: Exclusions, shown: readonly ScopeRepo[], all: readonly ScopeRepo[], on: boolean): Exclusions {
  if (shown.length === all.length) return on ? NO_EXCLUSIONS : { orgs: [...new Set(all.map((r) => r.org))], repos: [], branches: [] }
  const orgRepos = (repo: ScopeRepo) => all.filter((r) => r.org === repo.org)
  return shown.reduce((next, repo) => setRepo(next, repo, orgRepos(repo), on), ex)
}
