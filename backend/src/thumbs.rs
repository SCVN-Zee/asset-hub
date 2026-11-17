//! The thumbnail routes: the index (`GET /thumbs`) and the packs (`POST /thumbs/get` and
//! `/thumbs/put`). The index lives in the main D1 database and the bytes in the shards of
//! `shards.rs`. Nothing here talks to B2.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap, HashSet};

use futures_util::stream::{self, StreamExt};
use js_sys::{Array, Uint8Array};
use serde::Deserialize;
use serde_json::Value;
use wasm_bindgen::JsValue;
use worker::console_error;

use crate::ctx::RepoCtx;
use crate::http::{decode_text, json, quote, read_body_small, read_schema, stop, Halt, Handled};
use crate::js::{fmt_number, js_number, random_uuids};
use crate::shards::{shard_of, D1Thumbs, D1_MAX_PARAMS, GENERATION_SQL};
use crate::validate::{is_thumb_name, parse_cursor};

const MAX_LIST: f64 = 1000.0;
const MAX_GET_NAMES: usize = 40;
const MAX_PUT_ENTRIES: usize = 20;
const MAX_PUT_BYTES: usize = 3 * 1024 * 1024;
/// The most bytes of one thumbnail, shared with the desktop client.
pub use asset_hub_pack::MAX_THUMB_BYTES;
/// The generation of the thumbnail storage, sent in the list reply. A client that sees another
/// number than the one it synced with starts its thumbnail state again. The TypeScript Worker
/// (thumbnails in B2) sends none, and clients read that as 1. Epoch 3 marks the recreated shard
/// databases, which started empty, so every client must drop the thumbnails it believes are stored.
pub const THUMB_EPOCH: u32 = 3;
const MAX_GET_BODY_BYTES: usize = 8 * 1024;
/// Shard queries in flight at once. D1 allows six open connections for one request.
const SHARD_CONCURRENCY: usize = 6;
/// The reply cap of the Free plan is far above a full GET, so no runtime check is needed.
const MAX_REPLY_BYTES: usize = 48 * 1024 * 1024;
const _: () = assert!(MAX_GET_NAMES * MAX_THUMB_BYTES <= MAX_REPLY_BYTES);
/// The body cap bounds the BLOB conversion of one request, and a full pack of maximum entries
/// still fits under it with room for the entry headers.
const _: () = assert!(MAX_PUT_ENTRIES * MAX_THUMB_BYTES + 64 * 1024 <= MAX_PUT_BYTES);
/// The commit reads the previous rows with one `IN` list of every name, plus the repo and schema.
const _: () = assert!(MAX_PUT_ENTRIES + 2 <= D1_MAX_PARAMS);

#[derive(Debug, Deserialize, PartialEq)]
pub struct ThumbRow {
    pub name: String,
    pub size: f64,
    pub rev: f64,
}

/// The page size from `?limit=`: absent means the maximum, otherwise `Number(text)` must be an
/// integer from 1 to 1000.
pub fn parse_limit(raw: Option<&str>) -> Option<usize> {
    let limit = raw.map_or(MAX_LIST, js_number);
    (limit.fract() == 0.0 && (1.0..=MAX_LIST).contains(&limit)).then_some(limit as usize)
}

/// `{"items":[{"name":...,"size":...,"rev":...}],"next":"<rev>:<name>","epoch":3}`. One extra row
/// tells whether another page follows.
pub fn list_body(rows: &[ThumbRow], limit: usize) -> String {
    let items = &rows[..rows.len().min(limit)];
    let parts: Vec<String> =
        items.iter().map(|r| format!("{{\"name\":{},\"size\":{},\"rev\":{}}}", quote(&r.name), fmt_number(r.size), fmt_number(r.rev))).collect();
    let next = match items.last() {
        Some(last) if rows.len() > limit => format!(",\"next\":{}", quote(&format!("{}:{}", fmt_number(last.rev), last.name))),
        _ => String::new(),
    };
    format!("{{\"items\":[{}]{next},\"epoch\":{THUMB_EPOCH}}}", parts.join(","))
}

/// `GET /thumbs?after=<rev>:<name>&limit=1000`: the index, ordered by `(rev, name)`. `rev` counts
/// commits per repo and schema, so a replaced thumbnail always gets a higher value.
pub async fn list_thumbs(ctx: &RepoCtx<'_>) -> Handled<web_sys::Response> {
    let schema = read_schema(ctx.req)?;

    let url = ctx.req.url()?;
    let param = |name: &str| url.query_pairs().find(|(key, _)| key == name).map(|(_, value)| value.into_owned());
    let Some(limit) = parse_limit(param("limit").as_deref()) else {
        return Err(stop("badRequest", 400));
    };
    let after = param("after");
    let cursor = match after.as_deref() {
        None => None,
        Some(text) => Some(parse_cursor(text).ok_or_else(|| stop("badRequest", 400))?),
    };

    let db = ctx.db()?;
    // Rows with another id than a generation of this Worker name bytes that no shard holds.
    let base = format!("SELECT name, size, rev FROM thumbs WHERE repo = ? AND schema = ? AND {GENERATION_SQL}");
    let order = " ORDER BY rev, name LIMIT ?";
    let fetch_rows = limit as f64 + 1.0;
    let statement = match cursor {
        Some((rev, name)) => db.prepare(format!("{base} AND (rev, name) > (?, ?){order}")).bind(&[
            JsValue::from_str(ctx.slug),
            JsValue::from_f64(f64::from(schema)),
            JsValue::from_f64(js_number(rev)),
            JsValue::from_str(name),
            JsValue::from_f64(fetch_rows),
        ])?,
        None => db.prepare(format!("{base}{order}")).bind(&[JsValue::from_str(ctx.slug), JsValue::from_f64(f64::from(schema)), JsValue::from_f64(fetch_rows)])?,
    };
    let rows: Vec<ThumbRow> = statement.all().await?.results()?;
    Ok(json(list_body(&rows, limit), 200, &[])?)
}

