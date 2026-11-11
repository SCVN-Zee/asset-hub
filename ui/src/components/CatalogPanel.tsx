import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { UnityButton } from './UnityButton'
import type { CatalogAsset, CatalogLocation, StatsSummary } from '@shared/types'
import { formatBytes, shortPath } from '../format'
import type { CatalogStatus } from '@shared/ipc'
import type { SearchScope } from '../useCatalog'
import { summaryIssues } from '../playable'
import { formatDuration } from '../viewer/audio'
import { useBudget } from '../budget'
import { DownloadButton } from './DownloadButton'
import { ErrorNote } from './ErrorNote'
import { CubeIcon, ExportIcon } from './Icons'
import { CopyLinkButton } from './CopyLinkButton'

/** Branch chips shown before "+N more". */
const BRANCHES_SHOWN = 6

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

interface ScopeProps {
  scope: SearchScope
  onScope(scope: SearchScope): void
}

export function ScopeTabs({ scope, onScope }: ScopeProps) {
  const { t } = useTranslation()
  return (
    <div className="scope-tabs" role="group" aria-label={t('catalog.scope')}>
      {(['branch', 'repo', 'all'] as const).map((s) => (
        <button key={s} type="button" className="tab" aria-pressed={scope === s} onClick={() => onScope(s)}>
          {t(`catalog.scopes.${s}`)}
        </button>
      ))}
    </div>
  )
}

interface StatusProps {
  status: CatalogStatus
  error?: unknown
  onPause(): void
  onResume(): void
}

/** One line about background indexing, with pause / resume. */
export function CatalogStatusLine({ status, error, onPause, onResume }: StatusProps) {
  const { t } = useTranslation()
  if (error) return <ErrorNote error={error} />
  if (status.state === 'idle') return null
  return (
    <div className="catalog-status muted small" role="status">
      {status.state === 'running' && (
        <>
          <span>
            {t('catalog.indexing', { done: status.done, total: status.total })}
            {status.current && <span className="mono"> · {status.current.split('@').pop()}</span>}
          </span>
          <progress className="progress" max={Math.max(1, status.total)} value={status.done} />
          <button type="button" className="btn btn-small" onClick={onPause}>
            {t('catalog.pause')}
          </button>
        </>
      )}
      {status.state === 'paused' && (
        <>
          <span>{t('catalog.paused', { done: status.done, total: status.total })}</span>
          <button type="button" className="btn btn-small" onClick={onResume}>
            {t('catalog.resume')}
          </button>
        </>
      )}
      {status.state === 'error' && (
        <>
          <span>
            {status.error?.code === 'rateLimited' && status.error.resetAt
              ? t('catalog.rateLimited', { time: new Date(status.error.resetAt).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) })
              : t('catalog.failed', { message: status.error?.message ?? '' })}
          </span>
          <button type="button" className="btn btn-small" onClick={onResume}>
            {t('catalog.retry')}
          </button>
        </>
      )}
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

/** Where a file is: one row per repo + path, with a chip per branch that opens it there. */
export function LocationList({ locations, here, showRepo, onOpen }: LocationsProps) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  return (
    <ul className="location-list">
      {locations.map((l) => {
        const key = `${l.repo}\n${l.path}`
        const open = expanded.has(key)
        const branches = open ? l.branches : l.branches.slice(0, BRANCHES_SHOWN)
        return (
          <li key={key}>
            <div className="location-path" title={`${l.repo}/${l.path}`}>
              {showRepo && <span className="location-repo">{repoName(l.repo)}</span>}
              <span className="mono small">{shortPath(l.path)}</span>
            </div>
            <div className="chips">
              {branches.map((b) => {
                const current = here?.repo === l.repo && here.branch === b
                return (
                  <button key={b} type="button" className="chip" aria-current={current} disabled={current} onClick={() => onOpen(l.repo, b, l.path)} title={t('catalog.openIn', { branch: b })}>
                    {b}
                  </button>
                )
              })}
              {l.branches.length > BRANCHES_SHOWN && (
                <button
                  type="button"
                  className="chip chip-more"
                  onClick={() => setExpanded((s) => (s.has(key) ? new Set([...s].filter((k) => k !== key)) : new Set([...s, key])))}
                >
                  {open ? t('catalog.less') : t('catalog.more', { count: l.branches.length - BRANCHES_SHOWN })}
                </button>
              )}
            </div>
          </li>
        )
      })}
    </ul>
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
}

/** Right panel for a result found in other branches / repos. */
export function CatalogDetails({ asset, thumb, stats, delivery, exporting, onOpen, onExport, onImportToUnity }: DetailsProps) {
  const { t } = useTranslation()
  /** Every branch of every location: listings carry only the first few of each. */
  const [full, setFull] = useState<{ sha: string; locations: CatalogLocation[] }>()
  const sha = asset?.sha
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
  return (
    <aside className="details" aria-label={t('details.label')}>
      <div className="preview-box">{thumb ? <img className="catalog-thumb" src={thumb} alt="" /> : <CubeIcon size={72} />}</div>
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <h2>{asset.name}</h2>
        <p className="muted small" style={{ margin: 0 }}>
          {formatBytes(asset.size)} · {t('catalog.foundIn', { count: asset.branchCount })}
          {asset.repoCount > 1 && ` · ${t('catalog.repoCount', { count: asset.repoCount })}`}
        </p>
        <div className="field-row">
          <button type="button" className="btn btn-primary" onClick={() => onOpen(first.repo, first.branches[0], first.path)}>
            {t('catalog.open', { branch: first.branches[0] })}
          </button>
          {delivery !== 'file' && (
            <button type="button" className="btn icon-btn labeled" title={t('export.exportOne')} aria-label={t('export.exportOne')} disabled={exporting} onClick={() => onExport(asset)}>
              <ExportIcon />
              <span>{t('actions.export')}</span>
            </button>
          )}
          {delivery !== 'file' && <UnityButton compact disabled={exporting} onImport={(editor) => onImportToUnity(asset, editor)} />}
          <button type="button" className="btn" onClick={() => void window.api.openExternal(githubUrl(first.repo, first.branches[0], first.path))}>
            {t('details.openGitHub')}
          </button>
          <CopyLinkButton link={{ repo: first.repo, branch: first.branches[0], path: first.path }} />
        </div>
        {delivery === 'file' && <DownloadButton repo={first.repo} branch={first.branches[0]} path={first.path} />}
        {stats && <StatsLine stats={stats} />}
        <p className="muted small" style={{ margin: 0 }}>
          {t('catalog.openHint')}
        </p>
      </div>
      <section className="section">
        <span className="label-caps">{t('catalog.locations')}</span>
        <LocationList locations={full?.sha === asset.sha ? full.locations : asset.locations} showRepo={asset.repoCount > 1} onOpen={onOpen} />
      </section>
    </aside>
  )
}

const compact = (n: number) => (n >= 10_000 ? `${Math.round(n / 1000)}k` : n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n))

