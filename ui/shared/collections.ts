// Collections of assets someone keeps (Favourites, "Trees for level 3"…),
// kept in the settings on this machine. Items come from any repo and branch;
// an asset is in a collection by repo + path, whichever branch it was added
// from, and opens on that branch.

export interface CollectionItem {
  repo: string
  branch: string
  /** Repo path (project root included), as links carry it. */
  path: string
  /** Content when added: the thumbnail and measurements cached for it. */
  sha: string
  name: string
  /** Handler id (fbx, prefab…). */
  type: string
  size?: number
}

export interface Collection {
  id: string
  name: string
  items: CollectionItem[]
}

/** Always there, first; the star adds to it. Its name is translated, not stored. */
export const FAVORITES = 'favorites'

const keyOf = (i: Pick<CollectionItem, 'repo' | 'path'>) => `${i.repo}\n${i.path}`

/** The collections with Favourites first (made when missing). */
export function withFavorites(collections: readonly Collection[] | undefined): Collection[] {
  const list = collections ?? []
  const fav = list.find((c) => c.id === FAVORITES) ?? { id: FAVORITES, name: '', items: [] }
  return [fav, ...list.filter((c) => c.id !== FAVORITES)]
}

export function inCollection(c: Collection | undefined, item: Pick<CollectionItem, 'repo' | 'path'>): boolean {
  return !!c?.items.some((i) => keyOf(i) === keyOf(item))
}

/** Adds the item to the collection, or takes it out when it is there (newest first). */
export function toggleItem(collections: readonly Collection[] | undefined, id: string, item: CollectionItem): Collection[] {
  return withFavorites(collections).map((c) => {
    if (c.id !== id) return c
    const rest = c.items.filter((i) => keyOf(i) !== keyOf(item))
    return { ...c, items: rest.length === c.items.length ? [item, ...c.items] : rest }
  })
}

export function removeItem(collections: readonly Collection[] | undefined, id: string, item: Pick<CollectionItem, 'repo' | 'path'>): Collection[] {
  return withFavorites(collections).map((c) => (c.id === id ? { ...c, items: c.items.filter((i) => keyOf(i) !== keyOf(item)) } : c))
}

/** A new, empty collection (`id` from the caller, so it can be opened straight away). */
export function addCollection(collections: readonly Collection[] | undefined, id: string, name: string): Collection[] {
  return [...withFavorites(collections), { id, name: name.trim(), items: [] }]
}

export function renameCollection(collections: readonly Collection[] | undefined, id: string, name: string): Collection[] {
  return withFavorites(collections).map((c) => (c.id === id && id !== FAVORITES ? { ...c, name: name.trim() } : c))
}

/** Favourites cannot be deleted. */
export function deleteCollection(collections: readonly Collection[] | undefined, id: string): Collection[] {
  return withFavorites(collections).filter((c) => c.id === FAVORITES || c.id !== id)
}

/** Collections from stored or IPC data: anything malformed is dropped, not guessed. */
const str = (v: unknown) => (typeof v === 'string' ? v : '')

/** Items with every field they need; anything else dropped. */
export function sanitizeItems(items: unknown): CollectionItem[] {
  return (Array.isArray(items) ? items : []).flatMap((i): CollectionItem[] => {
    if (!i || typeof i !== 'object') return []
    const r = i as Record<string, unknown>
    if (![r.repo, r.branch, r.path, r.sha, r.name, r.type].every((v) => typeof v === 'string' && v)) return []
    return [{ repo: str(r.repo), branch: str(r.branch), path: str(r.path), sha: str(r.sha), name: str(r.name), type: str(r.type), ...(typeof r.size === 'number' ? { size: r.size } : {}) }]
  })
}

/** Recently viewed / exported assets: shown like collections, but kept by the app, not the user. */
export const RECENT_VIEWED = 'recent:viewed'
export const RECENT_EXPORTED = 'recent:exported'
export const RECENT_LIMIT = 30
export interface RecentLists {
  viewed: CollectionItem[]
  exported: CollectionItem[]
}

/** `items` to the front of `list` (newest first, each asset once, at most RECENT_LIMIT). */
export function pushRecent(list: readonly CollectionItem[], items: readonly CollectionItem[]): CollectionItem[] {
  const keys = new Set(items.map(keyOf))
  return [...items, ...list.filter((i) => !keys.has(keyOf(i)))].slice(0, RECENT_LIMIT)
}

export function sanitizeCollections(raw: unknown): Collection[] | undefined {
  if (!Array.isArray(raw)) return undefined
  const seen = new Set<string>()
  const out: Collection[] = []
  for (const c of raw) {
    if (!c || typeof c !== 'object') continue
    const { id, name, items } = c as Record<string, unknown>
    if (!str(id) || seen.has(str(id))) continue
    seen.add(str(id))
    out.push({ id: str(id), name: str(name), items: sanitizeItems(items) })
  }
  return out
}
