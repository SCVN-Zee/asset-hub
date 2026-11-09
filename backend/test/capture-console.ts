const METHODS = ['log', 'info', 'warn', 'error', 'debug'] as const

/** Records every console call until `restore()`; nothing is printed meanwhile. */
export function captureConsole(): { calls: unknown[][]; restore: () => void } {
  const calls: unknown[][] = []
  const originals = METHODS.map((m) => [m, console[m]] as const)
  for (const m of METHODS) console[m] = (...args: unknown[]) => void calls.push(args)
  return {
    calls,
    restore: () => {
      for (const [m, fn] of originals) console[m] = fn
    }
  }
}
