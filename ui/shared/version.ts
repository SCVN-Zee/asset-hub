/** `a` is a later version than `b` ("0.1.10" > "0.1.9"; a leading "v" is ignored). */
export function isNewer(a: string, b: string): boolean {
  const parts = (v: string) => v.replace(/^v/, '').split(/[.-]/).map((p) => Number.parseInt(p, 10) || 0)
  const [x, y] = [parts(a), parts(b)]
  for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0)
  return false
}
