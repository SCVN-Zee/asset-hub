import { Fragment, lazy, Suspense, useEffect, useRef, useState, type ReactNode } from 'react'
import { useTranslation } from 'react-i18next'
import type { RunningUnity } from '@shared/ipc'
import { UnityControl, useUnityEditors } from './UnityButton'
import { Section } from './Section'
import { summarizeStats, summaryIssues } from '../playable'
import { useBudget } from '../budget'
import { formatBytes, shortPath } from '../format'
import type { AssetTypeHandler } from '../handlers'
import { parse } from '../parse'
import type { Asset, CatalogLocation, DependencyReport, AnimClipInfo, MaterialLook, MaterialRef, ModelStats, SpriteBorder, PartRef, ParticleTotals, PrefabMaterialsResult, StatsSummary, TextureRef } from '@shared/types'
import { assetLink } from '@shared/deeplink'
import type { Collection, CollectionItem } from '@shared/collections'
import { RemoteError, unwrap } from '../remoteSource'
import { githubUrl, LocationList } from './CatalogPanel'
import { ErrorNote } from './ErrorNote'
import { CheckIcon, HalfCircleIcon, ExportListIcon, CopyIcon, CubeIcon, DownloadIcon, ExportIcon, ExternalIcon, FolderIcon, LinkIcon } from './Icons'
import type { LiveContext } from './ModelViewer'
import type { ImageInfo } from './ImageViewer'
import type { FxBackdrop } from './PrefabViewer'
import { AudioViewer } from './AudioViewer'
import { audioStats, formatDuration, type DecodedAudio } from '../viewer/audio'
import { FontViewer } from './FontViewer'
import { TmpFontViewer } from './TmpFontViewer'
import type { LoadedTmpFont } from '../viewer/tmpFont'
import { particleStats } from '../viewer/particleThumb'
import { fontStats, type LoadedFont } from '../viewer/font'
import { DownloadButton } from './DownloadButton'
import { BundleNotice, useModelBundle } from './ModelBundleButtons'
import { CollectionMenuItems, FavoriteButton } from './Collections'
import { BudgetLine, CalibrationStrip, CopyStatus, FactsGrid, MenuItem, MenuLabel, MenuSep, MoreMenu, useCopy, type Fact } from './InspectorParts'

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
  /** False while the export strip is showing: its Export is then the only orange button. */
  primary?: boolean
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
  /** Favourite star and collections menu for this asset. */
  collections?: { list: Collection[]; item: CollectionItem; onToggle(id: string): void; onCreate(id: string, name: string): void }
  /** Goes to the folder that holds this asset (left out when the grid already shows it). */
  onShowFolder?(): void
}

type PrefabSearch =
  | { status: 'idle' }
  /** Remembered prefabs use this model; their materials load when asked. */
  | { status: 'known'; result: PrefabMaterialsResult }
  | { status: 'running'; scope: 'cached' | 'near' | 'all' } | { status: 'ready'; result: PrefabMaterialsResult } | { status: 'error'; error: unknown }

type Deps = { status: 'idle' } | { status: 'loading' } | { status: 'error'; error: unknown } | { status: 'ready'; report: DependencyReport }

type Translate = (key: string, options?: Record<string, unknown>) => string

/** A fact of a detail section; `top` ones also stand under the name. */
type DetailFact = Fact & { top?: boolean }

function Dot({ tone }: { tone: 'ok' | 'warn' | 'info' }) {
  return <span className={`dot dot-${tone}`} aria-hidden="true" />
}

/** The notes of a detail section (what is not a budget verdict, which has its own line). */
function NoteList({ notes }: { notes: string[] }) {
  if (!notes.length) return null
  return (
    <ul className="warn-note warn-list">
      {notes.map((w) => (
        <li key={w}>{w}</li>
      ))}
    </ul>
  )
}

/** Folded summary of a detail section: how many notes it holds, if any. */
function notesSummary(notes: string[], t: Translate): ReactNode {
  return notes.length ? <span className="warn-text">{t('details.warnings', { count: notes.length })}</span> : undefined
}

const TEXTURE_TYPES: Record<number, string> = { 0: 'default', 1: 'normal', 2: 'gui', 4: 'cookie', 6: 'lightmap', 7: 'cursor', 8: 'sprite', 10: 'single' }
const COMPRESSION: Record<number, string> = { 0: 'none', 1: 'normal', 2: 'high', 3: 'low' }

