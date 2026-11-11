// Makes release/Asset-Hub-Installer.dmg: just the small "Install Asset Hub" installer, which downloads
// this version's build from the public release repo (package.json "releaseRepo"). Part of
// `bun run dist:web`, after `tauri build` leaves `Asset Hub.app.tar.gz` and its `.sig`.
// The two updater files are staged in release/ under the names the release uses (GitHub turns a space
// in an asset name into a dot, so they are named with dots here), and the SHA-256 of the archive is
// baked into the installer, so an old installer still installs a verified build and the release
// needs no checksum file.
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const root = new URL('..', import.meta.url).pathname
const release = join(root, 'release')
const bundled = join(root, 'target/release/bundle/macos')
const { version, releaseRepo } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
const baseURL = `https://github.com/${releaseRepo}/releases/download/v${version}/`

const ARCHIVE = 'Asset.Hub.app.tar.gz'
const built = join(bundled, 'Asset Hub.app.tar.gz')
for (const file of [built, `${built}.sig`]) {
  if (!existsSync(file)) throw new Error(`Missing ${file}: run "bunx tauri build" with TAURI_SIGNING_PRIVATE_KEY set (bun run dist:web does)`)
}
mkdirSync(release, { recursive: true })
copyFileSync(built, join(release, ARCHIVE))
copyFileSync(`${built}.sig`, join(release, `${ARCHIVE}.sig`))
const sha256 = createHash('sha256').update(readFileSync(join(release, ARCHIVE))).digest('hex')

// The installer app, compiled from AppleScript with this release filled in, wearing the app's icon.
const stage = join(release, 'web-installer')
rmSync(stage, { recursive: true, force: true })
mkdirSync(stage, { recursive: true })
const source = join(stage, 'installer.applescript')
writeFileSync(
  source,
  readFileSync(join(root, 'scripts/installer.applescript'), 'utf8')
    .replaceAll('{{BASE_URL}}', baseURL)
    .replaceAll('{{VERSION}}', version)
    .replaceAll('{{SHA256}}', sha256)
)
const app = join(stage, 'Install Asset Hub.app')
execFileSync('osacompile', ['-o', app, source])
rmSync(source)
copyFileSync(join(root, 'crates/app/icons/icon.icns'), join(app, 'Contents/Resources/applet.icns'))
copyFileSync(join(root, 'scripts/installer-helpers.sh'), join(app, 'Contents/Resources/installer-helpers.sh'))
execFileSync('codesign', ['--force', '--deep', '--sign', '-', app])

const dmg = join(release, 'Asset-Hub-Installer.dmg')
rmSync(dmg, { force: true })
execFileSync('hdiutil', ['create', '-volname', 'Install Asset Hub', '-srcfolder', stage, '-ov', '-format', 'UDZO', '-quiet', dmg])
rmSync(stage, { recursive: true, force: true })
console.log(`${dmg}  ${(statSync(dmg).size / 1024).toFixed(0)} KB  (installs ${version}, sha256 ${sha256}, from ${baseURL})`)
