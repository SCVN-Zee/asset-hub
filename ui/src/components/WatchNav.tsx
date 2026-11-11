import { useTranslation } from 'react-i18next'
import type { WatchedBranch } from '@shared/settings'
import { PinIcon } from './Icons'
import { SideGroup } from './Section'

/** Sidebar: the branches pinned (button next to the branch picker), each with how many assets came in since last seen. */
export function WatchNav({
  watched,
  news,
  current,
  onOpen,
  onRemove,
  onMenu
}: {
  watched: WatchedBranch[]
  news: Record<string, { added: string[]; more: boolean }>
  current: { repo: string; branch: string }
  onOpen(repo: string, branch: string): void
  onRemove(w: WatchedBranch): void
  /** Right-click on a row. */
  onMenu?(w: WatchedBranch, e: React.MouseEvent): void
}) {
  const { t } = useTranslation()
  if (!watched.length) return null
  return (
    <SideGroup id="watch" title={t('watch.title')} className="collections-nav" folded={watched.length}>
      <div className="tabs" role="group" aria-label={t('watch.title')}>
        {watched.map((w) => {
          const n = news[`${w.repo}\n${w.branch}`]
          const count = n?.added.length ?? 0
          return (
            <div key={`${w.repo}\n${w.branch}`} className="watch-row">
              <button type="button" className="tab" aria-pressed={current.repo === w.repo && current.branch === w.branch} title={`${w.repo} · ${w.branch}`} onClick={() => onOpen(w.repo, w.branch)} onContextMenu={onMenu && ((e) => onMenu(w, e))}>
                <span className="nav-icon">
                  <PinIcon size={14} filled={count > 0} />
                </span>
                {/* Repo over branch: both long, each gets the whole width (full names in the tooltip). */}
                <span className="watch-label">
                  <span className="watch-repo">{w.repo.split('/').pop()}</span>
                  <span className="watch-branch">{w.branch}</span>
                </span>
                {count > 0 && <span className="watch-count">{count}{n?.more ? '+' : ''}</span>}
              </button>
              <button type="button" className="watch-remove" aria-label={t('watch.stop')} title={t('watch.stop')} onClick={() => onRemove(w)}>
                ×
              </button>
            </div>
          )
        })}
      </div>
    </SideGroup>
  )
}
