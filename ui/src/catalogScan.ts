// What the prefab scan found, kept by blob SHA. Which branches to scan is worked out where the whole
// catalog is (`catalog.prefabScanTargets`).

import type { AssetFacts, FactKind } from '@shared/types'

/** Adds the SHAs a scan found to the known ones. Returns `prev` itself when nothing is new, so React skips the update. */
export function mergeKnown(prev: AssetFacts, kind: FactKind, shas: readonly string[]): AssetFacts {
  const known = prev[kind]
  if (shas.every((sha) => known?.has(sha))) return prev
  return { ...prev, [kind]: new Set([...(known ?? []), ...shas]) }
}
