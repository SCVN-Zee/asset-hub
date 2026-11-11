//! What a resolver may ask of the branch it resolves in (`DepContext` of `src/core/deps/fbx.ts`),
//! and the small helpers the resolvers share.

use std::collections::HashMap;
use std::future::Future;

use crate::errors::SourceError;
use crate::types::TreeEntry;

/// What a GUID is expected to point at, so a lookup only reads `.meta` files of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuidKind {
    Material,
    Texture,
    Shader,
    Any,
    /// Kinds outside the branch's GUID index (models, prefabs, scripts, other assets): always looked up on demand.
    Model,
    Prefab,
    Script,
    Other,
}

/// The files of a branch by path, in the order the listing gave them (like a JS `Map`).
#[derive(Debug, Clone, Default)]
pub struct ByPath {
    entries: Vec<TreeEntry>,
    index: HashMap<String, usize>,
}

impl ByPath {
    /// A later entry with the same path replaces the earlier one in place, as `Map.set` does.
    pub fn new(entries: impl IntoIterator<Item = TreeEntry>) -> Self {
        let mut out = Self::default();
        for e in entries {
            match out.index.get(&e.path) {
                Some(&i) => out.entries[i] = e,
                None => {
                    out.index.insert(e.path.clone(), out.entries.len());
                    out.entries.push(e);
                }
            }
        }
        out
    }

    pub fn get(&self, path: &str) -> Option<&TreeEntry> {
        self.index.get(path).map(|&i| &self.entries[i])
    }

    pub fn has(&self, path: &str) -> bool {
        self.index.contains_key(path)
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.path.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// What a resolver may ask of the branch it is resolving in. All paths are repo paths.
///
/// The futures are `Send` so that a resolver can run on a multi-threaded runtime.
pub trait DepContext: Sync {
    /// Unity project root prefix, e.g. `unity_project/game/` or an empty string.
    fn root(&self) -> &str;

    fn by_path(&self) -> &ByPath;

    fn path_for_guid(&self, guid: &str) -> Option<String>;

    /// Looks these GUIDs up before `path_for_guid` is asked for them, reading only the `.meta` files
    /// of that kind nearest the model. Does nothing when every GUID is known already.
    fn prepare(&self, _guids: &[String], _kind: GuidKind, _name_hints: &[String]) -> impl Future<Output = Result<(), SourceError>> + Send {
        async { Ok(()) }
    }

    /// `Some(true)` when a GUID was not found but the project was not fully searched; `None` when the
    /// context cannot tell.
    fn unsure(&self, _guid: &str) -> Option<bool> {
        None
    }

    /// Repo paths of files with this name (case-insensitive).
    fn paths_by_name(&self, name: &str) -> Vec<String>;

    /// The text of each path that exists; a path that does not exist is left out.
    fn read_texts(&self, paths: &[String]) -> impl Future<Output = Result<HashMap<String, String>, SourceError>> + Send;

    fn read_bytes(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, SourceError>> + Send;
}

/// A context that answers some GUIDs itself and leaves the rest to `inner`
/// (`{ ...ctx, pathForGuid: (g) => own.get(g) ?? ctx.pathForGuid(g) }`).
pub struct GuidOverlay<'a, C: DepContext> {
    pub inner: &'a C,
    pub guids: HashMap<String, String>,
}

impl<C: DepContext> DepContext for GuidOverlay<'_, C> {
    fn root(&self) -> &str {
        self.inner.root()
    }

    fn by_path(&self) -> &ByPath {
        self.inner.by_path()
    }

    fn path_for_guid(&self, guid: &str) -> Option<String> {
        self.guids.get(guid).cloned().or_else(|| self.inner.path_for_guid(guid))
    }

    fn prepare(&self, guids: &[String], kind: GuidKind, name_hints: &[String]) -> impl Future<Output = Result<(), SourceError>> + Send {
        self.inner.prepare(guids, kind, name_hints)
    }

    fn unsure(&self, guid: &str) -> Option<bool> {
        self.inner.unsure(guid)
    }

    fn paths_by_name(&self, name: &str) -> Vec<String> {
        self.inner.paths_by_name(name)
    }

    fn read_texts(&self, paths: &[String]) -> impl Future<Output = Result<HashMap<String, String>, SourceError>> + Send {
        self.inner.read_texts(paths)
    }

    fn read_bytes(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, SourceError>> + Send {
        self.inner.read_bytes(path)
    }
}

/// A set that remembers the order in which values were first added (a JS `Set`).
#[derive(Debug, Clone)]
pub struct OrderedSet<T: Clone + Eq + std::hash::Hash> {
    items: Vec<T>,
    seen: std::collections::HashSet<T>,
}

impl<T: Clone + Eq + std::hash::Hash> Default for OrderedSet<T> {
    fn default() -> Self {
        Self { items: Vec::new(), seen: Default::default() }
    }
}

impl<T: Clone + Eq + std::hash::Hash> OrderedSet<T> {
    pub fn add(&mut self, value: T) {
        if self.seen.insert(value.clone()) {
            self.items.push(value);
        }
    }

    pub fn extend(&mut self, values: impl IntoIterator<Item = T>) {
        for v in values {
            self.add(v);
        }
    }

    pub fn has(&self, value: &T) -> bool {
        self.seen.contains(value)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.items
    }

    pub fn to_vec(&self) -> Vec<T> {
        self.items.clone()
    }
}

/// The values once each, in the order they first appear (`[...new Set(values)]`).
pub fn unique<T: Clone + Eq + std::hash::Hash>(values: impl IntoIterator<Item = T>) -> Vec<T> {
    let mut set = OrderedSet::default();
    set.extend(values);
    set.into_vec()
}

/// Size of an entry: the real object size for an LFS file, else the size in git.
pub fn entry_size(entry: Option<&TreeEntry>) -> Option<u64> {
    let e = entry?;
    if e.lfs == Some(true) {
        e.lfs_size.or(e.size)
    } else {
        e.size
    }
}

/// Adds a warning once, keeping the order in which warnings were raised.
pub fn add_warning(warnings: &mut Vec<crate::deps::types::DependencyWarning>, w: crate::deps::types::DependencyWarning) {
    if !warnings.contains(&w) {
        warnings.push(w);
    }
}
