// Entry of the test page: a fake Tauri backend at the IPC level, then the real app.
// A test sets `window.__e2eConfig` (through an init script) before this runs, and reads `window.__e2e` after.

import { emit } from '@tauri-apps/api/event'
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks'
import { FLAT_THUMB_VERSION, MODEL_THUMB_VERSION, ANIM_THUMB_VERSION } from '../ui/src/viewer/thumbVersions'
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
  /** Size of the catalog the fake backend lists (a real one holds hundreds of thousands of assets). */
  catalogCount?: number
  /** Every file reads as a small PNG, so thumbnails of images render for real. */
  readableFiles?: boolean
  /**
   * Share (0 to 1) of the blobs that have a saved thumbnail: `thumbs_get` answers a data URL of about 12 KB for those,
   * and `stats_get` a measurement that is up to date, so such a card has nothing left to draw.
   * Without it, no thumbnail is saved.
   */
  cachedThumbs?: number
  /** One Unity editor is open, so Import into Unity (Cmd+I) exports at once. */
  unityEditor?: boolean
  /**
   * A background scan of the prefabs: `catalog_prefab_scan_targets` names one branch once, and reading it reports `flushes`
   * batches of `perFlush` prefab SHAs, `intervalMs` apart, for each kind of fact (effects, then UI prefabs, whose SHAs
   * start half a scan later, so some prefabs have both tags).
   */
  factScan?: { perFlush: number; intervalMs: number; flushes: number; /** Whose prefabs the scan finds: the catalog's (default) or the open branch's. */ of?: 'catalog' | 'branch' }
  /** A VFX tab that also lists the prefabs a scan found effects in. */
  effectsTab?: boolean
  /** Branch the app opens (branch scope). */
  lastRepo?: string
  lastBranch?: string
}

