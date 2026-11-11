# Product

<!-- impeccable:product-schema 1 -->

## Platform

web

Asset Hub is a macOS desktop app (Tauri 2). The interface is a React page in a WebKit web view, so it is designed as a web surface for a desktop window, not as a native AppKit UI. macOS 14 or later, Apple silicon.

## Users

The whole studio (Supercent) uses it: developers, artists, playable-ad makers, PMs and QA. Each person signs in with their own GitHub account and sees the Unity repos that account can read. Confirmed by the user, 2026-11-11.

Typical jobs:
- Find an asset (model, prefab, VFX, texture, audio, font, animation, material) that already exists somewhere in the studio's Unity repos.
- Look at it closely before taking it: 3D preview, effect playback, sizes, triangle counts, dependencies.
- Take it into their own Unity project as a `.unitypackage` with every dependency, or import it straight into an open Unity editor.
- Keep track of assets and branches: favourites, collections, recently viewed and exported, pinned branches with new-asset counts.

## Product Purpose

Asset Hub makes the studio's existing Unity work reusable. Without it, a person must clone a repo, open it in Unity, find the asset and export it by hand. With it, they browse any branch from GitHub, preview the asset in seconds, and export it with its dependencies. Success means a person finds the right asset and has it in their project in minutes, without cloning anything.

## Positioning

It reads Unity projects directly from GitHub branches, previews them without Unity, resolves the full dependency graph, and checks the package against the user's own Unity project before export. A team index, shared through GitHub, makes search across all branches and repos fast for everyone.

## Operating Context

- Used beside Unity, often on a laptop screen, many times a day in short sessions.
- Repos are large (hundreds to thousands of assets per branch, many branches per repo).
- Sign-in uses the GitHub CLI (recommended) or a fine-grained token kept in the Keychain.
- Playable ads (Luna) have hard size limits, so the app has playable-budget presets (standard 5 MB, tight 2 MB, custom).
- Links use the `assethub://` scheme, so people share assets in chat.
- A command-line tool (`asset-hub-cli`) and an MCP server give the same search and export to coding agents.

## Capabilities and Constraints

- Source repos and the target Unity project are read-only. Export produces a package for Unity's own import dialog.
- Every capability in the 2.0 UI stays, except the items listed below. The full inventory is in `plans/` for the redesign.
- Removed in the redesign by user decision (2026-11-11): the treasure-chest lobby and its sounds, the white flash and welcome toast, the AI pet and the AI assistant UI, and the chest metaphor for the export list. The export list stays as a plain feature.
- The workflow can be restructured: screens, grouping and control placement can change.
- UI languages: English, Korean, Vietnamese. Locale keys must stay aligned (`ui/src/i18n/locales.test.ts`).
- The page makes no network request and loads no remote fonts or scripts (CSP).
- Terminology still open: the export list name ("chest" in 2.0, "pack" in code) and "Pin branch" versus "watch".

## Brand Commitments

- Name: Asset Hub. Bundle id `vn.supercent.assethub`, link scheme `assethub://`. These stay.
- No other binding visual commitments. The 2.0 look (gunmetal, amber accent, Chakra Petch display face, chest logo) is replaced.

## Evidence on Hand

- Real repos and assets through the user's GitHub account. Parity screenshots of real assets in `plans/261109-1425-tauri-rust-bun-migration/reports/parity-261110-0040-assets/`.
- No testimonials, usage numbers or benchmarks are on hand. Do not invent them.

## Product Principles

1. The asset is the content. The interface frames previews and facts, and gets out of the way.
2. One clear place for each action. Export, import into Unity and collections each have one obvious home.
3. Facts a person decides on are visible without digging: size, triangles, budget fit, dependencies.
4. Fast for repeat use: keyboard first, state kept between screens, no ceremony at start.
5. Calm by default. No decoration that does not carry information.

## Accessibility & Inclusion

Keyboard operation for every action, visible focus, and respect for reduced motion. Text must fit Korean and Vietnamese strings as well as English.
