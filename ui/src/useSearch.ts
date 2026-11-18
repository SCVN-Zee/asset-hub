import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Asset, FolderNode, Suggestion } from '@shared/types'
import { parse, type SearchHit } from './parse'
import type { AssetTag } from './tags'

/** What a search found, with the caller's own asset objects (a catalog asset keeps its locations). */
export interface Found<A extends Asset> {
  assets: A[]
  related: string[]
  typos: string[]
  scores: ReadonlyMap<string, number>
}

/** A search built over a list of assets, in Rust, answered by handle. */
interface Index<A extends Asset> {
  handle: number
  assets: readonly A[]
  /** The caller's asset by blob SHA and id: what the index answers with is matched back to these. */
  byKey: Map<string, A>
}

const keyOf = (a: Asset) => `${a.sha}\n${a.id}`

const NO_TAGS: ReadonlyMap<string, readonly AssetTag[]> = new Map()

const EMPTY_FOLDERS: FolderNode = { name: '', path: '', children: [], assetCount: 0 }

const inFolder = (path: string, folder: string) => folder === '' || path.startsWith(`${folder}/`)

function everything<A extends Asset>(assets: readonly A[], folder: string): Found<A> {
  return { assets: folder ? assets.filter((a) => inFolder(a.path, folder)) : [...assets], related: [], typos: [], scores: new Map() }
}

function foundFrom<A extends Asset>(hit: SearchHit, index: Index<A>): Found<A> {
  return {
    assets: hit.assets.map((a) => index.byKey.get(keyOf(a)) ?? (a as A)),
    related: hit.related,
    typos: hit.typos,
    scores: new Map(hit.scores)
  }
}

interface Wanted<A extends Asset> {
  index: Index<A>
  query: string
  folder: string
}

interface Answered<A extends Asset> extends Wanted<A> {
  found: Found<A>
}

interface BuildInput<A extends Asset> {
  assets: readonly A[]
  extraNames: ReadonlyMap<string, string[]>
  tags: ReadonlyMap<string, readonly AssetTag[]>
}

export interface SearchOptions {
  /** False: no index is built (the scope is hidden); the last one is kept. */
  enabled?: boolean
  /**
   * Changes whenever `tags` was changed in place (the map keeps its identity). The tags are only read by a search with a
   * query, so the index is rebuilt for them only while a query is typed.
   */
  tagsVersion?: number
  /** True while tags are still being found: the index is then rebuilt for them at most every `TAG_BUILD_GAP_MS`, and at once when it stops. */
  tagsBusy?: boolean
}

/** Least time between two index builds while their inputs keep changing. */
const BUILD_GAP_MS = 500
/** Least time between two index builds that only take in tags found since the last one. */
const TAG_BUILD_GAP_MS = 4000

/**
 * Searches `assets` (matching groups, typos and folder names, best first) and lists their folders.
 * The work is done in Rust, so a result arrives after the keystroke: one search runs at a time, the
 * newest question is asked when it ends, and the last answer stays on screen meanwhile.
 * `extraNames` maps an asset's SHA to other names it answers to (the clips inside a model). `tags` maps an
 * asset's id to its tags: typing a tag's word ("ui", "animation", "particle") finds the assets that have it.
 */
