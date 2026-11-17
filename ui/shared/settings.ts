import { BUDGET_KEYS, BUDGET_PRESETS, type BudgetPreset, type PackageLayout } from '../src/bindings'
import type { PlayableBudget } from './types'
import { RECENT_LIMIT, sanitizeCollections, sanitizeItems, type Collection, type RecentLists } from './collections'

export const LANGUAGES = ['en', 'ko', 'vi'] as const
export type Language = (typeof LANGUAGES)[number]
/** In English, for telling the AI assistant which language to answer in. */
export const LANGUAGE_NAMES: Record<Language, string> = { en: 'English', vi: 'Vietnamese', ko: 'Korean' }

export interface Settings {
  language: Language
  /** Patterns hiding folders and files from browsing: gitignore syntax, one per entry, relative to the Unity project root (a bare name matches at any depth). */
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
  /** Favourites and named collections of assets (see shared/collections). */
  collections?: Collection[]
  /** Assets viewed and exported lately, newest first (kept by the app). */
  recent?: RecentLists
}

export const EXPORT_LAYOUTS: readonly PackageLayout[] = ['byType', 'grouped', 'source']
export const DEFAULT_EXPORT_LAYOUT: PackageLayout = 'byType'

export const DEFAULT_EXCLUDED_FOLDERS = ['_ThirdParty']

export function isLanguage(value: unknown): value is Language {
  return typeof value === 'string' && (LANGUAGES as readonly string[]).includes(value)
}

/** Merges untrusted stored/IPC data over defaults, dropping anything invalid. */
/** Every setting this version knows; anything else in the file was written by another version. */
export const SETTINGS_KEYS = [
  'language', 'excludedFolders', 'lastRepo', 'lastBranch', 'budgetPreset', 'customBudget',
  'exportLayout', 'targetProject', 'collections', 'recent'
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
    excludedFolders: folders,
    ...(typeof input.lastRepo === 'string' && input.lastRepo ? { lastRepo: input.lastRepo } : {}),
    ...(typeof input.lastBranch === 'string' && input.lastBranch ? { lastBranch: input.lastBranch } : {}),
    ...((BUDGET_PRESETS as readonly unknown[]).includes(input.budgetPreset) ? { budgetPreset: input.budgetPreset as BudgetPreset } : {}),
    ...sanitizeBudget(input.customBudget),
    ...((EXPORT_LAYOUTS as readonly unknown[]).includes(input.exportLayout) ? { exportLayout: input.exportLayout as PackageLayout } : {}),
    ...(typeof input.targetProject === 'string' && input.targetProject.trim() ? { targetProject: input.targetProject } : {}),
    ...(sanitizeCollections(input.collections) ? { collections: sanitizeCollections(input.collections) } : {}),
    ...(input.recent && typeof input.recent === 'object'
      ? {
          recent: {
            viewed: sanitizeItems((input.recent as Record<string, unknown>).viewed).slice(0, RECENT_LIMIT),
            exported: sanitizeItems((input.recent as Record<string, unknown>).exported).slice(0, RECENT_LIMIT)
          }
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
