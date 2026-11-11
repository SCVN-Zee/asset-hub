import { BUDGET_KEYS, BUDGET_PRESETS, type BudgetPreset, type PackageLayout } from '../src/bindings'
import type { PlayableBudget } from './types'
import { RECENT_LIMIT, sanitizeCollections, sanitizeItems, type Collection, type RecentLists } from './collections'

export const LANGUAGES = ['en', 'ko', 'vi'] as const
export type Language = (typeof LANGUAGES)[number]
/** In English, for telling the AI assistant which language to answer in. */
export interface WatchedBranch {
  repo: string
  branch: string
  /** The commit everything up to which has been seen. */
  seen: string
}

export const LANGUAGE_NAMES: Record<Language, string> = { en: 'English', vi: 'Vietnamese', ko: 'Korean' }

export interface Settings {
  language: Language
  /** Where clones, downloaded files and thumbnails are cached. */
  cacheDir: string
  /** Most the downloaded files may take, in GB (absent: `DEFAULT_CACHE_LIMIT_GB`; 0: no limit). */
  cacheLimitGB?: number
  /** Folder names hidden from browsing at any depth (e.g. third-party packs). */
  excludedFolders: string[]
  /** Repo and branch opened last, restored on the next launch. */
  lastRepo?: string
  lastBranch?: string
  /** Playable budget assets are judged against (absent: the 5 MB default). */
  budgetPreset?: BudgetPreset
  /** Limits of the `custom` preset; unset ones come from the default. */
  customBudget?: Partial<PlayableBudget>
  /** Where exported packages put files in Unity (absent: `DEFAULT_EXPORT_LAYOUT`). */
  exportLayout?: PackageLayout
  /** Unity project packages are meant for (read only): exports leave out what it already has. */
  targetProject?: string
  /** Folder a team shares (network drive, synced folder) holding an index everyone takes in. */
  sharedIndexDir?: string
  /** Favourites and named collections of assets (see shared/collections). */
  collections?: Collection[]
  /** The AI pet in the corner of the hub (absent: shown once ChatGPT is connected). */
  aiPet?: boolean
  /** Assets viewed and exported lately, newest first (kept by the app). */
  recent?: RecentLists
  /** Branches the user follows: new assets since `seen` (a commit) are pointed out. */
  watched?: WatchedBranch[]
  /** Usage notes sent to the team's feedback repo (absent: on). */
  usageStats?: boolean
  /** Signed in from last time: straight into the hub, without the 3D lobby (absent: off). */
  skipLobby?: boolean
}

export const EXPORT_LAYOUTS: readonly PackageLayout[] = ['byType', 'grouped', 'source']
export const DEFAULT_EXPORT_LAYOUT: PackageLayout = 'byType'

export const DEFAULT_EXCLUDED_FOLDERS = ['_ThirdParty']

export const CACHE_LIMITS_GB = [1, 2, 5, 10, 0] as const
export const DEFAULT_CACHE_LIMIT_GB = 2

export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value)
}

/** Merges untrusted stored/IPC data over defaults, dropping anything invalid. */
/** Every setting this version knows; anything else in the file was written by another version. */
export const SETTINGS_KEYS = [
  'language', 'cacheDir', 'cacheLimitGB', 'excludedFolders', 'lastRepo', 'lastBranch', 'budgetPreset', 'customBudget',
  'exportLayout', 'targetProject', 'sharedIndexDir', 'collections', 'aiPet', 'recent', 'watched', 'usageStats', 'skipLobby'
] as const satisfies ReadonlyArray<keyof Settings>

/**
 * Settings this version does not know, as they are in the file: an older and a newer copy of the app
 * share it (the installed app and one being tried, or the days around an update), and neither may
 * erase the other's choices when it saves its own.
 */
export function unknownSettings(raw: unknown): Record<string, unknown> {
  if (!raw || typeof raw !== 'object') return {}
  const known = new Set<string>(SETTINGS_KEYS)
  return Object.fromEntries(Object.entries(raw).filter(([k]) => !known.has(k)))
}

export function sanitizeSettings(raw: unknown, defaults: Settings): Settings {
  const input = raw && typeof raw === 'object' ? (raw as Record<string, unknown>) : {}
  const folders = Array.isArray(input.excludedFolders)
    ? [...new Set(input.excludedFolders.filter((f): f is string => typeof f === 'string').map((f) => f.trim()).filter(Boolean))]
    : defaults.excludedFolders
  return {
    language: isLanguage(input.language) ? input.language : defaults.language,
    cacheDir: typeof input.cacheDir === 'string' && input.cacheDir.trim() !== '' ? input.cacheDir : defaults.cacheDir,
    ...((CACHE_LIMITS_GB as readonly unknown[]).includes(input.cacheLimitGB) ? { cacheLimitGB: input.cacheLimitGB as number } : {}),
    excludedFolders: folders,
    ...(typeof input.lastRepo === 'string' && input.lastRepo ? { lastRepo: input.lastRepo } : {}),
    ...(typeof input.lastBranch === 'string' && input.lastBranch ? { lastBranch: input.lastBranch } : {}),
    ...((BUDGET_PRESETS as readonly unknown[]).includes(input.budgetPreset) ? { budgetPreset: input.budgetPreset as BudgetPreset } : {}),
    ...sanitizeBudget(input.customBudget),
    ...((EXPORT_LAYOUTS as readonly unknown[]).includes(input.exportLayout) ? { exportLayout: input.exportLayout as PackageLayout } : {}),
    ...(typeof input.targetProject === 'string' && input.targetProject.trim() ? { targetProject: input.targetProject } : {}),
    ...(typeof input.sharedIndexDir === 'string' && input.sharedIndexDir.trim() ? { sharedIndexDir: input.sharedIndexDir } : {}),
    ...(sanitizeCollections(input.collections) ? { collections: sanitizeCollections(input.collections) } : {}),
    ...(typeof input.aiPet === 'boolean' ? { aiPet: input.aiPet } : {}),
    ...(typeof input.usageStats === 'boolean' ? { usageStats: input.usageStats } : {}),
    ...(typeof input.skipLobby === 'boolean' ? { skipLobby: input.skipLobby } : {}),
    ...(input.recent && typeof input.recent === 'object'
      ? {
          recent: {
            viewed: sanitizeItems((input.recent as Record<string, unknown>).viewed).slice(0, RECENT_LIMIT),
            exported: sanitizeItems((input.recent as Record<string, unknown>).exported).slice(0, RECENT_LIMIT)
          }
        }
      : {}),
    ...(Array.isArray(input.watched)
      ? {
          watched: input.watched.flatMap((w): WatchedBranch[] => {
            const r = w as Record<string, unknown>
            return r && typeof r.repo === 'string' && typeof r.branch === 'string' && r.repo && r.branch ? [{ repo: r.repo, branch: r.branch, seen: typeof r.seen === 'string' ? r.seen : '' }] : []
          })
        }
      : {})
  }
}

/** Positive finite numbers for known limits only. */
function sanitizeBudget(raw: unknown): Pick<Settings, 'customBudget'> {
  if (!raw || typeof raw !== 'object') return {}
  const input = raw as Record<string, unknown>
  const budget: Partial<PlayableBudget> = {}
  for (const k of BUDGET_KEYS) {
    const v = input[k]
    if (typeof v === 'number' && Number.isFinite(v) && v > 0) budget[k] = v
  }
  return Object.keys(budget).length ? { customBudget: budget } : {}
}
