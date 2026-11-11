//! Keeps the cache folder in bounds.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::CacheConfig;
use crate::dto::CacheUsage;

/// Downloaded data that can always be fetched again: what the size limit removes, least recently
/// used first. `http` holds the revalidation store of GitHub API answers.
pub const DOWNLOADS: [&str; 4] = ["blobs", "tree", "guids", "http"];
/// Folders older versions wrote and nothing reads any more.
const STALE: [&str; 2] = ["prefabfx", "prefabfx2"];
/// Below the limit by this much after a cleanup, so it does not run again on the next download.
const PRUNE_TO: f64 = 0.9;
/// Downloads written between two checks of the limit.
const CHECK_EVERY: u64 = 200 * 1024 * 1024;
/// Temp files this old are left over from a crash, not a write in progress.
const STALE_TMP: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Clone)]
pub struct CachedFile {
    pub path: PathBuf,
    pub size: u64,
    pub used: SystemTime,
}

/// Every file under `dir` (a missing folder holds none). Symbolic links are not followed.
pub fn files_under(dir: &Path) -> Vec<CachedFile> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_owned()];
    while let Some(folder) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else { continue };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                if let Ok(meta) = entry.metadata() {
                    files.push(CachedFile { path: entry.path(), size: meta.len(), used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH) });
                }
            }
        }
    }
    files
}

/// Removes the files of `folders` (inside `root`), least recently used first, until they take no
/// more than `limit` bytes less a tenth. Does nothing while they fit, or without a limit (0).
pub fn prune_folders(root: &Path, folders: &[&str], limit: u64) {
    let mut files: Vec<CachedFile> = folders.iter().flat_map(|d| files_under(&root.join(d))).collect();
    let mut total: u64 = files.iter().map(|f| f.size).sum();
    if limit == 0 || total <= limit {
        return;
    }
    let goal = (limit as f64 * PRUNE_TO) as u64;
    files.sort_by_key(|f| f.used);
    for file in files {
        if total <= goal {
            break;
        }
        let _ = std::fs::remove_file(&file.path);
        total = total.saturating_sub(file.size);
    }
}

/// Downloads beyond the size limit go, least recently used first; leftovers of older versions and
/// crashed writes are removed at launch. Only folders the app writes are touched (the cache folder
/// may be one the user picked).
pub struct CacheJanitor {
    config: Arc<CacheConfig>,
    written: AtomicU64,
    running: tokio::sync::Mutex<()>,
}

impl CacheJanitor {
    pub fn new(config: Arc<CacheConfig>) -> Self {
        Self { config, written: AtomicU64::new(0), running: tokio::sync::Mutex::new(()) }
    }

    /// Counts a download written; checks the limit every so often.
    pub fn note_write(self: &Arc<Self>, bytes: u64) {
        if self.written.fetch_add(bytes, Ordering::Relaxed) + bytes < CHECK_EVERY {
            return;
        }
        self.written.store(0, Ordering::Relaxed);
        let janitor = self.clone();
        tauri::async_runtime::spawn(async move { janitor.prune().await });
    }

    pub async fn usage(&self) -> CacheUsage {
        let root = self.config.dir();
        tokio::task::spawn_blocking(move || {
            let sum = |files: Vec<CachedFile>| files.iter().map(|f| f.size).sum::<u64>();
            let all = sum(files_under(&root));
            let downloads = sum(DOWNLOADS.iter().flat_map(|d| files_under(&root.join(d))).collect());
            let thumbs = sum(files_under(&root.join("thumbs")));
            CacheUsage { downloads: downloads as f64, thumbs: thumbs as f64, other: all.saturating_sub(downloads + thumbs) as f64 }
        })
        .await
        .unwrap_or(CacheUsage { downloads: 0.0, thumbs: 0.0, other: 0.0 })
    }

    /// Removes the least recently used downloads until they fit the limit. A run that finds one
    /// going waits for it, then finds nothing left to do.
    pub async fn prune(&self) {
        let _running = self.running.lock().await;
        let (root, limit) = (self.config.dir(), self.config.limit());
        let _ = tokio::task::spawn_blocking(move || prune_folders(&root, &DOWNLOADS, limit)).await;
    }

    /// Leftovers: folders of older versions, temp files of interrupted writes.
    pub async fn clean_stale(&self) {
        let root = self.config.dir();
        let _ = tokio::task::spawn_blocking(move || {
            for folder in STALE {
                let _ = std::fs::remove_dir_all(root.join(folder));
            }
            let now = SystemTime::now();
            for file in files_under(&root) {
                let old = now.duration_since(file.used).is_ok_and(|age| age > STALE_TMP);
                if file.path.extension().is_some_and(|e| e == "tmp") && old {
                    let _ = std::fs::remove_file(&file.path);
                }
            }
        })
        .await;
    }

