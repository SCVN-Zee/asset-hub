// Entry of the test page: a fake Tauri backend at the IPC level, then the real app.
// A test sets `window.__e2eConfig` (through an init script) before this runs, and reads `window.__e2e` after.

import { emit } from '@tauri-apps/api/event'
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { BRANCH_COUNT, branchAssets, catalogAssets, DEFAULT_BRANCH, REPOS, statsFor } from './fixtures'

interface Status {
  state: 'idle' | 'running' | 'paused' | 'error'
  done: number
  total: number
  error?: { code: string; message: string; resetAt?: number }
}

export interface E2EConfig {
  /** How long `catalog_scan` takes to answer. */
  scanDelayMs?: number
  /** How long `source_list_repos` takes to answer. */
  reposDelayMs?: number
  /** How long `catalog_list` takes to answer. */
  listDelayMs?: number
  /** `catalog_snapshot` answers a listing of this many assets at once; without it, there is no snapshot. */
  snapshotCount?: number
  /** Branch the app opens (branch scope). */
  lastRepo?: string
  lastBranch?: string
}

declare global {
  interface Window {
    __e2eConfig?: E2EConfig
    __e2e: {
      calls: string[]
      /** Start time (ms since page load) of each `catalog_list` call, and the most that ran at once. */
      listStarts: number[]
      listMaxInFlight: number
      unknown: string[]
      status: Status
      emitStatus(status: Status): Promise<void>
    }
  }
}

const config = window.__e2eConfig ?? {}
const catalog = catalogAssets()
const branch = branchAssets()
const idle: Status = { state: 'idle', done: 0, total: 0 }

const e2e: Window['__e2e'] = {
  calls: [],
  listStarts: [],
  listMaxInFlight: 0,
  unknown: [],
  status: idle,
  emitStatus(status) {
    e2e.status = status
    return emit('catalog-progress', status)
  }
}
window.__e2e = e2e

const settings = { language: 'en', excludedFolders: [] as string[], lastRepo: config.lastRepo ?? REPOS[0], lastBranch: config.lastBranch ?? DEFAULT_BRANCH }

/** Assets handed to `search_build`, by handle. */
const indexes = new Map<number, Array<{ id: string; name: string; path: string }>>()

let listInFlight = 0
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms))
const ipcError = (code: string, message: string) => ({ code, message })

function descriptor(id: string, label: string, badge: string, previewKind: string, delivery = 'unitypackage') {
  return { id, labelKey: `assetTypes.${label}`, badge, previewKind, hidden: false, delivery, facts: null, materialsFromPrefabs: false, usesGuids: true, resolver: null, alsoShows: null, unlisted: null }
}

const descriptors = [
  descriptor('fbx', 'fbx', 'MDL', 'three'),
  descriptor('texture', 'texture', 'TEX', 'image', 'file'),
  descriptor('material', 'material', 'MAT', 'material'),
  descriptor('audio', 'audio', 'AUD', 'audio', 'file'),
  descriptor('prefab', 'prefab', 'PRE', 'prefab'),
  descriptor('animation', 'animation', 'ANM', 'animation'),
  descriptor('font', 'font', 'FNT', 'font', 'file')
]

/** A listing as the backend sends it: one location with one branch, and nothing the page can work out (see `hydrateListing`); `catalog_where_is` has the rest. */
function slim(a: (typeof catalog)[number]) {
  const [first] = a.locations
  const { id: _id, lfs, locations: _all, repos, ...rest } = a
  const location: Record<string, unknown> = { repo: first.repo, branches: first.branches.slice(0, 1) }
  if (first.path !== a.path) location.path = first.path
  if (first.branches.length > 1) location.branchTotal = first.branches.length
  return {
    ...rest,
    ...(lfs ? { lfs } : {}),
    locations: [location],
    ...(a.locations.length > 1 ? { locationTotal: a.locations.length } : {}),
    ...(repos.length === 1 && repos[0] === first.repo ? {} : { repos })
  }
}

function listingOf(assets: typeof catalog) {
  const repos = new Map<string, number>()
  for (const a of assets) for (const l of a.locations) repos.set(l.repo, (repos.get(l.repo) ?? 0) + 1)
  return { assets: assets.map(slim), facets: { repos: [...repos], branches: [[DEFAULT_BRANCH, assets.length]] } }
}

