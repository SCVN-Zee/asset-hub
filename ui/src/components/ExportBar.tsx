import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Asset, BudgetPreset, DependencyReport, PlayableBudget } from '@shared/types'
import { formatBytes } from '../format'
import type { ExportProgress, ExportResult, RunningUnity, TargetReport } from '@shared/ipc'
import { Dropdown } from './Dropdown'
import { ErrorNote } from './ErrorNote'
import { UnityButton } from './UnityButton'
import { ChevronDownIcon, CloseIcon, ExportIcon } from './Icons'
import { PLAYABLE_BUDGET } from '../playable'

export interface PackItem {
  asset: Asset
  /** Repo path of the model. */
  path: string
  report?: DependencyReport
}

export interface ExportOptions {
  layout: 'source' | 'grouped' | 'byType'
  skipByName: boolean
  /** Compared with this Unity project (from settings): what it already has is left out. */
  targetProject?: string
  /** Leave out files the project has under another GUID, pointing references at its copy. */
  reuseExisting: boolean
  /** Remove the project's scripts from prefabs (and their children) and leave the .cs files out. */
  stripScripts: boolean
}

export type ExportState =
  | { status: 'idle' }
  | { status: 'running'; progress?: ExportProgress }
  | { status: 'done'; result: ExportResult }
  | { status: 'error'; error: unknown }

const MB = 1024 * 1024

/** What the whole playable may weigh on the 2 MB networks (the tight preset's per-file limits are lower). */
const TIGHT_PLAYABLE_BYTES = 2 * MB

/**
 * What the export list may weigh before it counts as too heavy, from the active
 * budget: 2 MB for the tight preset, the budget's file limit otherwise (5 MB
 * standard, or the custom limit). `undefined` when playable checks are off.
 */
export function packCapacity(preset: BudgetPreset, budget: PlayableBudget | undefined): number | undefined {
  if (preset === 'off' || !budget) return undefined
  return preset === 'tight' ? TIGHT_PLAYABLE_BYTES : (budget.fileBytes ?? PLAYABLE_BUDGET.fileBytes)
}

/** The meter turns to a warning at this share of the limit. */
const NEAR_LIMIT = 0.85

interface Props {
  items: PackItem[]
  /** Bytes the list may weigh before it counts as too heavy; undefined without a budget. */
  capacity: number | undefined
  /** The active preset, named next to the meter. */
  budgetPreset: BudgetPreset
  /** Saved thumbnails by content hash. */
  thumbs: Record<string, string>
  options: ExportOptions
  state: ExportState
  onOptions(options: ExportOptions): void
  onExport(): void
  /** Opens the list's package in this Unity editor instead of saving it. */
  onImportToUnity(editor: RunningUnity): void
  onRemove(path: string): void
  onClear(): void
  onDismiss(): void
}

/** Files, size and warnings of a pack, from the dependency reports known so far. */
function summarize(items: readonly PackItem[], skipByName: boolean) {
  const files = new Set<string>()
  let bytes = 0
  let missing = 0
  let byName = 0
  const counted = new Set<string>()
  for (const { asset, path, report } of items) {
    if (!counted.has(path)) {
      counted.add(path)
      bytes += asset.size ?? 0
    }
    files.add(path)
    if (!report) continue
    const skip = new Set(skipByName ? report.embeddedTextures.filter((t) => t.path && t.matchedBy === 'name').map((t) => t.path!) : [])
    for (const f of report.files) if (!f.endsWith('.meta') && !skip.has(f)) files.add(f)
    for (const tex of [...report.materials.flatMap((m) => m.textures), ...report.embeddedTextures]) {
      if (tex.missing) missing++
      else if (tex.path && !skip.has(tex.path) && !counted.has(tex.path)) {
        counted.add(tex.path)
        bytes += tex.size ?? 0
        if (tex.matchedBy === 'name') byName++
      }
    }
    missing += report.materials.filter((m) => m.missing).length
  }
  return { files: files.size, bytes, missing, byName, pending: items.some((i) => !i.report) }
}

const fileName = (p: string) => p.split('/').pop()

