import { useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { buildTree, checkState, matching, ownerPrefix, partOf, pickSet, toggleLeaves, toPicks, type CheckState, type Selection, type TreeNode } from '../scopeTree'
import { ChevronRightIcon, SearchIcon } from './Icons'

interface Props {
  selection: Selection
  /** Every repo, as 'owner/name'. */
  repos: readonly string[]
  /** The branch names to choose from: the live list of one repo, or those the catalog knows. */
  branches: readonly string[]
  /** Branch names that are the default of a checked repo; they carry a badge. */
  defaultBranches: ReadonlySet<string>
  placeholder: string
  onChange(selection: Selection): void
}

/** Groups are open from the start while a column is this small, or while searching. */
const OPEN_BELOW = 60

export function Box({ state, label, onChange }: { state: CheckState; label: string; onChange(): void }) {
  return (
    <input
      type="checkbox"
      aria-label={label}
      checked={state === 'on'}
      // The mixed state has no attribute: it is set on the element.
      ref={(el) => {
        if (el) el.indeterminate = state === 'mixed'
      }}
      onChange={onChange}
    />
  )
}

interface ColumnProps {
  title: string
  allLabel: string
  /** The values left by the search. */
  values: readonly string[]
  tree: TreeNode[]
  checked: ReadonlySet<string>
  searching: boolean
  badgeOf?(value: string): string | undefined
  onToggle(leaves: readonly string[]): void
  onOnly(value: string): void
}

/** One column: an "all" row, then the values as a tree whose group rows check all they hold. */
function Column({ title, allLabel, values, tree, checked, searching, badgeOf, onToggle, onOnly }: ColumnProps) {
  const { t } = useTranslation()
  /** Groups the user opened or closed; the others follow the default. */
  const [folded, setFolded] = useState<ReadonlyMap<string, boolean>>(new Map())
  const openByDefault = searching || values.length <= OPEN_BELOW

  const rows = (nodes: TreeNode[], depth: number) =>
    nodes.map((n) => {
      const indent = { paddingLeft: depth * 16 }
      if (n.value !== undefined) {
        const value = n.value
        const badge = badgeOf?.(value)
        return (
          <li key={n.id} className="scope-row" style={indent}>
            <span className="scope-gap" />
            <label className="check" title={value}>
              <Box state={checkState(n.leaves, checked)} label={value} onChange={() => onToggle(n.leaves)} />
              <span className="scope-text">{n.label}</span>
              {badge && <span className="badge">{badge}</span>}
            </label>
            <button type="button" className="btn btn-small scope-only" onClick={() => onOnly(value)} aria-label={t('scope.onlyName', { name: value })}>
              {t('scope.only')}
            </button>
          </li>
        )
      }
      const open = folded.get(n.id) ?? openByDefault
      return (
        <li key={n.id} role="presentation">
          <div className="scope-row" style={indent}>
            <button
              type="button"
              className="scope-fold"
              aria-expanded={open}
              aria-label={n.label}
              onClick={() => setFolded((m) => new Map(m).set(n.id, !open))}
            >
              <ChevronRightIcon size={14} />
            </button>
            <label className="check">
              <Box state={checkState(n.leaves, checked)} label={n.label} onChange={() => onToggle(n.leaves)} />
              <span className="scope-text">{n.label}</span>
              <span className="muted small">{n.leaves.length}</span>
            </label>
          </div>
          {open && <ul className="scope-tree">{rows(n.children, depth + 1)}</ul>}
        </li>
      )
    })

  return (
    <section className="scope-col" aria-label={title}>
      <div className="scope-title">{title}</div>
      <div className="scope-row scope-all">
        <span className="scope-gap" />
        <label className="check">
          <Box state={values.length ? checkState(values, checked) : 'off'} label={allLabel} onChange={() => onToggle(values)} />
          <span className="scope-text">{allLabel}</span>
        </label>
      </div>
      <ul className="scope-list scope-tree">{rows(tree, 0)}</ul>
      {!values.length && <p className="muted small picker-empty">{t('picker.noMatch')}</p>}
    </section>
  )
}

/** One button for where the grid looks: the repos and the branches, each a checkbox tree in a popover. */
export function ScopePicker({ selection, repos, branches, defaultBranches, placeholder, onChange }: Props) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const root = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => {
      if (!root.current?.contains(e.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [open])
  useEffect(() => {
    if (!open) setQuery('')
  }, [open])

  const shownRepos = useMemo(() => matching(repos, query), [repos, query])
  const shownBranches = useMemo(() => matching(branches, query), [branches, query])
  const repoTree = useMemo(() => buildTree(shownRepos, 1), [shownRepos])
  const branchTree = useMemo(() => buildTree(shownBranches), [shownBranches])
  const checkedRepos = useMemo(() => pickSet(selection.repos, repos), [selection.repos, repos])
  const checkedBranches = useMemo(() => pickSet(selection.branches, branches), [selection.branches, branches])

  const repoPart = partOf(selection.repos)
  const branchPart = partOf(selection.branches)
  const repoLabel =
    repoPart.kind === 'all'
      ? t('scope.allRepos')
      : repoPart.kind === 'none'
        ? t('scope.noRepos')
        : repoPart.kind === 'some'
          ? t('catalog.repoCount', { count: repoPart.count })
          : repoPart.value
  const branchLabel =
    branchPart.kind === 'all'
      ? t('scope.allBranches')
      : branchPart.kind === 'none'
        ? t('scope.noBranches')
        : branchPart.kind === 'some'
          ? t('catalog.branchCount', { count: branchPart.count })
          : branchPart.value
  // One repo: its owner is dimmed before its name, as in "owner / repo".
  const owner = repoPart.kind === 'one' ? ownerPrefix(repoPart.value) : ''
  const repoText = repoPart.kind === 'one' ? repoPart.value.slice(owner.length) : repoLabel
  const text = `${repoLabel} · ${branchLabel}`

  return (
    <div className="picker crumb-seg" ref={root}>
      <button
        ref={trigger}
        type="button"
        className="select picker-trigger"
        aria-haspopup="dialog"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        disabled={!repos.length}
        title={text}
      >
        <span className="sr-only">{t('scope.label')}: </span>
        <span className="picker-value">
          {repos.length ? (
            <>
              {owner && <em>{owner.replace(/\/$/, '')} / </em>}
              {repoText} · {branchLabel}
            </>
          ) : (
            placeholder
          )}
        </span>
        <span className="picker-caret" data-open={open}>
          <ChevronRightIcon />
        </span>
      </button>
      {open && (
        <div
          className="picker-pop scope-pop"
          role="dialog"
          aria-label={t('scope.label')}
          onKeyDown={(e) => {
            if (e.key !== 'Escape') return
            e.preventDefault()
            setOpen(false)
            trigger.current?.focus()
          }}
        >
          <div className="search">
            <SearchIcon />
            <input autoFocus aria-label={t('scope.search')} placeholder={t('scope.search')} value={query} onChange={(e) => setQuery(e.target.value)} />
          </div>
          <div className="scope-cols">
            <Column
              title={t('browse.repo')}
              allLabel={t('catalog.allRepos', { count: shownRepos.length })}
              values={shownRepos}
              tree={repoTree}
              checked={checkedRepos}
              searching={!!query.trim()}
              onToggle={(leaves) => onChange({ ...selection, repos: toPicks(toggleLeaves(checkedRepos, leaves), repos) })}
              onOnly={(value) => onChange({ ...selection, repos: toPicks(new Set([value]), repos) })}
            />
            <Column
              title={t('browse.branch')}
              allLabel={t('catalog.allBranches', { count: shownBranches.length })}
              values={shownBranches}
              tree={branchTree}
              checked={checkedBranches}
              searching={!!query.trim()}
              badgeOf={(b) => (defaultBranches.has(b) ? t('browse.defaultBranch') : undefined)}
              onToggle={(leaves) => onChange({ ...selection, branches: toPicks(toggleLeaves(checkedBranches, leaves), branches) })}
              onOnly={(value) => onChange({ ...selection, branches: toPicks(new Set([value]), branches) })}
            />
          </div>
        </div>
      )}
    </div>
  )
}
