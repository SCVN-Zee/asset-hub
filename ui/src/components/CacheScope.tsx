import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { NO_EXCLUSIONS, allState, groupByOrg, setAll, orgState, pruneAbsent, repoState, search, setBranch, setOrg, setRepo, toScopeRepo, totalBytes, isBranchOff, type Exclusions, type ScopeRepo } from '../cacheScope'
import { formatBytes } from '../format'
import { Box } from './ScopePicker'
import { ChevronRightIcon, SearchIcon } from './Icons'

interface Props {
  /** What the index leaves out now, as saved. */
  exclusions: Exclusions | undefined
  /** Saves the exclusions; settles once the app has removed what the index no longer keeps. */
  onSave(exclusions: Exclusions): void | Promise<unknown>
  /** The stored index changed size. */
  onSaved(): void
}

/** A click waits this long for the next one; only the last state is saved, so one burst purges once. */
const SAVE_DELAY_MS = 1500

/** Orgs are open from the start while the tree is this small, or while searching. */
const OPEN_BELOW = 60

/** Which orgs, repos and branches the catalog index caches: a tree whose unchecked rows are left out. */
export function CacheScope({ exclusions, onSave, onSaved }: Props) {
  const { t } = useTranslation()
  const [repos, setRepos] = useState<ScopeRepo[]>()
  const [failed, setFailed] = useState(false)
  const [query, setQuery] = useState('')
  /** Rows the user opened or closed; the others follow the default. */
  const [folded, setFolded] = useState<ReadonlyMap<string, boolean>>(new Map())
  const [local, setLocal] = useState<Exclusions>(exclusions ?? NO_EXCLUSIONS)
  const [saveFailed, setSaveFailed] = useState(false)
  /** The exclusions the app holds, to go back to when a save fails. */
  const saved = useRef<Exclusions>(exclusions ?? NO_EXCLUSIONS)
  /** The state waiting for its save, and the timer that sends it. */
  const pending = useRef<Exclusions>()
  const timer = useRef<ReturnType<typeof setTimeout>>()
  /** Saves one after another. */
  const saving = useRef<Promise<unknown>>(Promise.resolve())
  const save = useRef(onSave)
  save.current = onSave
  const afterSave = useRef(onSaved)
  afterSave.current = onSaved

  const load = useCallback(() => {
    void window.api.cache.scope().then((r) => {
      setFailed(!r.ok)
      if (r.ok) setRepos(r.data.map(toScopeRepo))
    })
  }, [])
  useEffect(load, [load])

  const flush = useCallback(() => {
    clearTimeout(timer.current)
    const next = pending.current
    if (!next) return
    pending.current = undefined
    saving.current = saving.current
      .then(() => save.current(next))
      .then(
        () => {
          saved.current = next
          setSaveFailed(false)
        },
        () => {
          setLocal(saved.current)
          setSaveFailed(true)
        }
      )
      .then(() => {
        load()
        afterSave.current()
      })
  }, [load])
  // A change still waiting when the section closes is saved, not lost.
  useEffect(() => flush, [flush])

  const change = (next: Exclusions) => {
    const pruned = pruneAbsent(next, repos ?? [])
    setLocal(pruned)
    pending.current = pruned
    clearTimeout(timer.current)
    timer.current = setTimeout(flush, SAVE_DELAY_MS)
  }

  const groups = useMemo(() => groupByOrg(repos ?? []), [repos])
  const shown = useMemo(() => search(local, repos ?? [], query), [local, repos, query])
  const searching = !!query.trim()
  const shownByRepo = useMemo(() => new Map(shown.map((s) => [s.repo.fullName, s.branches])), [shown])
  const openByDefault = searching || (repos?.length ?? 0) <= OPEN_BELOW
  const toggle = (id: string, open: boolean) => setFolded((m) => new Map(m).set(id, !open))

  if (failed) return <p className="small warn-text">{t('settings.cacheScopeError')}</p>
  if (!repos) return <p className="muted small">{t('settings.cacheScopeLoading')}</p>

  const shownRepos = repos.filter((r) => shownByRepo.has(r.fullName))
  const everything = allState(local, shownRepos)

  const rows = groups.flatMap((group) => {
    const visible = group.repos.filter((r) => shownByRepo.has(r.fullName))
    if (!visible.length) return []
    const orgId = `o:${group.org}`
    const orgOpen = folded.get(orgId) ?? openByDefault
    return [
      <li key={orgId} role="presentation">
        <div className="scope-row">
          <button type="button" className="scope-fold" aria-expanded={orgOpen} aria-label={group.org} onClick={() => toggle(orgId, orgOpen)}>
            <ChevronRightIcon size={14} />
          </button>
          <label className="check">
            <Box state={orgState(local, group.repos)} label={group.org} onChange={() => change(setOrg(local, group.org, orgState(local, group.repos) !== 'on'))} />
            <span className="scope-text">{group.org}</span>
            <span className="muted small">{t('settings.cacheScopeRepos', { count: group.repos.length })}</span>
          </label>
          <span className="muted small cache-size">{formatBytes(totalBytes(group.repos))}</span>
        </div>
        {orgOpen && (
          <ul className="scope-tree">
            {visible.map((repo) => {
              const branches = shownByRepo.get(repo.fullName) ?? []
              const repoId = `r:${repo.fullName}`
              const repoOpen = folded.get(repoId) ?? searching
              const name = repo.fullName.slice(repo.org.length + 1)
              return (
                <li key={repoId} role="presentation">
                  <div className="scope-row" style={{ paddingLeft: 16 }}>
                    {branches.length ? (
                      <button type="button" className="scope-fold" aria-expanded={repoOpen} aria-label={repo.fullName} onClick={() => toggle(repoId, repoOpen)}>
                        <ChevronRightIcon size={14} />
                      </button>
                    ) : (
                      <span className="scope-gap" />
                    )}
                    <label className="check" title={repo.fullName}>
                      <Box state={repoState(local, repo)} label={repo.fullName} onChange={() => change(setRepo(local, repo, group.repos, repoState(local, repo) !== 'on'))} />
                      <span className="scope-text">{name}</span>
                    </label>
                    <span className="muted small cache-size">{formatBytes(repo.bytes)}</span>
                  </div>
                  {repoOpen && (
                    <ul className="scope-tree">
                      {branches.map((branch) => (
                        <li key={branch} className="scope-row" style={{ paddingLeft: 32 }}>
                          <span className="scope-gap" />
                          <label className="check" title={branch}>
                            <Box state={isBranchOff(local, repo, branch) ? 'off' : 'on'} label={`${repo.fullName}@${branch}`} onChange={() => change(setBranch(local, repo, group.repos, branch, isBranchOff(local, repo, branch)))} />
                            <span className="scope-text">{branch}</span>
                            {branch === repo.defaultBranch && <span className="badge">{t('browse.defaultBranch')}</span>}
                          </label>
                        </li>
                      ))}
                    </ul>
                  )}
                </li>
              )
            })}
          </ul>
        )}
      </li>
    ]
  })

  return (
    <div className="cache-scope">
      <h3>{t('settings.cacheScope')}</h3>
      <p className="muted small">{t('settings.cacheScopeHint')}</p>
      {saveFailed && (
        <p className="small warn-text" role="alert">
          {t('settings.cacheScopeSaveError')}
        </p>
      )}
      <div className="search">
        <SearchIcon />
        <input aria-label={t('settings.cacheScopeSearch')} placeholder={t('settings.cacheScopeSearch')} value={query} onChange={(e) => setQuery(e.target.value)} />
      </div>
      <div className="scope-row scope-all">
        <span className="scope-gap" />
        <label className="check">
          <Box
            state={everything}
            label={t('settings.cacheScopeAll', { count: shownRepos.length })}
            onChange={() => change(setAll(local, shownRepos, repos, everything !== 'on'))}
          />
          <span className="scope-text">{t('settings.cacheScopeAll', { count: shownRepos.length })}</span>
        </label>
        <span className="muted small cache-size">{formatBytes(totalBytes(shownRepos))}</span>
      </div>
      <ul className="scope-tree scope-list cache-scope-list">{rows}</ul>
      {!rows.length && <p className="muted small picker-empty">{repos.length ? t('picker.noMatch') : t('settings.cacheScopeEmpty')}</p>}
    </div>
  )
}
