import { useEffect, useRef, useState } from 'react'
import type { CatalogAsset, CatalogFacets } from '@shared/types'
import type { CatalogStatus } from '@shared/ipc'
import { unwrap } from './remoteSource'
import { useSharedImports } from './useSharedIndex'

/** Where a search looks: the open branch, every branch of the repo, or every repo. */
export type SearchScope = 'branch' | 'repo' | 'all'

/** Results are re-read at most this often while branches are being indexed. */
const REFRESH_MS = 3000
/** …and never more than this share of the time: reading every repo's results takes seconds. */
const REFRESH_SHARE = 0.1

/**
 * The cross-branch catalog for a scope: starts indexing when the scope is
 * opened, and re-reads the results as branches get listed.
 */
export function useCatalog(
  scope: SearchScope,
  repo: string,
  branch: string,
  excludedFolders: readonly string[],
  enabled: boolean,
  /** The results' repo and branch filters (applied where the catalog is). */
  filter: { repo: string; branch: string }
) {
  const [status, setStatus] = useState<CatalogStatus>({ state: 'idle', done: 0, total: 0 })
  const [assets, setAssets] = useState<CatalogAsset[]>()
  const [facets, setFacets] = useState<CatalogFacets>()
  const [error, setError] = useState<unknown>()
  const [version, setVersion] = useState(0)
  const lastRead = useRef(0)
  /** How long the last read took, to space out the next ones. */
  const readTime = useRef(0)
  const active = enabled && scope !== 'branch' && (scope === 'all' || !!repo)
  const target = scope === 'repo' ? repo : null
  const imports = useSharedImports()

  useEffect(() => {
    if (!enabled) return
    void window.api.catalog.status().then((r) => r.ok && setStatus(r.data))
    return window.api.catalog.onStatus(setStatus)
  }, [enabled])

  // Opening a scope (re)checks its branches and queues the new ones.
  useEffect(() => {
    if (!active) return
    setAssets(undefined)
    setError(undefined)
    // The open branch is listed first, so its results show up right away.
    void window.api.catalog.scan(target ?? undefined, repo ? { repo, branch } : undefined).then((r) => {
      if (!r.ok) setError(r.error)
      setVersion((v) => v + 1)
    })
  }, [active, target])

  // A teammate's index taken in: results read again.
  useEffect(() => {
    if (active && imports) setVersion((v) => v + 1)
  }, [imports])

  // New branches listed: read the results again, but not on every step.
  useEffect(() => {
    if (!active) return
    const wait = Math.max(0, lastRead.current + Math.max(REFRESH_MS, readTime.current / REFRESH_SHARE) - Date.now())
    const timer = setTimeout(() => setVersion((v) => v + 1), status.state === 'running' ? wait : 0)
    return () => clearTimeout(timer)
  }, [active, status.done, status.state])

  useEffect(() => {
    if (!active || !version) return
    let cancelled = false
    const started = Date.now()
    lastRead.current = started
    window.api.catalog.list(target, [...excludedFolders], { repo: filter.repo, branch: filter.branch }).then(
      (r) => {
        if (cancelled) return
        try {
          const listing = unwrap(r)
          setAssets(listing.assets)
          setFacets(listing.facets)
          // Counted once the page has taken the results in too (searching them, laying out the grid).
          setTimeout(() => (readTime.current = Date.now() - started))
        } catch (err) {
          setError(err)
        }
      },
      (err: unknown) => !cancelled && setError(err)
    )
    return () => {
      cancelled = true
    }
  }, [active, target, version, excludedFolders, filter.repo, filter.branch])

  return {
    status,
    assets: active ? assets : undefined,
    facets: active ? facets : undefined,
    error: active ? error : undefined,
    pause: () => void window.api.catalog.pause().then((r) => r.ok && setStatus(r.data)),
    resume: () => void window.api.catalog.resume().then((r) => r.ok && setStatus(r.data))
  }
}
