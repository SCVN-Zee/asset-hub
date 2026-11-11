import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { Asset, BudgetPreset, DependencyReport } from '@shared/types'
import { formatBytes } from '../format'
import type { ExportProgress, ExportResult, RunningUnity, TargetReport } from '@shared/ipc'
import { Dropdown } from './Dropdown'
import { ErrorNote } from './ErrorNote'
import { LogoMark } from './Logo'
import { leaveSlot } from '../motion'
import { UnityButton } from './UnityButton'
import { ChevronRightIcon, ExportIcon } from './Icons'

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

/** What the whole playable may weigh on the networks a preset stands for: the chest's capacity. */
export const packCapacity = (preset: BudgetPreset) => (preset === 'tight' ? 2 * MB : 5 * MB)

interface Props {
  items: PackItem[]
  /** Bytes the chest holds before it counts as too heavy. */
  capacity: number
  options: ExportOptions
  state: ExportState
  onOptions(options: ExportOptions): void
  onExport(): void
  /** Opens the chest's package in this Unity editor instead of saving it. */
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

export function ExportBar({ items, capacity, options, state, onOptions, onExport, onImportToUnity, onRemove, onClear, onDismiss }: Props) {
  const { t } = useTranslation()
  const sum = summarize(items, options.skipByName)
  const running = state.status === 'running'
  const progress = state.status === 'running' ? state.progress : undefined
  const over = sum.bytes > capacity
  // Folded, the chest is one line: what is in it, and Export. Open, its list and options too.
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

  return (
    <section className="export-bar" aria-label={t('export.title')}>
      {items.length > 0 && (
        <>
          <div className="export-head">
            <button type="button" className="pack-toggle" aria-expanded={open} title={open ? t('export.fold') : t('export.unfold')} onClick={toggle}>
              <span className="pack-chest">
                <LogoMark size={28} />
              </span>
              <strong className="pack-title">{t('export.chest')}</strong>
              <span className="pack-count">{items.length}</span>
              <span className="section-chevron" data-open={open || undefined}>
                <ChevronRightIcon size={12} />
              </span>
            </button>
            <span className="pack-room" data-over={over}>
              {over ? t('export.tooHeavy', { size: formatBytes(sum.bytes - capacity) }) : t('export.room', { size: formatBytes(capacity - sum.bytes) })}
            </span>
            <span className="muted small">
              {formatBytes(sum.bytes)}
              {sum.pending && ` · ${t('export.resolving')}`}
            </span>
            {sum.missing > 0 && <span className="badge badge-warn">{t('export.missing', { count: sum.missing })}</span>}
            <span style={{ flex: 1 }} />
            <UnityButton compact disabled={running} onImport={onImportToUnity} />
            <button type="button" className="btn btn-primary icon-btn pack-export" title={t('export.export', { count: items.length })} aria-label={t('export.export', { count: items.length })} onClick={onExport} disabled={running}>
              <ExportIcon />
              <span>{running ? t('export.exporting') : t('export.exportShort')}</span>
            </button>
          </div>
          <div className="pack-meter" data-over={over} title={`${formatBytes(sum.bytes)} / ${formatBytes(capacity)}`}>
            <span className="pack-meter-fill" style={{ width: `${Math.min(sum.bytes / capacity, 1) * 100}%` }} />
          </div>
          {open && (
          <>
          <ul className="export-items">
            {items.map((i) => (
              <li key={i.path} data-pack-id={i.asset.id}>
                <span className="dep-name">{i.asset.name}</span>
                <button
                  type="button"
                  className="link-btn"
                  aria-label={t('export.remove', { name: i.asset.name })}
                  onClick={(e) => leaveSlot(e.currentTarget.closest('li'), () => onRemove(i.path))}
                  disabled={running}
                >
                  ×
                </button>
              </li>
            ))}
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
          </>
          )}
        </>
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
    </section>
  )
}
