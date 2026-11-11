//! What a target Unity project has, read from disk (never written).
//!
//! This is the file-reading half of the target project check; comparing it with an export (the
//! plan against the target) lives with the exporter. Paths are `Assets/...` with forward slashes.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use regex::Regex;
use sha1::{Digest, Sha1};

/// Files read at the same time while scanning.
const READERS: usize = 16;

/// What the target project has.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectFiles {
    pub root: String,
    /// GUID from a `.meta` file to the asset path next to it.
    pub by_guid: HashMap<String, String>,
    pub sizes: HashMap<String, u64>,
    /// Asset paths of each file size, in path order.
    pub by_size: HashMap<u64, Vec<String>>,
}

/// A folder Unity opens as a project: `Assets` and `ProjectSettings` side by side.
pub fn is_unity_project(dir: &Path) -> bool {
    dir.join("Assets").exists() && dir.join("ProjectSettings").exists()
}

/// The GUID of a `.meta` file's text.
pub fn meta_guid(meta_text: &str) -> Option<String> {
    asset_hub_core::unity::yaml::meta_guid(meta_text)
}

/// Files under a folder of the project (`Assets/...`), skipping what Unity ignores: hidden files
/// and folders, and folders ending in `~`. Links are not followed. Sorted.
fn list_files(root: &Path, dir: &str) -> Vec<String> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(rel) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(root.join(&rel)) else { continue };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.ends_with('~') {
                continue;
            }
            let Ok(kind) = entry.file_type() else { continue };
            let path = format!("{rel}/{name}");
            if kind.is_dir() {
                pending.push(path);
            } else if kind.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

/// Runs `read` on every item over a few threads; results come back in item order.
fn read_all<T: Sync, R: Send>(items: &[T], read: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let next = AtomicUsize::new(0);
    let mut results: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..READERS)
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(at) else { return done };
                        done.push((at, read(item)));
                    }
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().unwrap_or_default()).collect()
    });
    results.sort_by_key(|(at, _)| *at);
    results.into_iter().map(|(_, r)| r).collect()
}

enum Read {
    Guid(String, String),
    Size(String, u64),
    Skipped,
}

/// Every file under `Assets/` with its size, and every GUID from the `.meta` files. A file that
/// vanished or cannot be read is simply not compared.
pub async fn scan_project(root: &Path) -> ProjectFiles {
    let root = root.to_owned();
    tokio::task::spawn_blocking(move || {
        let files = list_files(&root, "Assets");
        let reads = read_all(&files, |path| match path.strip_suffix(".meta") {
            Some(asset) => match std::fs::read_to_string(root.join(path)).ok().and_then(|text| meta_guid(&text)) {
                Some(guid) => Read::Guid(guid, asset.to_owned()),
                None => Read::Skipped,
            },
            None => std::fs::metadata(root.join(path)).map_or(Read::Skipped, |meta| Read::Size(path.clone(), meta.len())),
        });
        let mut found = ProjectFiles { root: root.to_string_lossy().into_owned(), ..ProjectFiles::default() };
        for read in reads {
            match read {
                Read::Guid(guid, asset) => {
                    found.by_guid.insert(guid, asset);
                }
                Read::Size(path, size) => {
                    found.by_size.entry(size).or_default().push(path.clone());
                    found.sizes.insert(path, size);
                }
                Read::Skipped => {}
            }
        }
        found
    })
    .await
    .unwrap_or_default()
}