/** One line of measurements with a playable-budget verdict, for cards and panels. */
export function StatsLine({ stats, short = false }: { stats: StatsSummary; short?: boolean }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const issues = budget ? summaryIssues(stats, budget) : []
  return (
    <span className="stats-line" title={!budget ? undefined : issues.map((i) => t(`budget.${i.key}`, { value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value.toLocaleString(), limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit.toLocaleString() })).join('\n') || t('ranking.fits')}>
      {budget && <span className={issues.length ? 'fit fit-no' : 'fit fit-ok'}>{issues.length ? `⚠ ${issues.length}` : '✓'}</span>}
      {stats.width ? (
        <span>{`${stats.width}×${stats.height}`}</span>
      ) : stats.duration !== undefined || stats.clipLength !== undefined ? (
        <span>{formatDuration(stats.duration ?? stats.clipLength ?? 0)}</span>
      ) : stats.particles !== undefined && stats.triangles === 0 ? (
        <span>{t('ranking.particles', { n: compact(stats.particles) })}</span>
      ) : stats.glyphs !== undefined ? (
        <span>{t('ranking.glyphs', { count: stats.glyphs })}</span>
      ) : stats.textureCount !== undefined ? (
        <span>{t('ranking.textures', { count: stats.textureCount })}</span>
      ) : (
        <span>{t('ranking.tris', { n: compact(stats.triangles) })}</span>
      )}
      {(!short || stats.width || stats.textureCount !== undefined || stats.glyphs !== undefined || stats.duration !== undefined) && <span>{stats.duration !== undefined && '≈ '}{formatBytes(stats.fileBytes + stats.textureBytes)}</span>}
      {!short && stats.bones > 0 && <span>{t('ranking.bones', { count: stats.bones })}</span>}
    </span>
  )
}