function textureFacts(info: ImageInfo, t: Translate): DetailFact[] {
  const imp = info.import
  return [
    { label: t('texture.size'), value: `${info.width} × ${info.height}`, top: true },
    { label: t('texture.file'), value: `${formatBytes(info.bytes)} · ${info.format}`, top: true },
    // As Unity imports it: alpha made from brightness, or dropped, says so.
    { label: t('texture.alpha'), value: imp?.alphaSource === 'gray' ? t('texture.alphaGray') : imp?.alphaSource === 'none' ? t('texture.alphaNone') : info.hasAlpha ? t('texture.yes') : t('texture.no'), top: true },
    ...(imp
      ? [
          { label: t('texture.type'), value: t(`texture.types.${TEXTURE_TYPES[imp.textureType ?? 0] ?? 'default'}`) },
          { label: t('texture.built'), value: `${info.built} px${imp.maxTextureSize ? ` (max ${imp.maxTextureSize})` : ''}` },
          { label: t('texture.compression'), value: t(`texture.compressions.${COMPRESSION[imp.compression ?? 1] ?? 'normal'}`) },
          { label: t('texture.mipmaps'), value: imp.mipmaps ? t('texture.yes') : t('texture.no') },
          { label: t('texture.srgb'), value: imp.sRGB === false ? t('texture.no') : t('texture.yes') }
        ]
      : [])
  ]
}

/** Size, format and Unity import settings of a texture, with what matters for playables. */
function TextureSection({ info }: { info: ImageInfo }) {
  const { t } = useTranslation()
  const imp = info.import
  const notes: string[] = []
  if (imp?.compression === 0) notes.push(t('texture.warn.uncompressed'))
  if (imp?.textureType === 8 && imp.mipmaps) notes.push(t('texture.warn.spriteMips'))
  return (
    <Section id="texture" defaultOpen={false} title={t('texture.title')} summary={notesSummary(notes, t)}>
      <FactsGrid facts={textureFacts(info, t)} />
      {imp && imp.overrides.length > 0 && (
        <p className="muted small" style={{ margin: 0 }}>
          {t('texture.overrides', { list: imp.overrides.map((o) => `${o.target}${o.maxTextureSize ? ` ${o.maxTextureSize}` : ''}`).join(', ') })}
        </p>
      )}
      {!imp && <p className="muted small" style={{ margin: 0 }}>{t('texture.noMeta')}</p>}
      <NoteList notes={notes} />
    </Section>
  )
}

const LOAD_TYPES: Record<number, string> = { 0: 'decompress', 1: 'compressed', 2: 'streaming' }
const AUDIO_FORMATS: Record<number, string> = { 0: 'PCM', 1: 'Vorbis', 2: 'ADPCM', 3: 'MP3', 7: 'AAC' }

function audioFacts(info: DecodedAudio, t: Translate): DetailFact[] {
  const imp = info.import
  const s = info.effective
  const stats = audioStats(info)
  const mono = imp?.forceToMono && info.channels > 1
  const rate = s?.sampleRateSetting === 2 && s.sampleRateOverride && s.sampleRateOverride < info.sampleRate ? s.sampleRateOverride : undefined
  const format = s?.compressionFormat ?? 1
  return [
    { label: t('audio.length'), value: formatDuration(info.duration), top: true },
    { label: t('audio.channels'), value: `${t(info.channels === 1 ? 'audio.mono' : 'audio.stereo')}${mono ? ` → ${t('audio.mono')}` : ''}`, top: true },
    { label: t('audio.rate'), value: `${(info.sampleRate / 1000).toLocaleString()} kHz${rate ? ` → ${(rate / 1000).toLocaleString()} kHz` : ''}` },
    { label: t('audio.file'), value: `${formatBytes(info.bytes)} · ${info.format}`, top: true },
    ...(imp
      ? [
          { label: t('audio.loadType'), value: t(`audio.loadTypes.${LOAD_TYPES[s?.loadType ?? 0] ?? 'decompress'}`) },
          { label: t('audio.compression'), value: `${AUDIO_FORMATS[format] ?? format}${(format === 1 || format === 3) && s?.quality !== undefined ? ` ${Math.round(s.quality * 100)}%` : ''}` }
        ]
      : []),
    { label: t('audio.built'), value: `≈ ${formatBytes(stats.fileBytes)}` }
  ]
}

/** Length, channels, rate and Unity import settings of a clip, with what matters for playables. */
function AudioSection({ info }: { info: DecodedAudio }) {
  const { t } = useTranslation()
  const imp = info.import
  const s = info.effective
  const format = s?.compressionFormat ?? 1
  const notes: string[] = []
  if (format === 0 && info.duration > 5) notes.push(t('audio.warn.pcm'))
  if ((s?.loadType ?? 0) === 0 && info.duration > 10) notes.push(t('audio.warn.decompress'))
  if (info.channels > 1 && !imp?.forceToMono && info.duration < 5) notes.push(t('audio.warn.mono'))
  return (
    <Section id="audio" defaultOpen={false} title={t('audio.title')} summary={notesSummary(notes, t)}>
      <FactsGrid facts={audioFacts(info, t)} />
      {imp && imp.overrides.length > 0 && (
        <p className="muted small" style={{ margin: 0 }}>
          {t('audio.overrides', { list: imp.overrides.map((o) => `${o.target}${o.compressionFormat !== undefined ? ` ${AUDIO_FORMATS[o.compressionFormat] ?? o.compressionFormat}` : ''}`).join(', ') })}
        </p>
      )}
      {!imp && <p className="muted small" style={{ margin: 0 }}>{t('audio.noMeta')}</p>}
      <NoteList notes={notes} />
    </Section>
  )
}

