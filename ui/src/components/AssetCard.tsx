import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import type { Asset, StatsSummary } from '@shared/types'
import { formatBytes } from '../format'
import type { AssetTypeHandler } from '../handlers'
import { useBudget } from '../budget'
import { summaryIssues } from '../playable'
import { formatDuration } from '../viewer/audio'
import { budgetIssuesTitle, compactCount } from './CatalogPanel'
import { AnimIcon, AudioIcon, CubeIcon, FontIcon, ImageIcon, SphereIcon, StarIcon } from './Icons'
import { WeightBar } from './WeightBar'
import { motionKey } from '../viewer/particleThumb'

interface Props {
  asset: Asset
  handler: AssetTypeHandler | undefined
  /** Data URL of a preview rendered earlier. */
  thumb?: string
  /** A thumbnail is being rendered in the background. */
  generating?: boolean
  /** Identical files (same content) elsewhere in the project, shown as one card. */
  copies?: number
  selected: boolean
  /** Click; `toggle` (Cmd/Ctrl) and `range` (Shift) pick several cards. */
  onSelect(how: { toggle: boolean; range: boolean }): void
  /** Picked among several (Cmd/Ctrl- or Shift-click). */
  picked?: boolean
  /** Called when the card enters or leaves the screen, to render thumbnails lazily. */
  onVisible?(visible: boolean): void
  /** Extra line under the size, e.g. where the file is in other branches. */
  note?: string
  /** Measurements, when known: triangle count and playable-budget verdict. */
  stats?: StatsSummary
  /** Already in the export list. */
  inList?: boolean
  /** In Favourites. */
  starred?: boolean
  /** Added to a watched branch since it was last marked seen. */
  fresh?: boolean
  /** Right-click: the card's actions. */
  onContextMenu?(e: React.MouseEvent): void
}

