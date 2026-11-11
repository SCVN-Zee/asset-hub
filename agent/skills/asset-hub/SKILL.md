---
name: asset-hub
description: Find Unity assets (models, prefabs, VFX, materials, textures, audio, fonts) in the team's GitHub repos and export them with their dependencies as a .unitypackage, via the CLI asset-hub-cli. Use when the user asks to find, inspect, or export an asset from the team's repos.
---

# Asset Hub CLI

`asset-hub-cli` searches the team's Unity GitHub repos, inspects an asset, and exports it with dependencies to a `.unitypackage`. It works with the Asset Hub window closed. It never imports into Unity and never changes the app's state.

Every command prints one JSON line on stdout: `{"ok":true,"data":...}` or `{"ok":false,"error":{"code","message"}}`. Add `--pretty` to indent. Read the JSON, not the exit code alone.

## Asset reference

`owner/repo@branch:path`, for example `supercent-io/luna_pizza@main:Assets/Art/VFX/Fire.prefab`. Branch names may contain `/`. Use the `ref` values that `search` returns; do not compose them by hand.

## Workflow

1. Search: `asset-hub-cli search "fire" --repo owner/repo --branch main`
2. Inspect (optional): `asset-hub-cli info <ref>` (type, size, dependency count) and `asset-hub-cli deps <ref>` (files an import copies; `--mode instant|quick|full`, default full).
3. Export: `mkdir -p ./Imports && asset-hub-cli export <ref> [<ref>...] --output ./Imports/fire.unitypackage --project .`
4. Tell the user the output path and what the export contained.

Other commands: `asset-hub-cli whoami` (account and token source), `asset-hub-cli repos`, `asset-hub-cli branches owner/repo`, `asset-hub-cli link <ref>` (an `assethub://open` URL; show it when the user wants to open the asset in the app).

## Search

- Use short English file-name words: `fire`, `water splash`. Use `a | b` for alternatives: `"fire | flame"`.
- `--type prefab|texture|...` narrows by asset type. `--limit N` (1-100, default 20).
- Scope:
  - `--repo R --branch B`: one branch. Fast. Prefer this.
  - No `--repo` and no `--branch`: every repo in the app's catalog.
  - `--repo R` alone fails with `bad_args`. Pass `--branch`, or `--where repo` to walk every branch of R (slow, use only when the user accepts that).
  - `--branch` without `--repo` is `bad_args`.
- If the user names no repo or branch, try the global search first. If it returns `catalog_empty`, stop that route (see below).

## Export rules

- Export only when the user named the output path or the Unity project (or both). If neither is clear, ask first.
- `--output` must end in `.unitypackage`, and its parent folder must already exist: run `mkdir -p <parent>` first.
- It refuses to overwrite an existing file. Pass `--force` only when the user asked to replace it.
- It refuses paths inside Asset Hub's data or cache folders. Choose a path in the user's project or a folder they named.
- `--project <dir>` must be a Unity project folder (it has `Assets/` and `ProjectSettings/`). It is only read, to leave out what the project already has.
- All refs in one export must share the same repo and branch (`mixed_branches` otherwise).
- Optional: `--layout source|grouped|by-type` (folder layout inside the package), `--keep-scripts` (keep scripts the export would remove).
- The export never imports into Unity. The user imports the file.

## Errors and exit codes

Exit codes: 0 ok, 1 user error, 2 bad arguments, 3 unauthorized, 4 network or rate limit, 5 unknown.

- `unauthorized` (3): ask the user to sign in to Asset Hub, run `gh auth login`, or set `ASSETHUB_GITHUB_TOKEN`. Do not try to sign in yourself.
- `catalog_empty`: Asset Hub has not indexed repos yet. Tell the user to open the app once, or pick a repo (`repos`) and branch (`branches owner/repo`) and search there. Never retry the global search in a loop.
- `bad_args` (2): fix the command from the message (often a scope rule above).
- Network or rate limit (4): wait and retry once; if it fails again, report it.
- Any other error: report the code and message to the user. Do not loop.

## Notes

- Measured stats (triangles and similar) appear in `info` only when the user opened the asset once in the app. Otherwise `stats` is `null` with a note; say so instead of guessing.
- Never print or log tokens.
