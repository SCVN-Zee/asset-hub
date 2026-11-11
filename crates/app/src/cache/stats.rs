//! Model measurements by content SHA, in one small JSON file (`<cacheDir>/stats.json`).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use super::{write_atomic, CacheConfig};
use asset_hub_core::stats as core_stats;

use crate::dto::{StatsSource, StatsSummary};
use crate::github_net::convert;

/// A write follows the first change by this long, so a burst of measurements is one write.
const WRITE_DELAY: Duration = Duration::from_secs(1);

/// True for a measurement with counts that are finite and not negative (the types carry the rest).
pub fn is_valid(summary: &StatsSummary) -> bool {
    serde_json::to_value(summary).is_ok_and(|value| core_stats::is_stats_summary(&value))
}

fn to_core(summary: &StatsSummary) -> Option<core_stats::StatsSummary> {
    convert(summary).ok()
}

/// `summary`, keeping the card versions of `kept` it does not have: measuring again does not
/// redraw the card.
pub fn keep_card_versions(kept: Option<&StatsSummary>, summary: &StatsSummary) -> StatsSummary {
    let kept = kept.and_then(to_core);
    to_core(summary).and_then(|s| convert(&core_stats::keep_card_versions(kept.as_ref(), &s)).ok()).unwrap_or_else(|| summary.clone())
}

/// What to keep when `summary` is saved over `kept`. A measurement from a thumbnail counts only
/// the textures, so it never replaces a full (viewer) one: the clip names and the card versions it
/// brings are added to it instead.
pub fn merge_stats(kept: Option<&StatsSummary>, summary: &StatsSummary) -> StatsSummary {
    let kept = kept.and_then(to_core);
    to_core(summary).and_then(|s| convert(&core_stats::merge_stats(kept.as_ref(), &s)).ok()).unwrap_or_else(|| summary.clone())
}

type Measurements = HashMap<String, StatsSummary>;

struct Inner {
    config: Arc<CacheConfig>,
    state: tokio::sync::Mutex<State>,
}

#[derive(Default)]
struct State {
    /// Read from disk the first time it is needed.
    data: Option<Measurements>,
    /// A write is waiting for its delay to pass.
    write_pending: bool,
}

#[derive(Clone)]
pub struct StatsStore {
    inner: Arc<Inner>,
}

impl StatsStore {
    pub fn new(config: Arc<CacheConfig>) -> Self {
        Self { inner: Arc::new(Inner { config, state: tokio::sync::Mutex::default() }) }
    }

    fn file(&self) -> std::path::PathBuf {
        self.inner.config.dir().join("stats.json")
    }