function fontFacts(font: LoadedFont, t: Translate): DetailFact[] {
  const { info } = font
  const imp = font.import
  return [
    { label: t('font.family'), value: info.family || '—' },
    { label: t('font.style'), value: info.style || '—' },
    { label: t('font.glyphCount'), value: info.glyphs.toLocaleString(), top: true },
    { label: t('font.file'), value: `${formatBytes(font.bytes)} · ${info.outlines === 'cff' ? 'OpenType (CFF)' : 'TrueType'}`, top: true },
    ...(imp ? [{ label: t('font.mode'), value: t(`font.modes.${imp.mode}`) }] : []),
    ...(imp?.fallbacks.length ? [{ label: t('font.fallbacks'), value: imp.fallbacks.join(', ') }] : [])
  ]
}

/** The scripts a font covers, as chips: a tick for a full script, a half circle for a partial one. */
function CoverageChips({ coverage }: { coverage: LoadedFont['info']['coverage'] }) {
  const { t } = useTranslation()
  return (
    <div className="chip-row">
      {coverage.map((c) => (
        <span key={c.script} className={`badge${c.full ? '' : ' badge-warn'}`} title={`${c.have.toLocaleString()} / ${c.total.toLocaleString()}`}>
          {c.full ? <CheckIcon size={12} /> : <HalfCircleIcon size={12} />} {t(`font.scripts.${c.script}`)}
        </span>
      ))}
    </div>
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
  const notes: string[] = []
  // A heavy CJK font is the usual cause of an over-budget font: say how to cut it down.
  if (budget && summaryIssues(fontStats(font), budget).length && coverage.some((c) => (c.script === 'korean' || c.script === 'chinese') && c.have > 1000)) notes.push(t('font.warn.subset'))
  if (vi && !vi.full) notes.push(t('font.warn.vietnamese', { chars: info.missingVietnamese }))
  if (imp && imp.mode !== 'dynamic') notes.push(t('font.warn.baked'))
  return (
    <Section id="font" defaultOpen={false} title={t('font.title')} summary={notesSummary(notes, t)}>
      <FactsGrid facts={fontFacts(font, t)} />
      <CoverageChips coverage={coverage} />
      {info.collection && info.collection > 1 && <p className="muted small" style={{ margin: 0 }}>{t('font.collection', { count: info.collection })}</p>}
      <NoteList notes={notes} />
    </Section>
  )
}

function tmpFontFacts(font: LoadedTmpFont, t: Translate): DetailFact[] {
  const { info } = font
  return [
    { label: t('font.family'), value: info.family || '—' },
    { label: t('font.style'), value: info.style || '—' },
    { label: t('tmpFont.characters'), value: info.characterCount.toLocaleString(), top: true },
    { label: t('tmpFont.atlasSize'), value: `${info.atlasWidth}×${info.atlasHeight}`, top: true },
    { label: t('tmpFont.pointSize'), value: info.pointSize ? `${info.pointSize}` : '—' },
    { label: t('tmpFont.mode'), value: t(info.dynamic ? 'tmpFont.dynamic' : 'tmpFont.static'), top: true },
    { label: t('tmpFont.built'), value: `≈ ${formatBytes(info.atlasWidth * info.atlasHeight)}` },
    ...(info.fallbacks.length ? [{ label: t('font.fallbacks'), value: String(info.fallbacks.length) }] : [])
  ]
}

/** What a TextMeshPro font asset holds, and what it costs a playable. */
function TmpFontSection({ font }: { font: LoadedTmpFont }) {
  const { t } = useTranslation()
  const { info } = font
  const { coverage } = info
  const vi = coverage.find((c) => c.script === 'vietnamese')
  const notes: string[] = []
  if (info.dynamic) notes.push(t('tmpFont.warn.dynamic'))
  if (vi && !vi.full) notes.push(t('font.warn.vietnamese', { chars: info.missingVietnamese }))
  return (
    <Section id="tmpfont" defaultOpen={false} title={t('tmpFont.title')} summary={notesSummary(notes, t)}>
      <FactsGrid facts={tmpFontFacts(font, t)} />
      <CoverageChips coverage={coverage} />
      <NoteList notes={notes} />
    </Section>
  )
}

/** Unity's default Max Particles: left as is, it reserves far more than most effects use. */
const DEFAULT_MAX_PARTICLES = 1000

function particleFacts(totals: ParticleTotals, t: Translate): DetailFact[] {
  return [
    { label: t('particles.emitters'), value: totals.emitters.toLocaleString(), top: true },
    { label: t('particles.peak'), value: `≈ ${totals.peak.toLocaleString()}`, top: true },
    { label: t('particles.length'), value: totals.looping ? t('particles.loops') : formatDuration(totals.duration), top: true }
  ]
}

/** An effect's emitters, what they add up to, and what costs most in a playable. */
function ParticleSection({ report, totals }: { report: DependencyReport; totals: ParticleTotals }) {
  const { t } = useTranslation()
  const emitters = report.particles ?? []
  const count = (pred: (e: (typeof emitters)[number]) => boolean) => emitters.filter(pred).length
  const notes: string[] = []
  const checks: Array<[number, string]> = [
    [count((e) => e.modules.includes('collision')), 'particles.warn.collision'],
    [count((e) => e.modules.includes('lights')), 'particles.warn.lights'],
    [count((e) => e.modules.includes('noise')), 'particles.warn.noise'],
    [count((e) => e.modules.includes('trails')), 'particles.warn.trails'],
    [count((e) => e.renderMode === 'mesh' && e.peak > 50), 'particles.warn.meshes'],
    [count((e) => e.prewarm), 'particles.warn.prewarm'],
    [count((e) => e.maxParticles === DEFAULT_MAX_PARTICLES && e.peak < 200), 'particles.warn.maxDefault']
  ]
  for (const [n, key] of checks) if (n) notes.push(t(key, { count: n }))
  const flags = (e: (typeof emitters)[number]) =>
    [e.shape, e.renderMode !== 'billboard' ? t(`particles.modes.${e.renderMode}`) : undefined, e.sheet && e.sheet[0] * e.sheet[1] > 1 ? `${e.sheet[0]}×${e.sheet[1]}` : undefined, ...e.modules.filter((m) => m !== 'textureSheet').map((m) => t(`particles.modules.${m}`))]
      .filter(Boolean)
      .join(' · ')
  return (
    <Section id="particles" defaultOpen={false} title={t('particles.title')} summary={notes.length ? notesSummary(notes, t) : t('particles.summary', { count: emitters.length })}>
      <FactsGrid facts={particleFacts(totals, t)} />
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
      <NoteList notes={notes} />
    </Section>
  )
}

const SURFACES = { opaque: 'opaque', cutout: 'cutout', transparent: 'transparent' } as const
const toHex = (v: number) => Math.round(Math.min(1, Math.max(0, v)) * 255).toString(16).padStart(2, '0')

function materialFacts(material: MaterialRef | undefined, look: MaterialLook | undefined, stats: StatsSummary | undefined, t: Translate): DetailFact[] {
  const color = look?.color && `#${toHex(look.color.r)}${toHex(look.color.g)}${toHex(look.color.b)}`
  return [
    ...(material?.shader ? [{ label: t('material.shader'), value: material.shader.label }] : []),
    ...(look
      ? [
          { label: t('material.surface'), value: t(`material.surfaces.${SURFACES[look.surface]}`) + (look.surface === 'transparent' && look.color && look.color.a < 1 ? ` · ${Math.round(look.color.a * 100)}%` : ''), top: true },
          ...(color
            ? [
                {
                  label: t('material.color'),
                  value: (
                    <span className="swatch-cell">
                      <span className="swatch" style={{ background: color }} aria-hidden="true" />
                      {color.toUpperCase()}
                    </span>
                  ),
                  top: true
                }
              ]
            : [])
        ]
      : []),
    ...(stats
      ? [
          { label: t('material.textures'), value: stats.textureCount ? `${stats.textureCount} · ${formatBytes(stats.textureBytes)}` : t('material.noTextures'), top: true },
          ...(stats.maxTextureSize ? [{ label: t('material.largest'), value: `${stats.maxTextureSize} px` }] : [])
        ]
      : [])
  ]
}

/** Shader, surface, colour and texture weight of a material. */
function MaterialSection({ facts }: { facts: DetailFact[] }) {
  const { t } = useTranslation()
  if (!facts.length) return null
  return (
    <Section id="material" defaultOpen={false} title={t('material.title')}>
      <FactsGrid facts={facts} />
    </Section>
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
  const facts: Fact[] = [
    { label: t('sprite.mode'), value: t(`sprite.modes.${SPRITE_MODES[s.mode] ?? 'single'}`) },
    ...(s.sprites.length ? [{ label: t('sprite.count'), value: String(s.sprites.length) }, { label: t('sprite.used'), value: `${Math.round(used * 100)}%` }] : []),
    ...(s.pixelsPerUnit ? [{ label: t('sprite.ppu'), value: String(s.pixelsPerUnit) }] : []),
    ...(s.border ? [{ label: t('sprite.border'), value: borderText(s.border) }] : []),
    ...(s.packingTag ? [{ label: t('sprite.packingTag'), value: s.packingTag }] : [])
  ]
  const sparse = s.sprites.length > 1 && used < 0.5
  return (
    <Section id="sprite" defaultOpen={false} title={t('sprite.title')} summary={sparse ? <span className="warn-text">{t('details.warnings', { count: 1 })}</span> : undefined}>
      <FactsGrid facts={facts} />
      {sparse && <p className="warn-note">{t('sprite.warn.empty', { percent: Math.round((1 - used) * 100) })}</p>}
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
    </Section>
  )
}

function animFacts(info: AnimClipInfo, t: Translate): DetailFact[] {
  const curves = info.transformCurves.length + info.floatCurves.length + info.spriteCurves
  return [
    { label: t('anim.length'), value: `${formatDuration(info.length)} · ${Math.round(info.length * info.sampleRate)} ${t('anim.frames')}`, top: true },
    { label: t('anim.rate'), value: `${info.sampleRate} fps`, top: true },
    { label: t('anim.type'), value: t(`anim.kinds.${info.kind}`) },
    { label: t('anim.loop'), value: info.loop ? t('texture.yes') : t('texture.no') },
    { label: t('anim.objects'), value: String(info.paths.length) },
    { label: t('anim.curves'), value: `${curves} · ${info.keyframes.toLocaleString()} ${t('anim.keys')}`, top: true },
    ...(info.events ? [{ label: t('anim.events'), value: String(info.events) }] : [])
  ]
}

/** Length, rate and what a clip animates, with what matters for playables. */
function AnimSection({ info }: { info: AnimClipInfo }) {
  const { t } = useTranslation()
  // Keys on (nearly) every frame: baked, not keyframed by hand; Keyframe Reduction would shrink it.
  const transformKeys = info.transformCurves.reduce((s, c) => s + c.keys.length, 0)
  const baked = info.transformCurves.length > 0 && info.length > 0.5 && transformKeys / (info.transformCurves.length * info.length * info.sampleRate) > 0.8
  return (
    <Section id="anim" defaultOpen={false} title={t('anim.title')} summary={baked ? <span className="warn-text">{t('details.warnings', { count: 1 })}</span> : undefined}>
      <FactsGrid facts={animFacts(info, t)} />
      {baked && <p className="warn-note">{t('anim.warn.baked')}</p>}
      {info.legacy && <p className="note">{t('anim.warn.legacy')}</p>}
    </Section>
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
        <span className="dep-type">{t('deps.tag.mat')}</span>
        <span className="dep-name">{m.name}</span>
        <span className="dep-meta">{m.unsure ? t('deps.checking') : m.missing ? t('deps.missing') : ''}</span>
      </div>
      {/* Built-in and package shaders come with Unity: nothing to do, so not listed. */}
      {m.shader?.kind === 'project' && (
        <div className="dep-row dep-child" title={m.shader.path ?? m.shader.guid}>
          <Dot tone={m.shader.kind === 'project' ? 'info' : 'ok'} />
          <span className="dep-type">{t('deps.tag.shd')}</span>
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

const PART_TAGS: Record<PartRef['kind'], string> = { model: 'fbx', prefab: 'pfb', script: 'cs', other: 'file' }

/** A model, nested prefab, script or other file a prefab uses. */
function PartRow({ part }: { part: PartRef }) {
  const { t } = useTranslation()
  const tone = part.unsure ? 'info' : part.missing ? 'warn' : part.kind === 'script' ? 'info' : 'ok'
  return (
    <li className="dep-row" title={part.path ?? part.guid}>
      <Dot tone={tone} />
      {/* A model in another format (OBJ, DAE…) is tagged by its extension, which is its name. */}
      <span className="dep-type">{part.kind === 'model' && part.path && !/\.fbx$/i.test(part.path) ? (part.path.split('.').pop() ?? '').toUpperCase() : t(`deps.tag.${PART_TAGS[part.kind]}`)}</span>
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
      <span className="dep-type">{t('deps.tag.tex')}</span>
      <span className="dep-name">
        {tex.prop && tex.prop !== tex.name && <span className="muted">{tex.prop} · </span>}
        {tex.missing ? tex.name : tex.path!.split('/').pop()}
      </span>
      <span className="dep-meta">{tex.unsure ? t('deps.checking') : tex.missing ? t('deps.missing') : tex.matchedBy === 'name' ? t('deps.byName') : formatBytes(tex.size)}</span>
    </li>
  )
}

/** "5 files · 1.3 MB: 2 materials, 3 textures": what an import would bring, in one line. */
function depsSummary(report: DependencyReport, t: Translate): string {
  const textures = new Map<string, number>()
  const addTexture = (tex: TextureRef) => {
    if (!tex.missing) textures.set(tex.path ?? `${tex.prop}:${tex.name}`, tex.size ?? 0)
  }
  for (const m of report.materials) m.textures.forEach(addTexture)
  report.embeddedTextures.forEach(addTexture)
  const parts = (report.parts ?? []).filter((p) => !p.missing)
  const bytes = [...textures.values()].reduce((s, n) => s + n, 0) + parts.reduce((s, p) => s + (p.size ?? 0), 0)
  const kinds: Array<[string, number]> = [
    ['materials', report.materials.filter((m) => !m.missing).length],
    ['textures', textures.size],
    ['models', parts.filter((p) => p.kind === 'model').length],
    ['prefabs', parts.filter((p) => p.kind === 'prefab').length],
    ['scripts', parts.filter((p) => p.kind === 'script').length]
  ]
  const list = kinds.filter(([, n]) => n > 0).map(([key, n]) => t(`deps.count.${key}`, { count: n }))
  const files = t('details.depsSummary', { count: report.files.filter((f) => !f.endsWith('.meta')).length })
  const head = bytes > 0 ? `${files} · ${formatBytes(bytes)}` : files
  return list.length ? `${head}: ${list.join(', ')}` : head
}

/** The viewer kinds that draw in 3D under the grey stage: they get the calibration strip. */
const CALIBRATED = new Set(['three', 'prefab', 'animation', 'material'])

export function AssetDetails({ asset, handler, live, onThumb, onReport, primary = true, inPack, exporting, onTogglePack, onExport, onImportToUnity, copies, onSelectCopy, excludedFolders, onPrefabFiles, onOpenLocation, onViewerLoading, onModelStats, playClips, models, viewerResizer, collections, onShowFolder }: Props) {
  const { t } = useTranslation()
  const budget = useBudget()
  const editors = useUnityEditors()
  const { state: copyState, copy } = useCopy()
  const [deps, setDeps] = useState<Deps>({ status: 'idle' })
  const [stats, setStats] = useState<ModelStats>()
  /** What the viewer measured, in the shape the budget judges. */
  const [summary, setSummary] = useState<StatsSummary>()
  /** For a texture: what the image viewer found. */
  const [imageInfo, setImageInfo] = useState<ImageInfo>()
  /** For a material: its colour / surface. */
  const [look, setLook] = useState<MaterialLook>()
  /** For an animation clip: what it animates. */
  const [animInfo, setAnimInfo] = useState<AnimClipInfo>()
  /** For an audio clip: what the player decoded. */
  const [audioInfo, setAudioInfo] = useState<DecodedAudio>()
  const [fontInfo, setFontInfo] = useState<LoadedFont>()
  const [tmpInfo, setTmpInfo] = useState<LoadedTmpFont>()
  const [totals, setTotals] = useState<ParticleTotals>()
  const [progress, setProgress] = useState<{ done: number; total: number }>()
  const [prefab, setPrefab] = useState<PrefabSearch>({ status: 'idle' })
  /** A suggested .mat (next to the FBX) the user chose to try. */
  const [picked, setPicked] = useState<{ material: MaterialRef; files: string[] }>()
  /** Other branches / repos with this exact file, from the catalog (empty until they are indexed). */
  const [elsewhere, setElsewhere] = useState<CatalogLocation[]>([])
  const [prefabProgress, setPrefabProgress] = useState<{ done: number; total: number }>()
  /** What an effect sits on (stage or dark); the strip under the viewer names it. */
  const [backdrop, setBackdrop] = useState<FxBackdrop>('stage')
  const repoPath = asset && live ? live.root + asset.path : undefined
  /** Bumped when the selection changes, so a search started for another model is ignored. */
  const searchToken = useRef(0)

  const report = deps.status === 'ready' ? deps.report : undefined
  const prefabResult = prefab.status === 'ready' ? prefab.result : undefined
  const prefabMaterial = prefabResult?.prefabs[0]?.materials.find((m) => !m.missing) ?? picked?.material
  const kind = handler?.preview.kind
  const exportable = !!live && !!handler?.resolvesDependencies && handler.delivery !== 'file'
  const bundle = useModelBundle(live && repoPath && exportable && (handler?.id === 'fbx' || handler?.id === 'prefab') ? { kind: handler.id, live, path: repoPath, report, prefabMaterial } : undefined)

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

  // The effect totals come from the Rust side; they join the facts once they are in.
  useEffect(() => {
    setTotals(undefined)
    if (!report?.particles) return
    let current = true
    parse.particleTotals(report.particles).then(
      (r) => current && setTotals(r),
      () => undefined
    )
    return () => {
      current = false
    }
  }, [report])

  useEffect(() => {
    searchToken.current++
    setStats(undefined)
    setSummary(undefined)
    setImageInfo(undefined)
    setLook(undefined)
    setAudioInfo(undefined)
    setFontInfo(undefined)
    setTmpInfo(undefined)
    setAnimInfo(undefined)
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

  const suggestions = !prefabResult?.prefabs.length ? (report?.suggested ?? []) : []
  /** The path as the card menus copy it: relative to the repo, root folder included. */
  const fullPath = repoPath ?? asset.path

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

  /** A measurement from a viewer: kept for the budget line and for ranking results. */
  const publish = (s: StatsSummary) => {
    setSummary(s)
    onModelStats?.(asset.sha, s)
  }
  const issues = budget && summary ? summaryIssues(summary, budget) : []

  const modelFacts: Fact[] = stats ? statCells(stats).map(([key, value]) => ({ label: t(`stats.${key}`), value: typeof value === 'number' ? value.toLocaleString() : value })) : []
  const detailFacts: DetailFact[] = imageInfo
    ? textureFacts(imageInfo, t)
    : audioInfo
      ? audioFacts(audioInfo, t)
      : fontInfo
        ? fontFacts(fontInfo, t)
        : tmpInfo
          ? tmpFontFacts(tmpInfo, t)
          : animInfo
            ? animFacts(animInfo, t)
            : kind === 'material'
              ? materialFacts(report?.materials[0], look, summary, t)
              : []
  const facts: Fact[] = [...modelFacts, ...detailFacts.filter((f) => f.top), ...(totals && report?.particles ? particleFacts(totals, t) : [])]
  const materialSectionFacts = kind === 'material' ? materialFacts(report?.materials[0], look, summary, t) : []

  const linkOf = () => assetLink({ repo: live!.repo, branch: live!.branch, path: repoPath! })
  const openGitHub = () => void window.api.openExternal(githubUrl(live!.repo, live!.branch, repoPath!))
  const noUnityId = 'insp-unity-note'

  /** Favourite star and the "…" menu: the end of every action row. */
  const rowEnd = (
    <span className="acts-end">
      {collections && <FavoriteButton collections={collections.list} item={collections.item} onToggle={collections.onToggle} />}
      <MoreMenu label={t('inspector.more')}>
        {collections && (
          <>
            <MenuLabel>{t('collections.title')}</MenuLabel>
            <CollectionMenuItems collections={collections.list} item={collections.item} onToggle={collections.onToggle} onCreate={collections.onCreate} />
            <MenuSep />
          </>
        )}
        {bundle && (
          <>
            <MenuItem icon={<DownloadIcon size={14} />} label={bundle.zip.label} hint={bundle.zip.hint} disabled={bundle.zip.disabled} onSelect={bundle.zip.onSelect} />
            {bundle.blender && <MenuItem icon={<CubeIcon size={14} />} label={bundle.blender.label} hint={bundle.blender.hint} disabled={bundle.blender.disabled} onSelect={bundle.blender.onSelect} />}
            <MenuSep />
          </>
        )}
        <MenuItem icon={<LinkIcon size={14} />} label={t('link.copy')} onSelect={() => void copy(linkOf())} />
        <MenuItem icon={<ExternalIcon size={14} />} label={t('details.openGitHub')} onSelect={openGitHub} />
        {onShowFolder && <MenuItem icon={<FolderIcon size={14} />} label={t('menu.showFolder')} onSelect={onShowFolder} />}
      </MoreMenu>
    </span>
  )

  return (
    <aside className="details" aria-label={t('details.label')}>
      <Suspense fallback={<div className="viewer" />}>
      {live && repoPath && kind === 'three' ? (
        <ModelViewer
          key={repoPath}
          live={live}
          path={repoPath}
          report={report}
          prefabMaterial={prefabMaterial}
          autoPlay={playClips}
          onStats={(s) => {
            setStats(s)
            publish(summarizeStats(s, 'viewer'))
          }}
          onThumb={(url) => onThumb(asset.sha, url)}
          onLoading={onViewerLoading}
        />
      ) : live && repoPath && kind === 'prefab' ? (
        // No key by path: the viewer follows its `path` itself, so the Luna runtime it hosts stays running from one prefab to the next.
        <PrefabViewer
          live={live}
          path={repoPath}
          report={report}
          onStats={(s) => {
            setStats(s)
            const summary = summarizeStats(s, report?.partial ? 'thumb' : 'viewer')
            publish(summary)
            if (report?.particles) void particleStats(report, summary).then(publish, () => undefined)
          }}
          onThumb={(url) => onThumb(asset.sha, url)}
          onLoading={onViewerLoading}
          onBackdrop={setBackdrop}
        />
      ) : live && repoPath && kind === 'animation' ? (
        <AnimViewer key={repoPath} live={live} path={repoPath} report={report} models={models} onInfo={setAnimInfo} onThumb={(url) => onThumb(asset.sha, url)} onLoading={onViewerLoading} />
      ) : live && repoPath && kind === 'material' ? (
        <MaterialViewer key={repoPath} live={live} path={repoPath} report={report} onLook={setLook} onStats={publish} onThumb={(url) => onThumb(asset.sha, url)} onLoading={onViewerLoading} />
      ) : live && repoPath && kind === 'audio' ? (
        <AudioViewer key={repoPath} live={live} path={repoPath} onInfo={setAudioInfo} onStats={publish} onLoading={onViewerLoading} />
      ) : live && repoPath && kind === 'tmpfont' ? (
        <TmpFontViewer key={repoPath} live={live} path={repoPath} onInfo={setTmpInfo} onStats={publish} onLoading={onViewerLoading} />
      ) : live && repoPath && kind === 'font' ? (
        <FontViewer key={repoPath} live={live} path={repoPath} onInfo={setFontInfo} onStats={publish} onLoading={onViewerLoading} />
      ) : live && repoPath && kind === 'image' ? (
        <ImageViewer key={repoPath} live={live} path={repoPath} onInfo={setImageInfo} onStats={publish} onLoading={onViewerLoading} />
      ) : (
        <div className="insp-preview">
          <CubeIcon size={72} />
          <span>{t('viewer.needsRepo')}</span>
        </div>
      )}
      </Suspense>
      {viewerResizer}
      {live && repoPath && kind && CALIBRATED.has(kind) && <CalibrationStrip dark={kind === 'prefab' && backdrop === 'dark'} />}

      <div className="insp-body">
        <div>
          <h2 className="insp-title">{asset.name}</h2>
          <div className="insp-path">
            <span className="mono" title={fullPath}>
              {fullPath.split('/').map((part, i) => (
                <Fragment key={i}>
                  {i > 0 && (
                    <>
                      /<wbr />
                    </>
                  )}
                  {part}
                </Fragment>
              ))}
            </span>
            <div className="insp-path-tools">
              <CopyStatus state={copyState} />
              <button type="button" className="btn icon-btn" title={t('details.copyPath')} aria-label={t('details.copyPath')} onClick={() => void copy(fullPath)}>
                <CopyIcon />
              </button>
              {live && repoPath && (
                <>
                  <button type="button" className="btn icon-btn" title={t('details.openGitHub')} aria-label={t('details.openGitHub')} onClick={openGitHub}>
                    <ExternalIcon />
                  </button>
                  <button type="button" className="btn icon-btn" title={`${t('link.copy')} · ${t('link.hint')}`} aria-label={t('link.copy')} onClick={() => void copy(linkOf())}>
                    <LinkIcon />
                  </button>
                </>
              )}
            </div>
          </div>
        </div>

        <FactsGrid facts={facts} />
        {budget && summary && <BudgetLine issues={issues} budget={budget} />}

        {live && repoPath && handler?.delivery === 'file' && (
          <DownloadButton repo={live.repo} branch={live.branch} path={repoPath} primary={primary}>
            {rowEnd}
          </DownloadButton>
        )}
        {exportable && repoPath && (
          <>
            <div className="acts">
              <button type="button" className={`btn${primary ? ' btn-primary' : ''}`} title={t('export.exportOne')} onClick={onExport} disabled={exporting}>
                <ExportIcon />
                {t('inspector.export')}
              </button>
              <UnityControl editors={editors} split disabled={exporting} onImport={onImportToUnity} describedBy={noUnityId} />
              <button type="button" className={`btn${inPack ? ' in-list' : ''}`} aria-pressed={inPack} title={inPack ? t('inspector.inListHint') : t('inspector.addToListHint')} onClick={onTogglePack} disabled={exporting}>
                <ExportListIcon filled={inPack} />
                {inPack ? t('inspector.inList') : t('inspector.addToList')}
              </button>
              {rowEnd}
            </div>
            {/* The reason the Unity button is off is text, not a tooltip. */}
            {!editors.length && (
              <p className="acts-note" id={noUnityId}>
                {t('unity.noneOpen')}
              </p>
            )}
            {bundle && <BundleNotice state={bundle.state} />}
          </>
        )}
        {live && repoPath && !exportable && handler?.delivery !== 'file' && <div className="acts">{rowEnd}</div>}
      </div>

      <div className="folds">
        {imageInfo && <TextureSection info={imageInfo} />}
        {imageInfo?.sprite && <SpriteSection info={imageInfo} />}
        {audioInfo && <AudioSection info={audioInfo} />}
        {fontInfo && <FontSection font={fontInfo} />}
        {tmpInfo && <TmpFontSection font={tmpInfo} />}
        {report?.particles && totals && <ParticleSection report={report} totals={totals} />}
        {animInfo && <AnimSection info={animInfo} />}
        {kind === 'material' && <MaterialSection facts={materialSectionFacts} />}

        {/* A texture is just itself: nothing to list. */}
        {handler?.resolvesDependencies && handler.usesGuids && (
          <Section
            id="deps"
            defaultOpen={false}
            title={t('details.deps')}
            summary={report ? <>{depsSummary(report, t)}{report.warnings.length > 0 && <span className="warn-text"> · {t('details.warnings', { count: report.warnings.length })}</span>}</> : undefined}
          >
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
                        <span className="dep-type">{t('deps.tag.fbx')}</span>
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
                  <div className="fold-group">
                    <p className="note">{t('deps.suggestedHint')}</p>
                    <ul className="dep-list">
                      {suggestions.map((s) => (
                        <li key={s.material.path}>
                          <div className="dep-row" title={s.material.path}>
                            <Dot tone="info" />
                            <span className="dep-type">{t('deps.tag.mat')}</span>
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
          <Section
            id="prefabUses"
            defaultOpen={false}
            title={t('prefab.title')}
            summary={prefab.status === 'ready' || prefab.status === 'known' ? (prefab.result.prefabs.length ? t('prefab.summary', { count: prefab.result.prefabs.length }) : t('prefab.noneFound')) : prefab.status === 'idle' ? t('prefab.unsearched') : undefined}
          >
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
                            <span className="dep-type">{t('deps.tag.pfb')}</span>
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
          <Section id="alsoIn" defaultOpen={false} title={t('catalog.alsoIn', { count: new Set(elsewhere.flatMap((l) => l.branches.map((b) => `${l.repo}@${b}`))).size })}>
            <LocationList locations={elsewhere} showRepo={elsewhere.some((l) => l.repo !== live?.repo)} onOpen={onOpenLocation} />
          </Section>
        )}
      </div>
    </aside>
  )
}
