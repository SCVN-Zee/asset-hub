import { lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { UnityButton } from './UnityButton'
import { Section } from './Section'
import { checkPlayableBudget, summarizeStats, summaryIssues } from '../playable'
import { useBudget } from '../budget'
import { formatBytes, shortPath } from '../format'
import type { AssetTypeHandler } from '../handlers'
import { parse } from '../parse'
import type { Asset, CatalogLocation, DependencyReport, AnimClipInfo, MaterialLook, MaterialRef, ModelStats, SpriteBorder, PartRef, ParticleTotals, PrefabMaterialsResult, StatsSummary, TextureRef } from '@shared/types'
import { RemoteError, unwrap } from '../remoteSource'
import { githubUrl, LocationList } from './CatalogPanel'
import { ErrorNote } from './ErrorNote'
import { CheckIcon, ChestIcon, CopyIcon, CubeIcon, ExportIcon, ExternalIcon } from './Icons'
import type { LiveContext } from './ModelViewer'
import type { ImageInfo } from './ImageViewer'
import { AudioViewer } from './AudioViewer'
import { audioStats, formatDuration, type DecodedAudio } from '../viewer/audio'
import { FontViewer } from './FontViewer'
import { CopyLinkButton } from './CopyLinkButton'
import { TmpFontViewer } from './TmpFontViewer'
import { tmpFontStats, type LoadedTmpFont } from '../viewer/tmpFont'
import { particleStats } from '../viewer/particleThumb'
import { fontStats, type LoadedFont } from '../viewer/font'
import { DownloadButton } from './DownloadButton'
import { ModelBundleButtons } from './ModelBundleButtons'

// Viewers drawing with three.js load when first opened: the app starts without the 3D engine.
const ModelViewer = lazy(() => import('./ModelViewer').then((m) => ({ default: m.ModelViewer })))
const ImageViewer = lazy(() => import('./ImageViewer').then((m) => ({ default: m.ImageViewer })))
const MaterialViewer = lazy(() => import('./MaterialViewer').then((m) => ({ default: m.MaterialViewer })))
const PrefabViewer = lazy(() => import('./PrefabViewer').then((m) => ({ default: m.PrefabViewer })))
const AnimViewer = lazy(() => import('./AnimViewer').then((m) => ({ default: m.AnimViewer })))

interface Props {
  asset: Asset | undefined
  handler: AssetTypeHandler | undefined
  /** Repo context; absent with sample data, where nothing can be downloaded. */
  live?: LiveContext
  onThumb(key: string, dataUrl: string): void
  /** Full measurements of the opened model, kept for ranking results. */
  onModelStats?(key: string, stats: StatsSummary): void
  /** Called once dependencies are known, so the pack can show sizes and warnings. */
  onReport(path: string, report: DependencyReport): void
  inPack: boolean
  exporting: boolean
  onTogglePack(): void
  onExport(): void
  /** Opens the package in this Unity editor instead of saving it. */
  onImportToUnity(editor: RunningUnity): void
  /** Byte-identical copies of this file, including itself. */
  copies?: Asset[]
  onSelectCopy(id: string): void
  /** Folders left out of prefab searches, like the grid. */
  excludedFolders: string[]
  /** Extra files (prefab materials) to include when this model is exported. */
  onPrefabFiles(path: string, files: string[]): void
  /** Opens the same file in another branch or repo. */
  onOpenLocation(repo: string, branch: string, path: string): void
  onViewerLoading?(loading: boolean): void
  /** Every model of the branch (repo paths), for picking a rig to play a clip on. */
  models?: readonly string[]
  /** Opened from a tab of animations: a model starts playing its first clip. */
  playClips?: boolean
  /** Bar under the preview that resizes it. */
  viewerResizer?: ReactNode
  /** Star and collections menu for this asset. */
  collectionActions?: ReactNode
}

type PrefabSearch =
  | { status: 'idle' }
  /** Remembered prefabs use this model; their materials load when asked. */
  | { status: 'known'; result: PrefabMaterialsResult }
  | { status: 'running'; scope: 'cached' | 'near' | 'all' } | { status: 'ready'; result: PrefabMaterialsResult } | { status: 'error'; error: unknown }

type Deps = { status: 'idle' } | { status: 'loading' } | { status: 'error'; error: unknown } | { status: 'ready'; report: DependencyReport }

function Dot({ tone }: { tone: 'ok' | 'warn' | 'info' }) {
  return <span className={`dot dot-${tone}`} aria-hidden="true" />
}

const TEXTURE_TYPES: Record<number, string> = { 0: 'default', 1: 'normal', 2: 'gui', 4: 'cookie', 6: 'lightmap', 7: 'cursor', 8: 'sprite', 10: 'single' }
const COMPRESSION: Record<number, string> = { 0: 'none', 1: 'normal', 2: 'high', 3: 'low' }

/** Size, format and Unity import settings of a texture, with what matters for playables. */
function TextureSection({ info }: { info: ImageInfo }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const imp = info.import
  const built = info.built
  const warnings: string[] = []
  if (budget && built > budget.textureSize) warnings.push(t('texture.warn.size', { size: built, limit: budget.textureSize }))
  if (imp?.compression === 0) warnings.push(t('texture.warn.uncompressed'))
  if (imp?.textureType === 8 && imp.mipmaps) warnings.push(t('texture.warn.spriteMips'))
  if (budget && info.bytes > budget.textureBytes) warnings.push(t('texture.warn.bytes', { size: formatBytes(info.bytes), limit: formatBytes(budget.textureBytes) }))
  const cells: Array<[string, string]> = [
    [t('texture.size'), `${info.width} × ${info.height}`],
    [t('texture.file'), `${formatBytes(info.bytes)} · ${info.format}`],
    // As Unity imports it: alpha made from brightness, or dropped, says so.
    [t('texture.alpha'), imp?.alphaSource === 'gray' ? t('texture.alphaGray') : imp?.alphaSource === 'none' ? t('texture.alphaNone') : info.hasAlpha ? t('texture.yes') : t('texture.no')],
    ...(imp
      ? ([
          [t('texture.type'), t(`texture.types.${TEXTURE_TYPES[imp.textureType ?? 0] ?? 'default'}`)],
          [t('texture.built'), `${built} px${imp.maxTextureSize ? ` (max ${imp.maxTextureSize})` : ''}`],
          [t('texture.compression'), t(`texture.compressions.${COMPRESSION[imp.compression ?? 1] ?? 'normal'}`)],
          [t('texture.mipmaps'), imp.mipmaps ? t('texture.yes') : t('texture.no')],
          [t('texture.srgb'), imp.sRGB === false ? t('texture.no') : t('texture.yes')]
        ] as Array<[string, string]>)
      : [])
  ]
  return (
    <section className="section">
      <span className="label-caps">{t('texture.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {imp && imp.overrides.length > 0 && (
        <p className="muted small" style={{ margin: 0 }}>
          {t('texture.overrides', { list: imp.overrides.map((o) => `${o.target}${o.maxTextureSize ? ` ${o.maxTextureSize}` : ''}`).join(', ') })}
        </p>
      )}
      {!imp && <p className="muted small" style={{ margin: 0 }}>{t('texture.noMeta')}</p>}
      {warnings.length > 0 && (
        <ul className="warn-note warn-list">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </section>
  )
}

const LOAD_TYPES: Record<number, string> = { 0: 'decompress', 1: 'compressed', 2: 'streaming' }
const AUDIO_FORMATS: Record<number, string> = { 0: 'PCM', 1: 'Vorbis', 2: 'ADPCM', 3: 'MP3', 7: 'AAC' }

/** Length, channels, rate and Unity import settings of a clip, with what matters for playables. */
function AudioSection({ info }: { info: DecodedAudio }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const imp = info.import
  const s = info.effective
  const stats = audioStats(info)
  const mono = imp?.forceToMono && info.channels > 1
  const rate = s?.sampleRateSetting === 2 && s.sampleRateOverride && s.sampleRateOverride < info.sampleRate ? s.sampleRateOverride : undefined
  const format = s?.compressionFormat ?? 1
  const warnings = (budget ? summaryIssues(stats, budget) : []).map((i) =>
    t(`budget.${i.key}`, { value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value, limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit })
  )
  if (format === 0 && info.duration > 5) warnings.push(t('audio.warn.pcm'))
  if ((s?.loadType ?? 0) === 0 && info.duration > 10) warnings.push(t('audio.warn.decompress'))
  if (info.channels > 1 && !imp?.forceToMono && info.duration < 5) warnings.push(t('audio.warn.mono'))
  const cells: Array<[string, string]> = [
    [t('audio.length'), formatDuration(info.duration)],
    [t('audio.channels'), `${t(info.channels === 1 ? 'audio.mono' : 'audio.stereo')}${mono ? ` → ${t('audio.mono')}` : ''}`],
    [t('audio.rate'), `${(info.sampleRate / 1000).toLocaleString()} kHz${rate ? ` → ${(rate / 1000).toLocaleString()} kHz` : ''}`],
    [t('audio.file'), `${formatBytes(info.bytes)} · ${info.format}`],
    ...(imp
      ? ([
          [t('audio.loadType'), t(`audio.loadTypes.${LOAD_TYPES[s?.loadType ?? 0] ?? 'decompress'}`)],
          [t('audio.compression'), `${AUDIO_FORMATS[format] ?? format}${(format === 1 || format === 3) && s?.quality !== undefined ? ` ${Math.round(s.quality * 100)}%` : ''}`]
        ] as Array<[string, string]>)
      : []),
    [t('audio.built'), `≈ ${formatBytes(stats.fileBytes)}`]
  ]
  return (
    <section className="section">
      <span className="label-caps">{t('audio.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {imp && imp.overrides.length > 0 && (
        <p className="muted small" style={{ margin: 0 }}>
          {t('audio.overrides', { list: imp.overrides.map((o) => `${o.target}${o.compressionFormat !== undefined ? ` ${AUDIO_FORMATS[o.compressionFormat] ?? o.compressionFormat}` : ''}`).join(', ') })}
        </p>
      )}
      {!imp && <p className="muted small" style={{ margin: 0 }}>{t('audio.noMeta')}</p>}
      {warnings.length > 0 && (
        <ul className="warn-note warn-list">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </section>
  )
}

/** Family, glyphs and scripts of a font, with what matters for playables. */
function FontSection({ font }: { font: LoadedFont }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const { info } = font
  const imp = font.import
  const { coverage } = info
  const vi = coverage.find((c) => c.script === 'vietnamese')
  const warnings = (budget ? summaryIssues(fontStats(font), budget) : []).map((i) => t(`budget.${i.key}`, { value: formatBytes(i.value), limit: formatBytes(i.limit) }))
  if (warnings.length && coverage.some((c) => (c.script === 'korean' || c.script === 'chinese') && c.have > 1000)) warnings.push(t('font.warn.subset'))
  if (vi && !vi.full) warnings.push(t('font.warn.vietnamese', { chars: info.missingVietnamese }))
  if (imp && imp.mode !== 'dynamic') warnings.push(t('font.warn.baked'))
  const cells: Array<[string, string]> = [
    [t('font.family'), info.family || '—'],
    [t('font.style'), info.style || '—'],
    [t('font.glyphCount'), info.glyphs.toLocaleString()],
    [t('font.file'), `${formatBytes(font.bytes)} · ${info.outlines === 'cff' ? 'OpenType (CFF)' : 'TrueType'}`],
    ...(imp ? ([[t('font.mode'), t(`font.modes.${imp.mode}`)]] as Array<[string, string]>) : []),
    ...(imp?.fallbacks.length ? ([[t('font.fallbacks'), imp.fallbacks.join(', ')]] as Array<[string, string]>) : [])
  ]
  return (
    <section className="section">
      <span className="label-caps">{t('font.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <div className="chip-row">
        {coverage.map((c) => (
          <span key={c.script} className={`badge${c.full ? '' : ' badge-warn'}`} title={`${c.have.toLocaleString()} / ${c.total.toLocaleString()}`}>
            {c.full ? '✓' : '◐'} {t(`font.scripts.${c.script}`)}
          </span>
        ))}
      </div>
      {info.collection && info.collection > 1 && <p className="muted small" style={{ margin: 0 }}>{t('font.collection', { count: info.collection })}</p>}
      {warnings.length > 0 && (
        <ul className="warn-note warn-list">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </section>
  )
}

/** What a TextMeshPro font asset holds, and what it costs a playable. */
function TmpFontSection({ font }: { font: LoadedTmpFont }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const { info } = font
  const { coverage } = info
  const vi = coverage.find((c) => c.script === 'vietnamese')
  const warnings = (budget ? summaryIssues(tmpFontStats(font), budget) : []).map((i) =>
    t(`budget.${i.key}`, { value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value, limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit })
  )
  if (info.dynamic) warnings.push(t('tmpFont.warn.dynamic'))
  if (vi && !vi.full) warnings.push(t('font.warn.vietnamese', { chars: info.missingVietnamese }))
  const cells: Array<[string, string]> = [
    [t('font.family'), info.family || '—'],
    [t('font.style'), info.style || '—'],
    [t('tmpFont.characters'), info.characterCount.toLocaleString()],
    [t('tmpFont.atlasSize'), `${info.atlasWidth}×${info.atlasHeight}`],
    [t('tmpFont.pointSize'), info.pointSize ? `${info.pointSize}` : '—'],
    [t('tmpFont.mode'), t(info.dynamic ? 'tmpFont.dynamic' : 'tmpFont.static')],
    [t('tmpFont.built'), `≈ ${formatBytes(info.atlasWidth * info.atlasHeight)}`],
    ...(info.fallbacks.length ? ([[t('font.fallbacks'), String(info.fallbacks.length)]] as Array<[string, string]>) : [])
  ]
  return (
    <section className="section">
      <span className="label-caps">{t('tmpFont.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v], i) => (
          // The atlas size and the atlas mode share a label, so the label alone is not a key.
          <div key={`${i}:${k}`} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <div className="chip-row">
        {coverage.map((c) => (
          <span key={c.script} className={`badge${c.full ? '' : ' badge-warn'}`} title={`${c.have.toLocaleString()} / ${c.total.toLocaleString()}`}>
            {c.full ? '✓' : '◐'} {t(`font.scripts.${c.script}`)}
          </span>
        ))}
      </div>
      {warnings.length > 0 && (
        <ul className="warn-note warn-list">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </section>
  )
}

/** Unity's default Max Particles: left as is, it reserves far more than most effects use. */
const DEFAULT_MAX_PARTICLES = 1000

/** An effect's emitters, what they add up to, and what costs most in a playable. */
function ParticleSection({ report }: { report: DependencyReport }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const emitters = report.particles ?? []
  // The totals come from the Rust side; the section shows once they are in.
  const [totals, setTotals] = useState<ParticleTotals>()
  useEffect(() => {
    let current = true
    setTotals(undefined)
    parse.particleTotals(report.particles ?? []).then(
      (r) => current && setTotals(r),
      () => undefined
    )
    return () => {
      current = false
    }
  }, [report])
  if (!totals) return null
  const count = (pred: (e: (typeof emitters)[number]) => boolean) => emitters.filter(pred).length
  const warnings: string[] = []
  if (budget && totals.peak > budget.particles) warnings.push(t('budget.particles', { value: totals.peak.toLocaleString(), limit: budget.particles.toLocaleString() }))
  const notes: Array<[number, string]> = [
    [count((e) => e.modules.includes('collision')), 'particles.warn.collision'],
    [count((e) => e.modules.includes('lights')), 'particles.warn.lights'],
    [count((e) => e.modules.includes('noise')), 'particles.warn.noise'],
    [count((e) => e.modules.includes('trails')), 'particles.warn.trails'],
    [count((e) => e.renderMode === 'mesh' && e.peak > 50), 'particles.warn.meshes'],
    [count((e) => e.prewarm), 'particles.warn.prewarm'],
    [count((e) => e.maxParticles === DEFAULT_MAX_PARTICLES && e.peak < 200), 'particles.warn.maxDefault']
  ]
  for (const [n, key] of notes) if (n) warnings.push(t(key, { count: n }))
  const cells: Array<[string, string]> = [
    [t('particles.emitters'), totals.emitters.toLocaleString()],
    [t('particles.peak'), `≈ ${totals.peak.toLocaleString()}`],
    [t('particles.length'), totals.looping ? t('particles.loops') : formatDuration(totals.duration)]
  ]
  const flags = (e: (typeof emitters)[number]) =>
    [e.shape, e.renderMode !== 'billboard' ? t(`particles.modes.${e.renderMode}`) : undefined, e.sheet && e.sheet[0] * e.sheet[1] > 1 ? `${e.sheet[0]}×${e.sheet[1]}` : undefined, ...e.modules.filter((m) => m !== 'textureSheet').map((m) => t(`particles.modules.${m}`))]
      .filter(Boolean)
      .join(' · ')
  return (
    <Section id="particles" defaultOpen={false} title={t('particles.title')} summary={t('particles.summary', { count: report.particles?.length ?? 0 })}>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      <ul className="emitter-list">
        {[...emitters]
          .sort((a, b) => b.peak - a.peak)
          .map((e) => (
            <li key={`${e.prefab ?? ''}:${e.id}`} className={e.renderMode === 'none' ? 'muted' : undefined}>
              <span className="dep-name" title={e.prefab}>
                {e.name || '—'}
                {e.prefab && <span className="muted small"> · {e.prefab.slice(e.prefab.lastIndexOf('/') + 1)}</span>}
              </span>
              <span className="muted small">{flags(e)}</span>
              <span className="mono small">{e.renderMode === 'none' ? t('particles.hidden') : `≈ ${e.peak}`}</span>
            </li>
          ))}
      </ul>
      <p className="muted small" style={{ margin: 0 }}>
        {t('particles.estimate')}
      </p>
      {warnings.length > 0 && (
        <ul className="warn-note warn-list">
          {warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
    </Section>
  )
}

/** Long values (shader name, load type) take a whole row so they stay on one line. */
const statClass = (v: ReactNode) => (typeof v === 'string' && v.length > 18 ? 'stat-wide' : undefined)

const SURFACES = { opaque: 'opaque', cutout: 'cutout', transparent: 'transparent' } as const
const toHex = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, '0')

/** Shader, surface, colour and texture weight of a material, with what matters for playables. */
function MaterialSection({ material, look, stats }: { material?: MaterialRef; look?: MaterialLook; stats?: StatsSummary }) {
  const { t } = useTranslation()
  const budget = useBudget()
  const color = look?.color && `#${toHex(look.color.r)}${toHex(look.color.g)}${toHex(look.color.b)}`
  const cells: Array<[string, ReactNode]> = [
    ...(material?.shader ? [[t('material.shader'), material.shader.label] as [string, ReactNode]] : []),
    ...(look
      ? ([
          [t('material.surface'), t(`material.surfaces.${SURFACES[look.surface]}`) + (look.surface === 'transparent' && look.color && look.color.a < 1 ? ` · ${Math.round(look.color.a * 100)}%` : '')],
          ...(color ? [[t('material.color'), <span key="c" className="swatch-cell"><span className="swatch" style={{ background: color }} aria-hidden="true" />{color.toUpperCase()}</span>]] : [])
        ] as Array<[string, ReactNode]>)
      : []),
    ...(stats
      ? ([
          [t('material.textures'), stats.textureCount ? `${stats.textureCount} · ${formatBytes(stats.textureBytes)}` : t('material.noTextures')],
          ...(stats.maxTextureSize ? [[t('material.largest'), `${stats.maxTextureSize} px`]] : [])
        ] as Array<[string, ReactNode]>)
      : [])
  ]
  const issues = stats && budget ? summaryIssues(stats, budget).filter((i) => i.key === 'textureSize' || i.key === 'textureBytes') : []
  if (!cells.length) return null
  return (
    <section className="section">
      <span className="label-caps">{t('material.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {issues.length > 0 && (
        <ul className="warn-note warn-list">
          {issues.map((i) => (
            <li key={i.key}>
              {t(`budget.${i.key}`, {
                value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value.toLocaleString(),
                limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit.toLocaleString()
              })}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}

const SPRITE_MODES: Record<number, string> = { 1: 'single', 2: 'multiple', 3: 'polygon' }
const borderText = (b: SpriteBorder) => `L${b.left} · B${b.bottom} · R${b.right} · T${b.top}`

/** Sprite settings: mode, pixels per unit, 9-slice border, and the sprites cut from a sheet. */
function SpriteSection({ info }: { info: ImageInfo }) {
  const { t } = useTranslation()
  const s = info.sprite!
  // How much of a sheet its sprites use: a mostly empty sheet wastes memory in a playable.
  const used = s.sprites.reduce((sum, r) => sum + r.width * r.height, 0) / Math.max(1, info.width * info.height)
  const cells: Array<[string, string]> = [
    [t('sprite.mode'), t(`sprite.modes.${SPRITE_MODES[s.mode] ?? 'single'}`)],
    ...(s.sprites.length ? ([[t('sprite.count'), String(s.sprites.length)], [t('sprite.used'), `${Math.round(used * 100)}%`]] as Array<[string, string]>) : []),
    ...(s.pixelsPerUnit ? ([[t('sprite.ppu'), String(s.pixelsPerUnit)]] as Array<[string, string]>) : []),
    ...(s.border ? ([[t('sprite.border'), borderText(s.border)]] as Array<[string, string]>) : []),
    ...(s.packingTag ? ([[t('sprite.packingTag'), s.packingTag]] as Array<[string, string]>) : [])
  ]
  return (
    <section className="section">
      <span className="label-caps">{t('sprite.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {s.sprites.length > 1 && used < 0.5 && <p className="warn-note">{t('sprite.warn.empty', { percent: Math.round((1 - used) * 100) })}</p>}
      {s.sprites.length > 0 && (
        <details className="sprite-list">
          <summary className="small">{t('sprite.list', { count: s.sprites.length })}</summary>
          <ol>
            {s.sprites.map((r, i) => (
              <li key={`${i}:${r.name}`} className="small">
                <span className="dep-name">{r.name}</span>
                <span className="muted">
                  {r.width}×{r.height}
                  {r.border && ` · ${t('sprite.sliced')}`}
                </span>
              </li>
            ))}
          </ol>
        </details>
      )}
    </section>
  )
}

/** Length, rate and what a clip animates, with what matters for playables. */
function AnimSection({ info }: { info: AnimClipInfo }) {
  const { t } = useTranslation()
  const curves = info.transformCurves.length + info.floatCurves.length + info.spriteCurves
  const cells: Array<[string, string]> = [
    [t('anim.length'), `${formatDuration(info.length)} · ${Math.round(info.length * info.sampleRate)} ${t('anim.frames')}`],
    [t('anim.rate'), `${info.sampleRate} fps`],
    [t('anim.type'), t(`anim.kinds.${info.kind}`)],
    [t('anim.loop'), info.loop ? t('texture.yes') : t('texture.no')],
    [t('anim.objects'), String(info.paths.length)],
    [t('anim.curves'), `${curves} · ${info.keyframes.toLocaleString()} ${t('anim.keys')}`],
    ...(info.events ? ([[t('anim.events'), String(info.events)]] as Array<[string, string]>) : [])
  ]
  // Keys on (nearly) every frame: baked, not keyframed by hand; Keyframe Reduction would shrink it.
  const transformKeys = info.transformCurves.reduce((s, c) => s + c.keys.length, 0)
  const baked = info.transformCurves.length > 0 && info.length > 0.5 && transformKeys / (info.transformCurves.length * info.length * info.sampleRate) > 0.8
  return (
    <section className="section">
      <span className="label-caps">{t('anim.title')}</span>
      <dl className="stat-grid">
        {cells.map(([k, v]) => (
          <div key={k} className={statClass(v)}>
            <dt>{k}</dt>
            <dd>{v}</dd>
          </div>
        ))}
      </dl>
      {baked && <p className="warn-note">{t('anim.warn.baked')}</p>}
      {info.legacy && <p className="note">{t('anim.warn.legacy')}</p>}
    </section>
  )
}

/** Stat cells worth showing: counts that are zero (no bones, no clips) or trivial (one mesh) are left out. */
function statCells(stats: ModelStats): Array<[string, string | number]> {
  const textureBytes = stats.textures.reduce((s, x) => s + x.bytes, 0)
  const cells: Array<[string, string | number | false]> = [
    // Nothing to show for what is not there (an effect has no triangles of its own).
    ['triangles', stats.triangles > 0 && stats.triangles.toLocaleString()],
    ['vertices', stats.vertices > 0 && stats.vertices.toLocaleString()],
    ['meshes', stats.meshes > 1 && stats.meshes],
    ['materials', stats.materials > 0 && stats.materials],
    ['bones', stats.bones > 0 && stats.bones],
    ['clips', stats.animations.length > 0 && stats.animations.length],
    ['file', stats.fileBytes > 0 && formatBytes(stats.fileBytes)],
    ['textures', textureBytes > 0 && formatBytes(textureBytes)]
  ]
  return cells.filter((c): c is [string, string | number] => c[1] !== false)
}

function MaterialRows({ m }: { m: MaterialRef }) {
  const { t } = useTranslation()
  return (
    <li>
      <div className="dep-row" title={m.path}>
        <Dot tone={m.unsure ? 'info' : m.missing ? 'warn' : 'ok'} />
        <span className="dep-type">MAT</span>
        <span className="dep-name">{m.name}</span>
        <span className="dep-meta">{m.unsure ? t('deps.checking') : m.missing ? t('deps.missing') : ''}</span>
      </div>
      {/* Built-in and package shaders come with Unity: nothing to do, so not listed. */}
      {m.shader?.kind === 'project' && (
        <div className="dep-row dep-child" title={m.shader.path ?? m.shader.guid}>
          <Dot tone={m.shader.kind === 'project' ? 'info' : 'ok'} />
          <span className="dep-type">SHD</span>
          <span className="dep-name">{m.shader.label}</span>
          <span className="dep-meta">{t('deps.shader.project')}</span>
        </div>
      )}
      <ul>
        {mergeSlots(m.textures).map((tex) => (
          <TextureRow key={`${tex.prop}:${tex.guid}`} tex={tex} />
        ))}
      </ul>
    </li>
  )
}

/** One row per texture: a file used by several slots (_MainTex, _OcclusionMap) is listed once. */
function mergeSlots(textures: readonly TextureRef[]): TextureRef[] {
  const byFile = new Map<string, TextureRef>()
  for (const tex of textures) {
    const key = tex.path ?? `${tex.prop}:${tex.name}`
    const seen = byFile.get(key)
    if (!seen) byFile.set(key, tex)
    else if (tex.prop && !seen.prop?.split(', ').includes(tex.prop)) byFile.set(key, { ...seen, prop: seen.prop ? `${seen.prop}, ${tex.prop}` : tex.prop })
  }
  return [...byFile.values()]
}

const PART_TYPES: Record<PartRef['kind'], string> = { model: 'FBX', prefab: 'PFB', script: 'CS', other: 'FILE' }

/** A model, nested prefab, script or other file a prefab uses. */
function PartRow({ part }: { part: PartRef }) {
  const { t } = useTranslation()
  const tone = part.unsure ? 'info' : part.missing ? 'warn' : part.kind === 'script' ? 'info' : 'ok'
  return (
    <li className="dep-row" title={part.path ?? part.guid}>
      <Dot tone={tone} />
      <span className="dep-type">{part.kind === 'model' && part.path && !/\.fbx$/i.test(part.path) ? (part.path.split('.').pop() ?? '').toUpperCase() : PART_TYPES[part.kind]}</span>
      <span className="dep-name">
        {part.name}
        {part.uses > 1 && <span className="muted"> ×{part.uses}</span>}
      </span>
      <span className="dep-meta">{part.unsure ? t('deps.checking') : part.missing ? t('deps.missing') : part.kind === 'script' ? t('deps.script') : formatBytes(part.size)}</span>
    </li>
  )
}

function TextureRow({ tex }: { tex: TextureRef }) {
  const { t } = useTranslation()
  const tone = tex.unsure ? 'info' : tex.missing ? 'warn' : tex.matchedBy === 'name' ? 'info' : 'ok'
  return (
    <li className="dep-row dep-child" title={tex.path}>
      <Dot tone={tone} />
      <span className="dep-type">TEX</span>
      <span className="dep-name">
        {tex.prop && tex.prop !== tex.name && <span className="muted">{tex.prop} · </span>}
        {tex.missing ? tex.name : tex.path!.split('/').pop()}
      </span>
      <span className="dep-meta">{tex.unsure ? t('deps.checking') : tex.missing ? t('deps.missing') : tex.matchedBy === 'name' ? t('deps.byName') : formatBytes(tex.size)}</span>
    </li>
  )
}

export function AssetDetails({ asset, handler, live, onThumb, onReport, inPack, exporting, onTogglePack, onExport, onImportToUnity, copies, onSelectCopy, excludedFolders, onPrefabFiles, onOpenLocation, onViewerLoading, onModelStats, playClips, models, viewerResizer, collectionActions }: Props) {
  const { t } = useTranslation()
  const budget = useBudget()
  const [deps, setDeps] = useState<Deps>({ status: 'idle' })
  const [stats, setStats] = useState<ModelStats>()
  /** For a texture: what the image viewer found. */
  const [imageInfo, setImageInfo] = useState<ImageInfo>()
  /** For a material: its colour / surface, and what its textures weigh. */
  const [look, setLook] = useState<MaterialLook>()
  /** For an animation clip: what it animates. */
  const [animInfo, setAnimInfo] = useState<AnimClipInfo>()
  /** For an audio clip: what the player decoded. */
  const [audioInfo, setAudioInfo] = useState<DecodedAudio>()
  const [fontInfo, setFontInfo] = useState<LoadedFont>()
  const [tmpInfo, setTmpInfo] = useState<LoadedTmpFont>()
  const [materialStats, setMaterialStats] = useState<StatsSummary>()
  const [progress, setProgress] = useState<{ done: number; total: number }>()
  const [copied, setCopied] = useState(false)
  const [prefab, setPrefab] = useState<PrefabSearch>({ status: 'idle' })
  /** A suggested .mat (next to the FBX) the user chose to try. */
  const [picked, setPicked] = useState<{ material: MaterialRef; files: string[] }>()
  /** Other branches / repos with this exact file, from the catalog (empty until they are indexed). */
  const [elsewhere, setElsewhere] = useState<CatalogLocation[]>([])
  const [prefabProgress, setPrefabProgress] = useState<{ done: number; total: number }>()
  const repoPath = asset && live ? live.root + asset.path : undefined
  /** Bumped when the selection changes, so a search started for another model is ignored. */
  const searchToken = useRef(0)

  useEffect(() => {
    setElsewhere([])
    if (!live || !asset) return
    let cancelled = false
    void window.api.catalog.whereIs(asset.sha).then((r) => {
      if (cancelled || !r.ok) return
      // Leave out the branch being viewed.
      const others = r.data
        .map((l) => (l.repo === live.repo ? { ...l, branches: l.branches.filter((b) => b !== live.branch) } : l))
        .filter((l) => l.branches.length)
      setElsewhere(others)
    })
    return () => {
      cancelled = true
    }
  }, [live?.repo, live?.branch, asset?.sha])

  useEffect(() => window.api.assets.onIndexProgress(setProgress), [])
  useEffect(() => window.api.assets.onPrefabProgress(setPrefabProgress), [])

  useEffect(() => {
    searchToken.current++
    setStats(undefined)
    setImageInfo(undefined)
    setLook(undefined)
    setAudioInfo(undefined)
    setFontInfo(undefined)
    setTmpInfo(undefined)
    setAnimInfo(undefined)
    setMaterialStats(undefined)
    setCopied(false)
    setProgress(undefined)
    setPrefab({ status: 'idle' })
    setPicked(undefined)
    if (!live || !repoPath || !handler?.resolvesDependencies) {
      setDeps({ status: 'idle' })
      return
    }
    let cancelled = false
    setDeps({ status: 'loading' })
    // Materials found on prefabs in an earlier search come back without downloading anything.
    // Only says whether remembered prefabs use it; loading their materials is up to the user.
    if (handler.materialsFromPrefabs)
      void window.api.assets.findPrefabMaterials(live.repo, live.branch, repoPath, live.root, 'known', excludedFolders).then((result) => {
      if (cancelled || !result.ok || !result.data.prefabs.length) return
      setPrefab({ status: 'known', result: result.data })
    })
    // Shown in steps: what is known at once, then nearby files and the whole branch side by side.
    // The whole-branch answer reads every .meta in one download and often arrives first (about 10 s on a
    // large repo, while nearby lookups take 20-30 s), so it no longer waits for the nearby one; once it is
    // there, a later nearby answer is dropped.
    let complete = false
    const load = (mode: 'instant' | 'quick' | 'full'): Promise<void> =>
      window.api.assets.resolve(live.repo, live.branch, repoPath, live.root, mode).then(
        (result) => {
          if (cancelled || complete) return
          try {
            const report = unwrap(result)
            if (!report.partial) complete = true
            setDeps({ status: 'ready', report })
            onReport(repoPath, report)
            if (mode === 'instant' && report.partial) {
              void load('quick')
              void load('full')
            }
          } catch (error) {
            // A failed later step keeps the earlier answer on screen.
            if (mode === 'instant') setDeps({ status: 'error', error })
          }
        },
        (error: unknown) => {
          if (!cancelled && mode === 'instant') setDeps({ status: 'error', error: new RemoteError({ code: 'unknown', message: String(error) }) })
        }
      )
    void load('instant')
    return () => {
      cancelled = true
    }
  }, [live?.repo, live?.branch, live?.root, repoPath, handler])

  if (!asset) {
    return (
      <aside className="details" aria-label={t('details.label')}>
        <p className="empty">{t('details.empty')}</p>
      </aside>
    )
  }

  const report = deps.status === 'ready' ? deps.report : undefined
  const prefabResult = prefab.status === 'ready' ? prefab.result : undefined
  const prefabMaterial = prefabResult?.prefabs[0]?.materials.find((m) => !m.missing) ?? picked?.material
  const suggestions = !prefabResult?.prefabs.length ? (report?.suggested ?? []) : []

  const pick = (s: { material: MaterialRef; files: string[] } | undefined) => {
    setPicked(s)
    if (repoPath) onPrefabFiles(repoPath, s?.files ?? [])
  }

  const searchPrefabs = async (scope: 'cached' | 'near' | 'all') => {
    if (!live || !repoPath) return
    const token = ++searchToken.current
    setPrefab({ status: 'running', scope })
    setPrefabProgress(undefined)
    try {
      const result = unwrap(await window.api.assets.findPrefabMaterials(live.repo, live.branch, repoPath, live.root, scope, excludedFolders))
      if (token !== searchToken.current) return
      setPrefab({ status: 'ready', result })
      onPrefabFiles(repoPath, result.files)
    } catch (error) {
      if (token === searchToken.current) setPrefab({ status: 'error', error })
    }
  }
  const issues = stats && budget ? checkPlayableBudget(stats, budget) : []
  const copyPath = async () => {
    try {
      await navigator.clipboard.writeText(asset.path)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <aside className="details" aria-label={t('details.label')}>
      <Suspense fallback={<div className="viewer" />}>
      {live && repoPath && handler?.preview.kind === 'three' ? (
        <ModelViewer
          key={repoPath}
          live={live}
          path={repoPath}
          report={report}
          prefabMaterial={prefabMaterial}
          autoPlay={playClips}
          onStats={(s) => {
            setStats(s)
            onModelStats?.(asset.sha, summarizeStats(s, 'viewer'))
          }}
          onThumb={(url) => onThumb(asset.sha, url)}
          onLoading={onViewerLoading}
        />
      ) : live && repoPath && handler?.preview.kind === 'prefab' ? (
        <PrefabViewer
          key={repoPath}
          live={live}
          path={repoPath}
          report={report}
          onStats={(s) => {
            setStats(s)
            const summary = summarizeStats(s, report?.partial ? 'thumb' : 'viewer')
            if (report?.particles) void particleStats(report, summary).then((withParticles) => onModelStats?.(asset.sha, withParticles), () => onModelStats?.(asset.sha, summary))
            else onModelStats?.(asset.sha, summary)
          }}
          onThumb={(url) => onThumb(asset.sha, url)}
          onLoading={onViewerLoading}
        />
      ) : live && repoPath && handler?.preview.kind === 'animation' ? (
        <AnimViewer key={repoPath} live={live} path={repoPath} report={report} models={models} onInfo={setAnimInfo} onThumb={(url) => onThumb(asset.sha, url)} onLoading={onViewerLoading} />
      ) : live && repoPath && handler?.preview.kind === 'material' ? (
        <MaterialViewer
          key={repoPath}
          live={live}
          path={repoPath}
          report={report}
          onLook={setLook}
          onStats={(s) => {
            setMaterialStats(s)
            onModelStats?.(asset.sha, s)
          }}
          onThumb={(url) => onThumb(asset.sha, url)}
          onLoading={onViewerLoading}
        />
      ) : live && repoPath && handler?.preview.kind === 'audio' ? (
        <AudioViewer key={repoPath} live={live} path={repoPath} onInfo={setAudioInfo} onStats={(s) => onModelStats?.(asset.sha, s)} onLoading={onViewerLoading} />
      ) : live && repoPath && handler?.preview.kind === 'tmpfont' ? (
        <TmpFontViewer key={repoPath} live={live} path={repoPath} onInfo={setTmpInfo} onStats={(s) => onModelStats?.(asset.sha, s)} onLoading={onViewerLoading} />
      ) : live && repoPath && handler?.preview.kind === 'font' ? (
        <FontViewer key={repoPath} live={live} path={repoPath} onInfo={setFontInfo} onStats={(s) => onModelStats?.(asset.sha, s)} onLoading={onViewerLoading} />
      ) : live && repoPath && handler?.preview.kind === 'image' ? (
        <ImageViewer key={repoPath} live={live} path={repoPath} onInfo={setImageInfo} onStats={(s) => onModelStats?.(asset.sha, s)} onLoading={onViewerLoading} />
      ) : (
        <div className="preview-box">
          <CubeIcon size={72} />
          <span>{t('viewer.needsRepo')}</span>
        </div>
      )}
      </Suspense>
      {viewerResizer}

      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        <h2>{asset.name}</h2>
        {live && handler?.delivery === 'file' && <DownloadButton repo={live.repo} branch={live.branch} path={live.root + asset.path} />}
        {live && handler?.resolvesDependencies && handler.delivery !== 'file' && (
          <div className="action-row">
            <button type="button" className="btn btn-primary icon-btn labeled" title={t('export.exportOne')} aria-label={t('export.exportOne')} onClick={onExport} disabled={exporting}>
              <ExportIcon />
              <span>{t('actions.export')}</span>
            </button>
            <button type="button" className="btn icon-btn labeled" aria-pressed={inPack} title={inPack ? t('export.inPack') : t('export.addToPack')} aria-label={inPack ? t('export.inPack') : t('export.addToPack')} onClick={onTogglePack} disabled={exporting}>
              <ChestIcon filled={inPack} />
              <span>{inPack ? t('actions.inChest') : t('actions.chest')}</span>
            </button>
            <UnityButton compact disabled={exporting} onImport={onImportToUnity} />
            {(handler.id === 'fbx' || handler.id === 'prefab') && <ModelBundleButtons kind={handler.id} live={live} path={live.root + asset.path} report={report} prefabMaterial={prefabMaterial} />}
            {collectionActions}
          </div>
        )}
        {!(live && handler?.resolvesDependencies && handler.delivery !== 'file') && collectionActions}
      </div>

      {stats && (statCells(stats).length > 0 || issues.length > 0) && (
        <Section id="stats" title={t('stats.title')} summary={issues.length > 0 ? <span className="warn-text">{t('details.issues', { count: issues.length })}</span> : undefined}>
          <dl className="stat-grid">
            {statCells(stats).map(([key, value]) => (
              <div key={key}>
                <dt>{t(`stats.${key}`)}</dt>
                <dd>{value}</dd>
              </div>
            ))}
          </dl>
          {issues.length > 0 && (
            <ul className="warn-note warn-list">
              {issues.map((i) => (
                <li key={i.key}>
                  {t(`budget.${i.key}`, {
                    value: i.key.endsWith('Bytes') ? formatBytes(i.value) : i.value.toLocaleString(),
                    limit: i.key.endsWith('Bytes') ? formatBytes(i.limit) : i.limit.toLocaleString()
                  })}
                </li>
              ))}
            </ul>
          )}
        </Section>
      )}

      {imageInfo && <TextureSection info={imageInfo} />}
      {imageInfo?.sprite && <SpriteSection info={imageInfo} />}
      {audioInfo && <AudioSection info={audioInfo} />}
      {fontInfo && <FontSection font={fontInfo} />}
      {tmpInfo && <TmpFontSection font={tmpInfo} />}
      {report?.particles && <ParticleSection report={report} />}
      {animInfo && <AnimSection info={animInfo} />}
      {handler?.preview.kind === 'material' && <MaterialSection material={report?.materials[0]} look={look} stats={materialStats} />}

      {/* A texture is just itself: nothing to list. */}
      {handler?.resolvesDependencies && handler.usesGuids && (
        <Section id="deps" defaultOpen={false} title={t('details.deps')} summary={report ? <>{t('details.depsSummary', { count: report.files.filter((f) => !f.endsWith('.meta')).length })}{report.warnings.length > 0 && <span className="warn-text"> · {t('details.warnings', { count: report.warnings.length })}</span>}</> : undefined}>
          {!live && <p className="note">{t('deps.needsRepo')}</p>}
          {deps.status === 'loading' && (
            <div className="note">
              {progress && progress.total > 0 && progress.done < progress.total ? (
                <>
                  {t('deps.indexing', { done: progress.done.toLocaleString(), total: progress.total.toLocaleString() })}
                  <progress className="progress" max={progress.total} value={progress.done} />
                </>
              ) : (
                t('deps.resolving')
              )}
            </div>
          )}
          {deps.status === 'error' && <ErrorNote error={deps.error} />}
          {report && (
            <>
              {report.warnings
                // Missing files already say "not in repo" on their own row.
                .filter((w) => w !== 'missingTexture' && w !== 'missingMaterial' && w !== 'missingModel')
                .filter((w) => w !== 'textureByName' || report.embeddedTextures.some((x) => !x.missing))
                .map((w) => (
                  <p key={w} className={w === 'textureByName' ? 'note' : 'warn-note'}>
                    {t(`deps.warning.${w}`)}
                  </p>
                ))}
              <ul className="dep-list">
                {report.parts?.map((p) => (
                  <PartRow key={p.guid} part={p} />
                ))}
                {report.materials.map((m) => (
                  <MaterialRows key={m.guid ?? m.name} m={m} />
                ))}
                {report.embeddedTextures.length > 0 && (
                  <li>
                    <div className="dep-row">
                      <Dot tone="info" />
                      <span className="dep-type">FBX</span>
                      <span className="dep-name">{t('deps.embedded')}</span>
                      <span className="dep-meta" />
                    </div>
                    <ul>
                      {report.embeddedTextures.map((tex) => (
                        <TextureRow key={tex.name} tex={tex} />
                      ))}
                    </ul>
                  </li>
                )}
              </ul>
              {suggestions.length > 0 && (
                <div className="section" style={{ marginTop: 4 }}>
                  <p className="note">{t('deps.suggestedHint')}</p>
                  <ul className="dep-list">
                    {suggestions.map((s) => (
                      <li key={s.material.path}>
                        <div className="dep-row" title={s.material.path}>
                          <Dot tone="info" />
                          <span className="dep-type">MAT</span>
                          <span className="dep-name">{s.material.name}</span>
                          <button
                            type="button"
                            className="btn btn-small"
                            aria-pressed={picked?.material.path === s.material.path}
                            onClick={() => pick(picked?.material.path === s.material.path ? undefined : s)}
                          >
                            {picked?.material.path === s.material.path ? t('deps.suggestedOn') : t('deps.suggestedUse')}
                          </button>
                        </div>
                        <ul className="dep-indent">
                          {s.material.textures.map((tex) => (
                            <TextureRow key={`${tex.prop}:${tex.guid}`} tex={tex} />
                          ))}
                        </ul>
                      </li>
                    ))}
                  </ul>
                </div>
              )}
            </>
          )}
        </Section>
      )}

      {live && report && handler?.materialsFromPrefabs && (
        <Section id="prefabUses" defaultOpen={false} title={t('prefab.title')}>
          {prefab.status === 'known' && (
            <>
              <p className="muted small" style={{ margin: 0 }}>
                {t('prefab.known', { count: prefab.result.prefabs.length, names: prefab.result.prefabs.slice(0, 3).map((p) => p.path.split('/').pop()).join(', ') })}
              </p>
              <div>
                <button type="button" className="btn" onClick={() => void searchPrefabs('cached')}>
                  {t('prefab.load')}
                </button>
              </div>
            </>
          )}
          {prefab.status === 'idle' && (
            <>
              <p className="muted small" style={{ margin: 0 }}>
                {t('prefab.hint')}
              </p>
              <div>
                <button type="button" className="btn" onClick={() => void searchPrefabs('near')}>
                  {t('prefab.search')}
                </button>
              </div>
            </>
          )}
          {prefab.status === 'running' && (
            <div className="note">
              {t(prefab.scope === 'all' ? 'prefab.scanningAll' : prefab.scope === 'cached' ? 'prefab.loading' : 'prefab.scanning')}
              {prefabProgress && prefabProgress.total > 0 && (
                <>
                  {' '}
                  {formatBytes(prefabProgress.done)} / {formatBytes(prefabProgress.total)}
                  <progress className="progress" max={prefabProgress.total} value={prefabProgress.done} />
                </>
              )}
            </div>
          )}
          {prefab.status === 'error' && <ErrorNote error={prefab.error} />}
          {prefabResult && (
            <>
              {prefabResult.prefabs.length === 0 ? (
                <p className="note">{t('prefab.none', { count: prefabResult.scanned })}</p>
              ) : (
                <>
                  <p className="note">{t('prefab.found', { count: prefabResult.prefabs.length })}</p>
                  <ul className="dep-list">
                    {prefabResult.prefabs.map((p) => (
                      <li key={p.path}>
                        <div className="dep-row" title={p.path}>
                          <Dot tone="ok" />
                          <span className="dep-type">PFB</span>
                          <span className="dep-name">{p.path.split('/').pop()}</span>
                          <span className="dep-meta" />
                        </div>
                        <ul className="dep-indent">
                          {p.materials.map((m) => (
                            <MaterialRows key={m.guid ?? m.name} m={m} />
                          ))}
                        </ul>
                      </li>
                    ))}
                  </ul>
                  {/* Only worth saying when there was a choice. */}
                  {prefabMaterial && prefabResult.prefabs.flatMap((p) => p.materials).length > 1 && (
                    <p className="muted small" style={{ margin: 0 }}>
                      {t('prefab.applied', { name: prefabMaterial.name })}
                    </p>
                  )}
                </>
              )}
              {prefabResult.remaining > 0 && (
                <div>
                  <button type="button" className={prefabResult.prefabs.length ? 'btn btn-small' : 'btn'} onClick={() => void searchPrefabs('all')}>
                    {t('prefab.scanAll', { count: prefabResult.remaining, size: formatBytes(prefabResult.remainingBytes) })}
                  </button>
                </div>
              )}
            </>
          )}
        </Section>
      )}

      {copies && copies.length > 1 && (
        <Section id="copies" defaultOpen={false} title={t('details.copies', { count: copies.length })}>
          <ul className="copy-list">
            {copies.map((c) => (
              <li key={c.id}>
                <button type="button" className="copy-item" aria-current={c.id === asset.id} onClick={() => onSelectCopy(c.id)} title={c.path}>
                  {shortPath(c.path)}
                </button>
              </li>
            ))}
          </ul>
        </Section>
      )}

      {elsewhere.length > 0 && (
        <section className="section">
          <span className="label-caps">{t('catalog.alsoIn', { count: new Set(elsewhere.flatMap((l) => l.branches.map((b) => `${l.repo}@${b}`))).size })}</span>
          <LocationList locations={elsewhere} showRepo={elsewhere.some((l) => l.repo !== live?.repo)} onOpen={onOpenLocation} />
        </section>
      )}

      <div className="path-row">
        <span className="mono small" title={asset.path}>
          {shortPath(asset.path)}
        </span>
        <button type="button" className="btn btn-small icon-btn-sm labeled" title={copied ? t('details.copied') : t('details.copyPath')} aria-label={copied ? t('details.copied') : t('details.copyPath')} onClick={copyPath}>
          {copied ? <CheckIcon /> : <CopyIcon />}
          <span>{copied ? t('actions.copied') : t('actions.copyPath')}</span>
        </button>
        {live && (
          <button type="button" className="btn btn-small icon-btn-sm labeled" title={t('details.openGitHub')} aria-label={t('details.openGitHub')} onClick={() => void window.api.openExternal(githubUrl(live.repo, live.branch, live.root + asset.path))}>
            <ExternalIcon />
            <span>GitHub</span>
          </button>
        )}
        {live && <CopyLinkButton iconOnly className="btn btn-small icon-btn-sm labeled" link={{ repo: live.repo, branch: live.branch, path: live.root + asset.path }} />}
      </div>
    </aside>
  )
}
