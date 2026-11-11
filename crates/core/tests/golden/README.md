# Golden fixtures

Frozen outputs of the TypeScript implementation. The Rust port must reproduce them.
All inputs are synthetic: hand-built bytes and text, plus every inline input from the existing `*.test.ts` files. Nothing comes from a team repository or a user's machine.

## Origin

The goldens are frozen from the 1.x TypeScript code and are not regenerated. The harvester that produced them (`scripts/golden/`) was removed with the Electron code; it is in git history before the commit that removed it.

## Layout

- `inputs/repo/proj/...`: the synthetic Unity project (textures, PSD and TIFF variants, materials, a shader, binary and ASCII FBX, prefabs, animation clips, a controller, TMP and TTF/OTF fonts, audio, `.gitattributes`, project settings).
- `expected/<module>/<function>-<hash>.json`: one case per distinct argument set from the unit tests. Hand-written cases are named `<function>.<label>.json`.
- `expected/blobs/<sha256-20>.bin`: binary values larger than 1024 bytes, referenced by hash.
- `expected/cache-formats/<name>/`: cache files written by the real TS writers (`commit-cache`, `blob-cache`, `thumbnails`, `stats`, `catalog-files`, `shared-index`). Each scenario JSON lists every file with its size and SHA-256.
- `expected/skipped-nondeterministic.json`: calls whose outcome differed between repeats (currently none).

Every case JSON has `source: {path, commit, sha256}` (the TS file, the last commit that touched it, and its content hash), and `origin` (`unit-test-input` or `synthetic`). It then has `args`, optional `argsAfter` (arguments after the call, for mutating functions), and `result` or `throws`.

## Conventions

- Tagged JSON: `{"$":"bytes","b64":...}` or `{"$":"bytes","blob":"<hash>"}` (typed arrays carry a `type`), `map`, `set`, `regexp`, `date`, `error`, `bigint`, `num` (`NaN`, `Infinity`, `-0`), `undefined` (array elements and top-level values only; object keys with `undefined` are dropped), and `{"$":"file","path":...}` (a path inside `inputs/repo`). Plain JSON stays plain.
- Dependency context: build it from `inputs/repo`. GUIDs come from the `.meta` files. The tree entry `sha` is the git blob SHA-1 of the file and `size` is its byte length. The root prefix is `proj/`. `pathsByName` is a case-insensitive basename scan. Reading texts and bytes uses the file listing.
- Clock: cases that stamp the current time (`tar`, `unityPackageTar`, `zipFiles`, thumbnail URL versions, the shared-index manifest) ran at `2026-11-09T09:00:00Z`. Time zone is UTC.
- Floats: compare with 1e-6 relative tolerance.
- Thumbnails: the store reads PNG, WebP and JPEG `.img` files plus a legacy `.txt` data URL, which it converts on first read. The WebKit webview returns PNG where Chromium returned WebP, so all three must load.

## Captured functions (case counts)

