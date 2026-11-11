const UNITS = ['B', 'KB', 'MB', 'GB']

/** Project path without its first two folders, which are the same for most assets: Assets/Supercent/X/a.fbx → ../X/a.fbx. */
export function shortPath(path: string): string {
  const parts = path.split('/')
  return parts.length > 3 ? `../${parts.slice(2).join('/')}` : path
}

export function formatBytes(bytes: number | undefined): string {
  if (bytes === undefined || !Number.isFinite(bytes) || bytes < 0) return '—'
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit++
  }
  const digits = unit === 0 || value >= 100 ? 0 : 1
  return `${value.toFixed(digits)} ${UNITS[unit]}`
}

export function baseName(path: string): string {
  const i = path.lastIndexOf('/')
  return i === -1 ? path : path.slice(i + 1)
}
