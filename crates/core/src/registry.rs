//! The asset type registry (`src/core/registry.ts`): the only place that knows which asset types
//! exist. The handlers themselves are in [`crate::handlers`].

use crate::handlers::{AssetHandler, HandlerDescriptor};
use crate::types::{Asset, EntryType, TreeEntry};

/// The file name of a repo path (`path.slice(path.lastIndexOf('/') + 1)`).
pub fn base_name(path: &str) -> &str {
    path.rfind('/').map_or(path, |i| &path[i + 1..])
}

/// The folder of a repo path; empty when the path has no folder.
pub fn dir_name(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("Asset handler \"{0}\" is already registered")]
    Duplicate(String),
}

/// Handlers in registration order.
#[derive(Debug, Default)]
pub struct HandlerRegistry {
    handlers: Vec<AssetHandler>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a handler; an id registered twice is refused.
    pub fn register(&mut self, handler: AssetHandler) -> Result<&mut Self, RegistryError> {
        if self.handlers.iter().any(|h| h.id == handler.id) {
            return Err(RegistryError::Duplicate(handler.id.to_string()));
        }
        self.handlers.push(handler);
        Ok(self)
    }

    pub fn get(&self, id: &str) -> Option<&AssetHandler> {
        self.handlers.iter().find(|h| h.id == id)
    }

    pub fn all(&self) -> &[AssetHandler] {
        &self.handlers
    }

    /// What each handler declares, for the renderer.
    pub fn descriptors(&self) -> Vec<HandlerDescriptor> {
        self.handlers.iter().map(AssetHandler::descriptor).collect()
    }

    /// First handler (in registration order) that claims the entry. `.meta` files are never assets.
    pub fn classify(&self, entry: &TreeEntry) -> Option<&AssetHandler> {
        if entry.kind != EntryType::Blob || entry.path.to_lowercase().ends_with(".meta") {
            return None;
        }
        self.handlers.iter().find(|h| h.matches(entry))
    }

    pub fn collect_assets(&self, entries: &[TreeEntry]) -> Vec<Asset> {
        entries
            .iter()
            .filter_map(|entry| {
                let handler = self.classify(entry)?;
                Some(Asset {
                    id: entry.path.clone(),
                    kind: handler.id.to_string(),
                    path: entry.path.clone(),
                    name: base_name(&entry.path).to_string(),
                    sha: entry.sha.clone(),
                    size: if entry.lfs == Some(true) { entry.lfs_size.or(entry.size) } else { entry.size },
                    lfs: entry.lfs.unwrap_or(false),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handlers::create_default_registry;

    #[test]
    fn registering_an_id_twice_is_refused() {
        let mut registry = create_default_registry();
        let again = registry.get("fbx").expect("fbx").clone();
        let err = registry.register(again).expect_err("duplicate");
        assert_eq!(err.to_string(), "Asset handler \"fbx\" is already registered");
        assert_eq!(registry.all().len(), 10);
    }

    #[test]
    fn a_folder_is_never_an_asset() {
        let registry = create_default_registry();
        let folder = TreeEntry { path: "Assets/Hero.fbx".into(), kind: EntryType::Tree, sha: "x".into(), size: None, lfs: None, lfs_size: None, lfs_oid: None };
        assert!(registry.classify(&folder).is_none());
    }
}
