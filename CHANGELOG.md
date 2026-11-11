# Changelog

What changed in each release of Asset Hub. `bun run release` publishes the section of the version in `package.json` as the release notes, so add each change under an **Unreleased** heading as it is made, and rename that heading to the version when releasing.

## [2.1.0] — 2026-11-11

### Changed
- **New interface:** the whole app is redesigned for daily use. A graphite shell with one top bar and a sidebar of types, your lists, pinned branches and folders. A dense grid of assets on a neutral 18% grey stage. An inspector on the right with the preview, the key facts (size, triangles, budget fit) and one row of actions.
- **Export list:** the chest is now a plain export list docked under the grid. It shows the asset and dependency counts, the total size against your playable budget, and the Import into Unity and Export buttons. It opens upward to list the items and the export options.
- **Settings** opens as a side panel over Browse, so your search, selection and export list stay as they were.
- **Sign-in** is a single calm screen: GitHub CLI first, a token as the other option.

### Removed
- The treasure-chest lobby and its sounds, the white flash and welcome message, the AI pet and the AI assistant panel. The app no longer installs or runs the Codex assistant; Codex files already downloaded are not deleted.

## [2.0.0] — 2026-11-11

Asset Hub 2 is a new app, rebuilt on Tauri with a Rust backend. It keeps the features of 1.0.5. It is a **fresh install**: run the new installer, which moves Asset Hub 1.x to the Trash.

### Changed
- **New app shell:** Tauri v2 and Rust replace Electron. GitHub access, caches, the catalog, export, sign-in and the assistant run in Rust; the page keeps the 3D viewers and the interface. The app is much smaller (about 20 MB to download instead of about 60 MB).
- **Needs macOS 14 (Sonoma) or later** on Apple silicon.
- **Starts fresh:** 2.0 keeps its data in `~/Library/Application Support/vn.supercent.assethub` and does not read 1.x's folder. Sign in again (the GitHub CLI option is recommended) and recreate your collections. The 1.x folder is never touched, so 1.x opens as before if you restore it from the Trash.
- **New installer:** it downloads one fixed release, checks it against a checksum built into the installer, and moves Asset Hub 1.x (in `/Applications` or `~/Applications`) to the Trash through Finder.
- **Signed updates:** 2.x updates itself with the Tauri updater from a separate release repo. Every update is signed with a key only the maintainer holds, and the app refuses an update whose signature does not match. A copy that is not in a writable folder or sits behind a symlink opens the release page instead.
- **Personal access tokens** live in the Keychain; macOS asks for permission once after each update.

## [1.0.5] — 2026-11-06

### Fixed
- **Prefabs and effects open sooner the first time on a branch:** a prefab no longer waits for a texture that is not in the branch (about 50 s down to under 20 s on a large repository). With the shared index, what one teammate's app has worked out about a branch they opened is shared too, so the same prefab opens in about 15 s on a Mac that never opened it, instead of about 45 s.

## [1.0.4] — 2026-11-06

### Added
- **The team shares one index:** what one person's app has listed and drawn (branch listings, thumbnails, measurements) is shared through GitHub, so searching All branches and All repos starts full on everyone's Mac. It keeps itself up to date: a teammate's additions are taken in at start and every 30 minutes, and yours are added when the shared one is more than 2 hours old. The first time takes a few minutes in the background. Needs access to the shared index repo; Settings › Shared index shows its state.

### Changed
- **Only what can be shown is listed:** EXR and HDR textures, AIFF and tracker audio, and .asset files that cannot be fonts are no longer listed, which also makes scanning lighter. They are still exported with the assets that use them.

## [1.0.3] — 2026-11-06

### Changed
- **Searching shows pictures sooner:** results from All branches and All repos get their thumbnails in a second or two each, instead of waiting for every branch to be indexed (minutes when results came from many branches). On a branch opened for the first time, model cards show their shape right away and their textures a few seconds later.

### Fixed
- **TIFF textures (.tif) show:** their cards, the image viewer and models using them now draw them.
- **Enter in the search box searches what you typed,** instead of replacing it with the first suggestion. Pick a suggestion with the arrow keys and Enter, a click, or Tab.

## [1.0.2] — 2026-11-06

