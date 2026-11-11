//! The cache folder: downloaded listings and files, thumbnails, measurements and the size limit.
//!
//! The folder is the `cacheDir` setting, which the user can change while the app runs, so every
//! store reads it through a shared [`CacheConfig`] on each call.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::SystemTime;

use crate::constants::DEFAULT_CACHE_LIMIT_GB;
use crate::dto::Settings;
use crate::http_cache::EtagStore;

pub mod blob;
pub mod commit;
pub mod janitor;
pub mod stats;
pub mod thumb_webp;
pub mod thumbs;

pub use blob::FileBlobCache;
pub use commit::FileCommitCache;
pub use janitor::CacheJanitor;
pub use stats::StatsStore;
pub use thumbs::ThumbnailStore;

/// Told how many bytes a download just wrote, so the janitor can check the size limit.
pub type OnWrite = Arc<dyn Fn(u64) + Send + Sync>;

/// Where the cache is and how large the downloads in it may grow.
pub struct CacheConfig {
    dir: RwLock<PathBuf>,
    limit: AtomicU64,
    /// The folder the user picked to share the index through (none: the team's GitHub repo does).
    shared_index_dir: RwLock<Option<PathBuf>>,
}

impl CacheConfig {
    /// `limit` in bytes; 0 for no limit.
    pub fn new(dir: PathBuf, limit: u64) -> Arc<Self> {
        Arc::new(Self { dir: RwLock::new(dir), limit: AtomicU64::new(limit), shared_index_dir: RwLock::new(None) })
    }

    pub fn from_settings(settings: &Settings) -> Arc<Self> {
        let config = Self::new(PathBuf::from(&settings.cache_dir), limit_bytes(settings.cache_limit_gb));
        config.apply(settings);
        config
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.read().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn limit(&self) -> u64 {
        self.limit.load(Ordering::Relaxed)
    }

    /// Takes the folder and limit of changed settings.
    pub fn apply(&self, settings: &Settings) {
        if let Ok(mut dir) = self.dir.write() {
            *dir = PathBuf::from(&settings.cache_dir);
        }
        self.limit.store(limit_bytes(settings.cache_limit_gb), Ordering::Relaxed);
        self.set_shared_index_dir(settings.shared_index_dir.as_deref().filter(|d| !d.is_empty()).map(PathBuf::from));
    }

    pub fn shared_index_dir(&self) -> Option<PathBuf> {
        self.shared_index_dir.read().ok().and_then(|d| d.clone())
    }

    pub fn set_shared_index_dir(&self, dir: Option<PathBuf>) {
        if let Ok(mut kept) = self.shared_index_dir.write() {
            *kept = dir;
        }
    }
}

/// The size limit of the settings in bytes (absent: the default; 0: no limit).
pub fn limit_bytes(limit_gb: Option<u32>) -> u64 {
    u64::from(limit_gb.unwrap_or(DEFAULT_CACHE_LIMIT_GB)) * 1024 * 1024 * 1024
}

/// A key as a file name: anything but letters, digits, `.`, `_` and `-` becomes `_`, and a name
/// of only dots (which would point at a folder) becomes underscores.
pub fn safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || (c == '.' && !name.chars().all(|d| d == '.')) { c } else { '_' }).collect()
}

/// A name that is safe to join onto a folder as it is (what [`safe`] would leave unchanged).
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Writes to a unique temp file, then renames it over the target: overlapping writes of one key
/// no longer collide, and a crash leaves either the old file or the new one.
pub async fn write_atomic(file: &Path, data: &[u8]) -> io::Result<()> {
    use std::sync::atomic::AtomicU32;
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    if let Some(parent) = file.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let mut name = file.as_os_str().to_owned();
    name.push(format!(".{}.{}.tmp", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)));
    let tmp = PathBuf::from(name);
    let result = async {
        tokio::fs::write(&tmp, data).await?;
        tokio::fs::rename(&tmp, file).await
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }
    result
}

/// Marks a cached file as just used, so the size limit removes others first.
pub async fn touch(file: &Path) {
    let file = file.to_owned();
    let _ = tokio::task::spawn_blocking(move || {
        std::fs::OpenOptions::new().write(true).open(&file).and_then(|f| f.set_modified(SystemTime::now()))
    })
    .await;
}

/// Everything the app caches, over one shared configuration.
pub struct Caches {
    pub config: Arc<CacheConfig>,
    pub janitor: Arc<CacheJanitor>,
    pub commits: FileCommitCache,
    pub blobs: FileBlobCache,
    pub thumbs: ThumbnailStore,
    pub stats: StatsStore,
    /// Revalidation of GitHub API answers; counts as a download.
    pub http: Arc<EtagStore>,
}

impl Caches {
    pub fn new(config: Arc<CacheConfig>) -> Self {
        let janitor = Arc::new(CacheJanitor::new(config.clone()));
        let on_write: OnWrite = {
            let janitor = janitor.clone();
            Arc::new(move |bytes| janitor.note_write(bytes))
        };
        Self {
            commits: FileCommitCache::new(config.clone(), Some(on_write.clone())),
            blobs: FileBlobCache::new(config.clone(), Some(on_write.clone())),
            http: EtagStore::new(config.clone(), Some(on_write)),
            thumbs: ThumbnailStore::new(config.clone()),
            stats: StatsStore::new(config.clone()),
            janitor,
            config,
        }
    }
}

#[cfg(test)]
pub mod test_support {
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    /// Writes `bytes` zero bytes at `root/rel`, last used `age_days` ago.
    pub fn put(root: &Path, rel: &str, bytes: usize, age_days: u64) {
        let file = root.join(rel);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, vec![0u8; bytes]).unwrap();
        let when = SystemTime::now() - Duration::from_secs(age_days * 86_400);
        std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(when).unwrap();
    }

    pub fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        names.sort();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_become_plain_file_names() {
        assert_eq!(safe("a/b c.d-e_f"), "a_b_c.d-e_f");
        assert_eq!(safe(".."), "__");
        assert_eq!(safe("..x"), "..x");
        assert!(is_plain_name("abc-1.2_x"));
        assert!(!is_plain_name("../settings"));
        assert!(!is_plain_name(""));
    }

    #[test]
    fn the_limit_defaults_to_two_gb_and_zero_means_none() {
        assert_eq!(limit_bytes(None), 2 * 1024 * 1024 * 1024);
        assert_eq!(limit_bytes(Some(5)), 5 * 1024 * 1024 * 1024);
        assert_eq!(limit_bytes(Some(0)), 0);
    }

    #[tokio::test]
    async fn an_atomic_write_replaces_the_file_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b/file.bin");
        write_atomic(&file, b"one").await.unwrap();
        write_atomic(&file, b"two").await.unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"two");
        assert_eq!(test_support::names(file.parent().unwrap()), ["file.bin"]);
    }
}
