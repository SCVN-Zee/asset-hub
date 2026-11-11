import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from 'react'
import { useTranslation } from 'react-i18next'
import { displayName } from '@shared/displayName'
import type { Asset, AssetFacts, AssetSource, CatalogAsset, CatalogFacets, DependencyReport, FactKind, RepoRef, StatsSummary } from '@shared/types'
import { formatBytes } from '../format'
import type { HandlerRegistry } from '../handlers'
import { parse } from '../parse'
import { useSearch } from '../useSearch'
import { AssetCard, SkeletonCard } from '../components/AssetCard'
import { AssetDetails } from '../components/AssetDetails'
import { ExportBar, packCapacity, type ExportOptions, type ExportState, type PackItem } from '../components/ExportBar'
import { unwrap } from '../remoteSource'
import { useThumbnailQueue, type ThumbKind } from '../useThumbnailQueue'
import { Dropdown } from '../components/Dropdown'
import { Picker } from '../components/Picker'
import { ErrorNote } from '../components/ErrorNote'
import { FolderTree } from '../components/FolderTree'
import { CloseIcon, PinIcon, ExportListIcon, CollectionIcon, CopyIcon, ExportIcon, ExternalIcon, FilterIcon, FolderIcon, LinkIcon, MessageIcon, RefreshIcon, StarIcon, UnityIcon } from '../components/Icons'
import { useContextMenu, type MenuItem } from '../components/ContextMenu'
import { preferredEditor, rememberEditor, useUnityEditors } from '../components/UnityButton'
import { assetLink } from '@shared/deeplink'
import { FeedbackDialog } from '../components/FeedbackDialog'
import { VersionBadge } from '../components/VersionBadge'
import { SearchBox } from '../components/SearchBox'
import { DRAWABLE } from '../viewer/formats'
import { CatalogDetails, CatalogStatusLine, githubUrl, locationNote } from '../components/CatalogPanel'
import { useCatalog, type SearchScope } from '../useCatalog'
import type { BudgetPreset } from '@shared/types'
import { BUDGET_PRESETS, rankForPlayable, summaryIssues, type SortMode } from '../playable'
import { useBudget } from '../budget'
import { parseAssetLink } from '@shared/deeplink'
import type { ExportRequest, RunningUnity } from '@shared/ipc'
import { FX_THUMB_VERSION, motionKey } from '../viewer/particleThumb'
import { ANIM_THUMB_VERSION, FLAT_THUMB_VERSION, MODEL_THUMB_VERSION } from '../viewer/thumbVersions'
import { useSharedImports } from '../useSharedIndex'
import { Splitter, useStoredSize } from '../components/Splitter'
import { CollectionsNav, CollectionView } from '../components/Collections'
import { addCollection, deleteCollection, FAVORITES, inCollection, pushRecent, RECENT_EXPORTED, RECENT_VIEWED, renameCollection, toggleItem, withFavorites, type Collection, type CollectionItem, type RecentLists } from '@shared/collections'
import { MultiBar } from '../components/MultiBar'
import { ShortcutHelp } from '../components/ShortcutHelp'
import { Popover } from '../components/Popover'
import { SideGroup } from '../components/Section'
import { WatchNav } from '../components/WatchNav'
import type { WatchedBranch } from '@shared/settings'
import { useStoredFlag } from '../useStoredState'

/** Grid cell size: small (about 126 px) or large (about 180 px), kept between launches. */
type CellSize = 'small' | 'large'
const CELL_SIZE_KEY = 'assethub.gridSize'
const CELL_MIN: Record<CellSize, string> = { small: '120px', large: '180px' }

function useCellSize(): [CellSize, (size: CellSize) => void] {
  const [size, setSize] = useState<CellSize>(() => {
    try {
      return localStorage.getItem(CELL_SIZE_KEY) === 'large' ? 'large' : 'small'
    } catch {
      return 'small'
    }
  })
  const set = useCallback((next: CellSize) => {
    setSize(next)
    try {
      localStorage.setItem(CELL_SIZE_KEY, next)
    } catch {
      // kept for this session
    }
  }, [])
  return [size, set]
}

/** Two buttons at the right of the toolbar: more small cells, or fewer large ones. */
function CellSizeToggle({ value, onChange }: { value: CellSize; onChange(size: CellSize): void }) {
  const { t } = useTranslation()
  const paths: Record<CellSize, string> = {
    small: 'M1 1h3v3H1zM5.5 1h3v3h-3zM10 1h3v3h-3zM1 5.5h3v3H1zM5.5 5.5h3v3h-3zM10 5.5h3v3h-3zM1 10h3v3H1zM5.5 10h3v3h-3zM10 10h3v3h-3z',
    large: 'M1 1h5v5H1zM8 1h5v5H8zM1 8h5v5H1zM8 8h5v5H8z'
  }
  return (
    <div className="size-toggle" role="group" aria-label={t('browse.cellSize')}>
      {(['small', 'large'] as const).map((size) => (
        <button key={size} type="button" aria-pressed={value === size} aria-label={t(`browse.cell.${size}`)} title={t(`browse.cell.${size}`)} onClick={() => onChange(size)}>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor" aria-hidden="true">
            <path d={paths[size]} />
          </svg>
        </button>
      ))}
    </div>
  )
}

/** An empty list: what is missing, and what to try. */
function EmptyState({ title, hint }: { title: string; hint?: string }) {
  return (
    <div className="empty" role="status">
      <strong>{title}</strong>
      {hint && <span>{hint}</span>}
    </div>
  )
}

interface Props {
  source: AssetSource
  registry: HandlerRegistry
  excludedFolders: string[]
  /** Signed-in GitHub login, shown in the top bar. */
  login?: string
  /** Repo/branch to reopen when they still exist. */
  initialRepo?: string
  initialBranch?: string
  onOpenBranch(repo: string, branch: string): void
  onSettings(): void
  /** A panel (Settings) is open over the hub: its shortcuts stand down until it closes. */
  paused?: boolean
  /** Playable budget preset: switchable next to the ranking, like in Settings. */
  budgetPreset: BudgetPreset
  onBudgetPreset(preset: BudgetPreset): void
  /** Kept in settings: it also applies to exports started outside the export bar, and survives restarts. */
  exportLayout: ExportOptions['layout']
  onExportLayout(layout: ExportOptions['layout']): void
  /** Unity project exports are compared with (settings), if any. */
  targetProject?: string
  /** Favourites and named collections (settings). */
  collections?: Collection[]
  onCollections(collections: Collection[]): void
  /** Branches followed for new assets, and saving them. */
  watched: WatchedBranch[]
  onWatched(watched: WatchedBranch[]): void
  /** Recently viewed / exported assets, and saving them. */
  recent?: RecentLists
  onRecent(recent: RecentLists): void
}

const ALL = 'all'
const NO_FACETS: CatalogFacets = { repos: [], branches: [] }
/** Cards rendered per step while scrolling. */
const PAGE = 60
/** Models bigger than this are only rendered when opened, not for grid thumbnails. */
const THUMB_MAX_BYTES = 40 * 1024 * 1024
/** Images are decoded on the page: very large PSDs would stall it, so they are only drawn when opened. */
const IMAGE_THUMB_MAX_BYTES = 24 * 1024 * 1024
/** Clips the browser decodes; their waveform is the thumbnail. */
const PLAYABLE_AUDIO = /\.(wav|mp3|ogg)$/i
const AUDIO_THUMB_MAX_BYTES = 24 * 1024 * 1024
/** CJK fonts run to tens of MB: their cards keep the icon until opened. */
const FONT_THUMB_MAX_BYTES = 20 * 1024 * 1024
/** Unity project folder of a repo path (the part before "Assets/"), for textures looked up next to it. */
/** Quick filters above the grid. */
const QUICK_CHIPS = ['small', 'lowPoly', 'animated'] as const
type QuickChip = (typeof QUICK_CHIPS)[number]

const projectRootOf = (path: string) => {
  const i = path.indexOf('Assets/')
  return i < 0 ? '' : path.slice(0, i)
}
const ARROWS = new Set(['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'])

/** PNG preview for Unity's import dialog, from a cached thumbnail data URL. */
async function thumbToPng(src: string, size = 128): Promise<Uint8Array | undefined> {
  try {
    const img = new Image()
    // Thumbnails from the cache come from their own scheme: drawn without tainting the canvas.
    img.crossOrigin = 'anonymous'
    img.src = src
    await img.decode()
    const canvas = document.createElement('canvas')
    canvas.width = canvas.height = size
    const scale = Math.max(size / img.width, size / img.height)
    canvas.getContext('2d')!.drawImage(img, (size - img.width * scale) / 2, (size - img.height * scale) / 2, img.width * scale, img.height * scale)
    const blob = await new Promise<Blob | null>((r) => canvas.toBlob(r, 'image/png'))
    return blob ? new Uint8Array(await blob.arrayBuffer()) : undefined
  } catch {
    return undefined
  }
}

/** 'owner/repo' → 'owner/' */
const ownerPrefix = (fullName: string) => fullName.slice(0, fullName.indexOf('/') + 1)
/** 'vn/net-catch' → 'vn/'; branches without a slash are ungrouped. */
const branchPrefix = (name: string) => (name.includes('/') ? name.slice(0, name.indexOf('/') + 1) : '')

type Load<T> = { status: 'loading' } | { status: 'error'; error: unknown } | { status: 'ready'; data: T }

