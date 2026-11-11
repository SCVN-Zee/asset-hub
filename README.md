# Asset Hub

Desktop app (Tauri 2, Rust and React) for macOS 14 or later on Apple silicon, for browsing Unity projects on GitHub: preview models, prefabs, VFX, textures, audio and fonts straight from a branch, collect what you need, and export it with every dependency (materials, textures, shaders, animations…) as a `.unitypackage`, checked against your own Unity project.

- [Install guide](docs/INSTALL.md): `brew install zeemrx0/tap/asset-hub`, or the installer DMG from the [releases](https://github.com/SCVN-Zee/asset-hub/releases)
- [Architecture and decisions](docs/architecture.md)
- [Changelog](CHANGELOG.md)
- [1.x plan](docs/history/PLAN.md) and [1.x roadmap](docs/history/ROADMAP.md): Electron-era history, kept for reference

## Run

Needs [bun](https://bun.sh), the Rust toolchain (`rustup`, stable) and Xcode's command line tools (`xcode-select --install`).

```bash
bun install
bun run prebuild:cli   # once on a clean checkout: builds the CLI the app bundles (needed by cargo test too)
bun run dev            # the app with hot reload (data in ~/Library/Application Support/vn.supercent.assethub-dev)
bun run test           # TypeScript tests (vitest)
bun run typecheck
cargo test --workspace
bun run bindings       # regenerate ui/src/bindings.ts after changing a command
```

Sign in with GitHub (GitHub CLI, installed by the app if missing, or a fine-grained token). The account needs read access to the asset repos.

## Release

Releases come from CI. Pushing a tag `v<version>` (stable, no `-`) runs [`.github/workflows/release.yml`](.github/workflows/release.yml) on `macos-15`: it checks the tag against `package.json`, runs the TypeScript and Rust tests, then `bun run release`, which builds and publishes the GitHub release to this repository (`releaseRepo` in `package.json`, `SCVN-Zee/asset-hub`). A last step, `scripts/ci/bump-homebrew-cask.sh`, renders [`packaging/homebrew/asset-hub.rb`](packaging/homebrew/asset-hub.rb) with the new version and the checksum of `Asset.Hub.app.tar.gz` and pushes it to the `zeemrx0/tap` Homebrew tap. The app, the installer and the release script all read `releaseRepo` from `package.json`.

The app updates itself with the Tauri updater, so every update is signed with a minisign key that only the maintainer holds. **One-time setup before the first CI release:**

```bash
bunx tauri signer generate -w ~/asset-hub-updater.key   # keep the key and its password in a password manager, never in the repo
```

1. Put the printed **public** key in `plugins.updater.pubkey` of `crates/app/tauri.conf.json`. It ships as a placeholder that no private key matches, so until then no update installs.
2. Add these repository secrets in `SCVN-Zee/asset-hub` (Settings › Secrets and variables › Actions): `TAURI_SIGNING_PRIVATE_KEY` (the key content), `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, and `HOMEBREW_TAP_TOKEN` (a fine-grained token with Contents read/write on `ZeeMRx0/homebrew-tap`). The workflow stops at its first step with a clear message when one is missing.

If the key is lost, installed copies cannot verify updates and users must run the installer again (see [docs/architecture.md](docs/architecture.md)).

To release: bump `version` in `package.json`, rename **Unreleased** in [CHANGELOG.md](CHANGELOG.md) to that version (its section becomes the release notes, and the script refuses without one), commit, then `git tag v<version> && git push origin v<version>`.

To build by hand, with `TAURI_SIGNING_PRIVATE_KEY` (the key, or its path) and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` in the environment, because the updater archive is signed while it is built:

```bash
bun run dist:web   # tauri build, then release/Asset-Hub-Installer.dmg (~1.4 MB) + Asset.Hub.app.tar.gz + .sig
bun run release    # builds with the key, runs the secret scan (scripts/secret-scan.mjs), then publishes v<version> to the release repo
bun run dist       # (optional) the full dmg from Tauri's own bundler
```

**Every installed copy updates itself from the release repo**, so tag only a build that has been tested and approved. The `.app` is signed ad hoc (`bundle.macOS.signingIdentity: "-"`), and an updated copy must not be reached through a symlink. See [docs/INSTALL.md](docs/INSTALL.md).

## Use from coding agents

`asset-hub-cli` searches and exports assets without the app window; the skill in `agent/skills/asset-hub/` teaches Claude Code and Codex to use it.

```bash
cargo install --path crates/app --bin asset-hub-cli --locked --root "$HOME/.local"   # installs to ~/.local/bin
ln -sfn "$PWD/agent/skills/asset-hub" ~/.claude/skills/asset-hub
mkdir -p ~/.codex/skills && ln -sfn "$PWD/agent/skills/asset-hub" ~/.codex/skills/asset-hub
asset-hub-cli search "fire" --repo owner/repo --branch main
asset-hub-cli export owner/repo@main:Assets/VFX/Fire.prefab --output ./Imports/fire.unitypackage --project .
```

Team setup (install from Settings, skill, MCP server, sign-in): [docs/AGENTS-USE.md](docs/AGENTS-USE.md).

On a clean checkout, run `bun run prebuild:cli` once before `cargo test` or `cargo clippy` (it creates `crates/app/binaries/`, which the Tauri build script requires because the app bundles the CLI); `bun run build` (and `tauri build`) runs it for you.

## Layout

```
Cargo.toml         Rust workspace root (Cargo.lock and target/ live here too)
crates/core/       Rust domain logic, no Tauri dependency
  src/github/        GitHub source: trees, files, Git LFS, the read-only guard
  src/unity/         Unity YAML, .meta, particles, animation clips, imported IDs
  src/deps/          dependency resolution (prefab → models, materials, textures, scripts…)
  src/export/        .unitypackage writer, target-project checks, shader mapping, script stripping
  src/handlers.rs    one handler per asset type, registered in create_default_registry()
  src/catalog.rs src/search/   index of every branch and search over it
  tests/golden/      frozen outputs of the 1.x TypeScript code that the port must reproduce
crates/app/        the Tauri shell (package asset-hub: the app and asset-hub-cli)
  src/commands/      the typed commands the page calls
  src/updater.rs     self-update (Tauri updater)
  src/headless/      asset-hub-cli and its MCP server
  capabilities/      what the page may call
  tauri.conf.json    bundle, CSP, updater key
ui/                the page (React, shown in the Tauri web view)
  shared/            TypeScript types and helpers the page shares (IPC contract, settings, AI tools)
  src/bindings.ts    generated by `bun run bindings`: never edit it
  src/api.ts         adapts the generated commands to window.api
  src/i18n/          i18next, locales/en.json (default) + vi.json + ko.json
  src/screens/       SignIn (3D lobby), Browse, Settings
  src/components/    folder tree, asset cards and details, export bar, collections…
  src/viewer/        three.js viewers (models, prefabs, VFX, lobby scene)
scripts/           installer, release, secret scan and sidecar build
```

**New asset type:** add a handler in `crates/core/src/handlers.rs`, register it in `create_default_registry()` and add the `assetTypes.<id>` key to every locale file. Tabs, filters and the folder tree pick it up.

**New UI string:** add it to `en.json`, `vi.json` and `ko.json`; `locales.test.ts` fails when a key is missing.

**New command:** write it in `crates/app/src/commands/`, list it in `crates/app/src/command_names.rs` and `lib.rs`, grant it in `crates/app/capabilities/main.json`, then run `bun run bindings`.

## Install problems (dev)

- **`failed to run custom build command` for `asset-hub`** about `binaries/asset-hub-cli-<triple>`: run `bun run prebuild:cli` (or `touch crates/app/binaries/asset-hub-cli-aarch64-apple-darwin` for a placeholder).
- **`rustc` or `cargo` not found:** install the toolchain with `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh`, then open a new shell.
- **`bun run build` stops at the signing step:** set `TAURI_SIGNING_PRIVATE_KEY` (a test key from `bunx tauri signer generate` is enough for a local build).
