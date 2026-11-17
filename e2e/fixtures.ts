// Synthetic, deterministic data for the fake backend: the same numbers on every run.

export const REPOS = ['acme/alpha', 'acme/beta', 'acme/gamma']
export const DEFAULT_BRANCH = 'main'
/** Prime counts, so the last grid row is partial for any column count above one. */
export const CATALOG_COUNT = 5003
export const BRANCH_COUNT = 1499
/** Assets whose name holds this word: a query for it finds only these. */
export const RARE_WORD = 'zzrare'
export const RARE_COUNT = 4

const TYPES = [
  { type: 'fbx', ext: 'fbx' },
  { type: 'texture', ext: 'png' },
  { type: 'material', ext: 'mat' },
  { type: 'audio', ext: 'wav' },
  { type: 'prefab', ext: 'prefab' },
  { type: 'animation', ext: 'anim' },
  { type: 'font', ext: 'ttf' }
]
const FOLDERS = ['Characters', 'Props', 'Environment', 'VFX', 'UI', 'Audio', 'Weapons', 'Vehicles']
const SUBFOLDERS = ['Common', 'Hero', 'Enemy', 'Level01', 'Level02', 'Shared']
const WORDS = ['Crate', 'Barrel', 'Shark', 'Tree', 'Robot', 'Sword', 'Coin', 'Gem', 'Door', 'Wall', 'Flame', 'Smoke', 'Button', 'Panel', 'Engine']
const LONG_TAIL = 'WithAnExtremelyLongDescriptiveNameThatNeedsToWrapOrTruncateInTheCard'

/** mulberry32: a small seeded generator. */
function random(seed: number): () => number {
  let a = seed
  return () => {
    a = (a + 0x6d2b79f5) | 0
    let t = Math.imul(a ^ (a >>> 15), 1 | a)
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296
  }
}

const sha = (n: number, salt: number) => {
  const next = random(n * 7919 + salt)
  return Array.from({ length: 5 }, () => Math.floor(next() * 0xffffffff).toString(16).padStart(8, '0')).join('')
}

export interface FixtureAsset {
  id: string
  type: string
  path: string
  name: string
  sha: string
  size: number
  lfs: boolean
}

export interface FixtureCatalogAsset extends FixtureAsset {
  locations: Array<{ repo: string; path: string; branches: string[] }>
  branchCount: number
  repoCount: number
  repos: string[]
}

function asset(i: number, salt: number): FixtureAsset {
  const next = random(i + salt)
  const { type, ext } = TYPES[Math.floor(next() * TYPES.length)]
  const word = WORDS[Math.floor(next() * WORDS.length)]
  const rare = i < RARE_COUNT ? `${RARE_WORD}_` : ''
  const long = next() < 0.08 ? LONG_TAIL : ''
  const name = `${rare}${word}_${i}${long}.${ext}`
  const path = `Assets/${FOLDERS[Math.floor(next() * FOLDERS.length)]}/${SUBFOLDERS[Math.floor(next() * SUBFOLDERS.length)]}/${name}`
  return { id: path, type, path, name, sha: sha(i, salt), size: 1024 + Math.floor(next() * 40_000_000), lfs: next() < 0.1 }
}

export function branchAssets(count = BRANCH_COUNT): FixtureAsset[] {
  return Array.from({ length: count }, (_, i) => asset(i, 11))
}

export function catalogAssets(count = CATALOG_COUNT): FixtureCatalogAsset[] {
  return Array.from({ length: count }, (_, i) => {
    const base = asset(i, 3)
    const next = random(i * 31 + 5)
    const spots = 1 + Math.floor(next() * 3)
    const locations = Array.from({ length: spots }, (_, k) => ({
      repo: REPOS[(i + k) % REPOS.length],
      path: base.path,
      branches: k === 0 ? [DEFAULT_BRANCH, `feature/f${i % 9}`] : [DEFAULT_BRANCH]
    }))
    return { ...base, locations, branchCount: locations.reduce((n, l) => n + l.branches.length, 0), repoCount: spots, repos: [...new Set(locations.map((l) => l.repo))] }
  })
}

/** Measurements for about half of the blobs, so cards show the optional fact line (triangles, size in pixels). */
export function statsFor(blob: string): Record<string, unknown> | undefined {
  const n = parseInt(blob.slice(0, 8), 16)
  if (n % 2) return undefined
  return {
    triangles: 500 + (n % 90_000),
    vertices: 300,
    bones: 0,
    clips: 0,
    materials: 1,
    fileBytes: 2048,
    textureBytes: 4096,
    maxTextureSize: 512,
    width: 64 + (n % 960),
    height: 64 + (n % 960),
    source: 'thumb'
  }
}
