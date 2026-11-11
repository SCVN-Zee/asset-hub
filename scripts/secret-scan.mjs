// A secret scan for the release: no private key or token may be in the repo (working tree and every
// commit) or in the release files. Used by publish-release.mjs, which refuses on any finding.
// Runs gitleaks as well when it is installed. Run alone: `bun scripts/secret-scan.mjs [extra files...]`.
import { execFileSync, spawnSync } from 'node:child_process'
import { readFileSync, statSync } from 'node:fs'
import { join } from 'node:path'

// Built from parts so that this file does not match its own patterns.
const MINISIGN_SECRET = ['untrusted comment: ', 'minisign secret key'].join('')
const MINISIGN_SECRET_B64 = Buffer.from(MINISIGN_SECRET.slice(0, 36)).toString('base64')

export const PATTERNS = [
  { name: 'private key block', regex: /-----BEGIN [A-Z ]*PRIVATE KEY-----/ },
  { name: 'minisign secret key', regex: new RegExp(`${MINISIGN_SECRET}|${MINISIGN_SECRET_B64}`) },
  { name: 'GitHub token', regex: /\bgh[pousr]_[A-Za-z0-9]{36,}\b/ },
  { name: 'GitHub fine-grained token', regex: /\bgithub_pat_[A-Za-z0-9_]{50,}\b/ }
]

/** The names of the patterns (and of the literal secret, when given) that `text` matches. */
export function scanText(text, literal) {
  const found = PATTERNS.filter(({ regex }) => regex.test(text)).map(({ name }) => name)
  if (literal && literal.length >= 20 && text.includes(literal)) found.push('the signing key from the environment')
  return found
}

function git(root, args) {
  return execFileSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 1 << 30 })
}

function readable(path) {
  try {
    const stat = statSync(path)
    if (!stat.isFile() || stat.size > 5 * 1024 * 1024) return undefined
    const bytes = readFileSync(path)
    return bytes.includes(0) ? undefined : bytes.toString('utf8')
  } catch {
    return undefined
  }
}

/**
 * Every finding as "where: what". `files` are extra files to scan (the release's own). `literal` is the
 * value of the signing key in the environment, which must not appear anywhere either.
 */
export function scan(root, { files = [], literal } = {}) {
  const findings = []
  const note = (where, text) => scanText(text, literal).forEach((what) => findings.push(`${where}: ${what}`))

  const tracked = git(root, ['ls-files', '-co', '--exclude-standard']).split('\n').filter(Boolean)
  for (const file of tracked) {
    if (/\.key$/.test(file)) findings.push(`${file}: a .key file is in the repo`)
    const text = readable(join(root, file))
    if (text !== undefined) note(file, text)
  }
  for (const file of files) {
    const text = readable(file)
    if (text !== undefined) note(file, text)
  }
  // Every added line of every commit on every branch.
  const history = git(root, ['log', '--all', '-p', '--no-color', '--format=commit %h'])
  let commit = ''
  const seen = new Set()
  for (const line of history.split('\n')) {
    if (line.startsWith('commit ')) commit = line.slice(7)
    else if (line.startsWith('+') && !line.startsWith('+++')) {
      for (const what of scanText(line, literal)) {
        const finding = `commit ${commit}: ${what}`
        if (!seen.has(finding)) (seen.add(finding), findings.push(finding))
      }
    }
  }

  const gitleaks = spawnSync('gitleaks', ['detect', '--source', root, '--no-banner', '--redact'], { encoding: 'utf8' })
  if (!gitleaks.error && gitleaks.status !== 0) findings.push(`gitleaks: ${(gitleaks.stdout + gitleaks.stderr).trim().split('\n').slice(-3).join(' ')}`)
  return findings
}

if (import.meta.main) {
  const root = new URL('..', import.meta.url).pathname
  const findings = scan(root, { files: process.argv.slice(2) })
  if (findings.length) {
    console.error(`Secret scan found ${findings.length} problem(s):\n${findings.map((f) => `  ${f}`).join('\n')}`)
    process.exit(1)
  }
  console.log('Secret scan: nothing found.')
}
