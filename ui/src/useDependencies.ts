import { useEffect, useRef, useState } from 'react'
import type { DependencyReport } from '@shared/types'
import type { LiveContext } from './components/ModelViewer'
import type { AssetTypeHandler } from './handlers'
import { RemoteError, unwrap } from './remoteSource'

export type Deps = { status: 'idle' } | { status: 'loading' } | { status: 'error'; error: unknown } | { status: 'ready'; report: DependencyReport }

/**
 * What a file needs (textures of a model, materials of a prefab), for the opened asset at `repoPath`.
 * Idle unless the handler resolves dependencies. `onReport` hears every report that is kept.
 */
export function useDependencies(live: LiveContext | undefined, repoPath: string | undefined, handler: AssetTypeHandler | undefined, onReport?: (path: string, report: DependencyReport) => void): Deps {
  const [deps, setDeps] = useState<Deps>({ status: 'idle' })
  const reportTo = useRef(onReport)
  reportTo.current = onReport

  useEffect(() => {
    if (!live || !repoPath || !handler?.resolvesDependencies) {
      setDeps({ status: 'idle' })
      return
    }
    let cancelled = false
    setDeps({ status: 'loading' })
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
            reportTo.current?.(repoPath, report)
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

  return deps
}
