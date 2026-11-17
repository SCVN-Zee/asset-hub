import type { SearchScope } from './useCatalog'

/** What is checked in one column: every value there is, or a list of them. */
export type Picks = 'all' | string[]

/** The repos and branches the grid looks at. */
export interface Selection {
  repos: Picks
  branches: Picks
}

/** A row of a column: a value to check, or a group of rows under a prefix ("feature/"). */
export interface TreeNode {
  id: string
  label: string
  /** The checkable value; groups have none. */
  value?: string
  /** Every value at or under this row. */
  leaves: string[]
  children: TreeNode[]
}

/**
 * Groups values by the text before each `/`, down to `maxDepth` levels: 'feature/ui/x' becomes
 * feature › ui › x. Plain values come first, in the given order; groups follow, A to Z.
 */
export function buildTree(values: readonly string[], maxDepth = Infinity, prefix = ''): TreeNode[] {
  const nodes: TreeNode[] = []
  const groups = new Map<string, string[]>()
  for (const value of values) {
    const rest = value.slice(prefix.length)
    const slash = maxDepth > 0 ? rest.indexOf('/') : -1
    if (slash > 0 && slash < rest.length - 1) {
      const key = rest.slice(0, slash)
      groups.set(key, [...(groups.get(key) ?? []), value])
    } else nodes.push({ id: `v:${value}`, label: rest, value, leaves: [value], children: [] })
  }
  const sorted = [...groups].sort(([a], [b]) => a.localeCompare(b))
  for (const [key, leaves] of sorted) {
    const next = `${prefix}${key}/`
    nodes.push({ id: `g:${next}`, label: key, leaves, children: buildTree(leaves, maxDepth - 1, next) })
  }
  return nodes
}

/** The values that contain the text, ignoring case. */
export const matching = (values: readonly string[], query: string) => {
  const q = query.trim().toLowerCase()
  return q ? values.filter((v) => v.toLowerCase().includes(q)) : [...values]
}

export type CheckState = 'on' | 'off' | 'mixed'

export function checkState(leaves: readonly string[], checked: ReadonlySet<string>): CheckState {
  const on = leaves.filter((v) => checked.has(v)).length
  return on === 0 ? 'off' : on === leaves.length ? 'on' : 'mixed'
}

/** A fully checked row is unchecked; any other state checks all its values. */
export function toggleLeaves(checked: ReadonlySet<string>, leaves: readonly string[]): Set<string> {
  const next = new Set(checked)
  const clear = checkState(leaves, checked) === 'on'
  for (const v of leaves) clear ? next.delete(v) : next.add(v)
  return next
}

export const pickSet = (picks: Picks, all: readonly string[]) => new Set(picks === 'all' ? all : picks)

/** Back to picks: every value checked is 'all'; values that are not in `all` any more are dropped. */
export function toPicks(checked: ReadonlySet<string>, all: readonly string[]): Picks {
  const kept = all.filter((v) => checked.has(v))
  return all.length > 0 && kept.length === all.length ? 'all' : kept
}

/** How the selection is searched: the view to open, and the repo and branch for the branch view. */
export interface Target {
  scope: SearchScope
  repo?: string
  branch?: string
  /** A column with nothing checked: there is nothing to show. */
  empty: boolean
}

/** One repo and one branch is the branch view; one repo is the catalog of that repo; else every repo's. */
export function targetOf(sel: Selection, allRepos: readonly string[]): Target {
  const repos = sel.repos === 'all' ? allRepos : sel.repos
  if (repos.length === 0 || sel.branches.length === 0) return { scope: 'all', empty: true }
  if (repos.length === 1 && sel.branches !== 'all' && sel.branches.length === 1) return { scope: 'branch', repo: repos[0], branch: sel.branches[0], empty: false }
  return repos.length === 1 ? { scope: 'repo', repo: repos[0], empty: false } : { scope: 'all', empty: false }
}

/** The catalog's filter; an empty list is no limit. */
export const filterOf = (sel: Selection) => ({
  repos: sel.repos === 'all' ? [] : sel.repos,
  branches: sel.branches === 'all' ? [] : sel.branches
})

/** The selection of one open branch. */
export const selectionOf = (repo: string, branch: string): Selection => ({ repos: repo ? [repo] : [], branches: branch ? [branch] : [] })

export type Part = { kind: 'all' } | { kind: 'none' } | { kind: 'one'; value: string } | { kind: 'some'; count: number }

/** What a column says in the trigger. */
export const partOf = (picks: Picks): Part =>
  picks === 'all' ? { kind: 'all' } : picks.length === 0 ? { kind: 'none' } : picks.length === 1 ? { kind: 'one', value: picks[0] } : { kind: 'some', count: picks.length }

/** 'owner/repo' → 'owner/'; a name without an owner gives ''. */
export const ownerPrefix = (fullName: string) => fullName.slice(0, fullName.indexOf('/') + 1)
