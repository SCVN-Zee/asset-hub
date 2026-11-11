//! GUID to path answered on demand (`LazyGuids` of `source.ts`): GUIDs whose `.meta` was read before
//! (on any branch) are known at once; for the others, `.meta` files of the expected kind are read
//! nearest-folder-first, a chunk at a time, until they are found.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use futures::lock::Mutex as AsyncMutex;

use crate::deps::context::GuidKind;
use crate::js::cached;
use crate::registry::base_name;
use crate::types::TreeEntry;

use super::{lock, SharedRec};

/// `.meta` files of each kind, for lookups that only need one kind.
pub(super) fn kind_pattern(kind: GuidKind) -> &'static str {
    match kind {
        GuidKind::Material => r"\.mat\.meta$",
        GuidKind::Texture => r"\.(png|jpe?g|tga|psd|psb|tiff?|exr|bmp|gif|hdr|cubemap|rendertexture)\.meta$",
        // Any extension ending in "shader" too: custom shader importers (Toony Colors Pro's .tcp2shader...).
        GuidKind::Shader => r"\.(shader|shadergraph|shadersubgraph|[a-z0-9]+shader|hlsl|cginc)\.meta$",
        GuidKind::Any => r"\.meta$",
        GuidKind::Model => r"\.(fbx|obj|blend|dae|3ds|max|ma|mb|asset|mesh)\.meta$",
        GuidKind::Prefab => r"\.(prefab|fbx|obj|blend)\.meta$",
        GuidKind::Script => r"\.(cs|dll)\.meta$",
        GuidKind::Other => {
            r"\.(wav|mp3|ogg|aiff?|anim|controller|overridecontroller|asset|ttf|otf|fontsettings|mask|physicmaterial|physicsmaterial2d|playable|signal|spriteatlas|spriteatlasv2|mixer|cubemap|flare|guiskin|renderTexture|lighting|terrainlayer)\.meta$"
        }
    }
}

pub(super) const EXTRA_KINDS: [GuidKind; 4] = [GuidKind::Model, GuidKind::Prefab, GuidKind::Script, GuidKind::Other];

/// `.meta` files read by one quick lookup before it gives up and leaves the rest to the full index.
pub(super) const LOOKUP_BUDGET: usize = 640;
/// One GraphQL request reads 40 files in about a second; four run side by side.
const LOOKUP_CHUNK: usize = 160;
pub(super) const LOOKUP_PARALLEL: usize = 4;

/// How a lookup reads a chunk of `.meta` files.
pub(super) enum Reader {
    /// Small reads through GraphQL, four side by side (a git fetch costs several seconds each time).
    Quick,
    /// Through the bulk reader; a search through many files downloads all of `all` once, first.
    Extra { all: Vec<TreeEntry>, warmed: AsyncMutex<bool> },
}

pub(super) struct LazyGuids {
    pub(super) known: Mutex<HashMap<String, String>>,
    unread: Mutex<Vec<TreeEntry>>,
    pub(super) by_sha: SharedRec,
    /// One lookup per kind reads at a time, so two never fetch the same files; kinds read in parallel.
    locks: Mutex<HashMap<GuidKind, Arc<AsyncMutex<()>>>>,
    pub(super) reader: Reader,
}

/// A `.meta` path without its extension.
pub(super) fn without_meta(path: &str) -> &str {
    path.get(..path.len().saturating_sub(".meta".len())).unwrap_or(path)
}

impl LazyGuids {
    pub(super) fn new(metas: &[TreeEntry], by_sha: SharedRec, reader: Reader) -> Self {
        let mut unread = Vec::new();
        let mut known = HashMap::new();
        {
            let shas = lock(&by_sha);
            for m in metas {
                match shas.get(&m.sha) {
                    None => unread.push(m.clone()),
                    Some(guid) => {
                        if let Some(guid) = guid.as_str().filter(|g| !g.is_empty()) {
                            known.insert(guid.to_string(), without_meta(&m.path).to_string());
                        }
                    }
                }
            }
        }
        Self { known: Mutex::new(known), unread: Mutex::new(unread), by_sha, locks: Mutex::default(), reader }
    }

    /// Every `.meta` read: a GUID not known now is not in the commit.
    pub(super) fn complete(&self) -> bool {
        lock(&self.unread).is_empty()
    }

    pub(super) fn knows(&self, guid: &str) -> bool {
        lock(&self.known).contains_key(guid)
    }

    pub(super) fn path_of(&self, guid: &str) -> Option<String> {
        lock(&self.known).get(guid).cloned()
    }

    /// The GUIDs of `guids` not known yet, each once.
    pub(super) fn wanted(&self, guids: &[String]) -> Vec<String> {
        let known = lock(&self.known);
        let mut seen = HashSet::new();
        guids.iter().filter(|g| !known.contains_key(*g) && seen.insert(g.as_str())).cloned().collect()
    }

    pub(super) fn lock_for(&self, kind: GuidKind) -> Arc<AsyncMutex<()>> {
        lock(&self.locks).entry(kind).or_default().clone()
    }

    /// The unread `.meta` files of `kind`: those named like `hints` first (an FBX names its materials),
    /// then the folders nearest the model.
    pub(super) fn candidates(&self, kind: GuidKind, near_path: &str, hints: &[String]) -> Vec<TreeEntry> {
        let near: Vec<&str> = near_path.split('/').collect();
        let affinity = |p: &str| {
            let parts: Vec<&str> = p.split('/').collect();
            near.iter().zip(&parts).take_while(|(a, b)| a == b).count()
        };
        let named: HashSet<String> = hints.iter().map(|h| h.to_lowercase()).collect();
        let stem = |p: &str| {
            let name = cached(r"\.meta$", "i").replace_first(base_name(p), "");
            cached(r"\.[^.]+$", "").replace_first(&name, "").to_lowercase()
        };
        let pattern = cached(kind_pattern(kind), "i");
        let mut scored: Vec<(usize, TreeEntry)> = lock(&self.unread)
            .iter()
            .filter(|m| pattern.test(&m.path))
            .map(|m| (if named.contains(&stem(&m.path)) { 1000 } else { 0 } + affinity(&m.path), m.clone()))
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, m)| m).collect()
    }

    /// Remembers what a chunk of `.meta` files said and drops the ones read from the unread list.
    pub(super) fn learn(&self, chunk: &[TreeEntry], texts: &HashMap<String, String>) {
        let mut shas = lock(&self.by_sha);
        let mut known = lock(&self.known);
        for m in chunk {
            let Some(text) = texts.get(&m.sha) else { continue };
            let guid = crate::unity::yaml::meta_guid(text).unwrap_or_default();
            shas.insert(m.sha.clone(), serde_json::Value::String(guid.clone()));
            if !guid.is_empty() {
                known.insert(guid, without_meta(&m.path).to_string());
            }
        }
        let read: HashSet<&str> = chunk.iter().filter(|m| texts.contains_key(&m.sha)).map(|m| m.sha.as_str()).collect();
        lock(&self.unread).retain(|m| !read.contains(m.sha.as_str()));
    }
}

/// The size of one read of a lookup, given what is left of its budget.
pub(super) fn chunk_len(budget: usize) -> usize {
    LOOKUP_CHUNK.min(budget)
}
