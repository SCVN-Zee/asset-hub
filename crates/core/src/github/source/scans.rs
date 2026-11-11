//! The parts of the source that learn from file contents: on-demand GUID lookups, the branch's GUID
//! index, dependency resolution, and the scans for effects, sprites, TextMeshPro fonts and the prefabs
//! that put materials on a model.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use futures::future::try_join_all;
use futures::lock::Mutex as AsyncMutex;
use futures::FutureExt;
use serde_json::{Map, Value};

use crate::deps::anim::resolve_animation_dependencies;
use crate::deps::context::{add_warning, unique, DepContext, GuidKind};
use crate::deps::fbx::{describe_materials, resolve_fbx_dependencies, MaterialQuery};
use crate::deps::material::resolve_material_dependencies;
use crate::deps::prefab::resolve_prefab_dependencies;
use crate::deps::self_dep::resolve_self;
use crate::deps::tmp_font::resolve_tmp_font_dependencies;
use crate::deps::types::{DependencyReport, PrefabMaterialsResult, PrefabScope, PrefabUsage};
use crate::errors::SourceError;
use crate::handlers::{tmp_candidate, Resolver};
use crate::js::{decode_utf8, OrderedMap};
use crate::re;
use crate::types::{FactKind, TreeEntry};
use crate::unity::ExcludeMatcher;
use crate::unity::particles::{effect_shas, prefab_effect_fact};
use crate::unity::prefab::{folder_affinity, model_materials};
use crate::unity::tmp_font::is_tmp_font_asset;
use crate::unity::yaml::meta_guid;

use super::ctx::{Base, ExtraLayer, SourceCtx};
use super::lazy_guids::{chunk_len, kind_pattern, without_meta, LazyGuids, Reader, EXTRA_KINDS, LOOKUP_PARALLEL};
use super::{
    is_blob, lock, not_found, size_of, unknown, FactProgress, GuidBuild, Inner, IndexProgress, OnFacts, OnIndex, ResolveMode, SharedRec, Snapshot, EFFECT_KIND, INDEX_FORMAT, INDEX_FORMAT_KEY, PREFAB_KIND, QUICK_LOOKUP_MS, SHA_KEY, SHA_KIND, SPRITE_KIND, TMP_KIND,
};
use crate::github::client::HttpFetch;

/// `.meta` files worth indexing: anything a model, material or shader can reference. Models are left
/// out: their `.meta` files are large and nothing a material uses points at them.
fn indexed(path: &str) -> bool {
    re!(r"\.(mat|png|jpe?g|tga|psd|psb|tiff?|exr|bmp|gif|hdr|cubemap|rendertexture|shader|shadergraph|shadersubgraph|[a-z0-9]+shader|hlsl|cginc)\.meta$", "i").test(path)
}

/// `.meta` files of the extra kinds (outside the index).
fn extra_meta(path: &str) -> bool {
    static PATTERN: std::sync::LazyLock<crate::js::Re> = std::sync::LazyLock::new(|| {
        let sources: Vec<&str> = EXTRA_KINDS.iter().map(|k| kind_pattern(*k)).collect();
        crate::js::Re::with_flags(&sources.join("|"), "i")
    });
    PATTERN.test(path)
}

/// Textures whose `.meta` can say "sprite".
fn sprite_texture(path: &str) -> bool {
    re!(r"\.(png|jpe?g|tga|psd|gif|bmp|tiff?)$", "i").test(path)
}

/// Bytes of prefabs read by a "nearby" search before it stops.
const NEAR_BUDGET: i64 = 16 * 1024 * 1024;
/// Bytes per bulk read, so progress can move during a big scan.
const PREFAB_CHUNK: u64 = 24 * 1024 * 1024;
/// Files read at once by a fact scan. Listings made with git carry no sizes, so the byte limit alone
/// would put every file of the repo in one chunk, all held in memory together.
const FACT_CHUNK_FILES: usize = 1500;
/// Smaller steps for nearby searches, so they can stop as soon as the model is found.
const NEAR_CHUNK: u64 = 6 * 1024 * 1024;
/// Files per request of the GUID index, and requests side by side.
const INDEX_BATCH: usize = 120;
const INDEX_CONCURRENCY: usize = 6;

fn fact_is(facts: &Map<String, Value>, sha: &str, wanted: &str) -> bool {
    facts.get(sha).and_then(Value::as_str) == Some(wanted)
}

/// The fact of a blob when it is a non-empty text (`facts[sha]` truthy in the TypeScript).
fn fact_text<'a>(facts: &'a Map<String, Value>, sha: &str) -> Option<&'a str> {
    facts.get(sha).and_then(Value::as_str).filter(|f| !f.is_empty())
}

/// The material GUIDs a prefab (by blob SHA) puts on a model, when it uses it.
fn materials_of(known: &Map<String, Value>, sha: &str, model_guid: &str) -> Option<Vec<String>> {
    let mats = known.get(sha)?.get(model_guid)?.as_array()?;
    (!mats.is_empty()).then(|| mats.iter().filter_map(|m| m.as_str().map(str::to_string)).collect())
}