- `src/core/audio/header.ts`: audioHeader (6)
- `src/core/catalog/catalog.ts`: RepoCatalog (1), catalogFacets (4), groupCatalog (3), listCatalog (18), planScan (1), whereIs (1)
- `src/core/checks/playable.ts`: budgetFor (4), imageSize (7), isStatsSummary (9), keepCardVersions (1), mergeStats (9), newerCard (6), summarizeStats (1), summaryIssues (5)
- `src/core/deps/anim.ts`: controllersPlaying (4), resolveAnimationDependencies (4), rigCandidates (9)
- `src/core/deps/fbx.ts`: closestPath (4), describeMaterials (1), resolveFbxDependencies (5)
- `src/core/deps/material.ts`: resolveMaterialDependencies (6)
- `src/core/deps/prefab.ts`: resolvePrefabDependencies (4)
- `src/core/deps/self.ts`: resolveSelf (16)
- `src/core/deps/tmpFont.ts`: resolveTmpFontDependencies (2)
- `src/core/export/modelBundle.ts`: blenderMaterial (1), blenderScript (2), bundleFileNames (2), modelBundleMaterials (2), prefabBundleMaterials (2)
- `src/core/export/pipelineConvert.ts`: isUrpMaterial (6), setKeywords (1), urpToBuiltin (17)
- `src/core/export/shaderMapping.ts`: mapMaterial (12), materialShader (17), parseShaderMapping (8), shaderMapper (1)
- `src/core/export/stripScripts.ts`: stripScripts (6)
- `src/core/export/tar.ts`: tar (3)
- `src/core/export/target.ts`: isYamlAsset (2), planAgainstTarget (8), remapGuids (5), shaderKind (3), shaderName (4), urpMismatch (8), urpVersion (4)
- `src/core/export/unitypackage.ts`: packagePathname (33), planPackage (9), unityPackageTar (7)
- `src/core/font/sfnt.ts`: fontCoverage (5), missingChars (3), parseFont (6), parseFontMeta (5)
- `src/core/format.ts`: formatBytes (4), shortPath (2)
- `src/core/github/client.ts`: assertReadOnly (28), nextLink (7)
- `src/core/github/lfs.ts`: isLfsPath (21), parseLfsAttributes (5), parseLfsPointer (9)
- `src/core/image/psd.ts`: decodePsd (6), isPsd (2)
- `src/core/image/tiff.ts`: decodeTiff (11), isTiff (2)
- `src/core/registry.ts`: HandlerRegistry (2), baseName (73), createDefaultRegistry (1), dirName (40), rules (1)
- `src/core/search/search.ts`: AssetSearch (45), editDistance (3), nameWords (1), normalize (1), parseQuery (2)
- `src/core/tree.ts`: buildFolderTree (2), isInFolder (3)
- `src/core/unity.ts`: findUnityProjectRoots (2), isInExcludedFolder (19), projectEntries (2)
- `src/core/unity/animClip.ts`: parseAnimClip (8)
- `src/core/unity/importedIds.ts`: importedObjectId (3), meshForFileId (3), xxHash64 (2)
- `src/core/unity/mesh.ts`: parseUnityMesh (6)
- `src/core/unity/particleSim.ts`: parseParticleSim (17), readMinMax (2), readMinMaxColor (1)
- `src/core/unity/particles.ts`: effectShas (2), parseParticleSystems (11), particleTotals (1), prefabEffectFact (11)
- `src/core/unity/prefab.ts`: folderAffinity (6), modelMaterials (8)
- `src/core/unity/prefabScene.ts`: fbxMeshNames (6), materialList (6), modelImportScale (5), parsePrefabScene (8), ref (14), refGuid (5), splitDocs (16)
- `src/core/unity/tmpFont.ts`: isTmpFontAsset (4), parseTmpFont (4), tmpAtlas (3)
- `src/core/unity/yaml.ts`: builtAudioBytes (3), effectiveAudioImport (5), extractFbxTextureNames (8), guessTextureRole (11), importedTextureSize (3), materialKeywords (11), metaGuid (57), parseAudioMeta (7), parseEmbeddedTexture (2), parseFbxMeta (11), parseMaterial (18), parseMaterialLook (14), parseMaterialProps (27), parseSpriteMeta (13), parseTextureMeta (14), shaderBlend (7), shaderDefaults (2), shaderName (2), shaderTextureProps (4)
- `src/main/caches.ts`: FileBlobCache (1), FileCommitCache (1), StatsStore (1), ThumbnailStore (1)
- `src/main/feedbackRepo.ts`: assertFeedbackWrite (7)
- `src/main/ghPrompt.ts`: parseGhLoginPrompt (4)
- `src/main/gitTree.ts`: gitEnv (2), parseCatFileBatch (1)
- `src/main/sharedIndex.ts`: exportIndex (1)
- `src/main/targetProject.ts`: contentHash (1)
- `src/main/unityEditors.ts`: parseUnityProcesses (1)
- `src/main/zip.ts`: zipFiles (1)
- `src/shared/collections.ts`: addCollection (2), deleteCollection (2), inCollection (1), renameCollection (2), sanitizeCollections (3), toggleItem (3), withFavorites (1)
- `src/shared/deeplink.ts`: assetLink (1), parseAssetLink (8)
- `src/shared/displayName.ts`: displayName (6)
- `src/shared/settings.ts`: sanitizeSettings (18), unknownSettings (4)
- `src/shared/usage.ts`: feedbackIssue (1), isWorthSending (3), newSession (1), noteEvent (3), noteIpc (6), parseUsageComment (2), sanitizeFeedback (3), usageComment (1)
- `src/shared/version.ts`: isNewer (6)

Also captured as scenarios: the full registry rules truth table (`alsoShows` and `unlisted` per handler), `RepoCatalog` operations, `AssetSearch` over Vietnamese, Korean and English names, the `shaderMapper` over the synthetic project, and the cache formats listed above.

## Not captured

- `src/main/appTree.ts`: not ported (the Tauri updater replaces the per-file manifest).
- `sampleCurve`, `evalMinMax`, `evalGradient`, `evalMinMaxColor`, `streamTexcoords` (particle evaluators): they stay in TypeScript, so the Rust port has no counterpart.
- File-system and process-bound helpers in `src/main` (temp-dir and spawn functions, `gitEnv` with the live environment): outputs depend on the machine. `gitEnv` has a hand case with a replaced environment. Cache classes are covered through the cache-format scenarios instead.
- `GitHubClient`, `GitHubAssetSource`, `src/core/source.ts` request paths: they need HTTP, and no trace was recorded (no network use allowed). Pure helpers (`assertReadOnly`, `nextLink`, LFS parsing) are captured.
- `CatalogService` runtime behaviour, `SettingsStore`, `CodexClient`, and the 1.x-only modules (`index`, `auth`, `ai`, `blender`, `exporter`, `github`, `catalog` IPC glue, `updater`, `usage` sender, installers): no pure behaviour to freeze. Catalog files written by `CatalogService` are covered in `catalog-files`.
