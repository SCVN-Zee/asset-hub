//! Shared domain types (`src/core/types.ts`, `src/core/source.ts`).

use serde::{Deserialize, Serialize};

/// Whether a git tree entry is a file or a folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    Blob,
    Tree,
}

/// One file or folder from a branch, shaped like GitHub's Git Trees API entries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: EntryType,
    /// Blob SHA from git.
    pub sha: String,
    /// Size in bytes as stored in git (for LFS files: the pointer size unless `lfs_size` is set).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// True when the blob is a Git LFS pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lfs: Option<bool>,
    /// Real object size from the LFS pointer, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lfs_size: Option<u64>,
    /// sha256 of the real object from the LFS pointer: a content hash usable for dedupe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lfs_oid: Option<String>,
}

/// An entry that a handler recognised as an asset of its type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Asset {
    /// Stable id within one repo and branch: the file path.
    pub id: String,
    /// Handler id, such as `fbx`.
    #[serde(rename = "type")]
    pub kind: String,
    pub path: String,
    pub name: String,
    pub sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub lfs: bool,
}

/// What a viewer draws for an asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewKind {
    Three,
    Material,
    Prefab,
    Animation,
    Audio,
    Font,
    Tmpfont,
    Image,
    UnityRender,
}

/// Kinds of assets known only by reading them (a scan per repo, kept by SHA).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FactKind {
    Effects,
    Sprites,
    TmpFonts,
}

/// A repository reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoRef {
    /// `owner/name`.
    pub full_name: String,
    pub default_branch: String,
}

/// What is known about assets beyond their measurements, for tabs that list other types: blob SHAs
/// per kind of scan.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<std::collections::HashSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sprites: Option<std::collections::HashSet<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmp_fonts: Option<std::collections::HashSet<String>>,
}

impl AssetFacts {
    /// The SHAs a scan of this kind found, when it ran.
    pub fn of(&self, kind: FactKind) -> Option<&std::collections::HashSet<String>> {
        match kind {
            FactKind::Effects => self.effects.as_ref(),
            FactKind::Sprites => self.sprites.as_ref(),
            FactKind::TmpFonts => self.tmp_fonts.as_ref(),
        }
    }
}
