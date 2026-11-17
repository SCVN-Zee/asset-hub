import { useEffect, useRef, useState } from 'react'
import type { CatalogAsset, CatalogFacets } from '@shared/types'
import type { CatalogStatus } from '@shared/ipc'
import { unwrap } from './remoteSource'

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
  /** The results' repo and branch filters (applied where the catalog is); an empty list is no limit. */
  filter: { repos: string[]; branches: string[] }
) {
  const [status, setStatus] = useState<CatalogStatus>({ state: 'idle', done: 0, total: 0 })
  const [assets, setAssets] = useState<CatalogAsset[]>()
  const [facets, setFacets] = useState<CatalogFacets>()
  const [error, setError] = useState<unknown>()
  const lastRead = useRef(0)
  /** How long the last read took, to space out the next ones. */
  const readTime = useRef(0)
  const active = enabled && scope !== 'branch' && (scope === 'all' || !!repo)
  const target = scope === 'repo' ? repo : null
  const filterKey = JSON.stringify(filter)
  const excludedKey = JSON.stringify(excludedFolders)
  /** What the results are for: a read answering another key is dropped. */
  const key = `${active}|${target}|${excludedKey}|${filterKey}`
  const latest = useRef({ active, target, excludedFolders, filter, key })
  latest.current = { active, target, excludedFolders, filter, key }
  /** One read at a time; `next` says another is wanted once the current one lands. */
  const loop = useRef({ busy: false, next: false, key: '' })
  /** The key a real listing last landed for: a snapshot of that key arriving later is not shown. */
  const landed = useRef('')
  /** The scan in flight, so opening the same scope again does not start another. */
  const scanning = useRef<string>()
  const previousState = useRef(status.state)

  const pump = useRef(() => {})
  pump.current = () => {
    const state = loop.current
    if (state.busy || !state.next) return
    const now = latest.current
    state.next = false
    if (!now.active) return
    state.busy = true
    state.key = now.key
    const started = Date.now()
    lastRead.current = started
    const finish = () => {
      // Counted once the page has taken the results in too (searching them, laying out the grid).
      setTimeout(() => {
        readTime.current = Date.now() - started
        state.busy = false
        pump.current()
      })
    }
    window.api.catalog.list(now.target, [...now.excludedFolders], now.filter).then(
      (r) => {
        if (latest.current.key === now.key) {
          try {
            const listing = unwrap(r)
            landed.current = now.key
            setAssets(listing.assets)
            setFacets(listing.facets)
            setError(undefined)
          } catch (err) {
            setError(err)
          }
        }
        finish()
      },
      (err: unknown) => {
        if (latest.current.key === now.key) setError(err)
        finish()
      }
    )
  }
  /** Asks for a read; one in flight is never cancelled, the next starts when it lands. */
  const request = () => {
    loop.current.next = true
    pump.current()
  }

  /** Asks for a read of the current key unless one is already running for it. */
  const ensure = () => {
    if (!(loop.current.busy && loop.current.key === latest.current.key)) request()
  }

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
    landed.current = ''
    // Read the cached listing now; the scan below may wait on the network before it returns.
    ensure()
    // The listing made last time shows while the read runs; a real listing always replaces it.
    const asked = latest.current
    void window.api.catalog.snapshot(asked.target, [...asked.excludedFolders], asked.filter).then((r) => {
      if (!r.ok || !r.data || latest.current.key !== asked.key || landed.current === asked.key) return
      setAssets(r.data.assets)
      setFacets(r.data.facets)
    })
    // The open branch is listed first, so its results show up right away.
    const scanKey = `${target}|${repo}|${branch}`
    if (scanning.current === scanKey) return
    scanning.current = scanKey
    const scanStarted = Date.now()
    void window.api.catalog.scan(target ?? undefined, repo ? { repo, branch } : undefined).then((r) => {
      if (scanning.current === scanKey) scanning.current = undefined
      if (!r.ok) setError(r.error)
      // A read still running began after the scan did and is the one to wait for; otherwise the scan found new branches since.
      if (!(loop.current.busy && lastRead.current >= scanStarted)) request()
    })
  }, [active, target])

  // Other excluded folders or filters: read again, keeping the cards on screen until the answer lands.
  useEffect(() => {
    if (active) ensure()
  }, [excludedKey, filterKey])

  // New branches listed: read the results again, but not on every step.
  useEffect(() => {
    const was = previousState.current
    previousState.current = status.state
    if (!active) return
    if (status.state !== 'running') {
      // Opening on an idle catalog is read by the effects above; only the end of a run asks again.
      if (was === 'running') request()
      return
    }
    const wait = Math.max(0, lastRead.current + Math.max(REFRESH_MS, readTime.current / REFRESH_SHARE) - Date.now())
    const timer = setTimeout(request, wait)
    return () => clearTimeout(timer)
  }, [active, status.done, status.state])

  return {
    status,
    assets: active ? assets : undefined,
    facets: active ? facets : undefined,
    error: active ? error : undefined,
    pause: () => void window.api.catalog.pause().then((r) => r.ok && setStatus(r.data)),
    resume: () => void window.api.catalog.resume().then((r) => r.ok && setStatus(r.data))
  }
}
