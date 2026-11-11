//! Downloaded file contents.

use std::path::PathBuf;
use std::sync::Arc;

use super::{safe, touch, write_atomic, CacheConfig, OnWrite};

/// `<cacheDir>/blobs/<ab>/<hash>`: contents of downloaded files, shared across repos and branches.
pub struct FileBlobCache {
    config: Arc<CacheConfig>,
    on_write: Option<OnWrite>,
}

impl FileBlobCache {
    pub fn new(config: Arc<CacheConfig>, on_write: Option<OnWrite>) -> Self {
        Self { config, on_write }
    }

    fn file(&self, key: &str) -> PathBuf {
        let key = safe(key);
        // Not "..": the prefix is a folder name.
        let prefix: String = key.chars().take(2).map(|c| if c == '.' { '_' } else { c }).collect();
        self.config.dir().join("blobs").join(prefix).join(key)
    }

    pub async fn get(&self, key: &str) -> Option<Vec<u8>> {
        let file = self.file(key);
        let bytes = tokio::fs::read(&file).await.ok()?;
        touch(&file).await;
        Some(bytes)
    }

    pub async fn set(&self, key: &str, bytes: &[u8]) -> std::io::Result<()> {
        write_atomic(&self.file(key), bytes).await?;
        if let Some(on_write) = &self.on_write {
            on_write(bytes.len() as u64);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keeps_bytes_under_a_two_letter_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FileBlobCache::new(CacheConfig::new(dir.path().to_owned(), 0), None);
        assert_eq!(cache.get("abcdef").await, None);
        cache.set("abcdef", &[0, 1, 255]).await.unwrap();
        assert_eq!(cache.get("abcdef").await, Some(vec![0, 1, 255]));
        assert_eq!(std::fs::read(dir.path().join("blobs/ab/abcdef")).unwrap(), [0, 1, 255]);
    }

    #[tokio::test]
    async fn a_key_cannot_leave_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let cache = FileBlobCache::new(CacheConfig::new(dir.path().to_owned(), 0), None);
        cache.set("../../escape", b"x").await.unwrap();
        assert!(dir.path().join("blobs/__/.._.._escape").exists());
    }
}
