// Publishes the build to the public release repo (package.json "releaseRepo") as GitHub release
// v<version>: the updater archive and its signature, latest.json (what installed copies read to update)
// and the web installer. Run it with the updater key in the environment:
//
//   TAURI_SIGNING_PRIVATE_KEY=<key or path> TAURI_SIGNING_PRIVATE_KEY_PASSWORD=<password> bun run release
//
// The key goes only to the build (`bun run dist:web`, run here so the archive is signed by it); the
// scan and `gh` never see it. gh must be signed in with write access to the release repo.
// The release notes are this version's section of CHANGELOG.md; there must be one. The secret scan
// (scripts/secret-scan.mjs) must find nothing. Every installed copy updates itself from this release,
// so publish only a build that has been approved.
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { scan } from './secret-scan.mjs'

const root = new URL('..', import.meta.url).pathname
const KEY_VARS = ['TAURI_SIGNING_PRIVATE_KEY', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD']
const ARCHIVE = 'Asset.Hub.app.tar.gz'
const PLATFORM = 'darwin-aarch64'

/** "## [2.0.0] — date" up to the next "## [" heading; undefined when the version has no section. */
export function changelogSection(changelog, version) {
  const heading = new RegExp(`^## \\[${version.replace(/\./g, '\\.')}\\][^\\n]*\\n([\\s\\S]*?)(?=^## \\[|(?![\\s\\S]))`, 'm')
  return heading.exec(changelog)?.[1].trim() || undefined
}

/** The static manifest of the Tauri updater for this release. */
export function buildLatestJson({ version, releaseRepo, notes, signature, date = new Date() }) {
  return {
    version,
    notes,
    pub_date: date.toISOString(),
    platforms: {
      [PLATFORM]: { url: `https://github.com/${releaseRepo}/releases/download/v${version}/${ARCHIVE}`, signature: signature.trim() }
    }
  }
}

function publish() {
  const { version, releaseRepo } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
  const key = process.env.TAURI_SIGNING_PRIVATE_KEY
  if (!key) throw new Error('Set TAURI_SIGNING_PRIVATE_KEY (and TAURI_SIGNING_PRIVATE_KEY_PASSWORD) for this run: the updater archive is signed with it.')

  const section = changelogSection(readFileSync(join(root, 'CHANGELOG.md'), 'utf8'), version)
  if (!section) throw new Error(`CHANGELOG.md has no "## [${version}]" section: rename "Unreleased" to the version first`)

  // The key reaches the build only; the later steps get the environment without it.
  const clean = { ...process.env }
  KEY_VARS.forEach((name) => delete clean[name])
  execFileSync('bun', ['run', 'dist:web'], { cwd: root, stdio: 'inherit' })

  const release = join(root, 'release')
  const files = [ARCHIVE, `${ARCHIVE}.sig`, 'Asset-Hub-Installer.dmg'].map((f) => join(release, f))
  const missing = files.filter((f) => !existsSync(f))
  if (missing.length) throw new Error(`Missing after the build: ${missing.join(', ')}`)

  const notes = `Install: run \`brew install zeemrx0/tap/asset-hub\`, or download **Asset-Hub-Installer.dmg** and run it (Apple silicon, macOS 14 or later). Installed copies offer the update by themselves.\n\n${section}`
  const latest = join(release, 'latest.json')
  writeFileSync(latest, `${JSON.stringify(buildLatestJson({ version, releaseRepo, notes, signature: readFileSync(files[1], 'utf8') }), null, 2)}\n`)

  const findings = scan(root, { files: [latest, files[1]], literal: key.includes('\n') || key.length > 60 ? key : undefined })
  if (findings.length) throw new Error(`Secret scan found ${findings.length} problem(s); nothing was published:\n${findings.map((f) => `  ${f}`).join('\n')}`)

  execFileSync('gh', ['release', 'create', `v${version}`, ...files, latest, '--repo', releaseRepo, '--title', `Asset Hub ${version}`, '--notes', notes], {
    stdio: 'inherit',
    env: clean
  })
}

if (import.meta.main) publish()
