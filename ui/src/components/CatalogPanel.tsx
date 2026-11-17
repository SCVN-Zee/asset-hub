import { useEffect, useMemo, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { UnityControl, useUnityEditors } from './UnityButton'
import { Section } from './Section'
import type { BudgetIssue, CatalogAsset, CatalogLocation, StatsSummary } from '@shared/types'
import { formatBytes } from '../format'
import type { CatalogStatus } from '@shared/ipc'
import type { SearchScope } from '../useCatalog'
import { summaryIssues } from '../playable'
import { useBudget } from '../budget'
import { DownloadButton } from './DownloadButton'
import { ErrorNote } from './ErrorNote'
import { CheckIcon, ChevronRightIcon, CopyIcon, CubeIcon, ExportIcon, ExternalIcon, LinkIcon } from './Icons'
import { BudgetLine, CopyStatus, FactsGrid, summaryFacts, useCopy } from './InspectorParts'
import { assetLink } from '@shared/deeplink'
import type { AssetTypeHandler } from '../handlers'
import { useDependencies } from '../useDependencies'
import { AssetViewer } from './AssetDetails'
import type { LiveContext } from './ModelViewer'

/** Branch rows shown per path before "Show all". */
const BRANCHES_SHOWN = 5

const repoName = (repo: string) => repo.split('/').pop() ?? repo

/** The file's page on GitHub, on that branch. */
export const githubUrl = (repo: string, branch: string, path: string) =>
  `https://github.com/${repo}/blob/${branch.split('/').map(encodeURIComponent).join('/')}/${path.split('/').map(encodeURIComponent).join('/')}`

/** Short card line: "xp_hero · 12 branches" or the single branch name. */
export function locationNote(asset: CatalogAsset, scope: SearchScope, t: (key: string, o?: Record<string, unknown>) => string): string {
  const first = asset.locations[0]
  const where = asset.branchCount === 1 ? first.branches[0] : t('catalog.branchCount', { count: asset.branchCount })
  if (scope === 'all') return asset.repoCount > 1 ? t('catalog.repoCount', { count: asset.repoCount }) + ` · ${where}` : `${repoName(first.repo)} · ${where}`
  return where
}

interface StatusProps {
  status: CatalogStatus
  error?: unknown
  onResume(): void
}

/** Shows a catalog failure with a retry. Running and paused indexing live in the top-bar task list. */
export function CatalogStatusLine({ status, error, onResume }: StatusProps) {
  const { t } = useTranslation()
  if (error) return <ErrorNote error={error} />
  if (status.state !== 'error') return null
  return (
    <div className="catalog-status" data-state={status.state} role="status">
      <span>
        {status.error?.code === 'rateLimited' && status.error.resetAt
          ? t('catalog.rateLimited', { time: new Date(status.error.resetAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) })
          : t('catalog.failed', { message: status.error?.message ?? '' })}
      </span>
      <button type="button" className="btn btn-small" onClick={onResume}>
        {t('catalog.retry')}
      </button>
    </div>
  )
}

interface LocationsProps {
  locations: readonly CatalogLocation[]
  /** Shown as current (not clickable). */
  here?: { repo: string; branch: string }
  /** Repo names are shown when locations span several repos. */
  showRepo: boolean
  onOpen(repo: string, branch: string, path: string): void
}

/** The filter box appears once the list is longer than this many branch rows. */
const FILTER_FROM = 12

const folderOf = (path: string) => path.split('/').slice(0, -1).join('/')

/** Where a file is: repos fold, each path lists its branches as rows that open the file there. */
export function LocationList({ locations, here, showRepo, onOpen }: LocationsProps) {
  const { t } = useTranslation()
  const { state: copyState, copy } = useCopy()
  const [copiedKey, setCopiedKey] = useState<string>()
  const [query, setQuery] = useState('')
  /** Groups the person folded or unfolded; the rest follow the default (first one open). */
  const [toggled, setToggled] = useState<ReadonlyMap<string, boolean>>(new Map())
  const [showAll, setShowAll] = useState<ReadonlySet<string>>(new Set())
  const needle = query.trim().toLowerCase()
  const total = locations.reduce((n, l) => n + l.branches.length, 0)
  const groups = useMemo(() => {
    const byRepo = new Map<string, CatalogLocation[]>()
    for (const l of locations) byRepo.set(l.repo, [...(byRepo.get(l.repo) ?? []), l])
    return [...byRepo].map(([repo, items]) => ({ repo, items, count: new Set(items.flatMap((l) => l.branches)).size }))
  }, [locations])
  const flip = <T,>(set: ReadonlySet<T>, v: T) => (set.has(v) ? new Set([...set].filter((x) => x !== v)) : new Set([...set, v]))
  const copyPath = (key: string, path: string) => {
    setCopiedKey(key)
    void copy(path)
  }

  const renderPath = (l: CatalogLocation) => {
    const key = `${l.repo}\n${l.path}`
    const matches = needle ? l.branches.filter((b) => b.toLowerCase().includes(needle)) : l.branches
    if (!matches.length) return null
    const open = !!needle || showAll.has(key)
    const rows = open ? matches : matches.slice(0, BRANCHES_SHOWN)
    const dir = folderOf(l.path) || l.path
    const justCopied = copiedKey === key && copyState === 'copied'
    return (
      <li key={key} className="location-item">
        <div className="location-dir-row">
          <span className="location-dir mono" title={`${l.repo}/${l.path}`}>
            <bdi>{dir}</bdi>
          </span>
          <button type="button" className="btn icon-btn location-copy" data-done={justCopied || undefined} title={t('details.copyPath')} aria-label={t('details.copyPath')} onClick={() => copyPath(key, l.path)}>
            {justCopied ? <CheckIcon size={12} /> : <CopyIcon size={12} />}
          </button>
        </div>
        <ul className="branch-rows">
          {rows.map((b) => {
            const current = here?.repo === l.repo && here.branch === b
            return (
              <li key={b}>
                <button type="button" className="branch-row" aria-current={current || undefined} disabled={current} title={t('catalog.openIn', { branch: b })} onClick={() => onOpen(l.repo, b, l.path)}>
                  <span className="branch-name">{b}</span>
                  {!current && <ChevronRightIcon size={12} />}
                </button>
              </li>
            )
          })}
          {!needle && matches.length > BRANCHES_SHOWN && (
            <li>
              <button type="button" className="branch-row branch-more" aria-expanded={open} onClick={() => setShowAll((s) => flip(s, key))}>
                {open ? t('catalog.less') : t('catalog.showAll', { count: matches.length })}
              </button>
            </li>
          )}
        </ul>
      </li>
    )
  }

  return (
    <div className="location-list">
      {total > FILTER_FROM && (
        <input type="search" className="input location-filter" value={query} placeholder={t('catalog.filterBranches')} aria-label={t('catalog.filterBranches')} onChange={(e) => setQuery(e.target.value)} />
      )}
      <ul className="location-groups">
        {groups.map((g, i) => {
          const paths = g.items.map(renderPath).filter(Boolean)
          if (!paths.length) return null
          if (!showRepo) return paths
          const open = !!needle || (toggled.get(g.repo) ?? (i === 0 || groups.length === 1))
          return (
            <li key={g.repo} className="location-group">
              <button type="button" className="location-group-head" aria-expanded={open} onClick={() => setToggled((m) => new Map(m).set(g.repo, !open))}>
                <span className="section-chevron" data-open={open || undefined}>
                  <ChevronRightIcon size={12} />
                </span>
                <span className="location-repo" title={g.repo}>
                  {repoName(g.repo)}
                </span>
                <span className="muted small">{t('catalog.branchCount', { count: g.count })}</span>
              </button>
              {open && <ul className="location-paths">{paths}</ul>}
            </li>
          )
        })}
      </ul>
      {needle && !groups.some((g) => g.items.some((l) => l.branches.some((b) => b.toLowerCase().includes(needle)))) && <p className="acts-note">{t('catalog.noBranchMatch')}</p>}
    </div>
  )
}

interface DetailsProps {
  asset?: CatalogAsset
  thumb?: string
  stats?: StatsSummary
  /** `file`: offer a download instead of a package export (textures). */
  delivery?: 'unitypackage' | 'file'
  exporting: boolean
  onOpen(repo: string, branch: string, path: string): void
  /** Exports the file as found on its main branch, without opening that branch. */
  onExport(asset: CatalogAsset): void
  onImportToUnity(asset: CatalogAsset, editor: RunningUnity): void
  /** Where the live viewer reads the file from: the location the thumbnail comes from. */
  source?: { path: string; ctx: LiveContext }
  handler?: AssetTypeHandler
  /** Every model of the branch, for picking a rig to play a clip on. */
  models?: readonly string[]
  /** Opened from a tab of animations: a model starts playing its first clip. */
  playClips?: boolean
  /** Bar under the preview that resizes it. */
  viewerResizer?: ReactNode
  onThumb?(key: string, dataUrl: string): void
  /** The viewer of `sha` started or stopped loading. */
  onViewerLoading?(loading: boolean, sha: string): void
  /** Full measurements of the viewed file, kept for ranking results. */
  onModelStats?(key: string, stats: StatsSummary): void
}

/** Right panel for a result found in other branches / repos: the same anatomy as an opened asset. */
export function CatalogDetails({ asset, thumb, stats, delivery, exporting, onOpen, onExport, onImportToUnity, source, handler, models, playClips, viewerResizer, onThumb, onViewerLoading, onModelStats }: DetailsProps) {
  const { t } = useTranslation()
  const budget = useBudget()
  const editors = useUnityEditors()
  const { state: copyState, copy } = useCopy()
  /** Every branch of every location: listings carry only the first few of each. */
  const [full, setFull] = useState<{ sha: string; locations: CatalogLocation[] }>()
  const sha = asset?.sha
  // A viewer says which asset it loads, so a load of the previous asset cannot pass for this one.
  const reportLoading = useMemo(() => (sha && onViewerLoading ? (loading: boolean) => onViewerLoading(loading, sha) : undefined), [onViewerLoading, sha])
  // Models, prefabs and materials need their textures; the viewer waits for this report.
  const deps = useDependencies(source?.ctx, source?.path, handler)
  const trimmed = !!asset && ((asset.locationTotal ?? 0) > asset.locations.length || asset.locations.some((l) => (l.branchTotal ?? l.branches.length) > l.branches.length))
  // In the listing's scope: a file listed in one repo (one repo's search) stays in that repo.
  const onlyRepo = asset?.repoCount === 1 ? asset.locations[0]?.repo : undefined
  useEffect(() => {
    if (!sha || !trimmed) return
    let cancelled = false
    void window.api.catalog.whereIs(sha).then((r) => {
      if (cancelled || !r.ok) return
      const locations = r.data.filter((l) => !onlyRepo || l.repo === onlyRepo).sort((a, b) => b.branches.length - a.branches.length)
      if (locations.length) setFull({ sha, locations })
    })
    return () => {
      cancelled = true
    }
  }, [sha, trimmed, onlyRepo])
  if (!asset) {
    return (
      <aside className="details" aria-label={t('details.label')}>
        <p className="empty">{t('catalog.pick')}</p>
      </aside>
    )
  }
  const first = asset.locations[0]
  const branch = first.branches[0]
  const noUnityId = 'catalog-unity-note'
  const issues = budget && stats ? summaryIssues(stats, budget) : []
  const facts = stats ? summaryFacts(stats, t) : [{ label: t('details.size'), value: formatBytes(asset.size) }]
  const openGitHub = () => void window.api.openExternal(githubUrl(first.repo, branch, first.path))
  const link = assetLink({ repo: first.repo, branch, path: first.path })
  return (
    <aside className="details" aria-label={t('details.label')}>
      <AssetViewer
        live={source?.ctx}
        path={source?.path}
        kind={handler?.preview.kind}
        report={deps.status === 'ready' ? deps.report : undefined}
        playClips={playClips}
        models={models}
        resizer={viewerResizer}
        onSummary={(s) => onModelStats?.(asset.sha, s)}
        onThumb={(url) => onThumb?.(asset.sha, url)}
        onLoading={reportLoading}
        fallback={<div className="insp-preview">{thumb ? <img className="catalog-thumb" src={thumb} alt="" /> : <CubeIcon size={72} />}</div>}
      />
      <div className="insp-body">
        <div>
          <div className="insp-path">
            <h2 className="insp-title">{asset.name}</h2>
            <div className="insp-path-tools">
              <CopyStatus state={copyState} />
              <button type="button" className="btn icon-btn" title={t('details.openGitHub')} aria-label={t('details.openGitHub')} onClick={openGitHub}>
                <ExternalIcon />
              </button>
              <button type="button" className="btn icon-btn" title={`${t('link.copy')} · ${t('link.hint')}`} aria-label={t('link.copy')} onClick={() => void copy(link)}>
                <LinkIcon />
              </button>
            </div>
          </div>
          <div className="insp-meta">
            <span className="acts-note">
              {[t('catalog.branchCount', { count: asset.branchCount }), asset.repoCount > 1 && t('catalog.repoCount', { count: asset.repoCount })].filter(Boolean).join(' · ')}
            </span>
            <button type="button" className="open-branch" title={`${t('catalog.openIn', { branch })}. ${t('catalog.openHint')}`} onClick={() => onOpen(first.repo, branch, first.path)}>
              {t('catalog.openBranch')}
              <ChevronRightIcon size={12} />
            </button>
          </div>
        </div>
        <FactsGrid facts={facts} variant="strip" />
        {budget && stats && <BudgetLine issues={issues} budget={budget} />}
        {delivery === 'file' ? (
          <DownloadButton repo={first.repo} branch={branch} path={first.path} />
        ) : (
          <>
            <div className="acts-split">
              <button type="button" className="btn btn-primary" title={t('export.exportOne')} disabled={exporting} onClick={() => onExport(asset)}>
                <ExportIcon />
                {t('inspector.export')}
              </button>
              <UnityControl editors={editors} split disabled={exporting} onImport={(editor) => onImportToUnity(asset, editor)} describedBy={noUnityId} />
            </div>
            {!editors.length && (
              <p className="acts-note" id={noUnityId}>
                {t('unity.noneOpen')}
              </p>
            )}
          </>
        )}
      </div>
      <div className="folds">
        <Section id="catalogLocations" title={t('catalog.locations')} summary={t('catalog.branchCount', { count: asset.branchCount })}>
          <LocationList locations={full?.sha === asset.sha ? full.locations : asset.locations} showRepo={asset.repoCount > 1} onOpen={onOpen} />
        </Section>
      </div>
    </aside>
  )
}

/** 1200 as "1.2k", 24000 as "24k". */
export const compactCount = (n: number) => (n >= 10_000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n))

/** What a budget verdict says, one line per issue (a tooltip). */
export function budgetIssuesTitle(issues: BudgetIssue[], t: (key: string, o?: Record<string, unknown>) => string): string {
  return issues.map((i) => t(`budget.${i.key}`, { value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value.toLocaleString(), limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit.toLocaleString() })).join('\n')
}
