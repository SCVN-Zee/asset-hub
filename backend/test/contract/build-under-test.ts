import { fileURLToPath } from 'node:url'

export const backendDir = fileURLToPath(new URL('../../', import.meta.url))

/** Parent of the per-run bundle folders. */
export const contractRoot = `${backendDir}.contract`