/// A failed store step. The text is a label, or the start of a D1 error message. It holds no bytes
/// and no secret: statements bind every value.
#[derive(Debug, PartialEq, Eq)]
pub struct ThumbErr(String);

impl ThumbErr {
    pub fn new(label: &str) -> Self {
        Self(label.to_owned())
    }

    pub fn label(&self) -> &str {
        &self.0
    }
}

/// Characters of a D1 error message that go to the log.
const ERROR_TEXT_CHARS: usize = 160;

impl From<worker::Error> for ThumbErr {
    fn from(err: worker::Error) -> Self {
        Self(err.to_string().chars().map(|c| if c.is_control() { ' ' } else { c }).take(ERROR_TEXT_CHARS).collect())
    }
}

impl From<ThumbErr> for Halt {
    fn from(err: ThumbErr) -> Self {
        Halt::Failed(err.0)
    }
}

/// A generation of a thumbnail: the name and the id of one immutable shard row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenRef {
    pub name: String,
    pub gen: String,
}

/// A shard row to insert.
pub struct NewGen<'a> {
    pub name: &'a str,
    pub gen: &'a str,
    pub bytes: &'a [u8],
}

/// An index row to commit: the generation that readers get from now on.
pub struct IndexEntry<'a> {
    pub name: &'a str,
    pub size: u64,
    pub gen: &'a str,
}

