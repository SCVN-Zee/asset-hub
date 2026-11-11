import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { Asset, FolderNode, Suggestion } from '@shared/types'
import { parse, type SearchHit } from './parse'

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

/**
 * Searches `assets` (matching groups, typos and folder names, best first) and lists their folders.
 * The work is done in Rust, so a result arrives after the keystroke: one search runs at a time, the
 * newest question is asked when it ends, and the last answer stays on screen meanwhile.
 * `extraNames` maps an asset's SHA to other names it answers to (the clips inside a model).
 */
export function useSearch<A extends Asset>(assets: readonly A[], extraNames: ReadonlyMap<string, string[]>, query: string, folder: string) {
  const [index, setIndex] = useState<Index<A>>()
  const [folders, setFolders] = useState<FolderNode>(EMPTY_FOLDERS)
  const [answered, setAnswered] = useState<Answered<A>>()
  const [failure, setFailure] = useState<unknown>()

  useEffect(() => {
    let cancelled = false
    parse.searchBuild(assets, Object.fromEntries(extraNames)).then(
      (handle) => {
        if (cancelled) return
        setIndex({ handle, assets, byKey: new Map(assets.map((a) => [keyOf(a), a])) })
        void parse.searchFolders(handle).then(
          (root) => !cancelled && setFolders(root),
          (error: unknown) => !cancelled && setFailure(error)
        )
      },
      (error: unknown) => !cancelled && setFailure(error)
    )
    return () => {
      cancelled = true
    }
  }, [assets, extraNames])

  const alive = useRef(true)
  useEffect(() => {
    alive.current = true
    return () => {
      alive.current = false
    }
  }, [])
  const run = useRef<{ busy: boolean; next?: Wanted<A> }>({ busy: false })
  const searching = query.trim() !== ''
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