function searchQuery(handle: number, query: string) {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean)
  const all = indexes.get(handle) ?? []
  const assets = all.filter((a) => words.every((w) => a.path.toLowerCase().includes(w)))
  return { assets, related: [], typos: [], scores: assets.map((a) => [a.id, 1]) }
}

async function handle(cmd: string, args: Record<string, unknown> = {}): Promise<unknown> {
  switch (cmd) {
    case 'get_settings':
      return settings
    case 'set_settings':
      Object.assign(settings, args.patch)
      return settings
    case 'auth_status':
      return { signedIn: true, login: 'e2e', accounts: ['e2e'], method: 'pat', persisted: true }
    case 'handler_descriptors':
      return descriptors
    case 'take_deep_link':
    case 'update_check':
      return null
    case 'team_status':
      return 'disabled'
    case 'team_popular':
      return {}
    case 'team_note_view':
    case 'thumbs_save':
      return null
    case 'thumbs_get':
      return {}
    case 'stats_get': {
      const found: Record<string, unknown> = {}
      for (const key of (args.keys as string[]) ?? []) {
        const stats = statsFor(key)
        if (stats) found[key] = stats
      }
      return found
    }
    case 'stats_save':
      return null
    case 'cache_usage':
      return { downloads: 0, thumbs: 0, other: 0 }
    case 'source_list_repos':
      await sleep(config.reposDelayMs ?? 0)
      return REPOS.map((fullName) => ({ fullName, defaultBranch: DEFAULT_BRANCH }))
    case 'source_list_branches':
      return [DEFAULT_BRANCH, 'feature/f1', 'feature/f2']
    case 'browse_assets':
      return { roots: [''], root: '', assets: branch.slice(0, BRANCH_COUNT), hidden: 0 }
    case 'catalog_status':
      return e2e.status
    case 'catalog_scan':
      await sleep(config.scanDelayMs ?? 0)
      return e2e.status
    case 'catalog_pause':
      e2e.status = { ...e2e.status, state: 'paused' }
      return e2e.status
    case 'catalog_resume':
      e2e.status = { ...e2e.status, state: 'running', error: undefined }
      return e2e.status
    case 'catalog_list': {
      e2e.listStarts.push(performance.now())
      listInFlight++
      e2e.listMaxInFlight = Math.max(e2e.listMaxInFlight, listInFlight)
      await sleep(config.listDelayMs ?? 0)
      listInFlight--
      return listingOf(catalog)
    }
    case 'catalog_snapshot':
      return config.snapshotCount ? listingOf(catalog.slice(0, config.snapshotCount)) : null
    case 'search_build': {
      const handleId = indexes.size + 1
      indexes.set(handleId, args.assets as Array<{ id: string; name: string; path: string }>)
      return handleId
    }
    case 'search_query':
      return searchQuery(args.handle as number, args.query as string)
    case 'catalog_prefab_scan_targets':
      return []
    case 'catalog_where_is':
      return catalog.find((a) => a.sha === args.sha)?.locations ?? []
    case 'search_suggest':
    case 'exporter_unity_editors':
    case 'assets_known_assets':
      return []
    case 'search_folders':
      return { name: '', path: '', children: [], assetCount: indexes.get(args.handle as number)?.length ?? 0 }
    case 'assets_read_file':
      // No file bytes: thumbnails that need them fail at once.
      throw ipcError('notFound', 'no file in the test backend')
    default:
      // Viewers and resolvers ask for file data the test backend does not have: they fail like a missing file.
      if (!e2e.unknown.includes(cmd)) {
        e2e.unknown.push(cmd)
        console.warn(`E2E-UNKNOWN ${cmd}`)
      }
      throw ipcError('notFound', `no ${cmd} in the test backend`)
  }
}

mockWindows('main')
mockIPC(
  (cmd, payload) => {
    e2e.calls.push(cmd)
    return handle(cmd, (payload ?? {}) as Record<string, unknown>)
  },
  { shouldMockEvents: true }
)

await import('../ui/src/main')
