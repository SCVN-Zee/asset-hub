// Which branches to read so every prefab of a catalog listing gets its tags (particle system, UI).
// The prefab scan reads one branch at a time and keeps what it finds by blob SHA, so a few branches
// that between them hold every prefab are enough; reading one branch downloads only the prefabs no
// earlier scan has read.

import type { CatalogAsset } from '@shared/types'

export interface ScanTarget {
  repo: string
  branch: string
}

/**
 * Branches to scan, most prefabs first, until each prefab of the listing is in a scanned branch.
 * `scanned` says whether a branch is read already (or was tried and failed): the prefabs in it need nothing more.
 */
export function prefabScanTargets(assets: readonly Pick<CatalogAsset, 'type' | 'locations'>[], scanned: (repo: string, branch: string) => boolean): ScanTarget[] {
  // Prefabs no scanned branch holds, each with the branches (of its first locations) that do hold it.
  let open: ScanTarget[][] = []
  for (const a of assets) {
    if (a.type !== 'prefab') continue
    const holders = a.locations.flatMap((l) => l.branches.map((branch) => ({ repo: l.repo, branch })))
    if (holders.length && !holders.some((h) => scanned(h.repo, h.branch))) open.push(holders)
  }
  const targets: ScanTarget[] = []
  while (open.length) {
    const counts = new Map<string, { target: ScanTarget; n: number }>()
    for (const holders of open) {
      for (const h of holders) {
        const key = `${h.repo}@${h.branch}`
        const entry = counts.get(key)
        if (entry) entry.n += 1
        else counts.set(key, { target: h, n: 1 })
      }
    }
    const best = [...counts.values()].reduce((a, b) => (b.n > a.n ? b : a))
    targets.push(best.target)
    open = open.filter((holders) => !holders.some((h) => h.repo === best.target.repo && h.branch === best.target.branch))
  }
  return targets
}