/** What the package does to the target project, before it is imported. */
function TargetSummary({ report }: { report: TargetReport }) {
  const { t } = useTranslation()
  const project = report.project.split(/[\\/]/).pop()
  const list = (items: Array<{ path: string; existing: string }>) => items.slice(0, 6).map((i) => `${fileName(i.path)} → ${i.existing}`).join(', ') + (items.length > 6 ? ' …' : '')
  const nothing =
    !report.same && !report.reused.length && !report.overwrite.length && !report.renamed.length && !report.shaderByName.length && !report.shaderKept.length && !report.shaderClash.length && !report.shaderMapped.length && !report.urpConverted.length && !report.mappingErrors.length && !report.missingScripts.length && !report.missingShaders.length && !report.urp
  return (
    <div className="target-summary small">
      <span className="muted">{nothing ? t('target.clean', { project }) : t('target.title', { project })}</span>
      {report.same > 0 && <span>{t('target.same', { count: report.same })}</span>}
      {report.reused.length > 0 && <span title={list(report.reused)}>{t('target.reused', { count: report.reused.length, list: list(report.reused) })}</span>}
      {report.overwrite.length > 0 && (
        <span className="warn-text" title={list(report.overwrite)}>
          {t('target.overwrite', { count: report.overwrite.length, list: list(report.overwrite) })}
        </span>
      )}
      {report.renamed.length > 0 && (
        <span className="warn-text" title={list(report.renamed)}>
          {t('target.renamed', { count: report.renamed.length, list: list(report.renamed) })}
        </span>
      )}
      {report.shaderByName.length > 0 && <span title={list(report.shaderByName)}>{t('target.shaderByName', { count: report.shaderByName.length, list: list(report.shaderByName) })}</span>}
      {report.shaderKept.length > 0 && <span title={list(report.shaderKept)}>{t('target.shaderKept', { count: report.shaderKept.length, list: list(report.shaderKept) })}</span>}
      {report.shaderClash.length > 0 && (
        <span className="warn-text" title={list(report.shaderClash)}>
          {t('target.shaderClash', { count: report.shaderClash.length, list: list(report.shaderClash) })}
        </span>
      )}
      {report.shaderMapped.length > 0 && <span title={list(report.shaderMapped)}>{t('target.shaderMapped', { count: report.shaderMapped.length, list: list(report.shaderMapped) })}</span>}
      {report.urpConverted.length > 0 && <span title={list(report.urpConverted)}>{t('target.urpConverted', { count: report.urpConverted.length, list: list(report.urpConverted) })}</span>}
      {report.missingScripts.length > 0 && (
        <span className="warn-text" title={list(report.missingScripts)}>
          {t('target.missingScripts', { count: report.missingScripts.length, list: list(report.missingScripts) })}
        </span>
      )}
      {report.missingShaders.length > 0 && (
        <span className="warn-text" title={report.missingShaders.map((s) => `${s.path}: ${s.existing}`).join('\n')}>
          {t('target.missingShaders', { count: report.missingShaders.length, project, list: report.missingShaders.map((s) => `${s.path} (${s.existing})`).join('; ') })}
        </span>
      )}
      {report.mappingErrors.map((error) => (
        <span key={error} className="warn-text">
          {t('target.mappingError', { error })}
        </span>
      ))}
      {report.urp && (
        <span className="warn-text">
          {report.urp.target ? t('target.urp', { source: report.urp.source, target: report.urp.target }) : t('target.noUrp', { source: report.urp.source })}
          {!report.urp.target && !!report.urp.needed && ` ${t('target.urpNeeded', { count: report.urp.needed })}`}
        </span>
      )}
    </div>
  )
}