### Added
- **Skip the lobby:** Settings › Lobby › "Skip the lobby when already signed in" opens straight into the hub. The lobby still shows to sign in.
- **A new lobby each time:** the camera, the gems' colours and how the gems and coins lie change every time the app opens. The chest's red band is now a cloth band tied in a knot on its side.

### Changed
- **New app icon:** the chest on the lobby's floor and pedestal, on a slate blue background.
- **Ticks are remembered:** Filters (merge identical files, only what fits) and the chest's options keep their ticks when the app is opened again.

### Fixed
- **Thumbnails on a new branch:** opening the Prefab tab of a large repository for the first time left the cards empty for minutes; they now fill in within about 20 seconds. Effects that loop for a very long time no longer freeze the app for seconds when they are previewed.
- **Settings no longer reset** when two versions of the app are used on the same Mac (for example right after an update).
- The chest's lid no longer passes through the sword when it opens.

## [1.0.1] — 2026-11-05

### Changed
- **Updates by itself at launch:** when a newer version is out, the app downloads it, checks it and restarts into it on its own — usually about 1 MB and a few seconds. The version button shows the progress, and tries again if something went wrong. (From 1.0.0, press the button once more to get here.)
- **Faster the first time a branch is opened:** the VFX tab finds a large repo's effects about five times faster (32 s instead of nearly 3 minutes for 1,580 effects), with the first cards still after a few seconds; an asset's full dependencies show in about 8 s instead of 30 s.

### Fixed
- Updates downloaded the whole app every time instead of only what changed.

## [1.0.0] — 2026-11-05

The first full release: browse every repo and branch, preview models, prefabs and effects, check their dependencies, take them to Unity or Blender, or ask the assistant.

### Changed
- **Smaller downloads:** the installer downloads about 54 MB instead of 83 MB. From this version on, an update usually downloads only the app's own files (about 1 MB) instead of the whole app; it is checked file by file against the release before it replaces anything, and falls back to the full download otherwise.
- The app is 16 MB smaller on disk: Chromium's software fallback for 3D (SwiftShader) is left out, as every Apple silicon Mac draws 3D on its GPU.
- The lobby shows who made the app, faintly, above the version.

### Fixed
- Files in a **Get .zip** download carry the time they were made instead of a date in 1979.

## [0.2.1] — 2026-11-04

### Added
- **FBX and prefabs to Blender:** an FBX or prefab now has **Get .zip** and **Blender** next to Export. Get .zip saves the model with its textures and `open_in_blender.py`; Blender (shown when it is installed) opens it straight away. A prefab comes as one .glb with every model where Unity places it.
- **Unity materials rebuilt in Blender:** colour tint, metallic and smoothness (from the map or the numbers), HDRP mask maps, occlusion, normal strength, emission, cutout and transparent surfaces, and unlit shaders. Materials are named after their .mat, and textures are packed into the .blend when it is saved.

### Changed
- Exporting to Blender does not fill the app's cache: files already cached are reused, the rest are not kept, and the temporary copy Blender opens deletes itself once loaded.

## [0.2.0] — 2026-11-03

### Added
- **Images in the AI chat:** attach up to 4 pictures to a message with the paperclip, by pasting (⌘V) or by dropping them on the chat; the assistant sees them (a screenshot of a bug, a reference to find assets like it).
- **Right-click menus:** on an asset card — export, import into Unity, chest, favourites, add to a collection, copy path or link, open on GitHub, show its folder; on several picked cards the same for all of them; on results from other branches and repos, on folders and on pinned branches.
- **Axis gizmo in the 3D viewers:** the Front / Side / Top buttons are replaced by Unity's axes in the viewer's corner, turning with the camera; click an axis to look from that side (again for the other side). Reset stays.

### Changed
- **"Alert new assets" is now "Pin branch":** pinned branches sit in the sidebar for one-click return, still with their new assets counted. Each shows the repo above the branch, so long names can be read.

### Fixed
- **"All repos" no longer freezes the app:** with many indexed repos it took over 10 s per refresh and refreshed every 3 s while indexing. Results are now grouped about four times faster, sent at about half the size, filtered by repo and branch where they are kept, and re-read less often while indexing.

## [0.1.10] — 2026-11-03