async fn run_resolver<C: DepContext>(resolver: Resolver, path: &str, ctx: &C) -> Result<DependencyReport, SourceError> {
    match resolver {
        Resolver::Fbx => resolve_fbx_dependencies(path, ctx).await,
        Resolver::Itself => resolve_self(path, ctx).await,
        Resolver::Material => resolve_material_dependencies(path, ctx).await,
        Resolver::Prefab => resolve_prefab_dependencies(path, ctx).await,
        Resolver::Animation => resolve_animation_dependencies(path, ctx).await,
        Resolver::TmpFont => resolve_tmp_font_dependencies(path, ctx).await,
    }
}

fn report(on_progress: Option<&OnFacts>, done: u64, total: u64, found: Vec<String>) {
    if let Some(f) = on_progress {
        f(&FactProgress { done, total, found });
    }
}

impl<F: HttpFetch + 'static> Inner<F> {
    fn context(self: &Arc<Self>, repo: &str, snap: &Arc<Snapshot>, root: &str, base: Base, extra: Option<ExtraLayer>) -> SourceCtx<F> {
        SourceCtx { inner: self.clone(), repo: repo.to_string(), snap: snap.clone(), root: root.to_string(), base, extra }
    }

    // ---- on-demand GUID lookups ---------------------------------------------------------------

    /// Reads `.meta` files of `kind` until every GUID of `guids` is known or the budget runs out: files
    /// named like `hints` first, then the folders nearest the model. At least one chunk is read, so
    /// what is nearest is always looked at; more only until `until` (ms since the epoch).
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn lazy_find(self: &Arc<Self>, lazy: &LazyGuids, repo: &str, commit: &str, guids: &[String], kind: GuidKind, near_path: &str, hints: &[String], limit: usize, until: i64) -> Result<(), SourceError> {
        if lazy.wanted(guids).is_empty() || lazy.complete() {
            return Ok(());
        }
        // One lookup per kind reads at a time, so two never fetch the same files.
        let gate = lazy.lock_for(kind);
        let _turn = gate.lock().await;
        let candidates = lazy.candidates(kind, near_path, hints);
        let (mut budget, mut i) = (limit, 0);
        while i < candidates.len() && budget > 0 && !lazy.wanted(guids).is_empty() && (i == 0 || self.client.now_ms() < until) {
            let end = (i + chunk_len(budget)).min(candidates.len());
            let chunk = &candidates[i..end];
            i = end;
            budget -= chunk.len();
            let texts = self.read_lazy_chunk(lazy, repo, commit, chunk).await?;
            lazy.learn(chunk, &texts);
        }
        Ok(())
    }

    /// The text of each `.meta` of a chunk, by blob SHA.
    async fn read_lazy_chunk(&self, lazy: &LazyGuids, repo: &str, commit: &str, chunk: &[TreeEntry]) -> Result<std::collections::HashMap<String, String>, SourceError> {
        match &lazy.reader {
            Reader::Quick => {
                // Small reads: GraphQL answers in about a second, a git fetch costs several seconds each time.
                let size = chunk.len().div_ceil(LOOKUP_PARALLEL);
                let slices: Vec<&[TreeEntry]> = (0..LOOKUP_PARALLEL).map(|i| &chunk[(i * size).min(chunk.len())..((i + 1) * size).min(chunk.len())]).filter(|p| !p.is_empty()).collect();
                let parts = try_join_all(slices.into_iter().map(|part| async move {
                    let paths: Vec<String> = part.iter().map(|e| e.path.clone()).collect();
                    self.client.get_blob_texts(repo, commit, &paths).await
                }))
                .await?;
                let mut by_path = std::collections::HashMap::new();
                for part in &parts {
                    for (p, t) in part.iter() {
                        by_path.insert(p.to_string(), t.clone());
                    }
                }
                Ok(chunk.iter().filter_map(|e| by_path.get(&e.path).map(|t| (e.sha.clone(), t.clone()))).collect())
            }
            Reader::Extra { all, warmed } => {
                // A search through many of them (a GUID not found nearby) goes through every model's
                // .meta, 160 at a time: each its own git fetch, and several searches at once read the
                // same ones. Downloaded once, together, the first time; every read after that is local.
                if chunk.len() >= 100 {
                    let mut done = warmed.lock().await;
                    if !*done {
                        let unknown_blobs: Vec<&TreeEntry> = {
                            let shas = lock(&lazy.by_sha);
                            all.iter().filter(|e| !shas.contains_key(&e.sha)).collect()
                        };
                        self.prefetch(repo, &unknown_blobs).await;
                        *done = true;
                    }
                }
                self.read_many_texts(repo, commit, chunk).await
            }
        }
    }

    /// A context that looks GUIDs up on demand; `None` when the full index is cached and cheaper.
    async fn quick_base(self: &Arc<Self>, repo: &str, snap: &Arc<Snapshot>, near_path: &str) -> Result<Option<Base>, SourceError> {
        if let Some(cache) = &self.options.cache {
            if let Some(Value::Object(stored)) = cache.get("guids", repo, &snap.commit).await.map_err(unknown)? {
                if stored.get(INDEX_FORMAT_KEY).and_then(Value::as_str) == Some(INDEX_FORMAT) {
                    return Ok(None);
                }
            }
        }
        let existing = lock(&snap.lazy).clone();
        let lazy = match existing {
            Some(lazy) => lazy,
            None => {
                let by_sha = self.shared_cache(SHA_KIND, repo).await?;
                let metas: Vec<TreeEntry> = snap.entries.iter().filter(|e| is_blob(e) && indexed(&e.path)).cloned().collect();
                let made = Arc::new(LazyGuids::new(&metas, by_sha, Reader::Quick));
                lock(&snap.lazy).get_or_insert(made).clone()
            }
        };
        Ok(Some(Base::Quick { lazy, until: self.client.now_ms() + QUICK_LOOKUP_MS, near_path: near_path.to_string(), lookups: true }))
    }

    /// Adds lookups for the kinds the GUID index leaves out (models, prefabs, scripts...): nearest
    /// folders first within a budget, or every file of that kind for the `Full` answer. `Instant` only
    /// uses what is known already.
    async fn with_extra(self: &Arc<Self>, repo: &str, snap: &Arc<Snapshot>, base: Base, near_path: &str, mode: ResolveMode) -> Result<SourceCtx<F>, SourceError> {
        let existing = lock(&snap.extra).clone();
        let extra = match existing {
            Some(extra) => extra,
            None => {
                let by_sha = self.shared_cache(SHA_KIND, repo).await?;
                let all: Vec<TreeEntry> = snap.entries.iter().filter(|e| is_blob(e) && extra_meta(&e.path) && !indexed(&e.path)).cloned().collect();
                let made = Arc::new(LazyGuids::new(&all, by_sha, Reader::Extra { all: all.clone(), warmed: AsyncMutex::new(false) }));
                lock(&snap.extra).get_or_insert(made).clone()
            }
        };
        let until = if mode == ResolveMode::Full { i64::MAX } else { self.client.now_ms() + QUICK_LOOKUP_MS };
        let layer = ExtraLayer { extra, mode, until, near_path: near_path.to_string(), asked: Default::default() };
        // The first argument is not needed again: the context owns what it reads from.
        Ok(self.context(repo, snap, &snap_root(snap, &base), base, Some(layer)))
    }

    /// Stores GUIDs learned by quick lookups, so other branches and launches reuse them.
    async fn save_learned(&self, repo: &str) -> Result<(), SourceError> {
        let rec = self.shared_cache(SHA_KIND, repo).await?;
        let snapshot = lock(&rec).clone();
        self.cache_set_lenient(SHA_KIND, repo, SHA_KEY, Value::Object(snapshot)).await;
        Ok(())
    }

    // ---- dependencies -------------------------------------------------------------------------

    pub(super) async fn resolve(self: &Arc<Self>, repo: &str, branch: &str, path: &str, root: &str, on_progress: Option<OnIndex>, mode: ResolveMode) -> Result<DependencyReport, SourceError> {
        let snap = self.snapshot(repo, branch).await?;
        let found = snap.by_path.get(path).and_then(|e| {
            let handler = self.options.registry.as_ref()?.classify(e)?;
            Some((handler.resolver?, handler.uses_guids))
        });
        let Some((resolver, uses_guids)) = found else { return Err(not_found(format!("No dependency resolver for {path}"))) };
        if !uses_guids {
            return run_resolver(resolver, path, &self.context(repo, &snap, root, Base::Empty, None)).await;
        }
        if mode != ResolveMode::Full && lock(&snap.guids).is_none() {
            if let Some(mut base) = self.quick_base(repo, &snap, path).await? {
                // `Instant` answers with GUIDs already known only; the rest shows as still being checked.
                if let (ResolveMode::Instant, Base::Quick { lookups, .. }) = (mode, &mut base) {
                    *lookups = false;
                }
                let ctx = self.with_extra_rooted(repo, &snap, root, base, path, mode).await?;
                let answer = run_resolver(resolver, path, &ctx).await?;
                if mode != ResolveMode::Instant {
                    self.save_learned(repo).await?;
                }
                return Ok(answer);
            }
        }
        let guids = self.guid_index(repo, &snap, on_progress).await?;
        let ctx = self.with_extra_rooted(repo, &snap, root, Base::Index(guids), path, mode).await?;
        let answer = run_resolver(resolver, path, &ctx).await?;
        if mode != ResolveMode::Instant && lock(&snap.extra).is_some() {
            self.save_learned(repo).await?;
        }
        Ok(answer)
    }

    async fn with_extra_rooted(self: &Arc<Self>, repo: &str, snap: &Arc<Snapshot>, root: &str, base: Base, near_path: &str, mode: ResolveMode) -> Result<SourceCtx<F>, SourceError> {
        let mut ctx = self.with_extra(repo, snap, base, near_path, mode).await?;
        ctx.root = root.to_string();
        Ok(ctx)
    }

    // ---- the GUID index -----------------------------------------------------------------------

    /// GUID to repo path for every indexed `.meta` in the commit; concurrent callers share one construction.
    pub(super) async fn guid_index(self: &Arc<Self>, repo: &str, snap: &Arc<Snapshot>, on_progress: Option<OnIndex>) -> Result<Arc<OrderedMap<String>>, SourceError> {
        if let Some(index) = lock(&snap.guids).clone() {
            return Ok(index);
        }
        let key = format!("{repo}@{}", snap.commit);
        let (fut, listeners) = {
            let mut builds = lock(&self.guid_builds);
            let entry = builds.entry(key.clone()).or_insert_with(|| {
                let listeners: Arc<std::sync::Mutex<Vec<OnIndex>>> = Arc::default();
                let (inner, repo, snap, shared_listeners, key) = (self.clone(), repo.to_string(), snap.clone(), listeners.clone(), key.clone());
                let fut = async move {
                    let tell = move |p: IndexProgress| {
                        let all: Vec<OnIndex> = lock(&shared_listeners).clone();
                        all.iter().for_each(|l| l(p));
                    };
                    let result = inner.make_guid_index(&repo, &snap, &tell).await;
                    lock(&inner.guid_builds).remove(&key);
                    result
                }
                .boxed()
                .shared();
                GuidBuild { fut, listeners }
            });
            (entry.fut.clone(), entry.listeners.clone())
        };
        if let Some(listener) = on_progress {
            lock(&listeners).push(listener);
        }
        fut.await
    }

    async fn make_guid_index(&self, repo: &str, snap: &Arc<Snapshot>, progress: &(dyn Fn(IndexProgress) + Send + Sync)) -> Result<Arc<OrderedMap<String>>, SourceError> {
        if let Some(cache) = &self.options.cache {
            if let Some(Value::Object(stored)) = cache.get("guids", repo, &snap.commit).await.map_err(unknown)? {
                if stored.get(INDEX_FORMAT_KEY).and_then(Value::as_str) == Some(INDEX_FORMAT) {
                    let mut index = OrderedMap::new();
                    for (guid, path) in stored.iter().filter(|(k, _)| k.as_str() != INDEX_FORMAT_KEY) {
                        if let Some(path) = path.as_str() {
                            index.insert(guid.clone(), path.to_string());
                        }
                    }
                    let index = Arc::new(index);
                    *lock(&snap.guids) = Some(index.clone());
                    return Ok(index);
                }
            }
        }
        let metas: Vec<TreeEntry> = snap.entries.iter().filter(|e| is_blob(e) && indexed(&e.path)).cloned().collect();
        let total = metas.len() as u64;
        // .meta files rarely change between commits and branches: only read the ones not seen before.
        let by_sha = self.shared_cache(SHA_KIND, repo).await?;
        let todo: Vec<TreeEntry> = {
            let known = lock(&by_sha);
            metas.iter().filter(|m| !known.contains_key(&m.sha)).cloned().collect()
        };
        let done = AtomicUsize::new(metas.len() - todo.len());
        progress(IndexProgress { done: done.load(Ordering::SeqCst) as u64, total });
        let learn = |sha: &str, text: &str| {
            lock(&by_sha).insert(sha.to_string(), Value::String(meta_guid(text).unwrap_or_default()));
        };

        let mut remaining = todo.clone();
        if !remaining.is_empty() {
            let shas = unique(remaining.iter().map(|m| m.sha.clone()));
            let bulk = self.options.hooks.read_blobs_bulk(repo, &shas);
            if let Some(bulk) = bulk {
                // No usable git or the fetch failed: read through the API instead.
                if let Ok(blobs) = bulk.await {
                    for (sha, bytes) in &blobs {
                        learn(sha, &decode_utf8(bytes));
                    }
                    // Anything not returned is retried through the API below, never cached as empty.
                    remaining.retain(|m| !lock(&by_sha).contains_key(&m.sha));
                    done.store(metas.len() - remaining.len(), Ordering::SeqCst);
                    progress(IndexProgress { done: done.load(Ordering::SeqCst) as u64, total });
                }
            }
        }

        if !remaining.is_empty() {
            let batches: Vec<&[TreeEntry]> = remaining.chunks(INDEX_BATCH).collect();
            let next = AtomicUsize::new(0);
            let worker = || async {
                while let Some(batch) = batches.get(next.fetch_add(1, Ordering::SeqCst)) {
                    let paths: Vec<String> = batch.iter().map(|m| m.path.clone()).collect();
                    let texts = self.client.get_blob_texts(repo, &snap.commit, &paths).await?;
                    // Only what actually came back is remembered; failed reads are tried again next time.
                    for m in batch.iter() {
                        if let Some(text) = texts.get(&m.path) {
                            learn(&m.sha, text);
                        }
                    }
                    let now = done.fetch_add(batch.len(), Ordering::SeqCst) + batch.len();
                    progress(IndexProgress { done: now as u64, total });
                }
                Ok::<(), SourceError>(())
            };
            try_join_all((0..INDEX_CONCURRENCY).map(|_| worker())).await?;
        }

        let (guids, all_read, snapshot) = {
            let known = lock(&by_sha);
            let mut guids = OrderedMap::new();
            for m in &metas {
                if let Some(guid) = known.get(&m.sha).and_then(Value::as_str).filter(|g| !g.is_empty()) {
                    guids.insert(guid.to_string(), without_meta(&m.path).to_string());
                }
            }
            (guids, metas.iter().all(|m| known.contains_key(&m.sha)), (!todo.is_empty()).then(|| known.clone()))
        };
        if let Some(snapshot) = snapshot {
            self.cache_set(SHA_KIND, repo, SHA_KEY, Value::Object(snapshot)).await?;
        }
        // A commit's index is stored only when every .meta was read, so a failed read is retried next time.
        if all_read {
            let mut stored: Map<String, Value> = guids.iter().map(|(g, p)| (g.to_string(), Value::String(p.clone()))).collect();
            stored.insert(INDEX_FORMAT_KEY.to_string(), Value::String(INDEX_FORMAT.to_string()));
            self.cache_set("guids", repo, &snap.commit, Value::Object(stored)).await?;
        }
        let guids = Arc::new(guids);
        *lock(&snap.guids) = Some(guids.clone());
        Ok(guids)
    }

    // ---- facts: effects, sprites, TextMeshPro fonts ---------------------------------------------

    pub(super) async fn known_assets(self: &Arc<Self>, repo: &str, branch: &str, kind: FactKind, on_progress: Option<OnFacts>) -> Result<Vec<String>, SourceError> {
        let snap = self.snapshot(repo, branch).await?;
        let blobs: Vec<TreeEntry> = snap.entries.iter().filter(|e| is_blob(e)).cloned().collect();
        let progress = on_progress.as_ref();
        match kind {
            FactKind::Effects => self.effect_prefabs(repo, &snap, &blobs, progress).await,
            FactKind::TmpFonts => {
                let candidates: Vec<TreeEntry> = blobs.into_iter().filter(|e| tmp_candidate().test(&e.path)).collect();
                let found = |list: &[TreeEntry], f: &Map<String, Value>| list.iter().filter(|e| fact_is(f, &e.sha, "1")).map(|e| e.sha.clone()).collect();
                let facts = self.scan_facts(repo, &snap.commit, TMP_KIND, &candidates, |t| if is_tmp_font_asset(t) { "1".into() } else { String::new() }, &found, progress).await?;
                Ok(candidates.iter().filter(|e| fact_is(&facts, &e.sha, "1")).map(|e| e.sha.clone()).collect())
            }
            FactKind::Sprites => {
                // The texture's .meta says so; the texture's own SHA is what the grid knows.
                let mut metas: Vec<TreeEntry> = Vec::new();
                let mut textures_of: OrderedMap<Vec<String>> = OrderedMap::new();
                for tex in blobs.iter().filter(|e| sprite_texture(&e.path)) {
                    let Some(meta) = snap.by_path.get(&format!("{}.meta", tex.path)) else { continue };
                    metas.push(meta.clone());
                    match textures_of.get_mut(&meta.sha) {
                        Some(list) => list.push(tex.sha.clone()),
                        None => textures_of.insert(meta.sha.clone(), vec![tex.sha.clone()]),
                    }
                }
                let sprites = |list: &[TreeEntry], f: &Map<String, Value>| -> Vec<String> {
                    list.iter().filter(|m| fact_is(f, &m.sha, "1")).flat_map(|m| textures_of.get(&m.sha).cloned().unwrap_or_default()).collect()
                };
                let is_sprite = |t: &str| if re!(r"^\s*textureType: 8\b", "m").test(t) { "1".to_string() } else { String::new() };
                let facts = self.scan_facts(repo, &snap.commit, SPRITE_KIND, &metas, is_sprite, &sprites, progress).await?;
                Ok(unique(sprites(&metas, &facts)))
            }
        }
    }

    /// Reads files not seen before (by SHA, per repo) in chunks and keeps one fact per file. `found`
    /// turns a list of files into the SHAs to report.
    #[allow(clippy::too_many_arguments)]
    async fn scan_facts(
        &self,
        repo: &str,
        commit: &str,
        kind: &str,
        entries: &[TreeEntry],
        fact: impl Fn(&str) -> String,
        found: &(dyn Fn(&[TreeEntry], &Map<String, Value>) -> Vec<String> + Sync),
        on_progress: Option<&OnFacts>,
    ) -> Result<Map<String, Value>, SourceError> {
        let rec = self.shared_cache(kind, repo).await?;
        let todo: Vec<TreeEntry> = {
            let facts = lock(&rec);
            let mut by_sha: OrderedMap<TreeEntry> = OrderedMap::new();
            for e in entries.iter().filter(|e| !facts.contains_key(&e.sha)) {
                by_sha.insert(e.sha.clone(), e.clone());
            }
            by_sha.values().cloned().collect()
        };
        let total: u64 = todo.iter().map(size_of).sum();
        let mut done = 0u64;
        // What earlier scans found shows at once; the rest as each chunk is read.
        report(on_progress, done, total, found(entries, &lock(&rec)));
        let (mut i, mut chunks) = (0, 0);
        while i < todo.len() {
            // The first chunk is read on its own, so results show within seconds; the rest is downloaded in one go.
            if chunks == 1 {
                let rest: Vec<&TreeEntry> = todo[i..].iter().collect();
                self.prefetch(repo, &rest).await;
            }
            let mut chunk: Vec<TreeEntry> = Vec::new();
            let mut bytes = 0u64;
            while i < todo.len() && chunk.len() < FACT_CHUNK_FILES && (bytes == 0 || bytes < PREFAB_CHUNK) {
                bytes += size_of(&todo[i]);
                chunk.push(todo[i].clone());
                i += 1;
            }
            let texts = self.read_many_texts(repo, commit, &chunk).await?;
            // Not read: left for the next call, never remembered as "no".
            let (found_now, snapshot) = {
                let mut facts = lock(&rec);
                for e in &chunk {
                    if let Some(text) = texts.get(&e.sha) {
                        facts.insert(e.sha.clone(), Value::String(fact(text)));
                    }
                }
                (found(&chunk, &facts), Value::Object(facts.clone()))
            };
            done += chunk.iter().map(size_of).sum::<u64>();
            report(on_progress, done, total, found_now);
            self.cache_set(kind, repo, SHA_KEY, snapshot).await?;
            chunks += 1;
        }
        let facts = lock(&rec).clone();
        Ok(facts)
    }

    /// Effect prefabs: their own particle systems, or variants of one (see `prefab_effect_fact`).
    async fn effect_prefabs(&self, repo: &str, snap: &Arc<Snapshot>, blobs: &[TreeEntry], on_progress: Option<&OnFacts>) -> Result<Vec<String>, SourceError> {
        let prefabs: Vec<TreeEntry> = blobs.iter().filter(|e| re!(r"\.prefab$", "i").test(&e.path)).cloned().collect();
        // Variants are known to be effects only at the end, once their base is.
        let own = |list: &[TreeEntry], f: &Map<String, Value>| unique(list.iter().filter(|e| fact_is(f, &e.sha, "1")).map(|e| e.sha.clone()));
        let facts = self.scan_facts(repo, &snap.commit, EFFECT_KIND, &prefabs, prefab_effect_fact, &own, on_progress).await?;

        // Variants name their base prefab by GUID: the .meta files say which GUID each prefab has.
        let nests = prefabs.iter().any(|e| fact_text(&facts, &e.sha).is_some_and(|f| f != "1"));
        let mut guid_to_sha: OrderedMap<String> = OrderedMap::new();
        if nests {
            let metas: Vec<TreeEntry> = prefabs.iter().filter_map(|e| snap.by_path.get(&format!("{}.meta", e.path)).cloned()).collect();
            let by_sha = self.shared_cache(SHA_KIND, repo).await?;
            let unread: Vec<TreeEntry> = {
                let known = lock(&by_sha);
                metas.iter().filter(|m| !known.contains_key(&m.sha)).cloned().collect()
            };
            if !unread.is_empty() {
                let texts = self.read_many_texts(repo, &snap.commit, &unread).await?;
                let snapshot = {
                    let mut known = lock(&by_sha);
                    for m in &unread {
                        if let Some(text) = texts.get(&m.sha) {
                            known.insert(m.sha.clone(), Value::String(meta_guid(text).unwrap_or_default()));
                        }
                    }
                    known.clone()
                };
                self.cache_set(SHA_KIND, repo, SHA_KEY, Value::Object(snapshot)).await?;
            }
            let known = lock(&by_sha);
            for e in &prefabs {
                let meta_sha = snap.by_path.get(&format!("{}.meta", e.path)).map(|m| m.sha.as_str()).unwrap_or("");
                if let Some(guid) = known.get(meta_sha).and_then(Value::as_str).filter(|g| !g.is_empty()) {
                    guid_to_sha.insert(guid.to_string(), e.sha.clone());
                }
            }
        }
        let mut known_facts: OrderedMap<String> = OrderedMap::new();
        for e in &prefabs {
            if let Some(f) = facts.get(&e.sha) {
                known_facts.insert(e.sha.clone(), f.as_str().unwrap_or_default().to_string());
            }
        }
        let known_facts: Vec<(String, String)> = known_facts.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let guid_to_sha: Vec<(String, String)> = guid_to_sha.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        Ok(effect_shas(&known_facts, &guid_to_sha))
    }

    // ---- materials that prefabs put on a model ------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn find_prefab_materials(
        self: &Arc<Self>,
        repo: &str,
        branch: &str,
        path: &str,
        root: &str,
        scope: PrefabScope,
        excluded_folders: &[String],
        on_progress: Option<OnIndex>,
    ) -> Result<PrefabMaterialsResult, SourceError> {
        let snap = self.snapshot(repo, branch).await?;
        let meta_path = format!("{path}.meta");
        let meta_text = self.read_texts(repo, &snap, std::slice::from_ref(&meta_path)).await?.remove(&meta_path);
        let Some(model_guid) = meta_text.as_deref().and_then(meta_guid) else { return Err(not_found(format!("{path} has no .meta GUID"))) };

        let assets = format!("{root}Assets/");
        let excluded = ExcludeMatcher::new(excluded_folders);
        let prefabs: Vec<TreeEntry> = snap
            .entries
            .iter()
            .filter(|e| is_blob(e) && e.path.ends_with(".prefab") && e.path.starts_with(&assets) && !excluded.is_excluded(&e.path[root.len()..]))
            .cloned()
            .collect();
        let known_rec = self.shared_cache(PREFAB_KIND, repo).await?;
        let mut ranked: Vec<(TreeEntry, u32)> = prefabs.iter().map(|e| (e.clone(), folder_affinity(path, &e.path, root))).collect();
        ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| size_of(&a.0).cmp(&size_of(&b.0))));
        // Just which remembered prefabs use it: no material lookups, nothing downloaded.
        if scope == PrefabScope::Known {
            let known = lock(&known_rec);
            let unchecked: Vec<&TreeEntry> = prefabs.iter().filter(|e| !known.contains_key(&e.sha)).collect();
            return Ok(PrefabMaterialsResult {
                scope,
                prefabs: ranked.iter().filter(|(e, _)| materials_of(&known, &e.sha, &model_guid).is_some()).map(|(e, _)| PrefabUsage { path: e.path.clone(), materials: Vec::new() }).collect(),
                scanned: prefabs.len() - unchecked.len(),
                remaining: unchecked.len(),
                remaining_bytes: unchecked.iter().map(|e| size_of(e)).sum(),
                files: Vec::new(),
                warnings: Vec::new(),
            });
        }
        let mut candidates: Vec<(TreeEntry, u32)> = if scope == PrefabScope::Cached {
            Vec::new()
        } else {
            let known = lock(&known_rec);
            ranked.iter().filter(|(e, _)| !known.contains_key(&e.sha)).cloned().collect()
        };
        if scope == PrefabScope::Near {
            let mut budget = NEAR_BUDGET;
            candidates.retain(|(e, score)| {
                if *score == 0 {
                    return false;
                }
                budget -= size_of(e) as i64;
                budget >= 0
            });
        }

        if !candidates.is_empty() {
            self.scan_prefabs(repo, &snap, &candidates, &known_rec, &ranked, &model_guid, scope, on_progress.as_ref()).await?;
        }
        // Materials need GUID to path only when some prefab uses the model: no index work otherwise.
        let used = {
            let known = lock(&known_rec);
            ranked.iter().any(|(e, _)| materials_of(&known, &e.sha, &model_guid).is_some())
        };
        let quick = if used { self.quick_base(repo, &snap, path).await? } else { None };
        let had_quick = quick.is_some();
        let mut answer = self.prefab_answer(repo, &snap, root, quick, &prefabs, &ranked, &known_rec, &model_guid, scope).await?;
        if had_quick {
            self.save_learned(repo).await?;
        }
        // Something was not found nearby: settle it with the full index.
        let unsure = |a: &PrefabMaterialsResult| a.prefabs.iter().any(|p| p.materials.iter().any(|m| m.unsure == Some(true) || m.textures.iter().any(|t| t.unsure == Some(true))));
        if unsure(&answer) {
            answer = self.prefab_answer(repo, &snap, root, None, &prefabs, &ranked, &known_rec, &model_guid, scope).await?;
        }
        Ok(answer)
    }

    /// Reads candidate prefabs (skipping ones another scan has read meanwhile) into the shared cache.
    #[allow(clippy::too_many_arguments)]
    async fn scan_prefabs(
        &self,
        repo: &str,
        snap: &Arc<Snapshot>,
        all: &[(TreeEntry, u32)],
        known_rec: &SharedRec,
        ranked: &[(TreeEntry, u32)],
        model_guid: &str,
        scope: PrefabScope,
        on_progress: Option<&OnIndex>,
    ) -> Result<(), SourceError> {
        let candidates: Vec<&(TreeEntry, u32)> = {
            let known = lock(known_rec);
            all.iter().filter(|(e, _)| !known.contains_key(&e.sha)).collect()
        };
        let total: u64 = candidates.iter().map(|(e, _)| size_of(e)).sum();
        let mut done = 0u64;
        let tell = |done: u64| {
            if let Some(f) = on_progress {
                f(IndexProgress { done, total });
            }
        };
        tell(done);
        // A nearby search stops early: it reads as it goes. The others read every candidate.
        if scope != PrefabScope::Near {
            let all_entries: Vec<&TreeEntry> = candidates.iter().map(|(e, _)| e).collect();
            self.prefetch(repo, &all_entries).await;
        }
        let chunk_bytes = if scope == PrefabScope::Near { NEAR_CHUNK } else { PREFAB_CHUNK };
        // Best affinity among prefabs (cached or read) that already use the model.
        let mut best_found: i64 = {
            let known = lock(known_rec);
            ranked.iter().filter(|(e, _)| materials_of(&known, &e.sha, model_guid).is_some()).map(|(_, s)| *s as i64).max().unwrap_or(-1).max(-1)
        };
        let mut read = 0;
        let mut i = 0;
        while i < candidates.len() {
            // Nearby search: stop once what is left is less related than a prefab already found.
            if scope == PrefabScope::Near && best_found >= 0 && (candidates[i].1 as i64) < best_found {
                break;
            }
            let mut chunk: Vec<&(TreeEntry, u32)> = Vec::new();
            let mut bytes = 0u64;
            while i < candidates.len() && (bytes == 0 || bytes < chunk_bytes) {
                let candidate = candidates[i];
                i += 1;
                // Another search running at the same time may have read it already.
                if lock(known_rec).contains_key(&candidate.0.sha) {
                    continue;
                }
                bytes += size_of(&candidate.0);
                chunk.push(candidate);
            }
            if chunk.is_empty() {
                continue;
            }
            let entries: Vec<TreeEntry> = chunk.iter().map(|(e, _)| e.clone()).collect();
            let texts = self.read_many_texts(repo, &snap.commit, &entries).await?;
            {
                let mut known = lock(known_rec);
                for (e, score) in &chunk {
                    // Not read: left for a later scan.
                    let Some(text) = texts.get(&e.sha) else { continue };
                    known.insert(e.sha.clone(), serde_json::to_value(model_materials(text)).map_err(|err| unknown(err.to_string()))?);
                    if materials_of(&known, &e.sha, model_guid).is_some() {
                        best_found = best_found.max(*score as i64);
                    }
                }
            }
            read += chunk.len();
            done += chunk.iter().map(|(e, _)| size_of(e)).sum::<u64>();
            tell(done);
        }
        if read > 0 {
            let snapshot = lock(known_rec).clone();
            self.cache_set(PREFAB_KIND, repo, SHA_KEY, Value::Object(snapshot)).await?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn prefab_answer(
        self: &Arc<Self>,
        repo: &str,
        snap: &Arc<Snapshot>,
        root: &str,
        // On-demand lookups; without it the full GUID index is used (and made if needed).
        quick: Option<Base>,
        prefabs: &[TreeEntry],
        ranked: &[(TreeEntry, u32)],
        known_rec: &SharedRec,
        model_guid: &str,
        scope: PrefabScope,
    ) -> Result<PrefabMaterialsResult, SourceError> {
        let known = lock(known_rec).clone();
        let used = ranked.iter().any(|(e, _)| materials_of(&known, &e.sha, model_guid).is_some());
        let base = match quick {
            Some(base) => Some(base),
            None if used => Some(Base::Index(self.guid_index(repo, snap, None).await?)),
            None => None,
        };
        let ctx = base.map(|base| self.context(repo, snap, root, base, None));
        let mut usages: Vec<PrefabUsage> = Vec::new();
        let mut files: Vec<String> = Vec::new();
        let mut warnings = Vec::new();
        for (e, _) in ranked {
            let Some(mats) = materials_of(&known, &e.sha, model_guid) else { continue };
            let Some(ctx) = &ctx else { return Err(unknown("a prefab uses the model but no GUID lookup is available")) };
            let queries: Vec<MaterialQuery> = mats.into_iter().map(|guid| MaterialQuery { name: None, guid }).collect();
            let described = describe_materials(&queries, ctx).await?;
            files = unique(files.into_iter().chain(described.files));
            for w in described.warnings {
                add_warning(&mut warnings, w);
            }
            usages.push(PrefabUsage { path: e.path.clone(), materials: described.materials });
        }
        let unchecked: Vec<&TreeEntry> = prefabs.iter().filter(|e| !known.contains_key(&e.sha)).collect();
        Ok(PrefabMaterialsResult {
            scope,
            prefabs: usages,
            scanned: prefabs.len() - unchecked.len(),
            remaining: unchecked.len(),
            remaining_bytes: unchecked.iter().map(|e| size_of(e)).sum(),
            files,
            warnings,
        })
    }
}

/// The root a context was made for is set by the caller; this only gives `with_extra` a starting value.
fn snap_root(_snap: &Snapshot, _base: &Base) -> String {
    String::new()
}
