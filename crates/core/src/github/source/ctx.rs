//! The `DepContext` a resolver gets from the GitHub source: the branch's files, GUIDs from the full
//! index or looked up on demand, and a second layer for the kinds the index leaves out.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::{Arc, Mutex};

use crate::deps::context::{ByPath, DepContext, GuidKind};
use crate::errors::{SourceError, SourceErrorCode};
use crate::js::OrderedMap;

use super::lazy_guids::{LazyGuids, EXTRA_KINDS, LOOKUP_BUDGET};
use super::{lock, Inner, ResolveMode, Snapshot};
use crate::github::client::HttpFetch;

/// Where a context finds GUIDs.
pub(super) enum Base {
    /// No GUID is ever looked up (a handler that does not use them).
    Empty,
    /// The branch's full GUID index.
    Index(Arc<OrderedMap<String>>),
    /// Looked up on demand, nearest folders first, until `until` (ms since the epoch). `lookups` is
    /// false for an `instant` answer, which uses only what is known.
    Quick { lazy: Arc<LazyGuids>, until: i64, near_path: String, lookups: bool },
}

/// Lookups for the kinds the GUID index leaves out (models, prefabs, scripts...).
pub(super) struct ExtraLayer {
    pub(super) extra: Arc<LazyGuids>,
    pub(super) mode: ResolveMode,
    pub(super) until: i64,
    pub(super) near_path: String,
    /// GUIDs a resolver asked an extra lookup for.
    pub(super) asked: Mutex<HashSet<String>>,
}

pub(super) struct SourceCtx<F: HttpFetch + 'static> {
    pub(super) inner: Arc<Inner<F>>,
    pub(super) repo: String,
    pub(super) snap: Arc<Snapshot>,
    pub(super) root: String,
    pub(super) base: Base,
    pub(super) extra: Option<ExtraLayer>,
}

impl<F: HttpFetch + 'static> SourceCtx<F> {
    async fn base_prepare(&self, guids: &[String], kind: GuidKind, hints: &[String]) -> Result<(), SourceError> {
        match &self.base {
            Base::Quick { lazy, until, near_path, lookups: true } => self.inner.lazy_find(lazy, &self.repo, &self.snap.commit, guids, kind, near_path, hints, LOOKUP_BUDGET, *until).await,
            _ => Ok(()),
        }
    }

    fn base_unsure(&self, guid: &str) -> Option<bool> {
        match &self.base {
            Base::Quick { lazy, .. } => Some(!lazy.complete() && !lazy.knows(guid)),
            _ => None,
        }
    }
}

#[allow(clippy::manual_async_fn)] // the trait spells the Send bound out
impl<F: HttpFetch + 'static> DepContext for SourceCtx<F> {
    fn root(&self) -> &str {
        &self.root
    }

    fn by_path(&self) -> &ByPath {
        &self.snap.by_path
    }

    fn path_for_guid(&self, guid: &str) -> Option<String> {
        let found = match &self.base {
            Base::Empty => None,
            Base::Index(index) => index.get(guid).cloned(),
            Base::Quick { lazy, .. } => lazy.path_of(guid),
        };
        found.or_else(|| self.extra.as_ref().and_then(|x| x.extra.path_of(guid)))
    }

    fn prepare(&self, guids: &[String], kind: GuidKind, name_hints: &[String]) -> impl Future<Output = Result<(), SourceError>> + Send {
        async move {
            let Some(layer) = self.extra.as_ref().filter(|_| EXTRA_KINDS.contains(&kind)) else { return self.base_prepare(guids, kind, name_hints).await };
            lock(&layer.asked).extend(guids.iter().cloned());
            if layer.mode == ResolveMode::Instant {
                return Ok(());
            }
            let limit = if layer.mode == ResolveMode::Full { usize::MAX } else { LOOKUP_BUDGET };
            self.inner.lazy_find(&layer.extra, &self.repo, &self.snap.commit, guids, kind, &layer.near_path, name_hints, limit, layer.until).await
        }
    }

    fn unsure(&self, guid: &str) -> Option<bool> {
        let Some(layer) = &self.extra else { return self.base_unsure(guid) };
        // After a full search an extra GUID that is still unknown is not in the branch (a package
        // script, a deleted file).
        Some(if lock(&layer.asked).contains(guid) { layer.mode != ResolveMode::Full && !layer.extra.complete() && !layer.extra.knows(guid) } else { self.base_unsure(guid).unwrap_or(false) })
    }

    fn paths_by_name(&self, name: &str) -> Vec<String> {
        self.snap.by_name.get(&name.to_lowercase()).cloned().unwrap_or_default()
    }

    fn read_texts(&self, paths: &[String]) -> impl Future<Output = Result<HashMap<String, String>, SourceError>> + Send {
        self.inner.read_texts(&self.repo, &self.snap, paths)
    }

    fn read_bytes(&self, path: &str) -> impl Future<Output = Result<Vec<u8>, SourceError>> + Send {
        async move {
            let Some(entry) = self.snap.by_path.get(path) else { return Err(SourceError::new(SourceErrorCode::NotFound, format!("{path} is not in the branch"))) };
            Ok(self.inner.read_entry(&self.repo, entry, true).await?.as_ref().clone())
        }
    }
}
