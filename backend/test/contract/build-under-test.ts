import { fileURLToPath } from 'node:url'

export type Build = 'ts' | 'rust'

/** The build under test, from `CONTRACT_TARGET`. */
export function contractTarget(): Build {
  const value = process.env.CONTRACT_TARGET ?? 'ts'
  if (value !== 'ts' && value !== 'rust') throw new Error(`CONTRACT_TARGET must be ts or rust, got ${value}`)
  return value
}

export const backendDir = fileURLToPath(new URL('../../', import.meta.url))

/** Parent of the per-run bundle folders. */
export const contractRoot = `${backendDir}.contract`