### Added
- **Effect timeline:** under an effect's play controls, drag to any moment of it or step a frame (1/30 s) at a time with ‹ › or the ←/→ keys; the time and frame number show next to it. Going back shows the same particles again.

### Fixed
- **Texture sheet animation on mesh particles** (e.g. Epic Toon FX's ToonPunchHeavy) plays its frames instead of showing the whole sheet.

## [0.1.9] — 2026-11-03

### Fixed
- **Import to Unity** no longer lists the same project twice: Unity's background import workers are skipped, so each open editor shows once.
- The **Unity editor list** is kept inside the window (above the button, or below when there is no room) instead of being cut off.
- In **Feedback**, a long repo, branch and asset path wraps inside the dialog.
- A disabled primary button keeps its colour on hover instead of its label vanishing.

## [0.1.8] — 2026-11-02

### Changed
- **Sign out** (GitHub) and **Disconnect** (ChatGPT) in Settings are a calm blue instead of red, which read as an error.
- The AI pet's new-answer badge is an amber "…" (a message waiting) in the face's own LED style, instead of a red "!".

## [0.1.7] — 2026-11-02

### Changed
- **Thumbnails come in faster:** cards not drawn yet are drawn four at a time instead of two — about a fifth more per minute, and the first screen fills in about 2.6 s instead of 3.5 s.
- **The installer is in English:** "Install Asset Hub" (was "Cài Asset Hub"), with its dialogs, progress and errors in English, as it cannot follow the app's language setting.

## [0.1.6] — 2026-11-02

### Changed
- **Faster start:** the hub opens the last repo and branch straight away instead of waiting for GitHub's repo and branch lists, and the lobby loads that branch's files while the chest opens, so assets show as soon as the hub does (about 4.5 s from launch instead of 7–12 s). The lobby also shows at once while the previous sign-in is checked, instead of a blank window.
- **Recent lists keep their order while open:** opening an asset from "Viewed lately" no longer moves it to the top under the pointer; it is first the next time the list is opened.

### Fixed
- Effects made with AllIn1Vfx (Dark Magic Orb and others), checked against Unity 2022.3:
  - Mesh particles with **Backface Tint** (front and back faces in their own colours) were white, and **Rim** ignored the mesh's shape: meshes now pass their normals to the shader.
  - Materials that write depth or cull a side (opaque or cutout waves) are drawn as Unity draws them, before the glows.
  - Colour gradients saved inside the material were read as white (glows grey instead of blue).
- Effects drawn in the wrong order: renderers now keep Unity's **Order in Layer** and **Sorting Fudge**, so a dark backdrop stays behind the glow over it.
- **Stretched particles** (rays, streaks, sparks) showed their texture turned 90° and ignored the renderer's **Pivot**: the texture now runs along the stretch and the pivot moves it, as in Unity. Effect cards are drawn again once.

## [0.1.5] — 2026-11-01

### Added
- **The AI pet has a face:** a round 16×16 LED face fills the avatar instead of the chest icon. Its expression follows the chat — resting (blinks, glances around), listening while you type, thinking, eyes scanning while a search runs, a happy face when the answer comes, a "?" when it asks back, X eyes on an error, sad after 👎 or Stop — and it falls asleep after 5 minutes alone, waking when the pointer comes near. Reactions stay 8 seconds once the bubble is closed.
- **URP materials converted for projects without URP:** exporting from a URP project to one without it (the "materials may show pink" case) now switches each material to the built-in shader that does the same job — Lit, Simple Lit and Baked Lit to Standard (opaque, cutout, fade or transparent, with the keywords, blend, depth and queue Unity's own editor sets), Unlit to Unlit/Texture, Transparent or Transparent Cutout, Particles to the legacy particle shader of their blend — properties renamed (`_BaseMap` → `_MainTex`, `_BaseColor` → `_Color`…). The team's own `.assethub/shader-mapping.json` rules still come first. Going to another pipeline still exports, with a warning that the result may break or look unlike the preview; materials still needing URP (Shader Graphs) are counted in it. Checked by importing into a Unity 2022.3 project without URP.
- **AI pet:** a click outside folds the bubble away; an answer that comes in while it is closed puts a red "!" on the avatar until it is opened.
- Effects whose particles are Unity meshes saved as YAML (`.mesh`, e.g. AllIn1Vfx's waves and rings: 89 in hb_creative_xp_hero) are drawn with their mesh.

### Fixed
- Effects made with AllIn1Vfx lost features whenever a material's keyword list was long: Unity wraps it onto a second line, which was not read (157 of 1,269 materials in hb_creative_xp_hero; Blue Fire showed square cards).
- AllIn1Vfx materials with a distortion map failed to compile and drew nothing like Unity (Digital Muzzle Flash, and others).
- Mesh particles (slashes, waves) use the ported shaders and their custom data too, and cards frame them by the mesh's real size instead of filling the card with it.
- The GrabPass variant's screen distortion is left see-through instead of drawn as a flat card.
- Effects drawn with Unity's legacy particle shaders by file ID got their names and blends wrong past Additive and Alpha Blended: the IDs now come from Unity itself, and Additive (Soft) blends softly, so smoke on a black texture (FX_Smoke_Small, Water Vapor, Dust) is see-through instead of a black card.
- Effects of projects in Gamma colour space (hb_creative_xp_hero is one) were computed in Linear: AllIn1Vfx's fades and thresholds cut most of some effects away (Blue Impact's sparks and ring, dim smoke). They are now drawn in the project's own colour space, read from its ProjectSettings. Checked frame by frame against Unity 2022.3 (`scripts/unity/RenderFx.cs`).
- Moving cards' frames landed later and later (each step rounded up to a 30th of a second): they now fall at the right times.
- Effect cards are drawn again once.

## [0.1.4] — 2026-10-31

### Added
- **Send feedback:** the 💬 button next to Settings sends a bug report, an idea or anything else to the team, optionally with the repo, branch and asset you are on. It becomes an issue in the private `supercent-io/asset-hub-feedback` repo; offline or without access it is kept and sent the next time the app opens.
- **Usage statistics:** each session (GitHub name, active time, version, language, repos opened, asset types viewed, how many searches, exports, imports into Unity, AI chats and errors — never search text or file contents) is sent as a comment on the user's `usage: <login>` issue in the same repo. It can be turned off in Settings › Usage statistics. `npm run usage` builds an HTML report from it (`--demo` for made-up data).

- **Effects made with AllIn1Vfx and VFX_Klaus look as in Unity:** both shaders are ported to the preview, line by line. AllIn1Vfx turns its features on per material with keywords (shapes, distortion, colour ramps, glow, fades, masks, HSV…), so each effect gets exactly the ones it uses; screen-space ones (soft particles, screen distortion) are left out. On hb_creative_xp_hero (main) they draw 281 of the 323 effects that use custom shaders. Effect cards are drawn again once.
- Particles carry the **Custom Data** module and the renderer's **vertex streams** to these shaders, in Unity's TEXCOORD layout, so dissolves and fades run over each particle's life.
- Textures saved inside a material (AllIn1Vfx gradients) and Unity's Default-Particle dot are read.
- `scripts/vfx-sheet.mjs` renders many effects onto one page to compare before and after a change; `scripts/ai-eval/` runs the assistant on 30 real requests per language (Vietnamese, English, Korean: 89/90 found on the first ask, 15 s on average).

### Changed
- Languages are listed English, 한국어, Tiếng Việt.
- URP's Particles/Unlit shader is shown by name.

## [0.1.3] — 2026-10-31

### Fixed
- The Path / GitHub / Link buttons under an asset's details showed their icon above the word, spilling out of the button; icon and word now sit on one line.

## [0.1.2] — 2026-10-31

### Added
- **Watch a branch:** the 👁 next to the branch picker follows it. On opening the app, every 30 minutes and on coming back to it, the app asks GitHub (read only) which asset files were added since you last marked it seen: the sidebar's **Watching** list shows the count per branch, new cards carry a **NEW** badge, and a banner offers "Only new assets" and "Mark all seen". Moved or renamed files do not count as new.
- **Import into Unity:** a button next to Export (asset details, cross-branch results, the chest, several picked cards) opens the package straight in an open Unity editor, which shows its import dialog — no file to save or drag. With several projects open, an arrow lists them (project and Unity version) and the one used last is remembered. The package is compared with that editor's project, so what it already has is left out.
- **Pick several cards:** Cmd/Ctrl-click picks cards one by one, Shift-click a run of them; a bar then adds them all to the chest or a collection, or imports them into Unity.
- **Keyboard shortcuts:** ⌘K search, ⌘A pick every card, Esc clear the picks, F favourite, E chest, ⌘I import into Unity (the project used last), ⌘J the AI pet; `?` lists them.
- **Recent:** "Viewed lately" and "Exported lately" in the sidebar (30 each, newest first), opened like a collection from any repo or branch.
- **Quick filters:** ≤ 500 KB, ≤ 5k tris and "With animation" chips above the grid; the ones on measurements keep measured assets only.
- AI pet: `name_words` shows it the words file names in a repo are really made of, so a description in Vietnamese finds the asset on the first try (30/30 on a set of real requests across pizza, crablife and smashdown, 17 s on average, no questions back). A 👎 on an answer saves the conversation on this Mac (`ai-feedback/`) to test the pet against later.
- **AI assistant, step 1: connect ChatGPT** (Settings › AI assistant). The app downloads Codex once (pinned 0.165.0, about 125 MB — codex and its code-mode host, each checked against its SHA-256; an identical copy already on the Mac is reused) and signs in with the user's own ChatGPT account in the browser — no API key. Codex runs with its own settings folder, apart from any Codex the user has. The chat itself comes next.
- **AI pet:** once ChatGPT is connected, the chest sits in the hub's bottom-right corner; click it for a small chat bubble. Answers are kept to one or two short sentences and stream in as they are written; Stop ends an answer, + starts a new chat, Esc closes the bubble. The conversation survives going to Settings and back. Settings › AI assistant has a switch to hide the pet.
- **The pet works in the hub:** it searches (filling the search box and the type tab, in the open branch or every repo), opens an asset with its preview, tells its size, measurements, playable budget and dependencies, and adds assets to the chest or a collection when asked. While it works the bubble says what it is doing ("Searching…"). Exporting stays a button for the user.
- **Reply buttons in the pet:** asked to find something without saying where, the pet offers **This branch · All branches · Other…** (Other lets you type, e.g. "every repo"); other unclear requests get 2–4 buttons too. It asks at most once per request, and not when you already said it ("find smoke in every repo" searches straight away). Search covers the open branch, every branch of the repo, or every repo; naming a repo ("in repo pizza") opens it and searches all its branches without a question — a whole word of a repo's name wins over a part of one (pizza → luna_pizza, not luna_pizzasimulator). The bubble is wider and reply buttons wrap, always in view. The pet turns Vietnamese words into the English names files would have and searches them all at once ("máy cắt lúa" → `harvester | combine | reaper | tractor | farm machine`), so it finds things on the first try.
- Search box: `a | b | c` finds assets matching any of the alternatives (each alternative still needs all of its words), best matches first.
- The assistant cannot change anything outside Asset Hub: Codex starts with shell, terminals, ChatGPT apps/connectors, browser and computer use, plugins, hooks, image tools and web search turned off, in a read-only sandbox that never asks for more. Tested by asking it outright to read `~/.zshrc`, run `ls`, write a file and fetch a web page: every attempt failed.
- **Read-only towards GitHub, enforced:** every request to GitHub passes one check before it is sent — reads (GET, HEAD, GraphQL queries, Git LFS downloads) go through; anything that could change a repo (PUT, PATCH, DELETE, other POSTs, GraphQL mutations, LFS uploads) is refused on the Mac, whatever asked for it, including the assistant later.
- Sign-in screen: the chest answers the START button — hovered or focused, it rattles, its lid lifts a little and the gold inside glows, with four-pointed blings sparkling on it.
- Sign-in screen sounds, made in the app (no sound files): a click on START, and a rattle, knock, creak and chime as the chest opens. A speaker button next to the languages mutes them; the choice is remembered.
- The version is shown at the bottom right of the sign-in screen, with the **Update** button when a newer release exists.
- **Sign out of GitHub** from Settings › Account: signing out can also log the account out of GitHub CLI on this Mac (on by default), so the next START signs in through the browser again and another account can be chosen. The button is red, and the Settings button in the top bar says it holds the account.

### Changed
- Icon buttons carry a short word (Export, Chest, Unity, Collection, Path, GitHub, Link, Watch); Unity's button shows the Unity cube and its arrow lines up with it. Dependencies are drawn as a tree (├ └) so nested textures read as belonging to their material.
- **Calmer hub.** The grid header is two rows: scope, then sort and one **Filters** button (quick filters, playable budget, merge identical files, clips without a model) with a count, and pills for what is on; thumbnail loading is a thin bar. The chest folds to one line (count, room left, size, Unity, Export) and opens for its list and options. In the details panel the viewer's controls appear on hover, actions are one row of icons (Export, Chest, Import into Unity, Favourite, Collection, then copy path / GitHub / link), and Dependency, Particles, prefab uses and copies fold with a summary line ("9 files · 1 warning"); stats left at zero are not listed. Sidebar groups (Watching, Recent, Collections, Folders) fold too; every fold is remembered.
- **One icon family** drawn for the app (same grid and stroke): export, import into Unity, the chest, star, collection, bell for watching (was an eye), clock and package for recent lists, filter, copy, external, link — replacing emoji and text glyphs.
- Names are shown without the company prefix: "SCVN-Dan" or "SC-Dan" reads "Dan" in the top bar, the greeting and Settings.
- Explanatory lines removed across the app (Settings help texts, the AI assistant's intro, the gh sign-out hint, the sign-in screen's email/SSO/install notes, the catalog's folder note); labels and controls speak for themselves. Status and warnings stay.
- Sign-in screen: the line "Sign in with your GitHub account" under START and the "Reads repos only · token kept in this Mac's Keychain" footer are gone.

### Fixed
- Export to a target project now warns when packed materials use a shader that project cannot read — not in the package and not in the project, or a custom importer's file (Toony Colors Pro `.tcp2shader`) where that plugin is missing: those materials would turn magenta. Found by importing into a real Unity project.
- Shaders of custom importers (any `*shader` extension, e.g. `.tcp2shader`) are found by GUID and packed beside their includes (they were left out, or put in Other/ away from the `.cginc` they include). Stored GUID indexes are rebuilt once for this.
- Import into Unity no longer opens an empty package when the project already has everything; it says so instead.
- Unity editors are named by their project folder (a copied project could show another project's productName).
- Going back to the hub from Settings no longer replays the lobby's white flash (it happened when Settings was opened during the greeting); the hub and Settings now switch with a short, soft fade.
- A card reached with the arrow keys had a bigger frame than a clicked one; both now have the same frame.

## [0.1.1] — 2026-10-30

### Added
- **Updates inside the app.** On every launch the app asks the release repo for the latest version; when there is a newer one, an **Update** button appears next to the version in the top bar. It downloads the new build, checks its SHA-256, replaces the app and restarts it. Settings and cache are kept. A copy that cannot replace itself (run from the .dmg, or from a folder the account cannot write to) opens the release page instead.
- The app's version is shown next to its name in the hub.
- **Remove scripts from prefabs** on export (on by default): the project's own script components (`.cs`, `.dll` in the repo) are removed from the prefab and all of its children, including scripts added to nested prefabs, and the script files are left out of the package. Unity and package components (UI, TextMeshPro…) are kept.

### Fixed
- The installer could not download the app (error code 3): the download address was not filled into the installer.
- The installer retries a dropped download up to 3 times, resuming where it stopped, and its error message shows curl's error code and message.
- Small font files (part of the Vietnamese characters) were blocked in the built app.
- At narrow widths the top bar keeps the Settings button visible: the app name gives way to the logo, and the update button has a fixed width.

### Changed
- README, install guide (`docs/INSTALL.md`, was `docs/CAI-DAT.md`) and release notes are in English.

## [0.1.0] — 2026-10-30

First release.

- Browse Unity projects on GitHub by repo and branch; preview FBX models, prefabs, VFX (particle systems), textures, sprites, materials, audio, animation clips and fonts.
- Search across every branch and repo, with a shared index.
- Collections and Favourites.
- Export a `.unitypackage` with all dependencies, checked against a target Unity project (files it already has, shaders by name, shader mapping, missing package scripts, URP version).
- Playable budget presets (5 MB / 2 MB / custom).
- Sign-in as a 3D game lobby; GitHub CLI installed by the app when missing.
- Languages: English, Vietnamese, Korean.
- Mac (Apple silicon) only; installed with a 1 MB web installer that downloads the app from GitHub Releases.