/// Content hash used on both sides of the comparison.
pub fn content_hash(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

pub async fn hash_project_file(root: &Path, path: &str) -> Option<String> {
    tokio::fs::read(root.join(path)).await.ok().map(|bytes| content_hash(&bytes))
}

pub async fn read_project_text(root: &Path, path: &str) -> Option<String> {
    tokio::fs::read_to_string(root.join(path)).await.ok()
}

/// Files of the project's packages (URP and others, embedded or downloaded) whose path matches,
/// with their GUIDs: shaders materials can be pointed at, scripts prefabs need.
pub async fn package_files(root: &Path, matches: &Regex) -> HashMap<String, String> {
    let (root, matches) = (root.to_owned(), matches.clone());
    tokio::task::spawn_blocking(move || {
        let wanted: Vec<String> = ["Packages", "Library/PackageCache"].iter().flat_map(|dir| list_files(&root, dir)).filter(|path| matches.is_match(path)).collect();
        let guids = read_all(&wanted, |path| std::fs::read_to_string(root.join(format!("{path}.meta"))).ok().and_then(|text| meta_guid(&text)));
        wanted.into_iter().zip(guids).filter_map(|(path, guid)| Some((path, guid?))).collect()
    })
    .await
    .unwrap_or_default()
}

/// Unity has resolved the project's packages (opened it once): their files are on disk.
pub fn has_package_cache(root: &Path) -> bool {
    root.join("Library/PackageCache").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn guid(c: char) -> String {
        c.to_string().repeat(32)
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let file = root.join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    /// The project of the 1.x test: a texture with its meta, files Unity ignores, a URP shader.
    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("ProjectSettings")).unwrap();
        write(root, "Assets/Art/rock.png", "PNGDATA");
        write(root, "Assets/Art/rock.png.meta", &format!("fileFormatVersion: 2\nguid: {}\nTextureImporter:\n", guid('a')));
        write(root, "Assets/Art/Hidden~/skip.png", "PNGDATA");
        write(root, "Assets/.DS_Store", "x");
        let urp = "Library/PackageCache/com.unity.render-pipelines.universal@14.0.8/Shaders";
        write(root, &format!("{urp}/Lit.shader"), "Shader \"Universal Render Pipeline/Lit\" {}");
        write(root, &format!("{urp}/Lit.shader.meta"), &format!("guid: {}\n", guid('b')));
        dir
    }

    #[test]
    fn knows_a_unity_project_folder() {
        let dir = project();
        assert!(is_unity_project(dir.path()));
        assert!(!is_unity_project(&dir.path().join("Assets")));
        assert!(!is_unity_project(&dir.path().join("none")));
    }

    #[tokio::test]
    async fn reads_guids_and_sizes_under_assets_skipping_what_unity_ignores() {
        let dir = project();
        let found = scan_project(dir.path()).await;
        assert_eq!(found.by_guid.into_iter().collect::<Vec<_>>(), [(guid('a'), "Assets/Art/rock.png".to_owned())]);
        assert_eq!(found.sizes.into_iter().collect::<Vec<_>>(), [("Assets/Art/rock.png".to_owned(), 7)]);
        assert_eq!(found.by_size[&7], ["Assets/Art/rock.png"]);
        assert_eq!(hash_project_file(dir.path(), "Assets/Art/rock.png").await, Some(content_hash(b"PNGDATA")));
        assert_eq!(hash_project_file(dir.path(), "Assets/none.png").await, None);
    }

    #[tokio::test]
    async fn files_of_one_size_are_listed_in_path_order() {
        let dir = project();
        for name in ["z.txt", "b.txt", "m.txt"] {
            write(dir.path(), &format!("Assets/{name}"), "1234567");
        }
        let found = scan_project(dir.path()).await;
        assert_eq!(found.by_size[&7], ["Assets/Art/rock.png", "Assets/b.txt", "Assets/m.txt", "Assets/z.txt"]);
    }

    #[tokio::test]
    async fn a_meta_without_a_guid_and_a_folder_that_is_missing_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Assets/a.png.meta", "fileFormatVersion: 2\n");
        let found = scan_project(dir.path()).await;
        assert!(found.by_guid.is_empty() && found.sizes.is_empty());
        let none = scan_project(&dir.path().join("missing")).await;
        assert!(none.sizes.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn links_are_not_followed() {
        let dir = project();
        std::os::unix::fs::symlink(dir.path().join("Assets/Art"), dir.path().join("Assets/Linked")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("Assets/Art/rock.png"), dir.path().join("Assets/rock-link.png")).unwrap();
        let found = scan_project(dir.path()).await;
        assert_eq!(found.sizes.keys().collect::<Vec<_>>(), ["Assets/Art/rock.png"]);
    }

    #[tokio::test]
    async fn finds_the_shaders_of_the_project_packages_with_their_guids() {
        let dir = project();
        let found = package_files(dir.path(), &Regex::new(r"\.shader$").unwrap()).await;
        let path = "Library/PackageCache/com.unity.render-pipelines.universal@14.0.8/Shaders/Lit.shader";
        assert_eq!(found.into_iter().collect::<Vec<_>>(), [(path.to_owned(), guid('b'))]);
        assert!(has_package_cache(dir.path()));
        assert!(!has_package_cache(&dir.path().join("Assets")));
    }

    #[tokio::test]
    async fn reads_project_text() {
        let dir = project();
        assert_eq!(read_project_text(dir.path(), "Assets/Art/rock.png").await.as_deref(), Some("PNGDATA"));
        assert_eq!(read_project_text(dir.path(), "Assets/none").await, None);
    }

    #[test]
    fn reads_the_guid_of_meta_text() {
        assert_eq!(meta_guid(&format!("fileFormatVersion: 2\r\nguid: {}\r\n", guid('c'))), Some(guid('c')));
        assert_eq!(meta_guid("  guid: abc"), None);
        assert_eq!(meta_guid(&format!("guid: {}", "G".repeat(32))), None);
    }

    #[test]
    fn hashes_like_sha1() {
        assert_eq!(content_hash(b"abc"), "a9993e364706816aba3e25717850c26c9cd0d89d");
    }
}