export function Browse({
  source,
  registry,
  excludedFolders,
  login,
  initialRepo,
  initialBranch,
  onOpenBranch,
  onSettings,
  paused = false,
  budgetPreset,
  onBudgetPreset,
  exportLayout,
  onExportLayout,
  targetProject,
  collections: storedCollections,
  recent,
  onRecent,
  watched,
  onWatched,
  onCollections
}: Props) {
  const { t } = useTranslation()
  // Collections: one may be open in the grid instead of the branch's folders.
  const collections = withFavorites(storedCollections)
  const [openCollection, setOpenCollection] = useState<string>()
  const recentLists: Collection[] = [
    { id: RECENT_VIEWED, name: '', items: recent?.viewed ?? [] },
    { id: RECENT_EXPORTED, name: '', items: recent?.exported ?? [] }
  ]
  // A recent list keeps the order it had when opened: an asset opened from it moves to the top next time, not under the pointer.
  const [frozenRecent, setFrozenRecent] = useState<Collection>()
  useEffect(() => {
    setFrozenRecent(recentLists.find((c) => c.id === openCollection))
    // Taken once per opening; later views must not reorder it.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openCollection])
  const shownCollection = frozenRecent?.id === openCollection ? frozenRecent : [...collections, ...recentLists].find((c) => c.id === openCollection)
  /** Remembers assets in a recent list (the latest settings are read at the time). */
  const recentRef = useRef(recent)
  recentRef.current = recent
  const remember = (list: keyof RecentLists, items: CollectionItem[]) => {
    if (!items.length) return
    const now = recentRef.current ?? { viewed: [], exported: [] }
    onRecent({ ...now, [list]: pushRecent(now[list], items) })
  }
  const favorites = new Set(collections[0].items.map((i) => `${i.repo}\n${i.path}`))
  // Detail panel width and preview height, as each person drags them (kept on this machine).
  const [detailsWidth, setDetailsWidth] = useStoredSize('assethub.detailsWidth')
  const [cellSize, setCellSize] = useCellSize()
  const [viewerHeight, setViewerHeight] = useStoredSize('assethub.viewerHeight')
  const budget = useBudget()
  const [repos, setRepos] = useState<Load<RepoRef[]>>({ status: 'loading' })
  // The last repo and branch open straight away; the repo and branch lists, which take a round trip
  // to GitHub each, fill in the pickers meanwhile.
  const resume = !!initialRepo && !!initialBranch
  const [repo, setRepo] = useState(resume ? initialRepo! : '')
  const [branches, setBranches] = useState<string[]>([])
  const [branch, setBranch] = useState(resume ? initialBranch! : '')
  /** Repo the current branch was listed for; until it matches `repo`, the branch is stale. */
  const [branchRepo, setBranchRepo] = useState(resume ? initialRepo! : '')
  /** The branch's Unity project roots, once its assets are listed. */
  const [tree, setTree] = useState<Load<{ roots: string[] }>>({ status: 'loading' })
  /** The project's assets (without the excluded folders) and how many those folders held back. */
  const [listing, setListing] = useState<{ assets: Asset[]; hidden: number }>({ assets: [], hidden: 0 })
  const [reloadKey, setReloadKey] = useState(0)
  const [root, setRoot] = useState('')
  const [typeFilter, setTypeFilter] = useState(ALL)
  const [folder, setFolder] = useState('')
  const [query, setQuery] = useState('')
  const [scope, setScope] = useState<SearchScope>('branch')
  /** Selected result in the other-branches / other-repos views (its blob SHA). */
  const [catalogSel, setCatalogSel] = useState<string>()
  /** Where to land once a branch opened from another view has loaded. */
  const nav = useRef<{ repo: string; branch: string; path: string }>()
  const [selectedId, setSelectedId] = useState<string>()
  const [thumbs, setThumbs] = useState<Record<string, string>>({})
  /** Model measurements by blob SHA, for ranking. */
  const [modelStats, setModelStats] = useState<Record<string, StatsSummary>>({})
  const [sortMode, setSortMode] = useState<SortMode>('relevance')
  // Ticks kept between launches (Filters and export-list options).
  const [onlyFits, setOnlyFits] = useStoredFlag('assethub.filter.onlyFits', false)
  /** Saved thumbnails are known; only then is it worth rendering missing ones. */
  const [thumbsLoaded, setThumbsLoaded] = useState(false)
  const [mergeCopies, setMergeCopies] = useStoredFlag('assethub.filter.mergeCopies', true)
  const [limit, setLimit] = useState(PAGE)
  const sentinel = useRef<HTMLDivElement>(null)
  const [pack, setPack] = useState<PackItem[]>([])
  const [skipByName, setSkipByName] = useStoredFlag('assethub.chest.skipByName', false)
  const [reuseExisting, setReuseExisting] = useStoredFlag('assethub.chest.reuseExisting', true)
  const [stripScripts, setStripScripts] = useStoredFlag('assethub.chest.stripScripts', true)
  const exportOptions: ExportOptions = { layout: exportLayout, skipByName, targetProject, reuseExisting, stripScripts }
  const [exportState, setExportState] = useState<ExportState>({ status: 'idle' })
  const reports = useRef(new Map<string, DependencyReport>())
  /** Materials found on prefabs, by model path: exported along with the model. */
  const prefabFiles = useRef(new Map<string, string[]>())

  useEffect(
    () => window.api.exporter.onProgress((progress) => setExportState((s) => (s.status === 'running' ? { status: 'running', progress } : s))),
    []
  )
  // A pack belongs to one repo + branch + project (its paths are relative to the project root).
  useEffect(() => {
    setPack([])
    reports.current.clear()
    prefabFiles.current.clear()
  }, [repo, branch, root])

  useEffect(() => {
    let cancelled = false
    setRepos({ status: 'loading' })
    source.listRepos().then(
      (list) => {
        if (cancelled) return
        setRepos({ status: 'ready', data: list })
        const has = (name?: string) => !!name && list.some((r) => r.fullName === name)
        setRepo((current) => (has(current) ? current : has(initialRepo) ? initialRepo! : (list[0]?.fullName ?? '')))
      },
      (error: unknown) => !cancelled && setRepos({ status: 'error', error })
    )
    return () => {
      cancelled = true
    }
  }, [source])

  const repoList = repos.status === 'ready' ? repos.data : []
  const repoNames = useMemo(() => repoList.map((r) => r.fullName), [repoList])
  const defaultBranch = repoList.find((r) => r.fullName === repo)?.defaultBranch

  useEffect(() => {
    if (!repo) return
    let cancelled = false
    // A branch of this repo already open (the last one, resumed): it stays while the list loads.
    const shown = repo === branchRepo ? branch : ''
    if (!shown) {
      setBranches([])
      setBranch('')
      setTree({ status: 'loading' })
    }
    source.listBranches(repo).then(
      (list) => {
        if (cancelled) return
        setBranches(list)
        setBranchRepo(repo)
        const preferred = repoList.find((r) => r.fullName === repo)?.defaultBranch
        const wanted = nav.current?.repo === repo ? nav.current.branch : undefined
        const next =
          wanted && list.includes(wanted)
            ? wanted
            : repo === initialRepo && initialBranch && list.includes(initialBranch)
              ? initialBranch
              : preferred && list.includes(preferred)
                ? preferred
                : (list[0] ?? '')
        if (next !== shown) setBranch(next)
      },
      (error: unknown) => !cancelled && setTree({ status: 'error', error })
    )
    return () => {
      cancelled = true
    }
    // repoList is read only for the default branch, and the open branch only on a repo change; neither retriggers branch loading.
  }, [source, repo])

  /** The branch's project and excluded folders the listing on screen was made for. */
  const listedFor = useRef('')
  /** The repo, branch and refresh the tree on screen belongs to (a project or excluded-folder change keeps it). */
  const opened = useRef('')
  const excludedKey = excludedFolders.join('\n')
  useEffect(() => {
    if (!repo || !branch || branchRepo !== repo) return
    const openKey = `${repo}\n${branch}\n${reloadKey}`
    const fresh = opened.current !== openKey
    const key = `${openKey}\n${root}\n${excludedKey}`
    if (!fresh && listedFor.current === key) return
    if (fresh) {
      onOpenBranch(repo, branch)
      setTree({ status: 'loading' })
      setListing({ assets: [], hidden: 0 })
    }
    let cancelled = false
    const at = { repo, branch }
    // The project in use: the open one, or the branch's first.
    parse.browseAssets(at, root || undefined, excludedFolders).then(
      (browsed) => {
        if (cancelled) return
        const chosen = browsed.root ?? ''
        opened.current = openKey
        listedFor.current = `${openKey}\n${chosen}\n${excludedKey}`
        setListing({ assets: browsed.assets, hidden: browsed.hidden })
        setTree({ status: 'ready', data: { roots: browsed.roots } })
        if (chosen !== root) setRoot(chosen)
        if (!fresh) return
        setFolder('')
        setSelectedId(undefined)
        if (nav.current?.repo === repo && nav.current.branch === branch) {
          // Opening a branch without a file (a watched branch) only picks the branch.
          if (nav.current.path) land(browsed.roots, nav.current.path)
          else nav.current = undefined
        }
      },
      (error: unknown) => !cancelled && setTree({ status: 'error', error })
    )
    return () => {
      cancelled = true
    }
    // excludedFolders is read through its key; `land` and `onOpenBranch` only use setters and refs.
  }, [source, repo, branch, reloadKey, branchRepo, root, excludedKey])

  const roots = tree.status === 'ready' ? tree.data.roots : []
  const assets = listing.assets
  const hiddenCount = listing.hidden
  // Clip names of measured models, keyed so the search is rebuilt only when they change (not on every thumbnail).
  const clipKey = Object.entries(modelStats)
    .filter(([, s]) => s.clips > 0 && s.clipNames)
    .map(([sha, s]) => `${sha}:${s.clipNames!.join(',')}`)
    .sort()
    .join('|')
  const clipNames = useMemo(() => new Map(Object.entries(modelStats).flatMap(([sha, s]) => (s.clipNames ? [[sha, s.clipNames] as const] : []))), [clipKey])
  /** Shas of models known to hold clips, for tabs that list them (`alsoShows`). */
  /** Shown anyway: assets their handler leaves out of lists (clips no model was found for). */
  const [showUnlisted, setShowUnlisted] = useStoredFlag('assethub.filter.showUnlisted', false)
  const unlistedKey = useMemo(() => Object.entries(modelStats).filter(([, s]) => s.rigged === false).map(([sha]) => sha).sort().join('|'), [modelStats])
  const withClips = useMemo(() => Object.entries(modelStats).filter(([, s]) => s.clips > 0).map(([sha]) => sha).sort().join('|'), [modelStats])
  /** Shas measured as effects or not (particles read), for the effects tab. */
  const withParticles = useMemo(() => Object.entries(modelStats).filter(([, s]) => s.particles !== undefined).map(([sha, s]) => `${sha}:${s.emitters ?? 0}`).sort().join('|'), [modelStats])
  /** Assets known by reading them (effect prefabs, sprites, TextMeshPro fonts), by blob SHA, from every branch scanned so far. */
  const [known, setKnown] = useState<AssetFacts>({})
  const [scan, setScan] = useState<{ kind: FactKind; status: 'scanning' | 'error'; done: number; total: number }>()
  /** Scans started (or done), as "kind:repo@branch". */
  const scans = useRef(new Set<string>())
  /** In the open tab: its own type, or another type the tab's handler also lists. */
  const listed = useCallback((a: Asset) => showUnlisted || !registry.get(a.type)?.unlisted?.(modelStats[a.sha]), [registry, showUnlisted, unlistedKey])
  const inTab = useCallback(
    (a: Asset, tab: string) =>
      tab === ALL ? !registry.get(a.type)?.hidden && listed(a) : (a.type === tab && listed(a)) || !!registry.get(tab)?.alsoShows?.(a, modelStats[a.sha], known),
    // modelStats matters only through which models hold clips, which prefabs are effects and which clips have no model.
    [registry, withClips, withParticles, known, listed]
  )
  /** In the open tab but left out (see `listed`), for the offer to show them. */
  const unlistedCount = useMemo(
    () => (showUnlisted || !unlistedKey ? 0 : assets.filter((a) => registry.get(a.type)?.unlisted?.(modelStats[a.sha]) && (typeFilter === ALL || a.type === typeFilter)).length),
    [assets, registry, typeFilter, showUnlisted, unlistedKey]
  )
  /** Every model of the branch, as repo paths (rigs to play clips on). */
  const modelPaths = useMemo(() => assets.filter((a) => registry.get(a.type)?.preview.kind === 'three').map((a) => root + a.path), [assets, registry, root])
  const ofType = useMemo(() => (typeFilter === ALL ? assets : assets.filter((a) => inTab(a, typeFilter))), [assets, typeFilter, inTab])
  // Matches groups (fish → shark), typos and folder names, best matches first (done in Rust, so answers follow the typing).
  const { found, folders: folderRoot, suggest: suggestHere } = useSearch(ofType, clipNames, query, folder)
  const visible = found.assets
  // Same blob SHA = byte-identical file; with merging on, one card stands for all copies.
  const copiesBySha = useMemo(() => {
    const map = new Map<string, Asset[]>()
    for (const a of ofType) map.set(a.sha, [...(map.get(a.sha) ?? []), a])
    return map
  }, [ofType])
  const shown = useMemo(() => {
    if (!mergeCopies) return visible
    const seen = new Set<string>()
    return visible.filter((a) => !seen.has(a.sha) && (seen.add(a.sha), true))
  }, [visible, mergeCopies])
  const merged = visible.length - shown.length
  const selected = visible.find((a) => a.id === selectedId)

  const [catalogRepo, setCatalogRepo] = useState('')
  const [catalogBranch, setCatalogBranch] = useState('')
  useEffect(() => {
    setCatalogRepo('')
    setCatalogBranch('')
  }, [scope])
  // Filtered by repo and branch where the catalog is: every repo's results are too many to send whole.
  const catalogFilter = useMemo(() => ({ repo: catalogRepo, branch: catalogBranch }), [catalogRepo, catalogBranch])
  const catalog = useCatalog(scope, repo, branch, excludedFolders, true, catalogFilter)
  /** Repos and branches present in the results, most files first, for the filters. */
  const catalogFacets = catalog.facets ?? NO_FACETS
  const catalogOfType = useMemo(() => (catalog.assets ?? []).filter((a) => inTab(a, typeFilter)), [catalog.assets, typeFilter, inTab])
  const { found: catalogFound, suggest: suggestCatalog } = useSearch(catalogOfType, clipNames, query, '')
  const catalogShown = catalogFound.assets
  const catalogSelected = catalogShown.find((a) => a.sha === catalogSel)
  const inCatalog = scope !== 'branch'
  /** Cards in the grid, whichever view is open. */
  // Ordered and filtered for reuse in playables (measurements come from thumbnails and opened models).
  // Watched branches: what was added since each was last marked seen (checked on opening, every 30 minutes, on coming back).
  const [news, setNews] = useState<Record<string, { head: string; added: string[]; more: boolean }>>({})
  const watchKey = (r: string, b: string) => `${r}\n${b}`
  const watchedKey = watched.map((w) => `${watchKey(w.repo, w.branch)}\n${w.seen}`).join('|')
  const lastCheck = useRef(0)
  useEffect(() => {
    const check = () => {
      lastCheck.current = Date.now()
      for (const w of watched)
        void window.api.source.watchCheck(w.repo, w.branch, w.seen).then((r) => r.ok && setNews((n) => ({ ...n, [watchKey(w.repo, w.branch)]: r.data })))
    }
    check()
    const timer = window.setInterval(check, 30 * 60_000)
    const onFocus = () => Date.now() - lastCheck.current > 5 * 60_000 && check()
    window.addEventListener('focus', onFocus)
    return () => {
      window.clearInterval(timer)
      window.removeEventListener('focus', onFocus)
    }
  }, [watchedKey])
  const watchingHere = watched.some((w) => w.repo === repo && w.branch === branch)
  const newHere = useMemo(() => new Set(watchingHere ? (news[watchKey(repo, branch)]?.added ?? []) : []), [news, repo, branch, watchingHere])
  const [newOnly, setNewOnly] = useState(false)
  useEffect(() => setNewOnly(false), [repo, branch])
  const toggleWatch = async () => {
    if (watchingHere) return onWatched(watched.filter((w) => !(w.repo === repo && w.branch === branch)))
    // From now on: whatever is on the branch today counts as seen.
    const r = await window.api.source.watchCheck(repo, branch, '')
    if (r.ok) onWatched([...watched, { repo, branch, seen: r.data.head }])
  }
  const markSeen = () => {
    const head = news[watchKey(repo, branch)]?.head
    if (!head) return
    onWatched(watched.map((w) => (w.repo === repo && w.branch === branch ? { ...w, seen: head } : w)))
    setNews((n) => ({ ...n, [watchKey(repo, branch)]: { head, added: [], more: false } }))
    setNewOnly(false)
  }
  /** Opens a branch (no file), e.g. a watched one from the sidebar. */
  const openBranch = (toRepo: string, toBranch: string) => {
    setScope('branch')
    setOpenCollection(undefined)
    if (toRepo !== repo) {
      nav.current = { repo: toRepo, branch: toBranch, path: '' }
      setRepo(toRepo)
    } else setBranch(toBranch)
  }

  /** Quick filters over the cards; those on measurements only keep measured assets. */
  const [chips, setChips] = useState<QuickChip[]>([])
  const toggleChip = (c: QuickChip) => setChips((on) => (on.includes(c) ? on.filter((x) => x !== c) : [...on, c]))
  /** Filters changed from the defaults, counted on the Filters button. */
  const activeFilters = chips.length + (budget ? 1 : 0) + (onlyFits && budget ? 1 : 0) + (mergeCopies ? 0 : 1) + (showUnlisted ? 1 : 0)
  const passesChips = useCallback(
    (a: Asset) => {
      if (newOnly && !newHere.has(root + a.path)) return false
      const s = modelStats[a.sha]
      return chips.every((c) =>
        c === 'small' ? (a.size ?? Infinity) <= 500 * 1024 : c === 'lowPoly' ? !!s && s.triangles > 0 && s.triangles <= 5000 : a.type === 'animation' || (!!s && s.clips > 0)
      )
    },
    [chips, modelStats, newOnly, newHere, root]
  )
  const rankedShown = useMemo(
    () => rankForPlayable(
        (onlyFits && budget ? shown.filter((a) => modelStats[a.sha] && !summaryIssues(modelStats[a.sha], budget).length) : shown).filter(passesChips),
        sortMode === 'used' || (sortMode === 'playable' && !budget) ? 'relevance' : sortMode,
        (a) => modelStats[a.sha],
        undefined,
        (a) => found.scores.get(a.id) ?? 0,
        budget
      ),
    [shown, sortMode, onlyFits, modelStats, found, budget, passesChips]
  )
  const rankedCatalog = useMemo(
    () =>
      rankForPlayable(
        (onlyFits && budget ? catalogShown.filter((a) => modelStats[a.sha] && !summaryIssues(modelStats[a.sha], budget).length) : catalogShown).filter(passesChips),
        sortMode === 'playable' && !budget ? 'relevance' : sortMode,
        (a) => modelStats[a.sha],
        (a) => a.branchCount,
        (a) => catalogFound.scores.get(a.id) ?? 0,
        budget
      ),
    [catalogShown, sortMode, onlyFits, modelStats, catalogFound, budget, passesChips]
  )
  const gridItems: readonly Asset[] = inCatalog ? rankedCatalog : rankedShown
  /** The inspector column only exists while an asset is selected; otherwise the grid takes the width. */
  const hasSelection = inCatalog ? !!catalogSelected : !!selected
  const packIds = useMemo(() => new Set(pack.map((i) => i.asset.id)), [pack])
  const typeLabel = typeFilter === ALL ? t('browse.allAssets') : t(registry.get(typeFilter)?.labelKey ?? 'browse.allAssets')
  const gridTitle = inCatalog ? t(`catalog.title.${scope}`, { repo: repo.split('/').pop() }) : folder && typeFilter !== ALL ? `${folder} · ${typeLabel}` : folder || typeLabel
  const gridCount = inCatalog ? rankedCatalog.length : rankedShown.length
  const measuredCount = (inCatalog ? catalogShown : shown).filter((a) => modelStats[a.sha]).length
  const unmeasured = (inCatalog ? catalogShown : shown).filter((a) => !modelStats[a.sha])
  const gridSelected = inCatalog ? catalogSel : selectedId
  const setGridSelected = inCatalog ? setCatalogSel : setSelectedId
  const searchFound = inCatalog ? catalogFound : found

  // Saved thumbnails of results from other branches (same content = same thumbnail).
  useEffect(() => {
    if (!catalog.assets?.length) return
    let cancelled = false
    void window.api.thumbs.get(catalog.assets.map((a) => a.sha)).then((more) => !cancelled && setThumbs((prev) => ({ ...more, ...prev })))
    void window.api.stats.get(catalog.assets.map((a) => a.sha)).then((more) => !cancelled && setModelStats((prev) => ({ ...more, ...prev })))
    return () => {
      cancelled = true
    }
  }, [catalog.assets])

  /** Selects `repoPath` in the loaded branch: picks its Unity project and clears filters that would hide it. */
  function land(roots: readonly string[], repoPath: string) {
    nav.current = undefined
    const projectRoot = [...roots].reverse().find((r) => repoPath.startsWith(r + 'Assets/')) ?? ''
    setRoot(projectRoot)
    setFolder('')
    setQuery('')
    setTypeFilter(ALL)
    setSelectedId(repoPath.slice(projectRoot.length))
  }

  /** Where to read a result from for its thumbnail: the filtered repo / branch when set, else its main location. */
  const catalogSource = (a: CatalogAsset) => {
    const at = a.locations.find((l) => !catalogRepo || l.repo === catalogRepo) ?? a.locations[0]
    const atBranch = catalogBranch && at.branches.includes(catalogBranch) ? catalogBranch : at.branches[0]
    return { path: at.path, ctx: { repo: at.repo, branch: atBranch, root: projectRootOf(at.path) } }
  }
  const previewKind = (a: Asset) => registry.get(a.type)?.preview.kind
  /** Thumbnails: models up to THUMB_MAX_BYTES, images the browser (or our PSD / TGA decoders) can draw. */
  const canRender = (a: Asset) =>
    (previewKind(a) === 'three' && (a.size ?? 0) <= THUMB_MAX_BYTES) ||
    previewKind(a) === 'material' ||
    previewKind(a) === 'prefab' ||
    previewKind(a) === 'animation' ||
    (previewKind(a) === 'font' && (a.size ?? 0) <= FONT_THUMB_MAX_BYTES) ||
    (previewKind(a) === 'tmpfont' && (a.size ?? 0) <= FONT_THUMB_MAX_BYTES) ||
    (previewKind(a) === 'image' && DRAWABLE.test(a.path) && (a.size ?? 0) <= IMAGE_THUMB_MAX_BYTES) ||
    (previewKind(a) === 'audio' && PLAYABLE_AUDIO.test(a.path) && (a.size ?? 0) <= AUDIO_THUMB_MAX_BYTES)
  const kindOf = (a: Asset): ThumbKind => {
    const kind = previewKind(a)
    return kind === 'image' || kind === 'material' || kind === 'audio' || kind === 'prefab' || kind === 'animation' || kind === 'font' || kind === 'tmpfont' ? kind : 'three'
  }

  /** Cards whose thumbnail is ready, out of those that can have one (huge or EXR files never do). */
  const renderable = gridItems.filter(canRender)
  const loadedCount = renderable.filter((a) => thumbs[a.sha]).length

  /** Prefab cards drawn without numbers (effects as a plain "FX" before particles were read), or effect / clip cards drawn the old way: tried once per session. */
  const unmeasuredPrefabs = useRef(new Set<string>())
  /** An effect card older than the current way of drawing them (textures piled up, one frame). */
  const staleEffect = (a: Asset) => kindOf(a) === 'prefab' && !!modelStats[a.sha]?.particles && modelStats[a.sha]?.fxThumb !== FX_THUMB_VERSION && !unmeasuredPrefabs.current.has(a.sha)
  /** A clip card drawn before the rig search last improved (shown as curves when no rig was found then). */
  const staleClip = (a: Asset) => kindOf(a) === 'animation' && modelStats[a.sha]?.animThumb !== ANIM_THUMB_VERSION && !unmeasuredPrefabs.current.has(a.sha)
  /** A model, prefab, material or texture card drawn before the last change to how they are drawn (some came out empty, others on a dark background). */
  const staleModel = (a: Asset) => ['three', 'prefab', 'material', 'image'].includes(kindOf(a)) && !!modelStats[a.sha] && modelStats[a.sha].modelThumb !== MODEL_THUMB_VERSION && !unmeasuredPrefabs.current.has(a.sha)
  /** An audio, font or TMP font card still drawn on its old dark fill, not transparent on the grid's stage. */
  const staleFlat = (a: Asset) => ['audio', 'font', 'tmpfont'].includes(kindOf(a)) && !!modelStats[a.sha] && modelStats[a.sha].modelThumb !== FLAT_THUMB_VERSION && !unmeasuredPrefabs.current.has(a.sha)
  const remeasure = (a: Asset) => {
    const s = modelStats[a.sha]
    if (!s) return kindOf(a) === 'prefab' && !!thumbs[a.sha] && !unmeasuredPrefabs.current.has(a.sha)
    // Results of other branches / repos are drawn with what is known (no index per branch): not again
    // here, or every one would build its branch's index; in full once their branch is opened.
    // A clip card has no clip names to wait for: its measurement is complete once drawn.
    return (s.clips > 0 && !s.clipNames && kindOf(a) !== 'animation') || (kindOf(a) === 'three' && s.source === 'thumb' && !s.matTextures && !inCatalog) || staleEffect(a) || staleClip(a) || staleModel(a) || staleFlat(a)
  }

  /** Queues a card's thumbnail, read from the open branch or (for results elsewhere) its main location. */
  const requestThumb = (a: Asset, order: number) => {
    if (inCatalog) {
      const src = catalogSource(a as CatalogAsset)
      thumbQueue.request(a.sha, src.path, order, src.ctx, kindOf(a))
    } else thumbQueue.request(a.sha, root + a.path, order, undefined, kindOf(a))
  }

  /** Renders (and so measures) every listed model not measured yet, top of the list first. */
  const measureAll = () => {
    unmeasured.forEach((a, i) => {
      if (canRender(a)) requestThumb(a, 10_000 + i)
    })
  }

  /** Exports a result from another branch / repo as it is on its main branch, without opening that branch. */
  const exportFromCatalog = async (a: CatalogAsset, unity?: RunningUnity) => {
    const at = a.locations[0]
    setExportState({ status: 'running' })
    const thumb = thumbs[a.sha]
    const png = thumb ? await thumbToPng(thumb) : undefined
    try {
      const result = await sendPackage(
        {
          repo: at.repo,
          branch: at.branches[0],
          root: projectRootOf(at.path),
          paths: [at.path],
          ...exportOptions,
          extraFiles: [],
          previews: png ? { [at.path]: png } : {},
          suggestedName: a.name.replace(/\.[^.]+$/, '')
        },
        unity
      )
      setExportState(result.canceled ? { status: 'idle' } : { status: 'done', result })
      if (!result.canceled) remember('exported', [catalogItem(a)])
    } catch (error) {
      setExportState({ status: 'error', error })
    }
  }

  /** Opens a file found in another branch or repo there, in the normal branch view. */
  const openLocation = (toRepo: string, toBranch: string, path: string) => {
    setScope('branch')
    nav.current = { repo: toRepo, branch: toBranch, path }
    if (toRepo !== repo) setRepo(toRepo)
    else if (toBranch !== branch) setBranch(toBranch)
    else if (tree.status === 'ready') land(tree.data.roots, path)
  }

  // assethub:// links open the asset they name: the link that launched the app, links clicked
  // while it runs, and links pasted into the search box.
  const openLocationRef = useRef(openLocation)
  openLocationRef.current = openLocation
  useEffect(() => {
    const take = async () => {
      const url = await window.api.takeDeepLink()
      const link = url ? parseAssetLink(url) : undefined
      if (link) openLocationRef.current(link.repo, link.branch, link.path)
    }
    void take()
    return window.api.onDeepLink(() => void take())
  }, [])
  const onQuery = (q: string) => {
    const link = parseAssetLink(q)
    if (!link) {
      setOpenCollection(undefined)
      return setQuery(q)
    }
    setQuery('')
    openLocation(link.repo, link.branch, link.path)
  }

  // Back to the first page only when the user asks for a different list, not when
  // measurements arrive and re-rank it (that used to jump the grid back to the top).
  const listKey = [scope, repo, branch, root, folder, typeFilter, query, sortMode, onlyFits, mergeCopies, catalogRepo, catalogBranch].join('\n')
  useEffect(() => setLimit(PAGE), [listKey])
  // Render more cards as the end of the grid scrolls into view.
  useEffect(() => {
    const el = sentinel.current
    if (!el) return
    const observer = new IntersectionObserver(([entry]) => entry.isIntersecting && setLimit((l) => l + PAGE), { rootMargin: '600px' })
    observer.observe(el)
    return () => observer.disconnect()
  }, [gridItems, limit])

  // Arrow keys move the selection through the grid (Up/Down jump a row).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (paused || !ARROWS.has(e.key) || e.altKey || e.metaKey || e.ctrlKey || !gridItems.length) return
      const target = e.target as HTMLElement
      if (target.closest('input, select, textarea, .picker-pop, .viewer')) return
      const grid = document.querySelector('.grid')
      const cols = grid ? getComputedStyle(grid).gridTemplateColumns.split(' ').length : 1
      const index = gridItems.findIndex((a) => a.id === gridSelected)
      const step = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : e.key === 'ArrowUp' ? -cols : cols
      const next = index < 0 ? 0 : Math.min(gridItems.length - 1, Math.max(0, index + step))
      e.preventDefault()
      setGridSelected(gridItems[next].id)
      if (next >= limit) setLimit(next + PAGE)
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [gridItems, gridSelected, limit, inCatalog, paused])

  /** Selection still to be scrolled into view; scrolling happens only when the selection changes. */
  const scrollTo = useRef<string>()
  useEffect(() => {
    scrollTo.current = gridSelected
  }, [gridSelected])
  useEffect(() => {
    const wanted = scrollTo.current
    if (!wanted) return
    // A card further down than the cards rendered so far (e.g. opened from another branch): render up to it first.
    const target = gridItems.find((a) => a.id === wanted) ?? (selected && gridItems.find((a) => a.sha === selected.sha))
    const index = target ? gridItems.indexOf(target) : -1
    if (index >= limit) {
      setLimit(index + PAGE)
      return
    }
    scrollTo.current = undefined
    const card = target && document.querySelector<HTMLElement>(`[data-asset-id="${CSS.escape(target.id)}"]`)
    card?.scrollIntoView({ block: 'nearest' })
    if (document.activeElement?.classList.contains('asset-card')) card?.focus({ preventScroll: true })
  }, [gridSelected, gridItems, limit])
/** An asset of this branch as a collection keeps it: where it is, so it opens there again. */
  const itemOf = (a: Asset): CollectionItem => ({ repo, branch, path: root + a.path, sha: a.sha, name: a.name, type: a.type, ...(a.size !== undefined ? { size: a.size } : {}) })
  /** A result of another branch / repo, where it lives first. */
  const catalogItem = (a: CatalogAsset): CollectionItem => {
    const at = a.locations[0]
    return { repo: at.repo, branch: at.branches[0], path: at.path, sha: a.sha, name: a.name, type: a.type, ...(a.size !== undefined ? { size: a.size } : {}) }
  }
  const live = useMemo(
    () => (!repo || !branch || branchRepo !== repo ? undefined : { repo, branch, root }),
    [source, repo, branch, branchRepo, root]
  )

  // Tabs that list assets by what is inside them (VFX, Sprite, TextMeshPro fonts) need the branch's files read once
  // (kept by SHA, shared by all branches): started the first time such a tab is opened, finished even if the user moves on.
  useEffect(() => {
    const kind = registry.get(typeFilter)?.facts
    if (!kind || !live) return
    const key = `${kind}:${live.repo}@${live.branch}`
    if (scans.current.has(key)) return
    scans.current.add(key)
    const add = (shas: readonly string[]) => setKnown((prev) => ({ ...prev, [kind]: new Set([...(prev[kind] ?? []), ...shas]) }))
    setScan({ kind, status: 'scanning', done: 0, total: 0 })
    const off = window.api.assets.onKnownProgress((k, { found, ...p }) => {
      if (k !== kind) return
      setScan((s) => (s?.kind === kind && s.status === 'scanning' ? { ...s, ...p } : s))
      // Assets appear as they are found, not only when every file has been read.
      if (found?.length) add(found)
    })
    void window.api.assets
      .knownAssets(live.repo, live.branch, kind)
      .then(unwrap)
      .then(
        (shas) => {
          add(shas)
          setScan((s) => (s?.kind === kind ? undefined : s))
        },
        () => {
          // Tried again the next time the tab is opened.
          scans.current.delete(key)
          setScan({ kind, status: 'error', done: 0, total: 0 })
        }
      )
      .finally(off)
  }, [typeFilter, live, registry])

  const sharedImports = useSharedImports()
  // Saved thumbnails and measurements of an open collection's assets, from any repo or branch (by content).
  const collectionShas = shownCollection?.items.map((i) => i.sha).join(',') ?? ''
  useEffect(() => {
    if (!collectionShas) return
    let cancelled = false
    const keys = collectionShas.split(',')
    void window.api.thumbs.get(keys).then((more) => !cancelled && setThumbs((prev) => ({ ...more, ...prev })), () => undefined)
    void window.api.stats.get(keys).then((more) => !cancelled && setModelStats((prev) => ({ ...more, ...prev })), () => undefined)
    return () => {
      cancelled = true
    }
  }, [collectionShas, thumbsLoaded])
  // Thumbnails rendered earlier, keyed by the model's content hash.
  useEffect(() => {
    setThumbsLoaded(false)
    if (!assets.length) return
    let cancelled = false
    const keys = assets.map((a) => a.sha)
    // Both known before rendering starts, so nothing is rendered or measured twice.
    void Promise.all([window.api.thumbs.get(keys).catch(() => ({})), window.api.stats.get(keys).catch(() => ({}))]).then(([found, measured]) => {
      if (cancelled) return
      setThumbs(found)
      setModelStats((prev) => ({ ...prev, ...measured }))
      setThumbsLoaded(true)
    })
    return () => {
      cancelled = true
    }
    // Read again when a teammate's index brought thumbnails and numbers in.
  }, [source, assets, sharedImports])

  /** Saves a package, or with `unity` opens it in that editor (its import dialog). */
  const sendPackage = async (request: ExportRequest, unity?: RunningUnity) =>
    unwrap(await (unity ? window.api.exporter.importToUnity(request, unity.pid) : window.api.exporter.unityPackage(request)))
  const runExport = async (items: PackItem[], unity?: RunningUnity) => {
    if (!live || !items.length) return
    setExportState({ status: 'running' })
    const previews: Record<string, Uint8Array> = {}
    for (const item of items) {
      const thumb = thumbs[item.asset.sha]
      const png = thumb ? await thumbToPng(thumb) : undefined
      if (png) previews[item.path] = png
    }
    const suggestedName = items.length === 1 ? items[0].asset.name.replace(/\.[^.]+$/, '') : `${live.repo.split('/').pop()}-${items.length}-assets`
    try {
      const result = await sendPackage({ ...live, paths: items.map((i) => i.path), ...exportOptions, extraFiles: items.flatMap((i) => prefabFiles.current.get(i.path) ?? []), previews, suggestedName }, unity)
      setExportState(result.canceled ? { status: 'idle' } : { status: 'done', result })
      if (!result.canceled) remember('exported', items.map((i) => itemOf(i.asset)))
    } catch (error) {
      setExportState({ status: 'error', error })
    }
  }

  const packItem = (asset: Asset): PackItem => ({ asset, path: root + asset.path, report: reports.current.get(root + asset.path) })
  const togglePack = (asset: Asset) => {
    setPack((items) => (items.some((i) => i.asset.id === asset.id) ? items.filter((i) => i.asset.id !== asset.id) : [...items, packItem(asset)]))
  }
  /** Cards picked with Cmd/Ctrl- or Shift-click in the branch view, to act on several at once. */
  const [picked, setPicked] = useState<string[]>([])
  const pickAnchor = useRef<string>()
  const pickedAssets = useMemo(() => picked.map((id) => assets.find((a) => a.id === id)).filter((a): a is Asset => !!a), [picked, assets])
  // Another branch, repo or view: nothing stays picked.
  useEffect(() => setPicked([]), [repo, branch, root, scope])
  const pickCard = (a: Asset, how: { toggle: boolean; range: boolean }) => {
    if (how.range && pickAnchor.current) {
      const ids = rankedShown.map((x) => x.id)
      const [from, to] = [ids.indexOf(pickAnchor.current), ids.indexOf(a.id)].sort((x, y) => x - y)
      if (from >= 0) setPicked((prev) => [...new Set([...prev, ...ids.slice(from, to + 1)])])
    } else if (how.toggle) {
      setPicked((prev) => {
        const base = prev.length ? prev : selectedId && selectedId !== a.id ? [selectedId] : []
        return base.includes(a.id) ? base.filter((id) => id !== a.id) : [...base, a.id]
      })
      pickAnchor.current = a.id
    } else {
      setPicked([])
      pickAnchor.current = a.id
    }
    setSelectedId(a.id)
  }

  // Right-click menus: on cards (one, or the cards picked), watched branches and folders.
  const contextMenu = useContextMenu()
  const unityEditors = useUnityEditors()
  const [notice, setNotice] = useState<string>()
  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(undefined), 1600)
    return () => window.clearTimeout(timer)
  }, [notice])
  const copy = (text: string) =>
    void navigator.clipboard.writeText(text).then(
      () => setNotice(t('menu.copied')),
      // A refused copy says so: silence would look like it worked.
      () => setNotice(t('inspector.copyFailed'))
    )
  /** "Import into Unity": straight into the one open editor (or the one used last), others in a submenu. */
  const unityItems = (run: (editor: (typeof unityEditors)[number]) => void, disabled: boolean): MenuItem[] => {
    if (!unityEditors.length) return [{ label: t('menu.noUnity'), icon: <UnityIcon size={14} />, disabled: true }]
    const go = (e: (typeof unityEditors)[number]) => {
      rememberEditor(e)
      run(e)
    }
    const first = preferredEditor(unityEditors)
    return [
      { label: t('unity.importInto', { project: first.project }), icon: <UnityIcon size={14} />, disabled, onSelect: () => go(first) },
      ...(unityEditors.length > 1
        ? [{ label: t('unity.pick'), disabled, submenu: unityEditors.map((e) => ({ label: e.project, hint: e.version, checked: e === first ? true : undefined, onSelect: () => go(e) })) }]
        : [])
    ]
  }
  const collectionSubmenu = (items: CollectionItem[]): MenuItem[] =>
    collections.map((c) => {
      const all = items.every((it) => inCollection(c, it))
      return {
        label: c.id === FAVORITES ? t('collections.favorites') : c.name,
        checked: all,
        onSelect: () => {
          let list = withFavorites(collections)
          // All in already: taken out; else the missing ones are added.
          for (const it of items) if (all || !inCollection(list.find((x) => x.id === c.id), it)) list = toggleItem(list, c.id, it)
          onCollections(list)
        }
      }
    })
  const busy = exportState.status === 'running'
  /** The grid goes to the folder that holds an asset (leaving a collection, clearing the search). */
  const showFolder = (dir: string) => {
    setOpenCollection(undefined)
    setFolder(dir)
    onQuery('')
  }
  const selectedDir = selected ? selected.path.split('/').slice(0, -1).join('/') : ''
  const cardMenu = (a: Asset, e: React.MouseEvent) => {
    const many = picked.length > 1 && picked.includes(a.id)
    if (many) {
      const list = pickedAssets
      return contextMenu.open(e, [
        { label: t('menu.many', { count: list.length }), disabled: true },
        '-',
        { label: t('actions.export'), icon: <ExportIcon size={14} />, disabled: busy, onSelect: () => void runExport(list.map(packItem)) },
        ...unityItems((editor) => void runExport(list.map(packItem), editor), busy),
        {
          label: t('menu.addToExport'),
          icon: <ExportListIcon size={14} />,
          onSelect: () => {
            const add = list.filter((x) => !pack.some((i) => i.asset.id === x.id))
            setPack((items) => [...items, ...add.map(packItem)])
          }
        },
        { label: t('menu.collection'), icon: <CollectionIcon size={14} />, submenu: collectionSubmenu(list.map(itemOf)) },
        '-',
        { label: t('menu.clearPick'), onSelect: () => setPicked([]) }
      ])
    }
    // Right-click selects the card, like a click.
    if (a.id !== selectedId) pickCard(a, { toggle: false, range: false })
    const path = root + a.path
    const item = itemOf(a)
    const inPack = pack.some((i) => i.asset.id === a.id)
    const starred = favorites.has(`${repo}\n${path}`)
    const file = registry.get(a.type)?.delivery === 'file'
    const dir = a.path.split('/').slice(0, -1).join('/')
    contextMenu.open(e, [
      ...(file
        ? []
        : ([
            { label: t('actions.export'), icon: <ExportIcon size={14} />, disabled: busy, onSelect: () => void runExport([packItem(a)]) },
            ...unityItems((editor) => void runExport([packItem(a)], editor), busy),
            { label: inPack ? t('menu.removeFromExport') : t('menu.addToExport'), icon: <ExportListIcon size={14} filled={inPack} />, onSelect: () => togglePack(a) },
            '-'
          ] as MenuItem[])),
      { label: starred ? t('menu.unfavorite') : t('menu.favorite'), icon: <StarIcon size={14} filled={starred} />, onSelect: () => onCollections(toggleItem(collections, FAVORITES, item)) },
      { label: t('menu.collection'), icon: <CollectionIcon size={14} />, submenu: collectionSubmenu([item]) },
      '-',
      { label: t('menu.copyPath'), icon: <CopyIcon size={14} />, onSelect: () => copy(path) },
      { label: t('menu.copyLink'), icon: <LinkIcon size={14} />, onSelect: () => copy(assetLink({ repo, branch, path })) },
      { label: t('menu.github'), icon: <ExternalIcon size={14} />, onSelect: () => void window.api.openExternal(githubUrl(repo, branch, path)) },
      ...(dir && dir !== folder
        ? ([
            {
              label: t('menu.showFolder'),
              icon: <FolderIcon size={14} />,
              hint: dir.split('/').pop(),
              onSelect: () => showFolder(dir)
            }
          ] as MenuItem[])
        : [])
    ])
  }
  const folderMenu = (dir: string, e: React.MouseEvent) =>
    contextMenu.open(e, [
      {
        label: t('menu.openFolder'),
        icon: <FolderIcon size={14} />,
        onSelect: () => {
          setOpenCollection(undefined)
          setFolder(dir)
        }
      },
      '-',
      { label: t('menu.copyPath'), icon: <CopyIcon size={14} />, onSelect: () => copy(root + dir) },
      {
        label: t('menu.github'),
        icon: <ExternalIcon size={14} />,
        onSelect: () => void window.api.openExternal(`https://github.com/${repo}/tree/${branch.split('/').map(encodeURIComponent).join('/')}/${(root + dir).split('/').map(encodeURIComponent).join('/')}`)
      }
    ])
  const catalogMenu = (a: CatalogAsset, e: React.MouseEvent) => {
    setCatalogSel(a.sha)
    const at = a.locations[0]
    const atBranch = at.branches[0]
    const file = registry.get(a.type)?.delivery === 'file'
    contextMenu.open(e, [
      { label: t('catalog.open', { branch: atBranch }), icon: <ExternalIcon size={14} />, onSelect: () => openLocation(at.repo, atBranch, at.path) },
      '-',
      ...(file
        ? []
        : ([
            { label: t('actions.export'), icon: <ExportIcon size={14} />, disabled: busy, onSelect: () => void exportFromCatalog(a) },
            ...unityItems((editor) => void exportFromCatalog(a, editor), busy),
            '-'
          ] as MenuItem[])),
      { label: t('menu.collection'), icon: <CollectionIcon size={14} />, submenu: collectionSubmenu([catalogItem(a)]) },
      { label: t('menu.copyPath'), icon: <CopyIcon size={14} />, onSelect: () => copy(at.path) },
      { label: t('menu.copyLink'), icon: <LinkIcon size={14} />, onSelect: () => copy(assetLink({ repo: at.repo, branch: atBranch, path: at.path })) },
      { label: t('menu.github'), icon: <ExternalIcon size={14} />, onSelect: () => void window.api.openExternal(githubUrl(at.repo, atBranch, at.path)) }
    ])
  }

  // Viewed: an asset kept open for a moment (not every card the arrow keys pass through).
  const viewedKey = inCatalog ? catalogSelected && `${catalogSelected.sha}` : selected && `${repo}\n${branch}\n${root + selected.path}`
  useEffect(() => {
    if (!viewedKey) return
    const timer = window.setTimeout(() => {
      const item = inCatalog && catalogSelected ? catalogItem(catalogSelected) : !inCatalog && selected ? itemOf(selected) : undefined
      if (!item) return
      remember('viewed', [item])
      window.api.usage.track({ kind: 'view', type: item.type })
    }, 1500)
    return () => window.clearTimeout(timer)
  }, [viewedKey])

  // A search counts once it has settled (only that one happened, never what was typed).
  useEffect(() => {
    if (!query.trim()) return
    const timer = window.setTimeout(() => window.api.usage.track({ kind: 'search' }), 1500)
    return () => window.clearTimeout(timer)
  }, [query])
  const [feedbackOpen, setFeedbackOpen] = useState(false)

  // Keyboard shortcuts (`?` lists them). Single keys only act when nothing is being typed.
  const [shortcutsOpen, setShortcutsOpen] = useState(false)
  const keyActions = useRef<(e: KeyboardEvent) => void>()
  keyActions.current = (e: KeyboardEvent) => {
    if (paused) return
    const typing = e.target instanceof Element && !!e.target.closest('input, textarea, select, [contenteditable]')
    const cmd = e.metaKey || e.ctrlKey
    const key = e.key.toLowerCase()
    if (cmd && key === 'k') {
      e.preventDefault()
      const box = document.querySelector<HTMLInputElement>('.search-box input')
      box?.focus()
      box?.select()
      return
    }
    if (e.key === 'Escape' && picked.length) return setPicked([])
    if (typing || e.altKey) return
    if (cmd && key === 'a' && !inCatalog && !shownCollection) {
      e.preventDefault()
      return setPicked(rankedShown.map((a) => a.id))
    }
    if (cmd && key === 'i') {
      e.preventDefault()
      const items = picked.length > 1 ? pickedAssets : selected ? [selected] : []
      if (!items.length || exportState.status === 'running') return
      // The project used last, or the only one open; with several and none chosen yet, the button's menu decides.
      void window.api.exporter.unityEditors().then((editors) => {
        let last = ''
        try {
          last = localStorage.getItem('assethub.lastUnityProject') ?? ''
        } catch {
          // none remembered
        }
        const editor = editors.find((x) => x.projectPath === last) ?? (editors.length === 1 ? editors[0] : undefined)
        if (editor) void runExport(items.map(packItem), editor)
      })
      return
    }
    if (cmd) return
    if (e.key === '?') return setShortcutsOpen(true)
    if (!selected || inCatalog) return
    if (key === 'f') return onCollections(toggleItem(collections, FAVORITES, itemOf(selected)))
    if (key === 'e') return togglePack(selected)
  }
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => keyActions.current?.(e)
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  }, [])

  const rememberReport = (path: string, report: DependencyReport) => {
    reports.current.set(path, report)
    setPack((items) => items.map((i) => (i.path === path ? { ...i, report } : i)))
  }

  const saveStats = (key: string, summary: StatsSummary) => {
    // A thumbnail's measurement never replaces one taken in the viewer (which counts every texture).
    // What is kept (the saved reading merged with what is known) comes back from the save.
    setModelStats((prev) => (prev[key] ? prev : { ...prev, [key]: summary }))
    void window.api.stats.save(key, summary).then(
      (kept) => kept && setModelStats((prev) => ({ ...prev, [key]: kept })),
      () => undefined
    )
  }
  const saveThumb = (key: string, dataUrl: string, summary?: StatsSummary, motion?: string) => {
    setThumbs((prev) => ({ ...prev, [key]: dataUrl }))
    void window.api.thumbs.save(key, dataUrl)
    if (motion) void window.api.thumbs.save(motionKey(key), motion)
    // Drawn this session: not drawn again, even if it is still not up to date.
    unmeasuredPrefabs.current.add(key)
    if (summary) saveStats(key, summary)
  }
  /** The opened model is loading: thumbnails wait so it gets the bandwidth. */
  const [viewerLoading, setViewerLoading] = useState(false)
  const thumbQueue = useThumbnailQueue(live, saveThumb, viewerLoading)

  // Cards on screen go first; once the queue is empty the rest of the list follows, top first,
  // a few at a time so cards scrolled into view can still jump ahead.
  useEffect(() => {
    if (!thumbsLoaded || viewerLoading || thumbQueue.busy.size || (!inCatalog && !live)) return
    renderable
      // Models measured before clip names, or before .mat textures counted, get one more render for them.
      .filter((a) => (!thumbs[a.sha] || remeasure(a)) && !thumbQueue.failedRecently(a.sha))
      .slice(0, 4)
      .forEach((a, i) => {
        if (!modelStats[a.sha]) unmeasuredPrefabs.current.add(a.sha)
        requestThumb(a, 1_000_000 + i)
      })
  }, [thumbsLoaded, viewerLoading, thumbQueue.busy, thumbs, modelStats, gridItems, inCatalog, live])

  /** Counts for the type tabs: of the open branch, or of the cross-branch results. */
  const typeSource: readonly Asset[] = inCatalog ? (catalog.assets ?? []) : assets
  const typeCounts = useMemo(() => {
    const counts = new Map<string, number>()
    for (const a of typeSource) counts.set(a.type, (counts.get(a.type) ?? 0) + 1)
    // Tabs that also list other types count them too.
    for (const h of registry.all()) if (h.alsoShows) counts.set(h.id, (counts.get(h.id) ?? 0) + typeSource.filter((a) => a.type !== h.id && inTab(a, h.id)).length)
    return counts
  }, [typeSource, registry, inTab])

  return (
    <div className="browse">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark" aria-hidden="true" />
          <span className="brand-name">Asset Hub</span>
          <VersionBadge />
        </div>
        <div className="crumb">
          <Picker
            label={t('browse.repo')}
            value={repo}
            options={repoNames}
            groupOf={ownerPrefix}
            ungroupedLabel=""
            placeholder={repos.status === 'loading' ? t('browse.loadingRepos') : t('browse.noRepos')}
            dimPrefix
            onChange={setRepo}
          />
          <Picker
            label={t('browse.branch')}
            value={branch}
            options={branches}
            groupOf={branchPrefix}
            ungroupedLabel={t('browse.branchNoPrefix')}
            tagOf={(b) => (b === defaultBranch ? t('browse.defaultBranch') : undefined)}
            placeholder={repo ? t('browse.loadingBranches') : '—'}
            onChange={setBranch}
          />
          {roots.length > 1 && (
            <div className="crumb-seg">
              <Dropdown value={root} options={roots.map((r) => ({ value: r, label: r || '/' }))} onChange={setRoot} label={t('browse.project')} />
            </div>
          )}
        </div>
        <SearchBox value={query} suggest={inCatalog ? suggestCatalog : suggestHere} onChange={onQuery} scope={scope} onScope={setScope} />
        <div className="topbar-actions">
          <button
            type="button"
            className="btn icon-btn watch-btn"
            aria-pressed={watchingHere}
            aria-label={watchingHere ? t('watch.stop') : t('watch.start')}
            title={watchingHere ? t('watch.stop') : t('watch.start')}
            disabled={!branch || branchRepo !== repo}
            onClick={() => void toggleWatch()}
          >
            <PinIcon filled={watchingHere} />
          </button>
          <button
            type="button"
            className="btn icon-btn"
            aria-label={t('browse.refresh')}
            title={t('browse.refresh')}
            disabled={!branch || tree.status === 'loading'}
            onClick={() => setReloadKey((k) => k + 1)}
          >
            <RefreshIcon />
          </button>
          <button type="button" className="btn icon-btn" aria-label={t('feedback.title')} title={t('feedback.title')} onClick={() => setFeedbackOpen(true)}>
            <MessageIcon />
          </button>
        </div>
        <button
          type="button"
          className="btn acct"
          aria-label={login ? `${displayName(login)} · ${t('common.settings')}` : t('common.settings')}
          title={t('browse.settingsAccount')}
          onClick={onSettings}
        >
          <span className="acct-avatar" aria-hidden="true">
            {login ? displayName(login).charAt(0).toUpperCase() : '?'}
          </span>
          <span className="acct-name">{login ? displayName(login) : t('common.settings')}</span>
        </button>
      </header>


      <div
        className="browse-body"
        data-details={hasSelection ? 'on' : 'off'}
        style={{ ...(detailsWidth ? { '--details-w': `${detailsWidth}px` } : {}), ...(viewerHeight ? { '--viewer-h': `${viewerHeight}px` } : {}) } as CSSProperties}
      >
        <aside className="sidebar">
          <div className="side-group">
            <span className="label-caps side-title">{t('browse.types')}</span>
            <div className="tabs" role="group" aria-label={t('browse.types')}>
              <button type="button" className="tab" aria-pressed={typeFilter === ALL} onClick={() => { setTypeFilter(ALL); setOpenCollection(undefined) }}>
                <span className="type-dot" style={{ background: 'var(--text)' }} />
                <span className="tab-label">{t('assetTypes.all')}</span>
                <span className="num">{typeSource.filter((a) => !registry.get(a.type)?.hidden).length}</span>
              </button>
              {/* A type with nothing in this view is left out, unless it is the one being filtered on. */}
              {registry.all().filter((h) => !h.hidden && ((typeCounts.get(h.id) ?? 0) > 0 || typeFilter === h.id)).map((h) => (
                <button key={h.id} type="button" className="tab" aria-pressed={typeFilter === h.id} onClick={() => { setTypeFilter(h.id); setOpenCollection(undefined) }}>
                  <span className="type-dot" style={{ background: `var(--type-${h.id}, var(--mute))` }} />
                  <span className="tab-label">{t(h.labelKey)}</span>
                  <span className="num">{typeCounts.get(h.id) ?? 0}</span>
                </button>
              ))}
            </div>
          </div>
          <CollectionsNav
            collections={collections}
            recent={recentLists}
            open={openCollection}
            onOpen={setOpenCollection}
            onCreate={(id, name) => onCollections(addCollection(collections, id, name))}
          />
          <WatchNav
            watched={watched}
            news={news}
            current={{ repo, branch }}
            onOpen={openBranch}
            onRemove={(w) => onWatched(watched.filter((x) => x !== w))}
            onMenu={(w, e) =>
              contextMenu.open(e, [
                { label: t('menu.openBranch'), icon: <PinIcon size={14} />, onSelect: () => openBranch(w.repo, w.branch) },
                { label: t('menu.copyBranch'), icon: <CopyIcon size={14} />, onSelect: () => copy(w.branch) },
                '-',
                { label: t('watch.stop'), danger: true, onSelect: () => onWatched(watched.filter((x) => x !== w)) }
              ])
            }
          />
          <SideGroup id="folders" title={t('browse.folders')} className="folders-nav">
            {/* Mount only once the branch is loaded so the initial expansion sees real folders. */}
            {!inCatalog && tree.status === 'ready' && (
              <FolderTree key={`${repo}@${branch}:${root}:${typeFilter}`} root={folderRoot} selected={folder} onSelect={(f) => { setFolder(f); setOpenCollection(undefined) }} onMenu={folderMenu} />
            )}
          </SideGroup>
          {!inCatalog && hiddenCount > 0 && (
            <p className="muted small" style={{ margin: 0 }}>
              {t('browse.hidden', { count: hiddenCount, folders: excludedFolders.join(', ') })}
            </p>
          )}
        </aside>

        <main className="content" style={{ '--cell': CELL_MIN[cellSize] } as CSSProperties}>
          {shownCollection ? (
            <CollectionView
              collection={shownCollection}
              registry={registry}
              thumbs={thumbs}
              stats={modelStats}
              current={{ repo, branch, path: selected ? root + selected.path : undefined }}
              onOpenItem={(item) => openLocation(item.repo, item.branch, item.path)}
              onRename={(name) => onCollections(renameCollection(collections, shownCollection.id, name))}
              onDelete={() => {
                onCollections(deleteCollection(collections, shownCollection.id))
                setOpenCollection(undefined)
              }}
              onClose={() => setOpenCollection(undefined)}
            />
          ) : (
          <>
          <div className="toolbar">
            <h1 title={gridTitle}>{gridTitle}</h1>
            <span className="num toolbar-count" title={t('browse.assetCount', { count: gridCount })}>
              {gridCount.toLocaleString()}
            </span>
            {/* Thumbnails still loading: a thin bar, the numbers on hover. */}
            {renderable.length > 0 && loadedCount < renderable.length && (
              <progress
                className="loaded-bar"
                max={renderable.length}
                value={loadedCount}
                aria-label={t('browse.loaded', { done: loadedCount.toLocaleString(), total: renderable.length.toLocaleString() })}
                title={`${t('browse.loaded', { done: loadedCount.toLocaleString(), total: renderable.length.toLocaleString() })} · ${t('browse.loadedHint')}`}
              />
            )}
            {(inCatalog ? catalogShown.length : shown.length) > 0 && (
              <>
                <Dropdown<SortMode>
                  size="sm"
                  value={sortMode}
                  options={(['relevance', ...(budget ? (['playable'] as const) : []), 'smallest', 'triangles', ...(inCatalog ? (['used'] as const) : [])] as const).map((m) => ({ value: m, label: t(`ranking.sorts.${m}`) }))}
                  onChange={setSortMode}
                  label={t('ranking.sort')}
                />
                <Popover
                  label={t('filters.title')}
                  trigger={
                    <>
                      <FilterIcon />
                      {t('filters.title')}
                      {activeFilters > 0 && <span className="count-dot">{activeFilters}</span>}
                    </>
                  }
                >
                  <span className="label-caps">{t('chips.label')}</span>
                  <div className="chips" role="group" aria-label={t('chips.label')}>
                    {QUICK_CHIPS.map((c) => (
                      <button key={c} type="button" className="chip" aria-pressed={chips.includes(c)} onClick={() => toggleChip(c)}>
                        {t(`chips.${c}`)}
                      </button>
                    ))}
                  </div>
                  <span className="label-caps">{t('budget.preset')}</span>
                  <Dropdown size="sm" value={budgetPreset} options={BUDGET_PRESETS.map((p) => ({ value: p, label: t(`budget.presets.${p}`) }))} onChange={onBudgetPreset} label={t('budget.preset')} title={t('budget.presetHint')} />
                  {budget && (
                    <label className="check small">
                      <input type="checkbox" checked={onlyFits} onChange={(e) => setOnlyFits(e.target.checked)} />
                      {t('ranking.onlyFits')}
                    </label>
                  )}
                  {!inCatalog && (
                    <>
                      <span className="label-caps">{t('filters.view')}</span>
                      <label className="check small">
                        <input type="checkbox" checked={mergeCopies} onChange={(e) => setMergeCopies(e.target.checked)} />
                        {t('browse.mergeCopies')}
                        {merged > 0 && <span className="num"> ({merged})</span>}
                      </label>
                      {(unlistedCount > 0 || (showUnlisted && unlistedKey)) && (
                        <label className="check small">
                          <input type="checkbox" checked={showUnlisted} onChange={(e) => setShowUnlisted(e.target.checked)} />
                          {t('filters.unlisted')}
                          {unlistedCount > 0 && <span className="num"> ({unlistedCount})</span>}
                        </label>
                      )}
                    </>
                  )}
                </Popover>
                {/* What is on, each with its own off switch. */}
                {chips.map((c) => (
                  <button key={c} type="button" className="pill" aria-label={t('filters.remove', { name: t(`chips.${c}`) })} onClick={() => toggleChip(c)}>
                    {t(`chips.${c}`)}{' '}
                    <span className="pill-x" aria-hidden="true">
                      <CloseIcon size={10} />
                    </span>
                  </button>
                ))}
                {budget && (
                  <button type="button" className="pill" onClick={() => onBudgetPreset('off')} aria-label={t('filters.remove', { name: t(`budget.presets.${budgetPreset}`) })}>
                    {t(`budget.presets.${budgetPreset}`)}{' '}
                    <span className="pill-x" aria-hidden="true">
                      <CloseIcon size={10} />
                    </span>
                  </button>
                )}
                {chips.length > 0 && !gridItems.length && <span className="muted small">{t('chips.none')}</span>}
                {(sortMode !== 'relevance' || (onlyFits && budget) || chips.some((c) => c !== 'small')) && (
                  <>
                    <span className="num" title={t('ranking.measured', { done: measuredCount, total: inCatalog ? catalogShown.length : shown.length })}>
                      {t('ranking.measured', { done: measuredCount, total: inCatalog ? catalogShown.length : shown.length })}
                    </span>
                    {unmeasured.some(canRender) && (
                      <button type="button" className="btn btn-small" onClick={measureAll}>
                        {t('ranking.measure', { count: unmeasured.filter(canRender).length })}
                      </button>
                    )}
                  </>
                )}
              </>
            )}
            <CellSizeToggle value={cellSize} onChange={setCellSize} />
          </div>
          {inCatalog && catalog.assets && (
            <div className="catalog-filters">
              {scope === 'all' && (
                <Dropdown
                  size="sm"
                  className="filter-trigger"
                  value={catalogRepo}
                  options={[
                    { value: '', label: t('catalog.allRepos', { count: catalogFacets.repos.length }) },
                    ...catalogFacets.repos.map(([r, n]) => ({ value: r, label: r.split('/').pop() ?? r, hint: String(n) }))
                  ]}
                  onChange={(r) => (setCatalogRepo(r), setCatalogBranch(''))}
                  label={t('catalog.filterRepo')}
                />
              )}
              <Dropdown
                size="sm"
                className="filter-trigger"
                value={catalogBranch}
                options={[
                  { value: '', label: t('catalog.allBranches', { count: catalogFacets.branches.length }) },
                  ...catalogFacets.branches.map(([b, n]) => ({ value: b, label: b, hint: String(n) }))
                ]}
                onChange={setCatalogBranch}
                label={t('catalog.filterBranch')}
              />
            </div>
          )}
          {inCatalog && <CatalogStatusLine status={catalog.status} error={catalog.error} onPause={catalog.pause} onResume={catalog.resume} />}
          {(searchFound.related.length > 0 || searchFound.typos.length > 0) && (
            <p className="related-line">
              {searchFound.related.length > 0 && t('search.related', { names: searchFound.related.slice(0, 8).join(', ') })}
              {searchFound.related.length > 0 && searchFound.typos.length > 0 && ' · '}
              {searchFound.typos.length > 0 && t('search.typos', { names: searchFound.typos.slice(0, 5).join(', ') })}
            </p>
          )}
          {inCatalog && !catalog.assets && !catalog.error && (
            <div className="grid">
              {Array.from({ length: 12 }, (_, i) => (
                <SkeletonCard key={i} />
              ))}
            </div>
          )}
          {inCatalog && catalog.assets && catalogShown.length === 0 && (
            catalog.status.state === 'running' ? <EmptyState title={t('catalog.emptyYet')} hint={t('catalog.emptyYetHint')} /> : <EmptyState title={t('browse.noResults')} hint={t('browse.noResultsHint')} />
          )}
          {inCatalog && catalogShown.length > 0 && (
            <>
              <div className="grid">
                {rankedCatalog.slice(0, limit).map((a, i) => (
                  <AssetCard
                    key={a.id}
                    asset={a}
                    handler={registry.get(a.type)}
                    thumb={thumbs[a.sha]}
                    selected={a.sha === catalogSel}
                    onSelect={() => setCatalogSel(a.sha)}
                    onContextMenu={(e) => catalogMenu(a, e)}
                    note={locationNote(a, scope, t)}
                    generating={thumbQueue.busy.has(a.sha)}
                    stats={modelStats[a.sha]}
                    onVisible={
                      thumbsLoaded && canRender(a)
                        ? (on) => {
                            if (!on) return thumbQueue.cancel(a.sha)
                            if (thumbs[a.sha] && modelStats[a.sha] && !staleEffect(a) && !staleClip(a) && !staleModel(a) && !staleFlat(a)) return
                            const src = catalogSource(a)
                            thumbQueue.request(a.sha, src.path, i, src.ctx, kindOf(a))
                          }
                        : undefined
                    }
                  />
                ))}
              </div>
              {limit < rankedCatalog.length && <div ref={sentinel} className="grid-sentinel" />}
            </>
          )}
          {repos.status === 'error' && <ErrorNote error={repos.error} className="content-note" />}
          {repos.status === 'ready' && repoList.length === 0 && <EmptyState title={t('browse.noRepos')} hint={t('browse.noReposHint')} />}
          {!inCatalog && tree.status === 'loading' && repo && (
            <>
              <p className="state-line" role="status">
                {t('browse.loading')}
              </p>
              <div className="grid">
                {Array.from({ length: 12 }, (_, i) => (
                  <SkeletonCard key={i} />
                ))}
              </div>
            </>
          )}
          {!inCatalog && tree.status === 'error' && <ErrorNote error={tree.error} className="content-note" />}
          {scan && registry.get(typeFilter)?.facts === scan.kind && (
            <p className="state-line" data-state={scan.status} role="status">
              {scan.status === 'error'
                ? t('scan.failed', { what: t(`scan.what.${scan.kind}`) })
                : scan.total
                  ? t('scan.reading', { what: t(`scan.what.${scan.kind}`), done: formatBytes(scan.done), total: formatBytes(scan.total) })
                  : t('scan.starting', { what: t(`scan.what.${scan.kind}`) })}
            </p>
          )}
          {!inCatalog && tree.status === 'ready' && visible.length === 0 && <EmptyState title={t('browse.noResults')} hint={t('browse.noResultsHint')} />}
          </>
          )}
          {(pack.length > 0 || exportState.status !== 'idle') && (
            <ExportBar
              items={pack}
              capacity={packCapacity(budgetPreset, budget)}
              budgetPreset={budgetPreset}
              thumbs={thumbs}
              options={exportOptions}
              state={exportState}
              onOptions={(o) => {
                setSkipByName(o.skipByName)
                setReuseExisting(o.reuseExisting)
                setStripScripts(o.stripScripts)
                if (o.layout !== exportLayout) onExportLayout(o.layout)
              }}
              onExport={() => void runExport(pack)}
              onImportToUnity={(editor) => void runExport(pack, editor)}
              onRemove={(path) => setPack((items) => items.filter((i) => i.path !== path))}
              onClear={() => setPack([])}
              onDismiss={() => setExportState({ status: 'idle' })}
            />
          )}
          {!shownCollection && (
          <>
          {!inCatalog && tree.status === 'ready' && shown.length > 0 && (
            <>
              {newHere.size > 0 && (
                <div className="watch-banner" role="status">
                  <strong>{t('watch.banner', { count: newHere.size, more: news[watchKey(repo, branch)]?.more ? '+' : '' })}</strong>
                  <label className="check small">
                    <input type="checkbox" checked={newOnly} onChange={(e) => setNewOnly(e.target.checked)} />
                    {t('watch.onlyNew')}
                  </label>
                  <span style={{ flex: 1 }} />
                  <button type="button" className="btn btn-small" onClick={markSeen}>
                    {t('watch.seen')}
                  </button>
                </div>
              )}
              <div className="grid">
                {rankedShown.slice(0, limit).map((a, i) => (
                  <AssetCard
                    key={a.id}
                    asset={a}
                    handler={registry.get(a.type)}
                    thumb={thumbs[a.sha]}
                    generating={thumbQueue.busy.has(a.sha)}
                    copies={mergeCopies ? copiesBySha.get(a.sha)?.length : undefined}
                    selected={a.id === selectedId || (!!selected && mergeCopies && selected.sha === a.sha)}
                    onSelect={(how) => pickCard(a, how)}
                    onContextMenu={(e) => cardMenu(a, e)}
                    picked={picked.length > 1 && picked.includes(a.id)}
                    inList={packIds.has(a.id)}
                    fresh={newHere.has(root + a.path)}
                    starred={favorites.has(`${repo}\n${root + a.path}`)}
                    stats={modelStats[a.sha]}
                    onVisible={
                      live && thumbsLoaded && canRender(a)
                        ? (on) => {
                            if (!on) return thumbQueue.cancel(a.sha)
                            if (thumbs[a.sha] && modelStats[a.sha] && !staleEffect(a) && !staleClip(a) && !staleModel(a) && !staleFlat(a)) return
                            thumbQueue.request(a.sha, root + a.path, i, undefined, kindOf(a))
                          }
                        : undefined
                    }
                  />
                ))}
              </div>
              {limit < rankedShown.length && <div ref={sentinel} className="grid-sentinel" />}
              {picked.length > 1 && (
                <MultiBar
                  count={picked.length}
                  collections={collections}
                  busy={exportState.status === 'running'}
                  onAddToList={() => {
                    const add = pickedAssets.filter((a) => !pack.some((i) => i.asset.id === a.id))
                    setPack((items) => [...items, ...add.map(packItem)])
                  }}
                  onCollection={(id) => {
                    let list = withFavorites(collections)
                    for (const a of pickedAssets) if (!inCollection(list.find((c) => c.id === id), itemOf(a))) list = toggleItem(list, id, itemOf(a))
                    onCollections(list)
                  }}
                  onImportToUnity={(editor) => void runExport(pickedAssets.map(packItem), editor)}
                  onClear={() => setPicked([])}
                />
              )}
            </>
          )}
          </>
          )}
        </main>

        {/* The grid keeps at least 360 px; the sidebar is 220–260 px. No asset selected: no inspector column. */}
        {hasSelection && (
        <>
        <Splitter axis="x" target="next" min={300} max={() => window.innerWidth - 260 - 360} onResize={setDetailsWidth} label={t('details.resizePanel')} />
        {inCatalog ? (
          <CatalogDetails
            asset={catalogSelected}
            thumb={catalogSelected && thumbs[catalogSelected.sha]}
            stats={catalogSelected && modelStats[catalogSelected.sha]}
            delivery={catalogSelected && registry.get(catalogSelected.type)?.delivery}
            exporting={exportState.status === 'running'}
            onOpen={openLocation}
            onExport={(a) => void exportFromCatalog(a)}
            onImportToUnity={(a, editor) => void exportFromCatalog(a, editor)}
          />
        ) : (
        <AssetDetails
          onOpenLocation={openLocation}
          onViewerLoading={setViewerLoading}
          onModelStats={saveStats}
          asset={selected}
          handler={selected && registry.get(selected.type)}
          playClips={registry.get(typeFilter)?.preview.kind === 'animation'}
          models={modelPaths}
          live={live}
          onThumb={saveThumb}
          onReport={rememberReport}
          primary={pack.length === 0}
          inPack={!!selected && pack.some((i) => i.asset.id === selected.id)}
          exporting={exportState.status === 'running'}
          onTogglePack={() => selected && togglePack(selected)}
          onExport={() => selected && void runExport([packItem(selected)])}
          onImportToUnity={(editor) => selected && void runExport([packItem(selected)], editor)}
          copies={selected ? copiesBySha.get(selected.sha) : undefined}
          onSelectCopy={setSelectedId}
          excludedFolders={excludedFolders}
          onPrefabFiles={(path, files) => prefabFiles.current.set(path, files)}
          collections={
            live && selected
              ? {
                  list: collections,
                  item: itemOf(selected),
                  onToggle: (id) => onCollections(toggleItem(collections, id, itemOf(selected))),
                  onCreate: (id, name) => onCollections(toggleItem(addCollection(collections, id, name), id, itemOf(selected)))
                }
              : undefined
          }
          onShowFolder={selected && selectedDir && selectedDir !== folder ? () => showFolder(selectedDir) : undefined}
          viewerResizer={<Splitter axis="y" target="prev" min={160} max={() => window.innerHeight - 220} onResize={setViewerHeight} label={t('details.resizeViewer')} />}
        />
        )}
        </>
        )}
      </div>
      {contextMenu.element}
      {notice && (
        <div className="menu-notice" role="status">
          {notice}
        </div>
      )}
      {shortcutsOpen && <ShortcutHelp onClose={() => setShortcutsOpen(false)} />}
      {feedbackOpen && (
        <FeedbackDialog
          context={repo ? { repo, branch, asset: inCatalog ? catalogSelected?.locations[0]?.path : selected && root + selected.path } : undefined}
          onClose={() => setFeedbackOpen(false)}
        />
      )}
    </div>
  )
}
