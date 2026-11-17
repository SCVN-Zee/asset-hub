import { execFileSync } from 'node:child_process'
import { mkdirSync, mkdtempSync, readdirSync, rmSync, statSync } from 'node:fs'
import { join } from 'node:path'
import type { TestProject } from 'vitest/node'
import { backendDir, contractRoot } from './build-under-test'

const STALE_RUN_MS = 24 * 60 * 60 * 1000

declare module 'vitest' {
  export interface ProvidedContext {
    /** Folder that holds the bundle this run tests. Private to the run. */
    bundleDir: string
  }
}

/** Removes run folders that a crashed run left behind. A run in progress is younger than a day. */
function pruneStaleRuns(): void {
  for (const name of readdirSync(contractRoot).filter((n) => n.startsWith('run-'))) {
    const dir = join(contractRoot, name)
    try {
      if (Date.now() - statSync(dir).mtimeMs > STALE_RUN_MS) rmSync(dir, { recursive: true, force: true })
    } catch {
      // Another run removed it first.
    }
  }
}

/**
 * Builds the bundle that production gets, once per run, into a folder
 * of its own. Concurrent runs never touch each other's bundle. The folder goes away after the run.
 */
export default function setup({ provide }: TestProject): () => void {
  mkdirSync(contractRoot, { recursive: true })
  pruneStaleRuns()
  const runDir = mkdtempSync(join(contractRoot, 'run-'))
  const out = join(runDir, 'worker')
  try {
    execFileSync('npx', ['wrangler', 'deploy', '--dry-run', '--outdir', out], { cwd: backendDir, stdio: 'pipe' })
  } catch (err) {
    rmSync(runDir, { recursive: true, force: true })
    const e = err as { stdout?: Buffer; stderr?: Buffer }
    throw new Error(`wrangler dry-run failed\n${e.stdout?.toString() ?? ''}\n${e.stderr?.toString() ?? ''}`)
  }
  provide('bundleDir', out)
  return () => rmSync(runDir, { recursive: true, force: true })
}