export function ExportBar({ items, capacity, budgetPreset, thumbs, options, state, onOptions, onExport, onImportToUnity, onRemove, onClear, onDismiss }: Props) {
  const { t } = useTranslation()
  const sum = summarize(items, options.skipByName)
  const running = state.status === 'running'
  const progress = state.status === 'running' ? state.progress : undefined
  const over = capacity !== undefined && sum.bytes > capacity
  const near = capacity !== undefined && !over && sum.bytes > capacity * NEAR_LIMIT
  const tone = over ? 'danger' : near ? 'warn' : 'ok'
  const deps = Math.max(0, sum.files - items.length)
  // The count lights up for a moment when the list grew (an asset was added).
  const seen = useRef(0)
  const grew = items.length > seen.current
  useEffect(() => {
    seen.current = items.length
  }, [items.length])
  // Folded, the list is one row. Open, its items and options show above that row.
  const [open, setOpen] = useState(() => {
    try {
      return localStorage.getItem('assethub.chestOpen') === '1'
    } catch {
      return false
    }
  })
  const toggle = () => {
    setOpen(!open)
    try {
      localStorage.setItem('assethub.chestOpen', open ? '0' : '1')
    } catch {
      // kept for this session
    }
  }
  const showList = open && items.length > 0

  // The strip tells its container how tall it is, so the multi-select bar can sit above it.
  const strip = useRef<HTMLElement>(null)
  useEffect(() => {
    const el = strip.current
    const container = el?.parentElement
    if (!el || !container) return
    const publish = () => container.style.setProperty('--export-h', `${el.offsetHeight}px`)
    publish()
    const watch = new ResizeObserver(publish)
    watch.observe(el)
    return () => {
      watch.disconnect()
      container.style.removeProperty('--export-h')
    }
  }, [])

  return (
    <section className="export-bar" ref={strip} aria-label={t('export.label')} data-over={over || undefined}>
      {showList && (
        <div className="export-panel">
          <ul className="export-items">
            {items.map((i) => {
              const thumb = thumbs[i.asset.sha]
              return (
                <li key={i.path}>
                  <span className="export-thumb">{thumb && <img src={thumb} alt="" />}</span>
                  <span className="export-item-name" title={i.path}>
                    {i.asset.name}
                  </span>
                  <span className="num export-item-size">{formatBytes(i.asset.size ?? 0)}</span>
                  <button type="button" className="link-btn" title={t('export.remove', { name: i.asset.name })} aria-label={t('export.remove', { name: i.asset.name })} onClick={() => onRemove(i.path)} disabled={running}>
                    <CloseIcon size={12} />
                  </button>
                </li>
              )
            })}
          </ul>
          <div className="export-actions">
            <div className="picker">
              <span className="muted small">{t('export.layout')}</span>
              <Dropdown
                value={options.layout}
                options={[
                  { value: 'source', label: t('export.layoutSource') },
                  { value: 'grouped', label: t('export.layoutGrouped') },
                  { value: 'byType', label: t('export.layoutByType') }
                ]}
                disabled={running}
                onChange={(layout: ExportOptions['layout']) => onOptions({ ...options, layout })}
                label={t('export.layout')}
              />
            </div>
            <label className="check">
              <input type="checkbox" checked={options.skipByName} disabled={running} onChange={(e) => onOptions({ ...options, skipByName: e.target.checked })} />
              {t('export.skipByName')}
            </label>
            <label className="check" title={t('export.stripScriptsHint')}>
              <input type="checkbox" checked={options.stripScripts} disabled={running} onChange={(e) => onOptions({ ...options, stripScripts: e.target.checked })} />
              {t('export.stripScripts')}
            </label>
            {options.targetProject && (
              <label className="check" title={t('target.reuseHint')}>
                <input type="checkbox" checked={options.reuseExisting} disabled={running} onChange={(e) => onOptions({ ...options, reuseExisting: e.target.checked })} />
                {t('target.reuse', { project: options.targetProject.split(/[\\/]/).pop() })}
              </label>
            )}
            {sum.byName > 0 && <span className="badge">{t('export.byName', { count: sum.byName })}</span>}
            <span style={{ flex: 1 }} />
            <button type="button" className="btn" onClick={onClear} disabled={running}>
              {t('export.clear')}
            </button>
          </div>
        </div>
      )}

      {running && (
        <div className="export-status" role="status">
          <span className="small">{progress ? t(`export.stage.${progress.stage}`, { done: progress.done, total: progress.total }) : t('export.choosing')}</span>
          <progress className="progress" max={progress?.total || undefined} value={progress ? progress.done : undefined} />
        </div>
      )}
      {state.status === 'done' && state.result.path && (
        <div className="export-status done" role="status">
          <span className="small">
            {state.result.unity ? (
              state.result.assets ? t('unity.done', { count: state.result.assets, size: formatBytes(state.result.bytes), project: state.result.unity }) : t('unity.nothingNew', { project: state.result.unity })
            ) : (
              <>
                {t('export.done', { count: state.result.assets ?? 0, size: formatBytes(state.result.bytes) })}{' '}
                <span className="mono">{state.result.path.split(/[\\/]/).pop()}</span>
              </>
            )}
          </span>
          {state.result.scriptsRemoved !== undefined && <span className="small muted">{t('export.scriptsRemoved', { count: state.result.scriptsRemoved })}</span>}
          {state.result.target && <TargetSummary report={state.result.target} />}
          {state.result.skipped && state.result.skipped.length > 0 && (
            <span className="small muted">{t('export.skipped', { count: state.result.skipped.length, list: state.result.skipped.map((s) => s.path.split('/').pop()).join(', ') })}</span>
          )}
          <div className="export-actions">
            {!state.result.unity && (
              <button type="button" className="btn" onClick={() => void window.api.exporter.showInFolder(state.result.path!)}>
                {t('export.showInFolder')}
              </button>
            )}
            <button type="button" className="btn" onClick={onDismiss}>
              {t('export.dismiss')}
            </button>
          </div>
        </div>
      )}
      {state.status === 'error' && (
        <div className="export-status">
          <ErrorNote error={state.error} />
          <div>
            <button type="button" className="btn" onClick={onDismiss}>
              {t('export.dismiss')}
            </button>
          </div>
        </div>
      )}

      {items.length > 0 && (
        <div className="export-row">
          <button type="button" className="pack-toggle" aria-expanded={open} title={open ? t('export.fold') : t('export.unfold')} onClick={toggle}>
            <strong>{t('export.label')}</strong>
            <span className="pack-chev">
              <ChevronDownIcon size={12} />
            </span>
          </button>
          <span key={items.length} className="pack-summary" data-fresh={grew || undefined}>
            {t('export.summaryLine', { assets: t('export.assetCount', { count: items.length }), deps: t('export.depCount', { count: deps }) })}
          </span>
          <span className="pack-meter-wrap">
            {capacity !== undefined && (
              <span className="pack-meter" data-tone={tone} role="meter" aria-valuemin={0} aria-valuemax={capacity} aria-valuenow={Math.min(sum.bytes, capacity)} aria-label={t('export.meterLabel')}>
                <span className="pack-meter-fill" style={{ width: `${Math.min(sum.bytes / capacity, 1) * 100}%` }} />
              </span>
            )}
            <span className="pack-meter-text small muted" data-tone={capacity === undefined ? undefined : tone}>
              <span className="num">{formatBytes(sum.bytes)}</span>
              {capacity !== undefined && (
                <>
                  {' '}
                  {t('export.of')} <span className="num">{formatBytes(capacity)}</span>
                  <span className="pack-budget-name"> · {t(`export.budgetName.${budgetPreset}`)}</span>
                </>
              )}
              {capacity !== undefined && over && (
                <>
                  {' · '}
                  <strong className="pack-over">{t('export.tooHeavy', { size: formatBytes(sum.bytes - capacity) })}</strong>
                </>
              )}
              {sum.pending && ` · ${t('export.resolving')}`}
            </span>
          </span>
          {sum.missing > 0 && <span className="badge badge-warn">{t('export.missing', { count: sum.missing })}</span>}
          <UnityButton split disabled={running} onImport={onImportToUnity} />
          <button type="button" className="btn btn-primary" title={t('export.export', { count: items.length })} onClick={onExport} disabled={running}>
            <ExportIcon />
            <span>{running ? t('export.exporting') : t('export.exportFiles', { count: sum.files })}</span>
          </button>
        </div>
      )}
    </section>
  )
}
