//! Folder tree of the assets (`src/core/tree.ts`).

use serde::{Deserialize, Serialize};

use crate::js::locale_compare;
use crate::registry::dir_name;
use crate::types::Asset;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderNode {
    pub name: String,
    /// Full folder path; empty for the root.
    pub path: String,
    pub children: Vec<FolderNode>,
    /// Assets in this folder and all sub-folders.
    pub asset_count: usize,
}

impl FolderNode {
    fn new(name: &str, path: String) -> Self {
        Self { name: name.to_string(), path, children: Vec::new(), asset_count: 0 }
    }
}

/// Builds a folder tree that only contains folders holding at least one asset.
pub fn build_folder_tree(assets: &[Asset]) -> FolderNode {
    let mut root = FolderNode::new("", String::new());
    for asset in assets {
        root.asset_count += 1;
        let dir = dir_name(&asset.path);
        if dir.is_empty() {
            continue;
        }
        let mut node = &mut root;
        for segment in dir.split('/') {
            let path = if node.path.is_empty() { segment.to_string() } else { format!("{}/{segment}", node.path) };
            let at = match node.children.iter().position(|c| c.path == path) {
                Some(at) => at,
                None => {
                    node.children.push(FolderNode::new(segment, path));
                    node.children.len() - 1
                }
            };
            node = &mut node.children[at];
            node.asset_count += 1;
        }
    }
    sort_tree(&mut root);
    root
}

fn sort_tree(node: &mut FolderNode) {
    // A stable sort, as `Array.prototype.sort` is.
    node.children.sort_by(|a, b| locale_compare(&a.name, &b.name));
    node.children.iter_mut().for_each(sort_tree);
}

/// True when `path` lies inside `folder` (an empty folder matches everything).
pub fn is_in_folder(path: &str, folder: &str) -> bool {
    folder.is_empty() || path.strip_prefix(folder).is_some_and(|rest| rest.starts_with('/'))
}
