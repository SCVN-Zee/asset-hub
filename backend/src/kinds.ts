/**
 * Fact kinds the desktop shares per repo. Must equal `SHARED_FACT_KINDS` in
 * `src/core/github/source.ts` (a desktop test compares the two).
 */
export const FACT_KINDS = ['metaguids', 'prefabmodels', 'prefabfx4', 'spritemeta', 'tmpfont'] as const

/** Names accepted by `/cache/:name`. */
export const CACHE_OBJECT_NAMES: readonly string[] = ['catalog', 'stats', ...FACT_KINDS.map((kind) => `facts-${kind}`)]