/// The steps of a PUT that touch D1. A real store wraps the main database and the shards. A test
/// store can fail any step before or after its effect, which black-box tests cannot cause.
pub trait ThumbStore {
    /// Charges `bytes` to a shard. `false` means the shard would pass its cap.
    async fn reserve_shard(&self, shard: usize, bytes: u64) -> Result<bool, ThumbErr>;
    /// Gives back a reservation of `bytes` and removes `gens` when they exist. The charge was
    /// `bytes` whether the rows landed or not, so this is exact in both cases.
    async fn release_shard(&self, shard: usize, bytes: u64, gens: &[GenRef]) -> Result<(), ThumbErr>;
    /// Inserts the rows of one shard in one batch.
    async fn insert_gens(&self, shard: usize, rows: &[NewGen<'_>]) -> Result<(), ThumbErr>;
    /// Commits the index in one batch and gives the generations it replaced.
    async fn commit_index(&self, entries: &[IndexEntry<'_>]) -> Result<Vec<GenRef>, ThumbErr>;
    /// The generations the index names for `names`.
    async fn indexed_gens(&self, names: &[String]) -> Result<Vec<GenRef>, ThumbErr>;
    /// Deletes rows that the index does not name and lowers the counter by the bytes it removed.
    async fn delete_gens(&self, shard: usize, gens: &[GenRef]) -> Result<(), ThumbErr>;
}

/// A decoded pack entry. In the Worker both fields borrow from the request body, so decoding
/// copies nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<'a> {
    pub name: Cow<'a, str>,
    pub bytes: Cow<'a, [u8]>,
}

/// A name sent twice keeps its last bytes at the place of its first appearance, as a JavaScript
/// `Map` does. Storing both would leave a generation nothing points at.
pub fn dedupe<'a>(decoded: Vec<(&'a str, &'a [u8])>) -> Vec<Entry<'a>> {
    let mut at: HashMap<&'a str, usize> = HashMap::new();
    let mut entries: Vec<Entry<'a>> = Vec::with_capacity(decoded.len());
    for (name, bytes) in decoded {
        match at.get(name) {
            Some(&i) => entries[i].bytes = Cow::Borrowed(bytes),
            None => {
                at.insert(name, entries.len());
                entries.push(Entry { name: Cow::Borrowed(name), bytes: Cow::Borrowed(bytes) });
            }
        }
    }
    entries
}

/// The positions of `names` for each shard, in shard order. A name with no shard is an error, so it
/// never lands in shard 0 by accident.
fn by_shard<'a>(names: impl Iterator<Item = &'a str>) -> Result<Vec<(usize, Vec<usize>)>, ThumbErr> {
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (i, name) in names.enumerate() {
        groups.entry(shard_of(name).ok_or_else(|| ThumbErr::new("badName"))?).or_default().push(i);
    }
    Ok(groups.into_iter().collect())
}

#[cfg(target_arch = "wasm32")]
fn log_error(label: &str) {
    console_error!("{label}");
}

#[cfg(not(target_arch = "wasm32"))]
fn log_error(_label: &str) {}

/// Runs `jobs` with at most `SHARD_CONCURRENCY` in flight and gives the outputs in order.
async fn limited<F: std::future::Future>(jobs: impl Iterator<Item = F>) -> Vec<F::Output> {
    stream::iter(jobs).buffered(SHARD_CONCURRENCY).collect().await
}

#[derive(Debug, PartialEq, Eq)]
pub enum PutOutcome {
    Stored(usize),
    StorageFull,
    /// A shard step failed. Everything this request wrote is undone as far as D1 allows.
    ShardFailed,
}

/// Deletes the rows of `gens` that no index row names, shard by shard. A failure is logged and
/// leaves the bytes charged, so the cap stays honest, and the sweep of `sweep.rs` reclaims them
/// later. Never fails.
pub async fn delete_unnamed<S: ThumbStore>(store: &S, gens: &[GenRef]) {
    let Ok(groups) = by_shard(gens.iter().map(|g| g.name.as_str())) else {
        log_error("thumbs.deleteFailed");
        return;
    };
    let outcomes = limited(groups.iter().map(|(shard, at)| {
        let rows: Vec<GenRef> = at.iter().filter_map(|&i| gens.get(i).cloned()).collect();
        async move { store.delete_gens(*shard, &rows).await }
    }))
    .await;
    for outcome in outcomes {
        if outcome.is_err() {
            log_error("thumbs.deleteFailed");
        }
    }
}

/// A PUT after its body checks: reserve per shard, insert the new generations, commit the index.
/// `gens[i]` is the new generation of `entries[i]`. The generations the commit replaced come back
/// with the outcome. The caller deletes them with `delete_unnamed`, after the reply if it can.
pub async fn stage_thumbs<S: ThumbStore>(store: &S, entries: &[Entry<'_>], gens: &[String]) -> Result<(PutOutcome, Vec<GenRef>), ThumbErr> {
    let groups = by_shard(entries.iter().map(|e| e.name.as_ref()))?;
    let bytes_of = |at: &[usize]| -> u64 { at.iter().filter_map(|&i| entries.get(i)).map(|e| e.bytes.len() as u64).sum() };
    let refs_of = |at: &[usize]| -> Vec<GenRef> {
        at.iter().filter_map(|&i| Some(GenRef { name: entries.get(i)?.name.to_string(), gen: gens.get(i)?.clone() })).collect()
    };

    // Reserve. A refused or failed reservation gives back the ones that held. A reservation that
    // failed with an unknown outcome stays charged.
    let reserved = limited(groups.iter().map(|(shard, at)| store.reserve_shard(*shard, bytes_of(at)))).await;
    if reserved.iter().any(|r| !matches!(r, Ok(true))) {
        let held: Vec<&(usize, Vec<usize>)> = groups.iter().zip(&reserved).filter(|(_, r)| matches!(r, Ok(true))).map(|(g, _)| g).collect();
        let freed = limited(held.iter().map(|(shard, at)| store.release_shard(*shard, bytes_of(at), &[]))).await;
        if freed.iter().any(Result::is_err) {
            log_error("thumbs.releaseFailed");
        }
        return Ok((if reserved.iter().any(Result::is_err) { PutOutcome::ShardFailed } else { PutOutcome::StorageFull }, Vec::new()));
    }

    // Insert. Rows are immutable and unnamed until the index commits.
    let inserted = limited(groups.iter().map(|(shard, at)| {
        let rows: Vec<NewGen<'_>> = at
            .iter()
            .filter_map(|&i| Some(NewGen { name: entries.get(i)?.name.as_ref(), gen: gens.get(i)?.as_str(), bytes: entries.get(i)?.bytes.as_ref() }))
            .collect();
        async move { store.insert_gens(*shard, &rows).await }
    }))
    .await;
    if inserted.iter().any(Result::is_err) {
        // A shard that inserted gives its rows back through the delete. A shard that failed may or
        // may not have landed, and its charge was the reservation either way.
        let undone = limited(groups.iter().zip(&inserted).map(|((shard, at), outcome)| {
            let rows = refs_of(at);
            let bytes = bytes_of(at);
            let landed = outcome.is_ok();
            async move {
                if landed {
                    store.delete_gens(*shard, &rows).await
                } else {
                    store.release_shard(*shard, bytes, &rows).await
                }
            }
        }))
        .await;
        if undone.iter().any(Result::is_err) {
            log_error("thumbs.undoFailed");
        }
        return Ok((PutOutcome::ShardFailed, Vec::new()));
    }

    // Commit.
    let index: Vec<IndexEntry<'_>> = entries
        .iter()
        .zip(gens)
        .map(|(e, gen)| IndexEntry { name: e.name.as_ref(), size: e.bytes.len() as u64, gen: gen.as_str() })
        .collect();
    match store.commit_index(&index).await {
        Ok(replaced) => Ok((PutOutcome::Stored(entries.len()), replaced)),
        Err(err) => {
            // The batch failed, but it may have committed. Delete only generations the index does
            // not name. If the index cannot be read, delete nothing and keep the bytes charged.
            let names: Vec<String> = entries.iter().map(|e| e.name.to_string()).collect();
            if let Ok(named) = store.indexed_gens(&names).await {
                let orphans: Vec<GenRef> =
                    entries.iter().zip(gens).map(|(e, gen)| GenRef { name: e.name.to_string(), gen: gen.clone() }).filter(|g| !named.contains(g)).collect();
                delete_unnamed(store, &orphans).await;
            }
            Err(err)
        }
    }
}

/// The names of a GET body `{"names":[...]}`: at most 40 thumbnail names. A name sent twice counts
/// once, in the place of its first appearance. `None` means a bad request.
pub fn parse_get_names(body: &str) -> Option<Vec<String>> {
    let parsed: Value = serde_json::from_str(body).ok()?;
    let list = parsed.get("names")?.as_array()?;
    if list.len() > MAX_GET_NAMES {
        return None;
    }
    let mut seen = HashSet::new();
    let mut names = Vec::with_capacity(list.len());
    for item in list {
        let name = item.as_str().filter(|n| is_thumb_name(n))?;
        if seen.insert(name) {
            names.push(name.to_owned());
        }
    }
    Some(names)
}

/// `POST /thumbs/get` `{ names }`: a binary pack of the thumbnails that exist, in request order. The
/// index decides which generation is served, so a stray newer row is never returned. A shard error
/// gives 503 and no bytes. The pack is a `Blob` of header and data parts built in JavaScript.
pub async fn get_thumbs(ctx: &RepoCtx<'_>) -> Handled<web_sys::Response> {
    let schema = read_schema(ctx.req)?;
    let Some(body) = read_body_small(ctx.req, MAX_GET_BODY_BYTES).await? else {
        return Err(stop("badRequest", 400));
    };
    let Some(wanted) = parse_get_names(&decode_text(&body)) else {
        return Err(stop("badRequest", 400));
    };

    let store = D1Thumbs::new(ctx, schema)?;
    let current: HashMap<String, String> = store.indexed_gens(&wanted).await?.into_iter().map(|g| (g.name, g.gen)).collect();
    // Names with no index row are left out.
    let present: Vec<GenRef> = wanted.iter().filter_map(|name| Some(GenRef { name: name.clone(), gen: current.get(name)?.clone() })).collect();

    let groups = by_shard(present.iter().map(|g| g.name.as_str()))?;
    let reads = limited(groups.iter().map(|(shard, at)| {
        let rows: Vec<GenRef> = at.iter().filter_map(|&i| present.get(i).cloned()).collect();
        let store = &store;
        async move { store.read_blobs(*shard, &rows).await }
    }))
    .await;
    let mut blobs: HashMap<String, Vec<u8>> = HashMap::new();
    for read in reads {
        match read {
            Ok(rows) => blobs.extend(rows),
            Err(err) => {
                console_error!("thumbs.read {}", err.label());
                return Err(stop("storageUnavailable", 503));
            }
        }
    }

    // One buffer holds the whole pack, so Wasm hands JavaScript a single copy.
    let mut pack: Vec<u8> = Vec::with_capacity(blobs.values().map(Vec::len).sum::<usize>() + present.len() * 64);
    for g in &present {
        let Some(data) = blobs.get(&g.name) else { continue };
        let length = u32::try_from(data.len()).map_err(|e| Halt::failed(e.to_string()))?;
        pack.extend_from_slice(&asset_hub_pack::encode_entry_header(&g.name, length).map_err(Halt::failed)?);
        pack.extend_from_slice(data);
    }
    let parts = Array::of1(&Uint8Array::from(pack.as_slice()));
    Ok(pack_reply(&parts)?)
}

fn pack_reply(parts: &Array) -> Result<web_sys::Response, JsValue> {
    let blob = web_sys::Blob::new_with_buffer_source_sequence(parts)?;
    let headers = web_sys::Headers::new()?;
    headers.set("content-type", "application/octet-stream")?;
    headers.set("cache-control", "private, no-store")?;
    let init = web_sys::ResponseInit::new();
    init.set_status(200);
    init.set_headers(&headers);
    web_sys::Response::new_with_opt_blob_and_init(Some(&blob), &init)
}

/// `POST /thumbs/put` with a binary pack: needs push access. Reserves shard bytes, inserts new
/// generations, commits the D1 index in one batch, then deletes the generations the batch replaced.
pub async fn put_thumbs(ctx: &RepoCtx<'_>) -> Handled<web_sys::Response> {
    let schema = read_schema(ctx.req)?;
    if !ctx.caller.can_push {
        return Err(stop("readOnly", 403));
    }
    let Some(body) = read_body_small(ctx.req, MAX_PUT_BYTES).await? else {
        return Err(stop("tooLarge", 413));
    };
    let Ok(decoded) = asset_hub_pack::decode_pack_ref(&body) else {
        return Err(stop("badPack", 400));
    };
    if decoded.is_empty() || decoded.len() > MAX_PUT_ENTRIES || !decoded.iter().all(|(name, _)| is_thumb_name(name)) {
        return Err(stop("badRequest", 400));
    }
    if decoded.iter().any(|(_, bytes)| bytes.len() > MAX_THUMB_BYTES) {
        return Err(stop("tooLarge", 413));
    }
    let entries = dedupe(decoded);
    let gens = random_uuids(entries.len())?;

    let store = D1Thumbs::new(ctx, schema)?;
    match stage_thumbs(&store, &entries, &gens).await? {
        (PutOutcome::Stored(count), replaced) => {
            // The replaced generations go after the reply, and a client that disconnects does not
            // cancel them. What a killed invocation still leaves behind, the sweep reclaims.
            ctx.exec.wait_until(async move { delete_unnamed(&store, &replaced).await });
            Ok(json(format!("{{\"stored\":{count}}}"), 200, &[])?)
        }
        (PutOutcome::StorageFull, _) => Err(stop("storageFull", 507)),
        (PutOutcome::ShardFailed, _) => Err(stop("storageUnavailable", 503)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, size: f64, rev: f64) -> ThumbRow {
        ThumbRow { name: name.into(), size, rev }
    }

    #[test]
    fn limit_follows_javascript_number_rules() {
        assert_eq!(parse_limit(None), Some(1000));
        assert_eq!(parse_limit(Some("10")), Some(10));
        assert_eq!(parse_limit(Some(" 10 ")), Some(10));
        assert_eq!(parse_limit(Some("1")), Some(1));
        assert_eq!(parse_limit(Some("1000")), Some(1000));
        assert_eq!(parse_limit(Some("1.0")), Some(1));
        assert_eq!(parse_limit(Some("1e2")), Some(100));
        assert_eq!(parse_limit(Some("0x10")), Some(16));
        assert_eq!(parse_limit(Some("0b11")), Some(3));
        for bad in ["", "0", "1001", "-1", "1.5", "abc", "NaN", "Infinity", "1e4", "0x", "10px"] {
            assert_eq!(parse_limit(Some(bad)), None, "{bad:?}");
        }
    }

    #[test]
    fn a_full_page_names_the_next_cursor() {
        let rows = vec![row("a", 4.0, 4.0), row("b", 4.0, 5.0)];
        assert_eq!(list_body(&rows, 1), r#"{"items":[{"name":"a","size":4,"rev":4}],"next":"4:a","epoch":3}"#);
    }

    #[test]
    fn the_last_page_has_no_next() {
        let rows = vec![row("a", 4.0, 4.0), row("b", 4.0, 5.0)];
        assert_eq!(list_body(&rows, 2), r#"{"items":[{"name":"a","size":4,"rev":4},{"name":"b","size":4,"rev":5}],"epoch":3}"#);
        assert_eq!(list_body(&rows, 1000), list_body(&rows, 2));
        assert_eq!(list_body(&[], 1000), r#"{"items":[],"epoch":3}"#);
    }

    use crate::shards::{select_params, SHARDS, SHARD_CAP_BYTES};
    use futures_executor::block_on;
    use futures_util::future::join;
    use std::cell::{Cell, RefCell};
    use std::future::Future;
    use std::pin::Pin;
    use std::task::{Context, Poll};

    /// A future that is pending once, so two writers can interleave.
    struct YieldOnce(bool);

    impl Future for YieldOnce {
        type Output = ();

        fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            if self.0 {
                return Poll::Ready(());
            }
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        /// The call fails and had no effect.
        Before,
        /// The call changed the data and then failed.
        After,
    }

    #[derive(Default)]
    struct Shard {
        usage: u64,
        /// `(name, gen)` to size.
        blobs: BTreeMap<(String, String), u64>,
    }

    /// The main index and eight shards, in memory.
    struct Fake {
        shards: RefCell<Vec<Shard>>,
        /// name to `(gen, size)`.
        index: RefCell<BTreeMap<String, (String, u64)>>,
        reserve_fault: Cell<Option<(usize, Fault)>>,
        refuse_shard: Cell<Option<usize>>,
        insert_fault: Cell<Option<(usize, Fault)>>,
        commit_fault: Cell<Fault>,
        reread_fails: Cell<bool>,
        interleave: bool,
        /// Another writer's row that lands in the index right before a commit fails.
        rival: RefCell<Option<(String, String, u64)>>,
        deletes: Cell<u32>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                shards: RefCell::new((0..SHARDS).map(|_| Shard::default()).collect()),
                index: RefCell::new(BTreeMap::new()),
                reserve_fault: Cell::new(None),
                refuse_shard: Cell::new(None),
                insert_fault: Cell::new(None),
                commit_fault: Cell::new(Fault::None),
                reread_fails: Cell::new(false),
                interleave: false,
                rival: RefCell::new(None),
                deletes: Cell::new(0),
            }
        }

        fn total_usage(&self) -> u64 {
            self.shards.borrow().iter().map(|s| s.usage).sum()
        }

        fn stored(&self) -> Vec<(String, String)> {
            self.shards.borrow().iter().flat_map(|s| s.blobs.keys().cloned()).collect()
        }

        fn stored_bytes(&self) -> u64 {
            self.shards.borrow().iter().flat_map(|s| s.blobs.values().copied()).sum()
        }

        fn named(&self, name: &str) -> Option<String> {
            self.index.borrow().get(name).map(|(gen, _)| gen.clone())
        }

        /// An earlier PUT: one stored row that the index names.
        fn seed(&self, name: &str, gen: &str, size: u64) {
            let mut shards = self.shards.borrow_mut();
            if let Some(shard) = shard_of(name).and_then(|at| shards.get_mut(at)) {
                shard.usage += size;
                shard.blobs.insert((name.to_owned(), gen.to_owned()), size);
            }
            self.index.borrow_mut().insert(name.to_owned(), (gen.to_owned(), size));
        }

        fn apply_insert(&self, shard: usize, rows: &[NewGen<'_>]) {
            if let Some(s) = self.shards.borrow_mut().get_mut(shard) {
                for r in rows {
                    s.blobs.insert((r.name.to_owned(), r.gen.to_owned()), r.bytes.len() as u64);
                }
            }
        }
    }

    impl ThumbStore for Fake {
        async fn reserve_shard(&self, shard: usize, bytes: u64) -> Result<bool, ThumbErr> {
            if let Some((at, fault)) = self.reserve_fault.get() {
                if at == shard && fault != Fault::None {
                    return Err(ThumbErr::new("Error"));
                }
            }
            if self.refuse_shard.get() == Some(shard) {
                return Ok(false);
            }
            let mut shards = self.shards.borrow_mut();
            let Some(s) = shards.get_mut(shard) else { return Err(ThumbErr::new("noShard")) };
            if s.usage + bytes > SHARD_CAP_BYTES {
                return Ok(false);
            }
            s.usage += bytes;
            Ok(true)
        }

        async fn release_shard(&self, shard: usize, bytes: u64, gens: &[GenRef]) -> Result<(), ThumbErr> {
            if let Some(s) = self.shards.borrow_mut().get_mut(shard) {
                s.usage = s.usage.saturating_sub(bytes);
                for g in gens {
                    s.blobs.remove(&(g.name.clone(), g.gen.clone()));
                }
            }
            Ok(())
        }

        async fn insert_gens(&self, shard: usize, rows: &[NewGen<'_>]) -> Result<(), ThumbErr> {
            match self.insert_fault.get() {
                Some((at, Fault::Before)) if at == shard => Err(ThumbErr::new("Error")),
                Some((at, Fault::After)) if at == shard => {
                    self.apply_insert(shard, rows);
                    Err(ThumbErr::new("Error"))
                }
                _ => {
                    self.apply_insert(shard, rows);
                    Ok(())
                }
            }
        }

        async fn commit_index(&self, entries: &[IndexEntry<'_>]) -> Result<Vec<GenRef>, ThumbErr> {
            if self.interleave {
                YieldOnce(false).await;
            }
            if self.commit_fault.get() == Fault::Before {
                if let Some((name, gen, size)) = self.rival.borrow_mut().take() {
                    self.seed(&name, &gen, size);
                }
                return Err(ThumbErr::new("Error"));
            }
            let mut replaced = Vec::new();
            let mut index = self.index.borrow_mut();
            for e in entries {
                if let Some((gen, _)) = index.insert(e.name.to_owned(), (e.gen.to_owned(), e.size)) {
                    replaced.push(GenRef { name: e.name.to_owned(), gen });
                }
            }
            drop(index);
            if self.commit_fault.get() == Fault::After {
                return Err(ThumbErr::new("Error"));
            }
            Ok(replaced)
        }

        async fn indexed_gens(&self, names: &[String]) -> Result<Vec<GenRef>, ThumbErr> {
            if self.reread_fails.get() {
                return Err(ThumbErr::new("Error"));
            }
            let index = self.index.borrow();
            Ok(names.iter().filter_map(|n| Some(GenRef { name: n.clone(), gen: index.get(n)?.0.clone() })).collect())
        }

        async fn delete_gens(&self, shard: usize, gens: &[GenRef]) -> Result<(), ThumbErr> {
            self.deletes.set(self.deletes.get() + 1);
            if let Some(s) = self.shards.borrow_mut().get_mut(shard) {
                let freed: u64 = gens.iter().filter_map(|g| s.blobs.remove(&(g.name.clone(), g.gen.clone()))).sum();
                s.usage = s.usage.saturating_sub(freed);
            }
            Ok(())
        }
    }

    fn name(n: usize) -> String {
        format!("{n:040x}")
    }

    /// An entry of 4 bytes.
    fn entry(n: usize) -> Entry<'static> {
        Entry { name: name(n).into(), bytes: vec![n as u8; 4].into() }
    }

    /// A name that starts with the hex digit `digit`.
    fn in_shard(digit: usize, n: usize) -> String {
        format!("{digit:x}{n:039x}")
    }

    fn gens_for(entries: &[Entry<'_>], tag: &str) -> Vec<String> {
        (0..entries.len()).map(|i| format!("{tag}{i}")).collect()
    }

    /// A whole PUT as the route runs it: the replaced generations are deleted after the commit.
    async fn store_thumbs<S: ThumbStore>(store: &S, entries: &[Entry<'_>], gens: &[String]) -> Result<PutOutcome, ThumbErr> {
        let (outcome, replaced) = stage_thumbs(store, entries, gens).await?;
        delete_unnamed(store, &replaced).await;
        Ok(outcome)
    }

    fn put(fake: &Fake, entries: &[Entry<'_>], tag: &str) -> Result<PutOutcome, ThumbErr> {
        block_on(store_thumbs(fake, entries, &gens_for(entries, tag)))
    }

    #[test]
    fn a_commit_that_threw_but_landed_keeps_the_live_generations_and_their_bytes() {
        let fake = Fake::new();
        fake.commit_fault.set(Fault::After);
        let entries = [entry(1), entry(2)];
        assert_eq!(put(&fake, &entries, "g"), Err(ThumbErr::new("Error")));
        assert_eq!(fake.stored().len(), 2);
        assert_eq!(fake.total_usage(), 8);
        assert_eq!(fake.deletes.get(), 0);
        assert_eq!(fake.named(&name(1)), Some("g0".to_owned()));
    }

    #[test]
    fn a_commit_that_threw_and_did_not_land_deletes_the_new_generations_and_releases_their_bytes() {
        let fake = Fake::new();
        fake.commit_fault.set(Fault::Before);
        assert_eq!(put(&fake, &[entry(1), entry(2)], "g"), Err(ThumbErr::new("Error")));
        assert!(fake.stored().is_empty());
        assert_eq!(fake.total_usage(), 0);
    }

    #[test]
    fn when_the_commit_outcome_cannot_be_read_nothing_is_deleted_and_the_bytes_stay_charged() {
        let fake = Fake::new();
        fake.commit_fault.set(Fault::After);
        fake.reread_fails.set(true);
        assert_eq!(put(&fake, &[entry(1), entry(2)], "g"), Err(ThumbErr::new("Error")));
        assert_eq!(fake.stored().len(), 2);
        assert_eq!(fake.deletes.get(), 0);
        assert_eq!(fake.total_usage(), 8);
    }

    #[test]
    fn a_commit_failure_deletes_only_the_generations_the_index_does_not_name() {
        let fake = Fake::new();
        fake.commit_fault.set(Fault::Before);
        // A rival writer commits name 1 just before this writer's commit fails.
        *fake.rival.borrow_mut() = Some((name(1), "rival".to_owned(), 6));
        assert_eq!(put(&fake, &[entry(1), entry(2)], "g"), Err(ThumbErr::new("Error")));
        assert_eq!(fake.stored(), vec![(name(1), "rival".to_owned())]);
        assert_eq!(fake.total_usage(), 6);
        assert_eq!(fake.named(&name(1)), Some("rival".to_owned()));
        assert_eq!(fake.named(&name(2)), None);
    }

    #[test]
    fn a_replace_deletes_the_old_generation_and_lowers_the_usage() {
        let fake = Fake::new();
        fake.seed(&name(1), "old", 10);
        assert_eq!(put(&fake, &[entry(1)], "new"), Ok(PutOutcome::Stored(1)));
        assert_eq!(fake.stored(), vec![(name(1), "new0".to_owned())]);
        assert_eq!(fake.total_usage(), 4);
        assert_eq!(fake.stored_bytes(), fake.total_usage());
    }

    #[test]
    fn a_refused_shard_gives_back_the_shards_that_reserved() {
        let fake = Fake::new();
        fake.refuse_shard.set(Some(3));
        let entries: Vec<Entry> = (0..8).map(|d| Entry { name: in_shard(d, 1).into(), bytes: vec![1; 4].into() }).collect();
        assert_eq!(put(&fake, &entries, "g"), Ok(PutOutcome::StorageFull));
        assert_eq!(fake.total_usage(), 0);
        assert!(fake.stored().is_empty());
        assert!(fake.index.borrow().is_empty());
    }

    #[test]
    fn a_shard_at_its_cap_refuses_and_the_others_are_given_back() {
        let fake = Fake::new();
        if let Some(s) = fake.shards.borrow_mut().get_mut(5) {
            s.usage = SHARD_CAP_BYTES - 3;
        }
        let entries: Vec<Entry> = (0..8).map(|d| Entry { name: in_shard(d, 1).into(), bytes: vec![1; 4].into() }).collect();
        assert_eq!(put(&fake, &entries, "g"), Ok(PutOutcome::StorageFull));
        assert_eq!(fake.total_usage(), SHARD_CAP_BYTES - 3);
        assert_eq!(put(&fake, &entries[..1], "h"), Ok(PutOutcome::Stored(1)));
    }

    #[test]
    fn a_failed_reservation_gives_back_the_shards_that_reserved_and_answers_unavailable() {
        let fake = Fake::new();
        fake.reserve_fault.set(Some((3, Fault::Before)));
        let entries: Vec<Entry> = (0..8).map(|d| Entry { name: in_shard(d, 1).into(), bytes: vec![1; 4].into() }).collect();
        assert_eq!(put(&fake, &entries, "g"), Ok(PutOutcome::ShardFailed));
        assert_eq!(fake.total_usage(), 0);
        assert!(fake.stored().is_empty());
    }

    #[test]
    fn a_failed_insert_undoes_the_shards_that_succeeded() {
        let fake = Fake::new();
        fake.insert_fault.set(Some((4, Fault::Before)));
        let entries: Vec<Entry> = (0..8).map(|d| Entry { name: in_shard(d, 1).into(), bytes: vec![1; 4].into() }).collect();
        assert_eq!(put(&fake, &entries, "g"), Ok(PutOutcome::ShardFailed));
        assert!(fake.stored().is_empty());
        assert_eq!(fake.total_usage(), 0);
        assert!(fake.index.borrow().is_empty());
    }

    #[test]
    fn an_insert_that_threw_but_landed_is_undone_too() {
        let fake = Fake::new();
        fake.insert_fault.set(Some((4, Fault::After)));
        let entries: Vec<Entry> = (0..8).map(|d| Entry { name: in_shard(d, 1).into(), bytes: vec![1; 4].into() }).collect();
        assert_eq!(put(&fake, &entries, "g"), Ok(PutOutcome::ShardFailed));
        assert!(fake.stored().is_empty());
        assert_eq!(fake.total_usage(), 0);
    }

    #[test]
    fn two_writers_of_one_name_leave_exactly_one_generation() {
        let mut fake = Fake::new();
        fake.interleave = true;
        let mine = [Entry { name: name(1).into(), bytes: vec![1; 4].into() }];
        let theirs = [Entry { name: name(1).into(), bytes: vec![2; 7].into() }];
        let (gen_a, gen_b) = (["a".to_owned()], ["b".to_owned()]);
        let a = store_thumbs(&fake, &mine, &gen_a);
        let b = store_thumbs(&fake, &theirs, &gen_b);
        let (first, second) = block_on(join(a, b));
        assert_eq!((first, second), (Ok(PutOutcome::Stored(1)), Ok(PutOutcome::Stored(1))));
        let stored = fake.stored();
        assert_eq!(stored.len(), 1);
        assert_eq!(fake.named(&name(1)).as_deref(), stored.first().map(|s| s.1.as_str()));
        assert_eq!(fake.total_usage(), fake.stored_bytes());
    }

    #[test]
    fn entries_group_by_shard_in_shard_order() {
        let names = [in_shard(9, 1), in_shard(1, 2), in_shard(0, 3), in_shard(8, 4)];
        let groups = by_shard(names.iter().map(String::as_str));
        assert_eq!(groups, Ok(vec![(0, vec![2, 3]), (1, vec![0, 1])]));
    }

    #[test]
    fn a_name_with_no_shard_is_an_error_and_never_lands_in_shard_zero() {
        assert_eq!(by_shard(["zz"].into_iter()), Err(ThumbErr::new("badName")));
        let fake = Fake::new();
        let entries = [Entry { name: "zz".into(), bytes: vec![1].into() }];
        assert_eq!(put(&fake, &entries, "g"), Err(ThumbErr::new("badName")));
        assert!(fake.stored().is_empty());
        assert_eq!(fake.total_usage(), 0);
    }

    #[test]
    fn the_largest_thumbnail_is_128_kib() {
        assert_eq!(MAX_THUMB_BYTES, 131_072);
    }

    #[test]
    fn a_name_sent_twice_keeps_its_last_bytes_and_first_place() {
        let entries = dedupe(vec![("a", &[1][..]), ("b", &[2][..]), ("a", &[3, 3][..])]);
        assert_eq!(entries, vec![Entry { name: "a".into(), bytes: vec![3, 3].into() }, Entry { name: "b".into(), bytes: vec![2].into() }]);
    }

    #[test]
    fn a_get_body_names_at_most_forty_valid_thumbnails() {
        let a = name(1);
        let b = format!("{}.fx", name(2));
        assert_eq!(parse_get_names(&format!(r#"{{"names":["{a}","{b}","{a}"]}}"#)), Some(vec![a.clone(), b.clone()]));
        assert_eq!(parse_get_names(r#"{"names":[]}"#), Some(Vec::new()));
        let forty = vec![format!("\"{a}\""); 40].join(",");
        assert_eq!(parse_get_names(&format!(r#"{{"names":[{forty}]}}"#)), Some(vec![a.clone()]));
        let many = vec![format!("\"{a}\""); 41].join(",");
        for bad in [
            format!(r#"{{"names":[{many}]}}"#),
            r#"{"names":["zz"]}"#.to_owned(),
            r#"{"names":[5]}"#.to_owned(),
            format!(r#"{{"names":["{}"]}}"#, name(255).to_uppercase()),
            format!(r#"{{"names":["{a}.png"]}}"#),
            r#"{"names":"not-an-array"}"#.to_owned(),
            r#"{}"#.to_owned(),
            "null".to_owned(),
            "not json".to_owned(),
        ] {
            assert_eq!(parse_get_names(&bad), None, "{bad}");
        }
    }

    #[test]
    fn the_worst_case_stays_inside_the_d1_query_and_parameter_limits() {
        // Free plan: 50 queries for one invocation, a batch counts as one, 100 bound values per statement.
        const QUERY_LIMIT: usize = 50;
        const PARAM_LIMIT: usize = 100;
        let put = SHARDS + SHARDS + 1 + SHARDS;
        assert_eq!(put, 25);
        // A failed commit adds the index re-read and one delete for each shard.
        let put_failed_commit = put + 1 + SHARDS;
        assert_eq!(put_failed_commit, 34);
        let get = 1 + SHARDS;
        assert_eq!(get, 9);
        assert!(put_failed_commit <= QUERY_LIMIT);
        assert!(get <= QUERY_LIMIT);
        assert_eq!(select_params(MAX_GET_NAMES), 82);
        assert!(select_params(MAX_GET_NAMES) <= PARAM_LIMIT);
        // The delete of a shard names at most 20 generations. Each statement binds its own values.
        assert!(select_params(MAX_PUT_ENTRIES) <= PARAM_LIMIT);
    }
}
