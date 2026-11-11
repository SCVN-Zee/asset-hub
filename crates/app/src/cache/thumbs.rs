//! Preview thumbnails as image files.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;

use super::thumb_webp::png_to_webp;
use super::{is_plain_name, safe, write_atomic, CacheConfig};

/// Scheme thumbnails are served on, so pages load them as files instead of receiving them over IPC.
pub const THUMB_SCHEME: &str = "assethub-thumb";

/// The most a saved thumbnail may take. A normal PNG card is far smaller; this only stops a
/// runaway page from filling the disk.
pub const MAX_THUMB_BYTES: usize = 8 * 1024 * 1024;

/// The image type of a file, read from its first bytes. A shared index holds WebP (1.x pages and
/// every thumbnail saved here), PNG (thumbnails of earlier 2.x versions) and JPEG. Anything else is
/// not an image the app wrote.
pub fn image_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

/// The bytes of a `data:image/(png|webp|jpeg);base64,` URL.
fn decode_data_url(data_url: &str) -> Option<Vec<u8>> {
    let rest = ["data:image/png;base64,", "data:image/webp;base64,", "data:image/jpeg;base64,"].iter().find_map(|prefix| data_url.strip_prefix(prefix))?;
    let packed: String = rest.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(packed).ok()
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A thumbnail found on disk, with its image type.
#[derive(Debug, PartialEq, Eq)]
pub struct Thumbnail {
    pub bytes: Vec<u8>,
    pub content_type: &'static str,
}

/// Thumbnails on disk per cache folder, with a version that changes when one is drawn again.
#[derive(Default)]
struct Index {
    dir: Option<PathBuf>,
    versions: HashMap<String, u64>,
}

/// `<cacheDir>/thumbs/<key>.img`, keyed by the asset's content hash. Pages get URLs on
/// [`THUMB_SCHEME`] and load only the images on screen. Thumbnails saved as data-URL text
/// (`.txt`) by earlier versions are served too, and converted when read.
pub struct ThumbnailStore {
    config: Arc<CacheConfig>,
    index: tokio::sync::Mutex<Index>,
}

impl ThumbnailStore {
    pub fn new(config: Arc<CacheConfig>) -> Self {
        Self { config, index: tokio::sync::Mutex::new(Index::default()) }
    }

    fn dir(&self) -> PathBuf {
        self.config.dir().join("thumbs")
    }

    /// The index of the current folder, read from disk the first time it is needed.
    async fn index(&self) -> tokio::sync::MutexGuard<'_, Index> {
        let mut index = self.index.lock().await;
        let dir = self.dir();
        if index.dir.as_ref() != Some(&dir) {
            let mut versions = HashMap::new();
            if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
                while let Ok(Some(entry)) = entries.next_entry().await {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if let Some(key) = name.strip_suffix(".img").or_else(|| name.strip_suffix(".txt")) {
                        versions.insert(key.to_owned(), 0);
                    }
                }
            }
            *index = Index { dir: Some(dir), versions };
        }
        index
    }

    /// URLs of the thumbnails that exist among `keys`. The version in the query changes when a
    /// thumbnail is drawn again, so the page loads the new image.
    pub async fn get_many(&self, keys: &[String]) -> HashMap<String, String> {
        let index = self.index().await;
        keys.iter().filter_map(|key| index.versions.get(&safe(key)).map(|v| (key.clone(), format!("{THUMB_SCHEME}://t/{}?v={v}", safe(key))))).collect()
    }

    /// The image behind a thumbnail URL's name.
    pub async fn read(&self, name: &str) -> Option<Thumbnail> {
        if !is_plain_name(name) {
            return None;
        }
        let file = self.dir().join(format!("{name}.img"));
        if let Ok(bytes) = tokio::fs::read(&file).await {
            return image_type(&bytes).map(|content_type| Thumbnail { bytes, content_type });
        }
        // Not converted yet: an earlier version's data URL.
        let legacy = self.dir().join(format!("{name}.txt"));
        let bytes = decode_data_url(&tokio::fs::read_to_string(&legacy).await.ok()?)?;
        let content_type = image_type(&bytes)?;
        write_atomic(&file, &bytes).await.ok()?;
        let _ = tokio::fs::remove_file(&legacy).await;
        Some(Thumbnail { bytes, content_type })
    }

    /// Keeps a rendered thumbnail. A PNG (what WKWebView answers when asked for WebP) is stored as
    /// WebP, unless that is not smaller or cannot be made; WebP and JPEG are stored as given. A URL
    /// that is not one of those, bytes that are not that image, or bytes beyond [`MAX_THUMB_BYTES`]
    /// are dropped.
    pub async fn save(&self, key: &str, data_url: &str) -> std::io::Result<()> {
        let Some(bytes) = decode_data_url(data_url) else { return Ok(()) };
        let Some(kind) = image_type(&bytes).filter(|_| bytes.len() <= MAX_THUMB_BYTES) else { return Ok(()) };
        let bytes = if kind == "image/png" {
            tokio::task::spawn_blocking(move || png_to_webp(&bytes).unwrap_or(bytes)).await.map_err(std::io::Error::other)?
        } else {
            bytes
        };
        let name = safe(key);
        write_atomic(&self.dir().join(format!("{name}.img")), &bytes).await?;
        let _ = tokio::fs::remove_file(self.dir().join(format!("{name}.txt"))).await;
        let mut index = self.index().await;
        let next = index.versions.get(&name).map_or(0, |v| v + 1).max(now_ms());
        index.versions.insert(name, next);
        Ok(())
    }

    /// Names of every thumbnail on disk (keys as stored), for a shared index.
    pub async fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.index().await.versions.keys().cloned().collect();
        names.sort();
        names
    }

    /// Adds a thumbnail from a shared index; one already here stays unless `replace` (a newer kind
    /// of card). A file that is not an image is refused.
    pub async fn import_file(&self, name: &str, bytes: &[u8], replace: bool) -> std::io::Result<bool> {
        if !is_plain_name(name) || image_type(bytes).is_none() {
            return Ok(false);
        }
        if self.index().await.versions.contains_key(name) && !replace {
            return Ok(false);
        }
        let _ = tokio::fs::remove_file(self.dir().join(format!("{name}.txt"))).await;
        write_atomic(&self.dir().join(format!("{name}.img")), bytes).await?;
        let mut index = self.index().await;
        let next = index.versions.get(name).map_or(0, |v| v + 1).max(now_ms());
        index.versions.insert(name.to_owned(), next);
        Ok(true)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::cache::test_support::names;

    /// A 1x1 PNG.
    pub const PNG: &str = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

    fn store(dir: &std::path::Path) -> ThumbnailStore {
        ThumbnailStore::new(CacheConfig::new(dir.to_owned(), 0))
    }

    fn data_url(mime: &str, bytes: &[u8]) -> String {
        format!("data:image/{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    fn webp() -> Vec<u8> {
        let mut bytes = b"RIFF".to_vec();
        bytes.extend_from_slice(&[4, 0, 0, 0]);
        bytes.extend_from_slice(b"WEBPVP8 ");
        bytes
    }

    fn jpeg() -> Vec<u8> {
        vec![0xff, 0xd8, 0xff, 0xe0, 0, 16]
    }

    #[tokio::test]
    async fn saves_images_as_files_and_hands_out_urls_only_for_those_that_exist() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("sha1", PNG).await.unwrap();
        let urls = store.get_many(&["sha1".into(), "sha2".into()]).await;
        assert_eq!(urls.keys().collect::<Vec<_>>(), ["sha1"]);
        assert!(urls["sha1"].starts_with("assethub-thumb://t/sha1?v="), "{}", urls["sha1"]);
        let found = store.read("sha1").await.unwrap();
        assert_eq!(found.content_type, "image/png");
        assert_eq!(found.bytes[0], 0x89);
    }

    /// A real 320 x 240 card as WKWebView draws it, and the size of the WebP that a 1.x page saved for the same card.
    const CARD_A: &[u8] = include_bytes!("../testdata/thumb-card-a.png");
    const CARD_A_1X_WEBP_BYTES: usize = 4_238;

    #[tokio::test]
    async fn stores_a_png_card_as_webp_and_keeps_an_earlier_png_readable() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("card", &data_url("png", CARD_A)).await.unwrap();
        let saved = store.read("card").await.unwrap();
        assert_eq!(saved.content_type, "image/webp");
        assert!(saved.bytes.len() * 2 < CARD_A.len(), "{} bytes", saved.bytes.len());
        assert!(saved.bytes.len() * 2 <= CARD_A_1X_WEBP_BYTES * 3);
        // A PNG written by an earlier version still displays, and is not rewritten by reading.
        std::fs::write(dir.path().join("thumbs/old.img"), CARD_A).unwrap();
        let fresh = ThumbnailStore::new(CacheConfig::new(dir.path().to_owned(), 0));
        let old = fresh.read("old").await.unwrap();
        assert_eq!((old.content_type, old.bytes.as_slice()), ("image/png", CARD_A));
        assert_eq!(fresh.get_many(&["old".into(), "card".into()]).await.len(), 2);
    }

    #[tokio::test]
    async fn keeps_a_png_that_cannot_be_made_smaller_as_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("tiny", PNG).await.unwrap();
        assert_eq!(store.read("tiny").await.unwrap().content_type, "image/png");
    }

    #[tokio::test]
    async fn serves_each_image_with_the_type_of_its_first_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("p", PNG).await.unwrap();
        store.save("w", &data_url("webp", &webp())).await.unwrap();
        store.save("j", &data_url("jpeg", &jpeg())).await.unwrap();
        assert_eq!(store.read("p").await.unwrap().content_type, "image/png");
        assert_eq!(store.read("w").await.unwrap().content_type, "image/webp");
        assert_eq!(store.read("j").await.unwrap().content_type, "image/jpeg");
        // The bytes are kept as given, not re-encoded.
        assert_eq!(std::fs::read(dir.path().join("thumbs/w.img")).unwrap(), webp());
    }

    #[tokio::test]
    async fn the_declared_type_does_not_decide_the_served_type() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        // A PNG sent as "webp" is a PNG.
        let png = base64::engine::general_purpose::STANDARD.decode(PNG.strip_prefix("data:image/png;base64,").unwrap()).unwrap();
        store.save("x", &data_url("webp", &png)).await.unwrap();
        assert_eq!(store.read("x").await.unwrap().content_type, "image/png");
    }

    #[tokio::test]
    async fn drops_what_is_not_an_image_or_is_too_large_or_the_wrong_kind_of_url() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("text", &data_url("png", b"<html>not an image</html>")).await.unwrap();
        store.save("gif", "data:image/gif;base64,R0lGODlhAQABAAAAACw=").await.unwrap();
        store.save("junk", "data:image/png;base64,@@@").await.unwrap();
        store.save("web", "https://example.com/a.png").await.unwrap();
        let mut huge = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        huge.resize(MAX_THUMB_BYTES + 1, 0);
        store.save("huge", &data_url("png", &huge)).await.unwrap();
        assert!(!dir.path().join("thumbs").exists());
        // Exactly the cap is still kept.
        huge.truncate(MAX_THUMB_BYTES);
        store.save("edge", &data_url("png", &huge)).await.unwrap();
        assert_eq!(names(&dir.path().join("thumbs")), ["edge.img"]);
    }

    #[tokio::test]
    async fn serves_and_converts_data_url_thumbnails_of_earlier_versions() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("thumbs")).unwrap();
        std::fs::write(dir.path().join("thumbs/old.txt"), PNG).unwrap();
        let store = store(dir.path());
        assert_eq!(store.get_many(&["old".into()]).await.len(), 1);
        assert_eq!(store.read("old").await.unwrap().content_type, "image/png");
        assert_eq!(names(&dir.path().join("thumbs")), ["old.img"]);
    }

    #[tokio::test]
    async fn changes_the_url_when_a_thumbnail_is_drawn_again() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("sha1", PNG).await.unwrap();
        let first = store.get_many(&["sha1".into()]).await["sha1"].clone();
        store.save("sha1", PNG).await.unwrap();
        assert_ne!(store.get_many(&["sha1".into()]).await["sha1"], first);
    }

    #[tokio::test]
    async fn refuses_names_outside_the_cache_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("settings.img"), b"x").unwrap();
        let store = store(dir.path());
        assert!(store.read("../settings").await.is_none());
        assert!(store.read("").await.is_none());
        assert!(!store.import_file("../x", &jpeg(), true).await.unwrap());
    }

    #[tokio::test]
    async fn a_file_that_is_not_an_image_is_not_served() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("thumbs")).unwrap();
        std::fs::write(dir.path().join("thumbs/bad.img"), b"<html>").unwrap();
        assert!(store(dir.path()).read("bad").await.is_none());
    }

    #[tokio::test]
    async fn imports_from_a_shared_index_keeping_what_is_here_unless_told_to_replace() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        assert!(store.import_file("a", &webp(), false).await.unwrap());
        assert!(!store.import_file("a", &jpeg(), false).await.unwrap());
        assert_eq!(store.read("a").await.unwrap().content_type, "image/webp");
        assert!(store.import_file("a", &jpeg(), true).await.unwrap());
        assert_eq!(store.read("a").await.unwrap().content_type, "image/jpeg");
        assert!(!store.import_file("b", b"nothing", true).await.unwrap());
        assert_eq!(store.names().await, ["a"]);
    }

    #[tokio::test]
    async fn follows_a_changed_cache_folder() {
        let (one, two) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let config = CacheConfig::new(one.path().to_owned(), 0);
        let store = ThumbnailStore::new(config.clone());
        store.save("a", PNG).await.unwrap();
        let mut settings = crate::settings::default_settings(two.path());
        settings.cache_limit_gb = Some(0);
        config.apply(&settings);
        assert!(store.get_many(&["a".into()]).await.is_empty());
        store.save("b", PNG).await.unwrap();
        assert_eq!(names(&two.path().join("thumbs")), ["b.img"]);
    }
}
