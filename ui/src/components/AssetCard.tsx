import { useEffect, useRef, useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import type { Asset, StatsSummary } from '@shared/types'
import { formatBytes } from '../format'
import type { AssetTypeHandler } from '../handlers'
import { StatsLine } from './CatalogPanel'
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
  /** In Favourites. */
  starred?: boolean
  /** Added to a watched branch since it was last marked seen. */
  fresh?: boolean
  /** Right-click: the card's actions. */
  onContextMenu?(e: React.MouseEvent): void
}

export function AssetCard({ asset, handler, thumb, generating, copies, selected, onSelect, picked, onVisible, note, stats, starred, fresh, onContextMenu }: Props) {
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

  return (
    <button
      ref={ref}
      type="button"
      className="asset-card"
      data-asset-id={asset.id}
      aria-pressed={selected}
      data-picked={picked || undefined}
      onClick={(e) => onSelect({ toggle: e.metaKey || e.ctrlKey, range: e.shiftKey })}
      onContextMenu={onContextMenu}
      onMouseEnter={startMotion}
      onMouseLeave={() => setHover(false)}
      onFocus={startMotion}
      onBlur={() => setHover(false)}
      title={asset.path}
    >
      <span className="asset-thumb">
        {thumb ? <img className="asset-img" src={thumb} alt="" loading="lazy" /> : handler?.preview.kind === 'image' ? <ImageIcon size={52} /> : handler?.preview.kind === 'material' ? <SphereIcon size={52} /> : handler?.preview.kind === 'audio' ? <AudioIcon size={52} /> : handler?.preview.kind === 'animation' ? <AnimIcon size={52} /> : handler?.preview.kind === 'font' || handler?.preview.kind === 'tmpfont' ? <FontIcon size={52} /> : <CubeIcon size={56} />}
        {handler && (
          <span className="type-badge" style={{ '--type': `var(--type-${handler.id})` } as CSSProperties}>
            {handler.badge}
          </span>
        )}
        {/* A model holding animation clips. */}
        {handler?.preview.kind === 'three' && stats && stats.clips > 0 ? (
          <span className="clips-badge" title={stats.clipNames?.join(', ')}>
            ▶ {stats.clips}
          </span>
        ) : null}
        {starred && (
          <span className="star-badge" aria-label={t('collections.favorites')}>
            <StarIcon size={13} filled />
          </span>
        )}
        {fresh && <span className="new-badge">{t('watch.new')}</span>}
        {copies && copies > 1 ? <span className="copies-badge" title={t('browse.copiesTitle', { count: copies })}>×{copies}</span> : null}
        {hover && motion && <span className="asset-motion" style={{ backgroundImage: `url("${motion}")` }} aria-hidden="true" />}
        {generating && !thumb && <span className="thumb-spinner" aria-label={t('browse.generatingThumb')} />}
      </span>
      <span className="asset-meta">
        <span className="asset-name">{asset.name}</span>
        {stats && <WeightBar stats={stats} />}
        <span className="asset-sub">
          <span>{formatBytes(asset.size)}</span>
          {asset.lfs && <span className="badge">LFS</span>}
        </span>
        {stats && <StatsLine stats={stats} short />}
        {note && <span className="asset-note">{note}</span>}
      </span>
    </button>
  )
}

/** Placeholder card shown while a branch is loading. */
export function SkeletonCard() {
  return (
    <span className="asset-card skeleton" aria-hidden="true">
      <span className="asset-thumb" />
      <span className="asset-meta">
        <span className="skeleton-line" />
        <span className="skeleton-line short" />
      </span>
    </span>
  )
}