export function AssetCard({ asset, handler, thumb, generating, copies, selected, onSelect, picked, onVisible, note, stats, starred, fresh, inList, onContextMenu }: Props) {
  const { t } = useTranslation()
  const ref = useRef<HTMLButtonElement>(null)
  const visibleRef = useRef(onVisible)
  visibleRef.current = onVisible
  // An effect filmed through its run plays on hover; read from the cache the first time.
  const filmed = !!stats?.fxThumb && !!thumb
  const [motion, setMotion] = useState<string | null>()
  const [hover, setHover] = useState(false)
  useEffect(() => setMotion(undefined), [asset.sha, thumb])
  const startMotion = () => {
    setHover(true)
    if (!filmed || motion !== undefined) return
    setMotion(null)
    const key = motionKey(asset.sha)
    void window.api.thumbs.get([key]).then((found) => setMotion(found[key] ?? null), () => setMotion(null))
  }

  useEffect(() => {
    const el = ref.current
    if (!el) return
    const observer = new IntersectionObserver(([entry]) => visibleRef.current?.(entry.isIntersecting), { rootMargin: '200px' })
    observer.observe(el)
    return () => observer.disconnect()
    // Re-observed when the callback appears, so a card already on screen reports itself.
  }, [!!onVisible])

  const budget = useBudget()
  const issues = budget && stats ? summaryIssues(stats, budget) : []
  const kind = handler?.preview.kind
  const fact = keyFact(kind, stats, t)
  const weight = asset.size !== undefined ? formatBytes(asset.size) : stats ? formatBytes(stats.fileBytes + stats.textureBytes) : undefined
  const icon = 40
  const playing = hover && !!motion

  return (
    <button
      ref={ref}
      type="button"
      className="asset-card"
      data-asset-id={asset.id}
      aria-pressed={selected}
      data-picked={picked || undefined}
      data-in-list={inList || undefined}
      onClick={(e) => onSelect({ toggle: e.metaKey || e.ctrlKey, range: e.shiftKey })}
      onContextMenu={onContextMenu}
      onMouseEnter={startMotion}
      onMouseLeave={() => setHover(false)}
      onFocus={startMotion}
      onBlur={() => setHover(false)}
      title={asset.path}
    >
      <span className="asset-thumb" data-kind={kind} data-playing={playing || undefined}>
        {thumb ? <img className="asset-img" src={thumb} alt="" loading="lazy" /> : kind === 'image' ? <ImageIcon size={icon} /> : kind === 'material' ? <SphereIcon size={icon} /> : kind === 'audio' ? <AudioIcon size={icon} /> : kind === 'animation' ? <AnimIcon size={icon} /> : kind === 'font' || kind === 'tmpfont' ? <FontIcon size={icon} /> : <CubeIcon size={icon} />}
        {hover && motion && <span className="asset-motion" style={{ backgroundImage: `url("${motion}")` }} aria-hidden="true" />}
        {handler && (
          <span className="type-badge" style={{ '--type': `var(--type-${handler.id}, var(--mute))` } as CSSProperties}>
            <span className="type-badge-label">{t(handler.labelKey)}</span>
          </span>
        )}
        <span className="cell-marks">
          {issues.length > 0 && (
            <span className="cell-mark cell-mark-warn" title={budgetIssuesTitle(issues, t)} aria-label={t('browse.overBudget')}>
              !
            </span>
          )}
          {copies && copies > 1 ? (
            <span className="cell-mark" title={t('browse.copiesTitle', { count: copies })}>
              ×{copies}
            </span>
          ) : null}
          {picked && (
            <span className="cell-mark cell-mark-pick" title={t('browse.picked')} aria-label={t('browse.picked')}>
              ✓
            </span>
          )}
          {inList && (
            <span className="cell-mark cell-mark-list" title={t('browse.inExportList')} aria-label={t('browse.inExportList')}>
              ✓
            </span>
          )}
        </span>
        <span className="cell-tags">
          {fresh && <span className="cell-tag cell-tag-new">{t('watch.new')}</span>}
          {/* A model holding animation clips. */}
          {kind === 'three' && stats && stats.clips > 0 ? (
            <span className="cell-tag" title={stats.clipNames?.join(', ')}>
              ▶ {stats.clips}
            </span>
          ) : null}
        </span>
        {starred && (
          <span className="star-badge" aria-label={t('collections.favorites')}>
            <StarIcon size={12} filled />
          </span>
        )}
        {generating && !thumb && <span className="thumb-spinner" aria-label={t('browse.generatingThumb')} />}
        {stats && <WeightBar stats={stats} />}
      </span>
      <span className="asset-name">{asset.name}</span>
      <span className="asset-fact">
        {weight && <span className="num">{weight}</span>}
        {fact && <span className="num">{fact}</span>}
        {asset.lfs && <span className="num asset-lfs">LFS</span>}
      </span>
      {note && <span className="asset-note">{note}</span>}
    </button>
  )
}

/** The one fact that decides for each kind of asset, besides its weight; nothing when it is not known (yet) or the type has none. */
function keyFact(kind: string | undefined, stats: StatsSummary | undefined, t: (key: string, o?: Record<string, unknown>) => string): string | undefined {
  if (!stats) return undefined
  switch (kind) {
    case 'three':
    case 'prefab':
      if (stats.triangles > 0) return t('ranking.tris', { n: compactCount(stats.triangles) })
      return stats.particles !== undefined ? t('ranking.particles', { n: compactCount(stats.particles) }) : undefined
    case 'image':
      return stats.width ? `${stats.width}×${stats.height}` : undefined
    case 'animation':
      return stats.clipLength !== undefined ? formatDuration(stats.clipLength) : undefined
    case 'audio':
      return stats.duration !== undefined ? formatDuration(stats.duration) : undefined
    case 'material':
      return stats.textureCount ? t('ranking.textures', { count: stats.textureCount }) : undefined
    default:
      return undefined
  }
}

/** Placeholder card shown while a branch is loading. */
export function SkeletonCard() {
  return (
    <span className="asset-card skeleton" aria-hidden="true">
      <span className="asset-thumb" />
      <span className="skeleton-line" />
      <span className="skeleton-line short" />
    </span>
  )
}
