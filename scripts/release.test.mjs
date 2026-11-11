// Tests of the release scripts' logic that needs no network, no dialog and no signing key:
// the installer's shell checks, the updater manifest, the changelog notes and the secret scan.
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterAll, describe, expect, it } from 'vitest'
import { buildLatestJson, changelogSection } from './publish-release.mjs'
import { scanText } from './secret-scan.mjs'

const dir = mkdtempSync(join(tmpdir(), 'release-test-'))
afterAll(() => rmSync(dir, { recursive: true, force: true }))

const helpers = join(import.meta.dirname, 'installer-helpers.sh')
/** Runs one helper function the way the installer does: `. helpers; fn args`; its exit status. */
const helper = (fn, ...args) => spawnSync('/bin/sh', ['-c', `. "$1"; shift; ${fn} "$@"`, 'sh', helpers, ...args]).status

function appWith(name, version) {
  const app = join(dir, name)
  mkdirSync(join(app, 'Contents'), { recursive: true })
  writeFileSync(
    join(app, 'Contents/Info.plist'),
    `<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleShortVersionString</key><string>${version}</string></dict></plist>`
  )
  return app
}

describe('installer: which installed copy is replaced', () => {
  it('treats a version below 2 as 1.x', () => {
    for (const version of ['1.0.5', '0.9.0', '1', '1.99.9']) expect(helper('is_legacy_app', appWith(`a-${version}.app`, version)), version).toBe(0)
  })

  it('leaves 2.x and later alone', () => {
    for (const version of ['2.0.0', '2.0.1', '10.0.0']) expect(helper('is_legacy_app', appWith(`b-${version}.app`, version)), version).toBe(1)
  })

  it('leaves an app whose version is missing or not a number alone', () => {
    expect(helper('is_legacy_app', join(dir, 'missing.app'))).toBe(1)
    expect(helper('is_legacy_app', appWith('c-text.app', 'beta'))).toBe(1)
    expect(helper('is_legacy_app', appWith('c-empty.app', ''))).toBe(1)
  })
})

describe('installer: the baked checksum', () => {
  const file = join(dir, 'Asset.Hub.app.tar.gz')
  writeFileSync(file, 'the release archive')
  const sha = createHash('sha256').update('the release archive').digest('hex')

  it('accepts the archive that matches', () => {
    expect(helper('sha256_matches', file, sha)).toBe(0)
  })

  it('refuses a tampered archive', () => {
    const tampered = join(dir, 'tampered.tar.gz')
    writeFileSync(tampered, 'the release archive!')
    expect(helper('sha256_matches', tampered, sha)).toBe(1)
  })

  it('refuses an empty or wrong expected value and a missing file', () => {
    expect(helper('sha256_matches', file, '')).toBe(1)
    expect(helper('sha256_matches', file, sha.replace(/^./, '0'))).toBe(1)
    expect(helper('sha256_matches', join(dir, 'nothing'), sha)).toBe(1)
  })
})

describe('installer: the script compiles with a release filled in', () => {
  it('has no placeholder left and compiles', () => {
    const source = execFileSync('cat', [join(import.meta.dirname, 'installer.applescript')], { encoding: 'utf8' })
      .replaceAll('{{BASE_URL}}', 'https://github.com/o/r/releases/download/v2.0.0/')
      .replaceAll('{{VERSION}}', '2.0.0')
      .replaceAll('{{SHA256}}', 'a'.repeat(64))
    expect(source).not.toContain('{{')
    const script = join(dir, 'installer.applescript')
    writeFileSync(script, source)
    execFileSync('osacompile', ['-o', join(dir, 'Install.app'), script])
  })
})

describe('publish: latest.json and the notes', () => {
  it('lists the archive and its signature under the Apple silicon platform', () => {
    const json = buildLatestJson({
      version: '2.0.1',
      releaseRepo: 'owner/repo',
      notes: 'Fixed things',
      signature: 'c2lnbmF0dXJl\n',
      date: new Date('2026-11-11T00:00:00Z')
    })
    expect(json).toEqual({
      version: '2.0.1',
      notes: 'Fixed things',
      pub_date: '2026-11-11T00:00:00.000Z',
      platforms: { 'darwin-aarch64': { url: 'https://github.com/owner/repo/releases/download/v2.0.1/Asset.Hub.app.tar.gz', signature: 'c2lnbmF0dXJl' } }
    })
  })

  it('takes the notes of one version only, and none when it has no section', () => {
    const log = '# Changelog\n\n## [2.0.1] — d\n\n### Fixed\n- b\n\n## [2.0.0] — d\n\n### Added\n- a\n'
    expect(changelogSection(log, '2.0.0')).toBe('### Added\n- a')
    expect(changelogSection(log, '2.0.1')).toBe('### Fixed\n- b')
    expect(changelogSection(log, '2.0.2')).toBeUndefined()
  })
})

describe('secret scan', () => {
  it('finds private keys and tokens', () => {
    expect(scanText('-----BEGIN ' + 'OPENSSH PRIVATE KEY-----\nabc')).toEqual(['private key block'])
    expect(scanText('untrusted comment: ' + 'minisign secret key\nRW...')).toEqual(['minisign secret key'])
    expect(scanText(Buffer.from('untrusted comment: ' + 'minisign secret key\nRWRT').toString('base64'))).toEqual(['minisign secret key'])
    expect(scanText('token=' + 'ghp_' + 'a'.repeat(36))).toEqual(['GitHub token'])
  })

  it('finds the key from the environment by its value', () => {
    const literal = 'x'.repeat(40)
    expect(scanText(`key: ${literal}`, literal)).toEqual(['the signing key from the environment'])
  })

  it('passes a public key and plain text', () => {
    expect(scanText('untrusted comment: minisign public key: 85FF168E4F34A316\nRWQW')).toEqual([])
    expect(scanText('releaseRepo: owner/name')).toEqual([])
  })
})
