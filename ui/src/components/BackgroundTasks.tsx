import { useTranslation } from 'react-i18next'
import type { FactKind } from '@shared/types'
import type { CatalogStatus } from '@shared/ipc'
import type { ExportState } from './ExportBar'
import { formatBytes } from '../format'
import { Popover } from './Popover'

export type ScanTask = { kind: FactKind; status: 'scanning' | 'error'; done: number; total: number }

type Task = { id: string; name: string; detail: string; done?: number; total?: number; action?: { label: string; run(): void } }

function Row({ task }: { task: Task }) {
  return (
    <div className="task-row">
      <div className="task-head">
        <span>{task.name}</span>
        <span className="num small">{task.detail}</span>
        {task.action && (
          <button type="button" className="btn btn-small" onClick={task.action.run}>
            {task.action.label}
          </button>
        )}
      </div>
      {task.total ? <progress className="loaded-bar" max={task.total} value={task.done ?? 0} /> : null}
    </div>
  )
}

/** Top-bar button, visible only while something runs in the background; its panel lists every running task. */
export function BackgroundTasks({
  thumbs,
  catalog,
  onPause,
  onResume,
  scan,
  exportState
}: {
  thumbs: { done: number; total: number }
  catalog: CatalogStatus
  onPause?(): void
  onResume?(): void
  scan?: ScanTask
  exportState: ExportState
}) {
  const { t } = useTranslation()
  const tasks: Task[] = []
  if (thumbs.total > 0 && thumbs.done < thumbs.total) {
    tasks.push({ id: 'thumbs', name: t('tasks.thumbnails'), detail: t('browse.loaded', { done: thumbs.done.toLocaleString(), total: thumbs.total.toLocaleString() }), ...thumbs })
  }
  if (catalog.state === 'running' || catalog.state === 'paused') {
    const key = catalog.state === 'running' ? 'catalog.indexing' : 'catalog.paused'
    const run = catalog.state === 'running' ? onPause : onResume
    tasks.push({
      id: 'catalog',
      name: t('tasks.catalog'),
      detail: t(key, { done: catalog.done.toLocaleString(), total: catalog.total.toLocaleString() }),
      done: catalog.done,
      total: catalog.total,
      action: run ? { label: t(catalog.state === 'running' ? 'catalog.pause' : 'catalog.resume'), run } : undefined
    })
  }
  if (scan?.status === 'scanning') {
    const what = t(`scan.what.${scan.kind}`)
    tasks.push(
      scan.total
        ? { id: 'scan', name: t('tasks.scan'), detail: t('scan.reading', { what, done: formatBytes(scan.done), total: formatBytes(scan.total) }), done: scan.done, total: scan.total }
        : { id: 'scan', name: t('tasks.scan'), detail: t('scan.starting', { what }) }
    )
  }
  if (exportState.status === 'running') {
    const p = exportState.progress
    tasks.push(
      p
        ? { id: 'export', name: t('tasks.export'), detail: t(`export.stage.${p.stage}`, { done: p.done.toLocaleString(), total: p.total.toLocaleString() }), done: p.done, total: p.total }
        : { id: 'export', name: t('tasks.export'), detail: t('export.choosing') }
    )
  }
  if (!tasks.length) return null
  const lead = tasks.find((x) => x.total) ?? tasks[0]
  const label = t('tasks.running', { count: tasks.length })
  return (
    <Popover
      className="tasks-popover"
      label={label}
      trigger={
        <span className="tasks-trigger" title={label}>
          {lead.total ? <progress className="loaded-bar" max={lead.total} value={lead.done ?? 0} /> : <progress className="loaded-bar" />}
          <span className="num small">{tasks.length}</span>
        </span>
      }
    >
      <span className="label-caps">{t('tasks.title')}</span>
      {tasks.map((task) => (
        <Row key={task.id} task={task} />
      ))}
    </Popover>
  )
}
