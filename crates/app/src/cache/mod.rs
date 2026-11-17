//! The cache folder: downloaded listings and files, thumbnails, measurements and the size budget.
//!
//! The folder is fixed (`AppPaths::cache_dir`); every store reads it through a shared
//! [`CacheConfig`]. The app sizes the downloads itself (see [`downloads_budget`]).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::SystemTime;

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

/// Where the cache is.
pub struct CacheConfig {
    dir: PathBuf,
}

impl CacheConfig {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self { dir })
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }
}

const GIB: u64 = 1024 * 1024 * 1024;
/// The most the downloads may take, and what they may take on a roomy disk.
const BUDGET_MAX: u64 = 5 * GIB;
/// The least the downloads may be held to, however little space is left.
const BUDGET_MIN: u64 = 512 * 1024 * 1024;
/// A disk with this much space free (or more) does not shrink the budget.
const ROOMY_DISK: u64 = 20 * GIB;

/// How many bytes the downloads may take: 5 GiB, or a quarter of the free disk space (but not
/// less than 512 MiB) when under 20 GiB is free. `None` (the space could not be read) gives 5 GiB.
pub fn downloads_budget(free_bytes: Option<u64>) -> u64 {
    match free_bytes {
        Some(free) if free < ROOMY_DISK => (free / 4).clamp(BUDGET_MIN, BUDGET_MAX),
        _ => BUDGET_MAX,
    }
}

/// Free space, in bytes, on the volume that holds `path` (or the nearest folder of it that exists).
#[cfg(unix)]
pub fn free_space(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let existing = path.ancestors().find(|p| p.exists())?;
    let c_path = std::ffi::CString::new(existing.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is a valid NUL-terminated string and `stat` is a writable `statvfs`.
    if unsafe { libc::statvfs(c_path.as_ptr(), stat.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statvfs` returned 0, so it filled `stat`.
    let stat = unsafe { stat.assume_init() };
    // The field widths differ between platforms.
    #[allow(clippy::unnecessary_cast)]
    Some((stat.f_bavail as u64).saturating_mul(stat.f_frsize as u64))
}

/// Free space is not read on other platforms, so the budget stays at its largest.
#[cfg(not(unix))]
pub fn free_space(_path: &Path) -> Option<u64> {
    None
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

/// Marks a cached file as just used, so the downloads budget removes others first.
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
    fn the_downloads_budget_follows_the_free_space() {
        assert_eq!(downloads_budget(Some(500 * GIB)), 5 * GIB);
        assert_eq!(downloads_budget(Some(20 * GIB)), 5 * GIB);
        assert_eq!(downloads_budget(Some(19 * GIB)), 19 * GIB / 4);
        assert_eq!(downloads_budget(Some(8 * GIB)), 2 * GIB);
        assert_eq!(downloads_budget(Some(GIB)), 512 * 1024 * 1024);
        assert_eq!(downloads_budget(Some(0)), 512 * 1024 * 1024);
        assert_eq!(downloads_budget(None), 5 * GIB);
    }

    #[cfg(unix)]
    #[test]
    fn free_space_is_read_from_the_nearest_existing_folder() {
        let dir = tempfile::tempdir().unwrap();
        assert!(free_space(dir.path()).is_some());
        assert!(free_space(&dir.path().join("not/yet/made")).is_some());
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
