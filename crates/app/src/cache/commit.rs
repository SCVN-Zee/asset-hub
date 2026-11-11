//! Listings and indexes kept per commit.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::de::DeserializeOwned;
use serde::Serialize;

use super::{safe, touch, write_atomic, CacheConfig, OnWrite};

/// `<cacheDir>/<kind>/<owner>__<repo>/<commit>.json.gz`: trees, GUID indexes and the like.
/// Gzipped: a tree listing shrinks about six times.
pub struct FileCommitCache {
    config: Arc<CacheConfig>,
    on_write: Option<OnWrite>,
}

impl FileCommitCache {
    pub fn new(config: Arc<CacheConfig>, on_write: Option<OnWrite>) -> Self {
        Self { config, on_write }
    }

    fn file(&self, kind: &str, repo: &str, commit_sha: &str) -> PathBuf {
        self.config.dir().join(safe(kind)).join(safe(&repo.replacen('/', "__", 1))).join(format!("{}.json.gz", safe(commit_sha)))
    }

    /// The value kept for the commit; a missing or unreadable file is no value.
    pub async fn get<T: DeserializeOwned>(&self, kind: &str, repo: &str, commit_sha: &str) -> Option<T> {
        let file = self.file(kind, repo, commit_sha);
        let packed = tokio::fs::read(&file).await.ok()?;
        let json = tokio::task::spawn_blocking(move || {
            let mut json = Vec::new();
            GzDecoder::new(packed.as_slice()).read_to_end(&mut json).ok().map(|_| json)
        })
        .await
        .ok()??;
        let value = serde_json::from_slice(&json).ok()?;
        touch(&file).await;
        Some(value)
    }

    /// Repos that have a listing of `kind` (folders are named `owner__name`; GitHub owners
    /// cannot contain `__`).
    pub async fn repos(&self, kind: &str) -> Vec<String> {
        let Ok(mut entries) = tokio::fs::read_dir(self.config.dir().join(safe(kind))).await else { return Vec::new() };
        let mut repos = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.contains("__") {
                repos.push(name.replacen("__", "/", 1));
            }
        }
        repos.sort();
        repos
    }

    pub async fn set<T: Serialize + ?Sized>(&self, kind: &str, repo: &str, commit_sha: &str, value: &T) -> std::io::Result<()> {
        let json = serde_json::to_vec(value).map_err(std::io::Error::other)?;
        let packed = tokio::task::spawn_blocking(move || {
            let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
            encoder.write_all(&json)?;
            encoder.finish()
        })
        .await
        .map_err(std::io::Error::other)??;
        write_atomic(&self.file(kind, repo, commit_sha), &packed).await?;
        if let Some(on_write) = &self.on_write {
            on_write(packed.len() as u64);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::test_support::names;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn cache(dir: &std::path::Path) -> FileCommitCache {
        FileCommitCache::new(CacheConfig::new(dir.to_owned(), 0), None)
    }

    #[tokio::test]
    async fn stores_listings_gzipped_and_reads_them_back() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        let tree = serde_json::json!([{ "path": "Assets/A.fbx", "sha": "a" }]);
        cache.set("tree", "org/repo", "c1", &tree).await.unwrap();
        let file = dir.path().join("tree/org__repo/c1.json.gz");
        let mut json = String::new();
        GzDecoder::new(std::fs::File::open(&file).unwrap()).read_to_string(&mut json).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&json).unwrap(), tree);
        assert_eq!(cache.get::<serde_json::Value>("tree", "org/repo", "c1").await, Some(tree));
    }

    #[tokio::test]
    async fn a_missing_or_corrupt_file_is_no_value() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        assert_eq!(cache.get::<serde_json::Value>("tree", "org/repo", "none").await, None);
        std::fs::create_dir_all(dir.path().join("tree/org__repo")).unwrap();
        std::fs::write(dir.path().join("tree/org__repo/bad.json.gz"), b"not gzip").unwrap();
        assert_eq!(cache.get::<serde_json::Value>("tree", "org/repo", "bad").await, None);
    }

    #[tokio::test]
    async fn lists_the_repos_that_have_a_listing() {
        let dir = tempfile::tempdir().unwrap();
        let cache = cache(dir.path());
        cache.set("guids", "org/b", "c1", &1).await.unwrap();
        cache.set("guids", "org/a", "c1", &1).await.unwrap();
        assert_eq!(cache.repos("guids").await, ["org/a", "org/b"]);
        assert!(cache.repos("tree").await.is_empty());
        assert_eq!(names(&dir.path().join("guids")), ["org__a", "org__b"]);
    }

    #[tokio::test]
    async fn reports_what_it_writes_for_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        let written = Arc::new(AtomicU64::new(0));
        let seen = written.clone();
        let on_write: OnWrite = Arc::new(move |n| {
            seen.fetch_add(n, Ordering::Relaxed);
        });
        FileCommitCache::new(CacheConfig::new(dir.path().to_owned(), 0), Some(on_write)).set("guids", "org/repo", "c1", &serde_json::json!({ "a": "b" })).await.unwrap();
        assert!(written.load(Ordering::Relaxed) > 0);
    }
}