declare global {
  interface Window {
    __e2eConfig?: E2EConfig
    __e2e: {
      calls: string[]
      /** Number of keys in each `thumbs_get` request. */
      thumbGetSizes: number[]
      /** Every key `thumbs_get` answered with a thumbnail. */
      thumbAnswered: string[]
      /** Path of every `assets_read_file` request. */
      readPaths: string[]
      /** Paths that had a preview image in each `exporter_unity_package` request. */
      exportPreviews: string[][]
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
const catalog = catalogAssets(config.catalogCount)
const branch = branchAssets()
// A 1x1 PNG: what a readable file holds, so image thumbnails really render.
const PNG_BYTES = Uint8Array.from(atob('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=='), (c) => c.charCodeAt(0))
const idle: Status = { state: 'idle', done: 0, total: 0 }

const e2e: Window['__e2e'] = {
  calls: [],
  thumbGetSizes: [],
  thumbAnswered: [],
  readPaths: [],
  exportPreviews: [],
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

/** SHAs of the catalog's prefabs, in order: what a simulated fact scan finds. */
const prefabShas = (of: 'catalog' | 'branch' = 'catalog') => (of === 'branch' ? branch : catalog).filter((a) => a.type === 'prefab').map((a) => a.sha)
let scanTargetsGiven = false
const scannedKinds = new Set<string>()

/** Reports what the simulated scan of `kind` finds, one batch at a time. */
async function simulateKnownScan(kind: string, scan: NonNullable<E2EConfig['factScan']>) {
  const shas = prefabShas(scan.of)
  const first = kind === 'uiPrefabs' ? Math.floor((scan.perFlush * scan.flushes) / 2) : 0
  for (let flush = 0; flush < scan.flushes; flush++) {
    await sleep(scan.intervalMs)
    const found = shas.slice(first + flush * scan.perFlush, first + (flush + 1) * scan.perFlush)
    await emit('known-progress', { kind, done: flush + 1, total: scan.flushes, found })
  }
}

/** Which blobs have a saved thumbnail: a fixed share of them, picked by their hash. */
const hasSavedThumb = (key: string) => (config.cachedThumbs ?? 0) > 0 && parseInt(key.slice(8, 16), 16) / 0x1_0000_0000 < (config.cachedThumbs ?? 0)
const PNG_BINARY = String.fromCharCode(...PNG_BYTES)
/** A different, flat string for each key, as long as a real thumbnail's (`repeat` would share its parts and hold almost no memory); a PNG followed by padding, which decoders ignore. */
const savedThumb = (key: string) => `data:image/png;base64,${btoa(PNG_BINARY + Array.from({ length: 220 }, () => key).join(''))}`
/** A measurement that no card wants repeated: drawn the way thumbnails are drawn now. */
function upToDate(key: string) {
  const type = catalogTypeOf(key)
  const flat = type === 'audio' || type === 'font'
  return { triangles: 600, vertices: 300, bones: 0, clips: 0, materials: 1, fileBytes: 2048, textureBytes: 4096, maxTextureSize: 512, width: 64, height: 64, source: 'thumb', modelThumb: flat ? FLAT_THUMB_VERSION : MODEL_THUMB_VERSION, animThumb: ANIM_THUMB_VERSION }
}
let typeByBlob: Map<string, string> | undefined
const catalogTypeOf = (key: string) => (typeByBlob ??= new Map(catalog.map((a) => [a.sha, a.type]))).get(key)

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
  descriptor('font', 'font', 'FNT', 'font', 'file'),
  ...(config.effectsTab ? [{ ...descriptor('vfx', 'vfx', 'VFX', 'prefab'), facts: 'effects', alsoShows: { kind: 'fact', fromType: 'prefab', fact: 'effects' } }] : [])
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
    case 'thumbs_get': {
      const keys = (args.keys as string[]) ?? []
      e2e.thumbGetSizes.push(keys.length)
      const found: Record<string, string> = {}
      for (const key of keys) {
        if (!hasSavedThumb(key)) continue
        found[key] = savedThumb(key)
        e2e.thumbAnswered.push(key)
      }
      return found
    }
    case 'stats_get': {
      const found: Record<string, unknown> = {}
      for (const key of (args.keys as string[]) ?? []) {
        const stats = hasSavedThumb(key) ? upToDate(key) : statsFor(key)
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
      if (!config.factScan || scanTargetsGiven) return []
      scanTargetsGiven = true
      return [{ repo: REPOS[0], branch: DEFAULT_BRANCH }]
    // Preview images reach the export as ids of bytes sent ahead: the id stands for the bytes.
    case 'stash_bytes':
      return `stash-${e2e.calls.length}`
    case 'exporter_import_to_unity':
    case 'exporter_unity_package': {
      const request = args.request as { previews: Record<string, string> }
      e2e.exportPreviews.push(Object.keys(request.previews))
      return { canceled: true }
    }
    case 'catalog_where_is':
      return catalog.find((a) => a.sha === args.sha)?.locations ?? []
    case 'assets_known_assets': {
      const kind = args.kind as string
      if (config.factScan && !scannedKinds.has(kind)) {
        scannedKinds.add(kind)
        await simulateKnownScan(kind, config.factScan)
      }
      return []
    }
    case 'exporter_unity_editors':
      return config.unityEditor ? [{ pid: 1, projectPath: '/e2e/project', project: 'project' }] : []
    case 'search_suggest':
      return []
    case 'search_folders':
      return { name: '', path: '', children: [], assetCount: indexes.get(args.handle as number)?.length ?? 0 }
    case 'assets_read_file':
      e2e.readPaths.push(args.path as string)
      // Without `readableFiles` there are no file bytes: thumbnails that need them fail at once.
      if (config.readableFiles) return PNG_BYTES.buffer.slice(0)
      throw ipcError('notFound', 'no file in the test backend')
    case 'texture_info':
      if (config.readableFiles) return { kind: null, width: 1, height: 1, import: null, sprite: null, role: null, builtSize: 1 }
    // Without `readableFiles` it is unknown to the test backend, like the other file readers below.
    // falls through
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