export function useSearch<A extends Asset>(assets: readonly A[], extraNames: ReadonlyMap<string, string[]>, query: string, folder: string, tags: ReadonlyMap<string, readonly AssetTag[]> = NO_TAGS, options: SearchOptions = {}) {
  const enabled = options.enabled ?? true
  const enabledRef = useRef(enabled)
  enabledRef.current = enabled
  const tagsVersion = options.tagsVersion ?? 0
  const tagsBusy = options.tagsBusy ?? false
  const latest = useRef<BuildInput<A>>({ assets, extraNames, tags })
  latest.current = { assets, extraNames, tags }
  const [index, setIndex] = useState<Index<A>>()
  const [folders, setFolders] = useState<FolderNode>(EMPTY_FOLDERS)
  const [answered, setAnswered] = useState<Answered<A>>()
  const [failure, setFailure] = useState<unknown>()

  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])

  // One build at a time, the newest inputs next, and a pause between builds: every build sends the whole list
  // to Rust, which reads it on the app's main thread, so a burst of changes (a scan finding assets) froze the app.
  const build = useRef<{ busy: boolean; next?: BuildInput<A>; builtAt: number; tagsVersion: number; assets?: readonly A[] }>({ busy: false, builtAt: 0, tagsVersion: 0 })
  const requestBuild = useRef(() => {})
  requestBuild.current = () => {
    build.current.next = latest.current
    build.current.tagsVersion = tagsVersion
    if (build.current.busy) return
    build.current.busy = true
    void (async () => {
      try {
        for (let wanted = build.current.next; wanted && enabledRef.current; wanted = build.current.next) {
          build.current.next = undefined
          try {
            const handle = await parse.searchBuild(wanted.assets, Object.fromEntries(wanted.extraNames), Object.fromEntries([...wanted.tags].map(([id, list]) => [id, [...list]])))
            if (!alive.current) return
            const list = wanted.assets
            // The same assets again (only tags changed): their lookup and folders are still right.
            const sameAssets = build.current.assets === list
            build.current.assets = list
            build.current.builtAt = Date.now()
            setIndex((prev) => ({ handle, assets: list, byKey: sameAssets && prev?.assets === list ? prev.byKey : new Map(list.map((a) => [keyOf(a), a])) }))
            if (!sameAssets) {
              const root = await parse.searchFolders(handle)
              if (!alive.current) return
              setFolders(root)
            }
          } catch (error) {
            if (alive.current) setFailure(error)
          }
          if (build.current.next) await new Promise((resolve) => setTimeout(resolve, BUILD_GAP_MS))
        }
      } finally {
        build.current.busy = false
      }
    })()
  }
  useEffect(() => {
    if (!enabled) {
      build.current.next = undefined
      return
    }
    requestBuild.current()
  }, [assets, extraNames, tags, enabled])
  const searching = query.trim() !== ''
  // Tags changed in place: a query reads them, so the index takes them in, but not on every batch a scan finds.
  useEffect(() => {
    if (!enabled || !searching || tagsVersion === build.current.tagsVersion) return
    const wait = tagsBusy ? Math.max(0, build.current.builtAt + TAG_BUILD_GAP_MS - Date.now()) : 0
    const timer = setTimeout(() => requestBuild.current(), wait)
    return () => clearTimeout(timer)
  }, [enabled, searching, tagsVersion, tagsBusy])
  const run = useRef<{ busy: boolean; next?: Wanted<A> }>({ busy: false })
  useEffect(() => {
    if (!index || !searching) return
    run.current.next = { index, query, folder }
    if (run.current.busy) return
    run.current.busy = true
    void (async () => {
      try {
        for (let wanted = run.current.next; wanted; wanted = run.current.next) {
          run.current.next = undefined
          try {
            const hit = await parse.searchQuery(wanted.index.handle, wanted.query, wanted.folder)
            if (alive.current) setAnswered({ ...wanted, found: foundFrom(hit, wanted.index) })
          } catch (error) {
            if (alive.current) setFailure(error)
          }
        }
      } finally {
        run.current.busy = false
      }
    })()
  }, [index, query, folder, searching])

  const found = useMemo<Found<A>>(() => {
    if (!searching) return everything(assets, folder)
    if (answered) return answered.found
    return everything(assets, folder)
  }, [searching, answered, assets, folder])
  /** The answer is for what is typed now (not one asked before the last keystroke). */
  const settled = !searching || (!!answered && answered.index === index && answered.query === query && answered.folder === folder)

  const suggest = useCallback((text: string): Promise<Suggestion[]> => (index ? parse.searchSuggest(index.handle, text) : Promise.resolve([])), [index])
  return { found, folders, suggest, settled, failure }
}