    /// The measurements, from disk on the first call. Entries that do not parse or are not valid
    /// are left out; a file that does not parse is no measurements.
    async fn load<'a>(&self, state: &'a mut State) -> &'a mut Measurements {
        if state.data.is_none() {
            let raw = tokio::fs::read(self.file()).await.ok().and_then(|bytes| serde_json::from_slice::<serde_json::Map<String, Value>>(&bytes).ok());
            let data = raw
                .into_iter()
                .flatten()
                .filter_map(|(key, value)| serde_json::from_value::<StatsSummary>(value).ok().filter(is_valid).map(|summary| (key, summary)))
                .collect();
            state.data = Some(data);
        }
        state.data.get_or_insert_default()
    }

    pub async fn get_many(&self, keys: &[String]) -> HashMap<String, StatsSummary> {
        let mut state = self.inner.state.lock().await;
        let all = self.load(&mut state).await;
        keys.iter().filter_map(|key| all.get(key).map(|summary| (key.clone(), summary.clone()))).collect()
    }

    /// Every measurement, for a shared index.
    pub async fn all(&self) -> HashMap<String, StatsSummary> {
        let mut state = self.inner.state.lock().await;
        self.load(&mut state).await.clone()
    }

    /// Measurements from a shared index: taken where none is known here, or where it is fuller (a
    /// viewer measurement over a thumbnail one). Returns how many were taken.
    pub async fn import_many(&self, raw: &Value) -> usize {
        let Some(entries) = raw.as_object() else { return 0 };
        let mut state = self.inner.state.lock().await;
        let all = self.load(&mut state).await;
        let mut taken = 0;
        for (key, value) in entries {
            let Some(summary) = serde_json::from_value::<StatsSummary>(value.clone()).ok().filter(is_valid) else { continue };
            let kept = all.get(key);
            if kept.is_some_and(|k| !(k.source == StatsSource::Thumb && summary.source == StatsSource::Viewer)) {
                continue;
            }
            let merged = keep_card_versions(kept, &summary);
            all.insert(key.clone(), merged);
            taken += 1;
        }
        if taken > 0 {
            self.schedule_write(&mut state);
        }
        taken
    }

    /// Keeps a measurement (merged with the one kept) and answers what is kept now; an invalid one is
    /// ignored (nothing answered).
    pub async fn save(&self, key: &str, summary: &StatsSummary) -> Option<StatsSummary> {
        if !is_valid(summary) {
            return None;
        }
        let mut state = self.inner.state.lock().await;
        let all = self.load(&mut state).await;
        let merged = merge_stats(all.get(key), summary);
        all.insert(key.to_owned(), merged.clone());
        self.schedule_write(&mut state);
        Some(merged)
    }

    fn schedule_write(&self, state: &mut State) {
        if state.write_pending {
            return;
        }
        state.write_pending = true;
        let store = self.clone();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(WRITE_DELAY).await;
            store.flush().await;
        });
    }

    /// Writes the measurements now if a write is waiting (also when the app quits).
    pub async fn flush(&self) {
        let mut state = self.inner.state.lock().await;
        if !std::mem::take(&mut state.write_pending) {
            return;
        }
        let Some(data) = &state.data else { return };
        // Sorted, so the file does not change when nothing did.
        let sorted: std::collections::BTreeMap<_, _> = data.iter().collect();
        if let Ok(json) = serde_json::to_vec(&sorted) {
            let _ = write_atomic(&self.file(), &json).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    pub fn summary(source: &str, extra: Value) -> StatsSummary {
        let mut base = json!({ "triangles": 10, "vertices": 5, "bones": 0, "clips": 0, "materials": 1, "fileBytes": 100, "textureBytes": 50, "maxTextureSize": 256, "source": source });
        base.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        serde_json::from_value(base).unwrap()
    }

    fn store(dir: &std::path::Path) -> StatsStore {
        StatsStore::new(CacheConfig::new(dir.to_owned(), 0))
    }

    #[test]
    fn rejects_negative_and_non_finite_counts() {
        assert!(is_valid(&summary("viewer", json!({}))));
        assert!(!is_valid(&summary("viewer", json!({ "triangles": -1 }))));
        let mut bad = summary("viewer", json!({}));
        bad.bones = f64::NAN;
        assert!(!is_valid(&bad));
        bad.bones = f64::INFINITY;
        assert!(!is_valid(&bad));
    }

    #[test]
    fn a_thumbnail_measurement_never_replaces_a_viewer_one() {
        let kept = summary("viewer", json!({ "triangles": 999, "fxThumb": 1 }));
        let thumb = summary("thumb", json!({ "triangles": 1, "clipNames": ["Walk"], "fxThumb": 2, "modelThumb": 3 }));
        let merged = merge_stats(Some(&kept), &thumb);
        assert_eq!(merged.triangles, 999.0);
        assert_eq!(merged.source, StatsSource::Viewer);
        assert_eq!(merged.clip_names, Some(vec!["Walk".to_owned()]));
        assert_eq!((merged.fx_thumb, merged.model_thumb, merged.anim_thumb), (Some(2.0), Some(3.0), None));
        // Clip names already known stay.
        let known = summary("viewer", json!({ "clipNames": ["Run"] }));
        assert_eq!(merge_stats(Some(&known), &thumb).clip_names, Some(vec!["Run".to_owned()]));
    }

    #[test]
    fn a_new_measurement_keeps_the_card_versions_it_lacks() {
        let kept = summary("thumb", json!({ "fxThumb": 4, "animThumb": 2 }));
        let viewer = summary("viewer", json!({ "animThumb": 3, "triangles": 7 }));
        let merged = merge_stats(Some(&kept), &viewer);
        assert_eq!((merged.triangles, merged.fx_thumb, merged.anim_thumb), (7.0, Some(4.0), Some(3.0)));
        assert_eq!(merge_stats(None, &viewer), viewer);
        // A thumbnail measurement over a thumbnail one replaces it.
        let again = summary("thumb", json!({ "triangles": 2 }));
        assert_eq!(merge_stats(Some(&kept), &again).triangles, 2.0);
    }

    #[tokio::test]
    async fn saves_reads_back_and_writes_the_file_on_flush() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("sha1", &summary("viewer", json!({}))).await;
        store.save("bad", &summary("viewer", json!({ "bones": -2 }))).await;
        assert_eq!(store.get_many(&["sha1".into(), "bad".into(), "none".into()]).await.keys().collect::<Vec<_>>(), ["sha1"]);
        assert!(!dir.path().join("stats.json").exists(), "the write waits for its delay");
        store.flush().await;
        let again = StatsStore::new(CacheConfig::new(dir.path().to_owned(), 0));
        assert_eq!(again.get_many(&["sha1".into()]).await["sha1"], summary("viewer", json!({})));
    }

    #[tokio::test]
    async fn saving_answers_what_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        let viewer = store.save("fish", &summary("viewer", json!({ "triangles": 999 }))).await.unwrap();
        assert_eq!(viewer.triangles, 999.0);
        // A thumbnail's reading comes back merged into the viewer's, not replacing it.
        let after = store.save("fish", &summary("thumb", json!({ "triangles": 1, "clipNames": ["Walk"] }))).await.unwrap();
        assert_eq!((after.triangles, after.source), (999.0, StatsSource::Viewer));
        assert_eq!(after.clip_names, Some(vec!["Walk".to_owned()]));
        // An invalid reading is ignored.
        assert!(store.save("fish", &summary("viewer", json!({ "triangles": -1 }))).await.is_none());
    }

    #[tokio::test]
    async fn reading_a_file_with_bad_entries_keeps_the_good_ones() {
        let dir = tempfile::tempdir().unwrap();
        let good = serde_json::to_value(summary("thumb", json!({}))).unwrap();
        let file = json!({ "good": good, "negative": { "triangles": -1 }, "junk": 5 });
        std::fs::write(dir.path().join("stats.json"), file.to_string()).unwrap();
        assert_eq!(store(dir.path()).all().await.keys().collect::<Vec<_>>(), ["good"]);
        std::fs::write(dir.path().join("stats.json"), "not json").unwrap();
        assert!(store(dir.path()).all().await.is_empty());
    }

    #[tokio::test]
    async fn takes_measurements_from_a_shared_index_where_none_is_known_or_where_fuller() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path());
        store.save("have-viewer", &summary("viewer", json!({ "triangles": 1 }))).await;
        store.save("have-thumb", &summary("thumb", json!({ "triangles": 2 }))).await;
        let v = |t: u32| serde_json::to_value(summary("viewer", json!({ "triangles": t }))).unwrap();
        let th = |t: u32| serde_json::to_value(summary("thumb", json!({ "triangles": t }))).unwrap();
        let taken = store.import_many(&json!({ "new": th(9), "have-viewer": v(8), "have-thumb": v(7), "bad": { "x": 1 } })).await;
        assert_eq!(taken, 2);
        let all = store.all().await;
        assert_eq!((all["new"].triangles, all["have-viewer"].triangles, all["have-thumb"].triangles), (9.0, 1.0, 7.0));
        assert_eq!(store.import_many(&json!("nope")).await, 0);
    }
}