    /// Empties the downloads (they come back as they are needed); thumbnails and measurements stay.
    pub async fn clear(&self) -> std::io::Result<()> {
        let root = self.config.dir();
        tokio::task::spawn_blocking(move || {
            for folder in DOWNLOADS {
                match std::fs::remove_dir_all(root.join(folder)) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                    _ => {}
                }
            }
            Ok(())
        })
        .await
        .map_err(std::io::Error::other)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::test_support::{names, put};
    use crate::cache::FileBlobCache;

    fn janitor(root: &Path, limit: u64) -> CacheJanitor {
        CacheJanitor::new(CacheConfig::new(root.to_owned(), limit))
    }

    #[tokio::test]
    async fn removes_the_least_recently_used_downloads_until_under_the_limit_never_thumbnails() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "blobs/aa/old", 400, 3);
        put(root, "tree/org__repo/c1.json.gz", 400, 2);
        put(root, "blobs/bb/new", 400, 0);
        put(root, "thumbs/x.img", 5000, 9);
        janitor(root, 700).prune().await;
        assert!(names(&root.join("blobs/aa")).is_empty());
        assert!(names(&root.join("tree/org__repo")).is_empty());
        assert_eq!(names(&root.join("blobs/bb")), ["new"]);
        assert_eq!(names(&root.join("thumbs")), ["x.img"]);
    }

    #[tokio::test]
    async fn the_revalidation_store_counts_as_a_download() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "http/ab/old", 400, 3);
        put(root, "blobs/aa/new", 400, 0);
        janitor(root, 500).prune().await;
        assert!(names(&root.join("http/ab")).is_empty());
        assert_eq!(names(&root.join("blobs/aa")), ["new"]);
    }

    #[tokio::test]
    async fn keeps_everything_without_a_limit() {
        let dir = tempfile::tempdir().unwrap();
        put(dir.path(), "blobs/aa/old", 400, 3);
        janitor(dir.path(), 0).prune().await;
        assert_eq!(names(&dir.path().join("blobs/aa")), ["old"]);
    }

    #[tokio::test]
    async fn cleans_folders_of_older_versions_and_temp_files_left_by_crashes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "prefabfx2/org__repo/x.json", 10, 0);
        put(root, "blobs/aa/f.1.2.tmp", 10, 1);
        put(root, "blobs/aa/g.1.3.tmp", 10, 0);
        janitor(root, 0).clean_stale().await;
        assert!(!names(root).contains(&"prefabfx2".to_owned()));
        assert_eq!(names(&root.join("blobs/aa")), ["g.1.3.tmp"]);
    }

    #[tokio::test]
    async fn measures_downloads_thumbnails_and_the_rest_apart() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "blobs/aa/f", 100, 0);
        put(root, "thumbs/x.img", 20, 0);
        put(root, "stats.json", 3, 0);
        let usage = janitor(root, 0).usage().await;
        assert_eq!((usage.downloads, usage.thumbs, usage.other), (100.0, 20.0, 3.0));
    }

    #[tokio::test]
    async fn clears_downloads_but_keeps_thumbnails_and_measurements() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "blobs/aa/f", 100, 0);
        put(root, "http/ab/f", 100, 0);
        put(root, "thumbs/x.img", 20, 0);
        put(root, "stats.json", 3, 0);
        janitor(root, 0).clear().await.unwrap();
        assert_eq!(names(root), ["stats.json", "thumbs"]);
    }

    #[tokio::test]
    async fn blob_reads_mark_files_as_used() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        put(root, "blobs/ke/key1", 10, 5);
        let blobs = FileBlobCache::new(CacheConfig::new(root.to_owned(), 0), None);
        assert_eq!(blobs.get("key1").await.unwrap().len(), 10);
        put(root, "blobs/ot/other", 10, 1);
        janitor(root, 15).prune().await;
        assert_eq!(names(&root.join("blobs/ke")), ["key1"]);
        assert!(names(&root.join("blobs/ot")).is_empty());
    }

    #[tokio::test]
    async fn a_missing_cache_folder_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let janitor = janitor(&dir.path().join("none"), 1);
        janitor.prune().await;
        janitor.clean_stale().await;
        janitor.clear().await.unwrap();
        let usage = janitor.usage().await;
        assert_eq!((usage.downloads, usage.thumbs, usage.other), (0.0, 0.0, 0.0));
    }
}
