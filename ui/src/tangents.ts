// JSON has no Infinity: the Rust side writes a step tangent (an infinite slope) as `null`.

/**
 * Walks parsed JSON and turns the `null` that stands for an infinite tangent (`inS`, `outS`) back into
 * `Infinity`. Done in place; answers the value it was given.
 */
export function reviveTangents<T>(value: T): T {
  const walk = (node: unknown): void => {
    if (Array.isArray(node)) {
      for (const item of node) walk(item)
    } else if (node && typeof node === 'object') {
      const record = node as Record<string, unknown>
      for (const [key, child] of Object.entries(record)) {
        if ((key === 'inS' || key === 'outS') && Array.isArray(child)) record[key] = child.map((v) => (v === null ? Infinity : v))
        else walk(child)
      }
    }
  }
  walk(value)
  return value
}
