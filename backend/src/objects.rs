//! `GET` and `PUT /cache/:name`: gzip objects in B2, indexed by a head row in D1. D1 decides the
//! winner of a race. B2 only holds versioned keys, so a loser never overwrites the current object.

use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{console_error, D1Database, Response};

use crate::b2::{B2Error, B2};
use crate::config::B2Config;
use crate::ctx::RepoCtx;
use crate::http::{header, json, quote, read_schema, stop, Halt, Handled};
use crate::js::{js_trim, random_uuid, read_body_js};
use crate::kinds::is_cache_object_name;
use crate::storage::{changes, discard, number, release, reserve, Blob};

const MAX_OBJECT_BYTES: usize = 20 * 1024 * 1024;
const CONTENT_TYPE: &str = "application/gzip";

/// The head row of an object: the version that readers get.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Head {
    pub etag: String,
    pub key: String,
    pub version_id: String,
    pub size: u64,
}

/// The precondition of a PUT: replace the head with this ETag, or create when none exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Precondition {
    Replace(String),
    Create,
}

/// `If-Match: <etag>` replaces, `If-None-Match: *` creates. `None` means the writer sent neither,
/// which is a 428. A bare `If-Match: *` is refused too: a concrete ETag proves the writer saw the
/// current version, `*` would not.
pub fn precondition(if_match: Option<&str>, if_none_match: Option<&str>) -> Option<Precondition> {
    if let Some(value) = if_match {
        if js_trim(value) == "*" {
            return None;
        }
        if !value.is_empty() {
            return Some(Precondition::Replace(js_trim(value).to_owned()));
        }
    }
    (if_none_match == Some("*")).then_some(Precondition::Create)
}

/// True when an `If-None-Match` value (a list of ETags or `*`) names `etag`, ignoring weak markers.
pub fn etag_listed(header: &str, etag: &str) -> bool {
    header.split(',').any(|part| {
        let part = js_trim(part);
        let tag = part.strip_prefix("W/").unwrap_or(part);
        tag == "*" || tag == etag
    })
}

/// How a store step failed.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreErr {
    /// B2 answered with an error status: the request is known not to have changed the bucket.
    Refused(String),
    /// Any other failure. The text is safe to log.
    Other(String),
}

impl From<StoreErr> for Halt {
    fn from(err: StoreErr) -> Self {
        match err {
            StoreErr::Refused(label) | StoreErr::Other(label) => Halt::Failed(label),
        }
    }
}

impl From<worker::Error> for StoreErr {
    fn from(_: worker::Error) -> Self {
        StoreErr::Other("Error".to_owned())
    }
}

/// The steps of a cache PUT that touch D1 and B2. A real store wraps both. A test store can fail any
/// step before or after its effect, which black-box tests cannot cause.
pub trait ObjectStore {
    async fn read_head(&self) -> Result<Option<Head>, StoreErr>;
    async fn reserve(&self, bytes: u64) -> Result<bool, StoreErr>;
    /// Uploads the body under `key` and gives the version id.
    async fn upload(&self, key: &str) -> Result<String, StoreErr>;
    /// Moves the head to `next` only if it still is the version the writer saw.
    async fn switch_head(&self, pre: &Precondition, next: &Head) -> Result<bool, StoreErr>;
    /// Whether the head is the version with `etag`. `None` when D1 cannot answer.
    async fn is_head(&self, etag: &str) -> Option<bool>;
    /// Deletes versions D1 does not reference and releases their bytes. Never fails.
    async fn discard(&self, blobs: &[Blob]);
    async fn release(&self, bytes: u64) -> Result<(), StoreErr>;
}

#[derive(Debug, PartialEq, Eq)]
pub enum PutOutcome {
    Stored { etag: String },
    PreconditionFailed,
    StorageFull,
}

/// A PUT after its body checks: early head check, reserve, upload, head switch, cleanup.
pub async fn store_object<S: ObjectStore>(store: &S, pre: &Precondition, size: u64, schema: u32, slug: &str, name: &str, uuid: &str) -> Result<PutOutcome, StoreErr> {
    // A cheap early exit. The conditional statement below stays the authority. For a replace, the
    // head with the expected ETag is the one that statement replaces, because ETags never repeat.
    let head = store.read_head().await?;
    let mismatch = match pre {
        Precondition::Replace(etag) => head.as_ref().map(|h| &h.etag) != Some(etag),
        Precondition::Create => head.is_some(),
    };
    if mismatch {
        return Ok(PutOutcome::PreconditionFailed);
    }

    if !store.reserve(size).await? {
        return Ok(PutOutcome::StorageFull);
    }

    let key = format!("repos/{slug}/s{schema}/{name}/{uuid}.json.gz");
    // A failed upload releases its bytes only when B2 answered with an error. A lost answer leaves
    // the outcome unknown, so those bytes stay charged.
    let version_id = match store.upload(&key).await {
        Ok(version_id) => version_id,
        Err(err) => {
            if matches!(err, StoreErr::Refused(_)) {
                let _ = store.release(size).await;
            }
            return Err(err);
        }
    };
    let next = Head { etag: format!("\"{uuid}\""), key: key.clone(), version_id: version_id.clone(), size };
    let fresh = Blob { key, version_id, size };

    let switched = match store.switch_head(pre, &next).await {
        Ok(switched) => switched,
        Err(err) => {
            // The call failed, but the commit may have happened. Only D1 can say which.
            let committed = store.is_head(&next.etag).await;
            if committed == Some(false) {
                store.discard(std::slice::from_ref(&fresh)).await;
            }
            // Unknown: delete nothing and keep the bytes charged.
            if committed != Some(true) {
                return Err(err);
            }
            true
        }
    };
    if !switched {
        store.discard(&[fresh]).await;
        return Ok(PutOutcome::PreconditionFailed);
    }
    if let Some(old) = head {
        store.discard(&[Blob { key: old.key, version_id: old.version_id, size: old.size }]).await;
    }
    Ok(PutOutcome::Stored { etag: next.etag })
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

async fn read_head(db: &D1Database, slug: &str, schema: u32, name: &str) -> worker::Result<Option<Head>> {
    db.prepare("SELECT etag, key, version_id, size FROM objects WHERE repo = ? AND schema = ? AND name = ?")
        .bind(&[text(slug), number(u64::from(schema)), text(name)])?
        .first::<Head>(None)
        .await
}

/// The real store: one cache object in D1 and B2.
struct LiveStore<'a> {
    db: &'a D1Database,
    b2: &'a B2,
    body: &'a js_sys::Uint8Array,
    slug: &'a str,
    schema: u32,
    name: &'a str,
}

impl ObjectStore for LiveStore<'_> {
    async fn read_head(&self) -> Result<Option<Head>, StoreErr> {
        Ok(read_head(self.db, self.slug, self.schema, self.name).await?)
    }

    async fn reserve(&self, bytes: u64) -> Result<bool, StoreErr> {
        Ok(reserve(self.db, bytes).await?)
    }

    async fn upload(&self, key: &str) -> Result<String, StoreErr> {
        self.b2.put(key, self.body, CONTENT_TYPE).await.map_err(|err| match err {
            B2Error::Http { .. } => StoreErr::Refused(err.log_line()),
            other => StoreErr::Other(other.log_line()),
        })
    }

    async fn switch_head(&self, pre: &Precondition, next: &Head) -> Result<bool, StoreErr> {
        let statement = match pre {
            Precondition::Replace(expected) => self
                .db
                .prepare("UPDATE objects SET etag = ?, key = ?, version_id = ?, size = ? WHERE repo = ? AND schema = ? AND name = ? AND etag = ?")
                .bind(&[text(&next.etag), text(&next.key), text(&next.version_id), number(next.size), text(self.slug), number(u64::from(self.schema)), text(self.name), text(expected)])?,
            Precondition::Create => self
                .db
                .prepare("INSERT INTO objects (etag, key, version_id, size, repo, schema, name) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (repo, schema, name) DO NOTHING")
                .bind(&[text(&next.etag), text(&next.key), text(&next.version_id), number(next.size), text(self.slug), number(u64::from(self.schema)), text(self.name)])?,
        };
        Ok(changes(&statement.run().await?)? > 0)
    }

    async fn is_head(&self, etag: &str) -> Option<bool> {
        let statement = self
            .db
            .prepare("SELECT 1 AS present FROM objects WHERE repo = ? AND schema = ? AND name = ? AND etag = ?")
            .bind(&[text(self.slug), number(u64::from(self.schema)), text(self.name), text(etag)])
            .ok()?;
        let row = statement.first::<serde_json::Value>(None).await.ok()?;
        Some(row.is_some())
    }

    async fn discard(&self, blobs: &[Blob]) {
        discard(self.b2, self.db, blobs).await;
    }

    async fn release(&self, bytes: u64) -> Result<(), StoreErr> {
        Ok(release(self.db, bytes).await?)
    }
}

/// Name and schema of a cache object, or the early 404 for an unknown name and 400 for a bad schema.
fn object_id(ctx: &RepoCtx<'_>, name: &str) -> Handled<u32> {
    if !is_cache_object_name(name) {
        return Err(stop("notFound", 404));
    }
    read_schema(ctx.req)
}

/// `GET /cache/:name`: the stored gzip body, untouched. `If-None-Match` gives 304 without a B2
/// call. A replace can delete the version between the head read and the body read, so a missing
/// body re-reads the head once. A second miss gives 503. A B2 error status gives 503 too: B2 cannot
/// serve right now, for example after its daily cap.
pub async fn get_object(ctx: &RepoCtx<'_>, name: &str) -> Handled<web_sys::Response> {
    let schema = object_id(ctx, name)?;
    let if_none_match = header(ctx.req, "if-none-match")?.filter(|value| !value.is_empty());
    let db = ctx.db()?;
    let b2 = B2::new(&B2Config::load(ctx.env)?);

    for _ in 0..2 {
        let Some(head) = read_head(&db, ctx.slug, schema, name).await? else {
            return Err(stop("notFound", 404));
        };
        if if_none_match.as_deref().is_some_and(|value| etag_listed(value, &head.etag)) {
            let mut res = Response::empty()?.with_status(304);
            res.headers_mut().set("etag", &head.etag)?;
            return Ok(res.into());
        }
        match b2.get(&head.key).await {
            Ok(Some(res)) => return Ok(passthrough(&res, &head.etag)?),
            Ok(None) => {}
            Err(err @ B2Error::Http { .. }) => {
                console_error!("{}", err.log_line());
                return Err(stop("storageUnavailable", 503));
            }
            Err(err) => return Err(err.into()),
        }
    }
    Err(stop("storageUnavailable", 503))
}

/// A new reply that carries the B2 body stream as it is, with no copy into Wasm memory.
fn passthrough(res: &web_sys::Response, etag: &str) -> Result<web_sys::Response, JsValue> {
    let headers = web_sys::Headers::new()?;
    headers.set("etag", etag)?;
    headers.set("content-type", CONTENT_TYPE)?;
    headers.set("cache-control", "private, no-store")?;
    let init = web_sys::ResponseInit::new();
    init.set_status(200);
    init.set_headers(&headers);
    web_sys::Response::new_with_opt_readable_stream_and_init(res.body().as_ref(), &init)
}

/// `PUT /cache/:name`: needs push access, a gzip body, and `If-Match: <etag>` (replace) or
/// `If-None-Match: *` (create). A lost race gives 412 and the client merges again.
pub async fn put_object(ctx: &RepoCtx<'_>, name: &str) -> Handled<web_sys::Response> {
    let schema = object_id(ctx, name)?;
    if !ctx.caller.can_push {
        return Err(stop("readOnly", 403));
    }
    let if_match = header(ctx.req, "if-match")?;
    let if_none_match = header(ctx.req, "if-none-match")?;
    let Some(pre) = precondition(if_match.as_deref(), if_none_match.as_deref()) else {
        return Err(stop("preconditionRequired", 428));
    };

    // The body stays in JavaScript memory: a 20 MB copy into Wasm would cost CPU the request lacks.
    let Some(body) = read_body_js(ctx.req, MAX_OBJECT_BYTES).await? else {
        return Err(stop("tooLarge", 413));
    };
    if body.length() < 2 || body.get_index(0) != 0x1f || body.get_index(1) != 0x8b {
        return Err(stop("notGzip", 400));
    }

    let db = ctx.db()?;
    let b2 = B2::new(&B2Config::load(ctx.env)?);
    let store = LiveStore { db: &db, b2: &b2, body: &body, slug: ctx.slug, schema, name };
    let uuid = random_uuid()?;
    match store_object(&store, &pre, u64::from(body.length()), schema, ctx.slug, name, &uuid).await? {
        PutOutcome::Stored { etag } => {
            // Cloudflare compresses JSON at the edge and then weakens the ETag header (`W/"..."`),
            // which can never satisfy `If-Match`. `no-transform` keeps the header strong, and the
            // body carries the ETag as well.
            Ok(json(format!("{{\"etag\":{}}}", quote(&etag)), 200, &[("etag", &etag), ("cache-control", "no-transform")])?)
        }
        PutOutcome::PreconditionFailed => Err(stop("preconditionFailed", 412)),
        PutOutcome::StorageFull => Err(stop("storageFull", 507)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use std::cell::{Cell, RefCell};

    #[test]
    fn if_match_replaces_and_if_none_match_creates() {
        assert_eq!(precondition(Some("\"a\""), None), Some(Precondition::Replace("\"a\"".into())));
        assert_eq!(precondition(Some("  \"a\" "), None), Some(Precondition::Replace("\"a\"".into())));
        assert_eq!(precondition(None, Some("*")), Some(Precondition::Create));
        assert_eq!(precondition(Some("\"a\""), Some("*")), Some(Precondition::Replace("\"a\"".into())));
        // An empty `If-Match` is false in JavaScript, so `If-None-Match` decides.
        assert_eq!(precondition(Some(""), Some("*")), Some(Precondition::Create));
    }

    #[test]
    fn a_missing_or_wildcard_precondition_is_refused() {
        assert_eq!(precondition(None, None), None);
        assert_eq!(precondition(Some("*"), None), None);
        assert_eq!(precondition(Some(" * "), Some("*")), None);
        // `If-None-Match: *` is an exact match, with no trimming.
        assert_eq!(precondition(None, Some(" * ")), None);
        assert_eq!(precondition(None, Some("\"a\"")), None);
    }

    #[test]
    fn if_none_match_lists_split_trim_and_ignore_weak_markers() {
        let etag = "\"abc\"";
        assert!(etag_listed("\"abc\"", etag));
        assert!(etag_listed("W/\"abc\"", etag));
        assert!(etag_listed("\"x\" , W/\"abc\"", etag));
        assert!(etag_listed("*", etag));
        assert!(etag_listed("\"x\", *", etag));
        assert!(!etag_listed("\"other\"", etag));
        assert!(!etag_listed("", etag));
        assert!(!etag_listed("W/W/\"abc\"", etag));
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        None,
        /// The call fails before it has any effect.
        Before,
        /// The effect happens, then the call fails.
        After,
    }

    /// A store in memory. It models the head row, the byte counter and the B2 versions.
    struct Fake {
        head: RefCell<Option<Head>>,
        reserved: Cell<u64>,
        cap: u64,
        versions: RefCell<Vec<Blob>>,
        counter: Cell<u32>,
        deletes: Cell<u32>,
        switch_fault: Cell<Fault>,
        reread_fails: Cell<bool>,
        upload_fault: RefCell<Option<StoreErr>>,
        /// Another writer replaces the head right before this writer's switch.
        rival: RefCell<Option<Head>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                head: RefCell::new(None),
                reserved: Cell::new(0),
                cap: u64::MAX,
                versions: RefCell::new(Vec::new()),
                counter: Cell::new(0),
                deletes: Cell::new(0),
                switch_fault: Cell::new(Fault::None),
                reread_fails: Cell::new(false),
                upload_fault: RefCell::new(None),
                rival: RefCell::new(None),
            }
        }

        /// Stores a version and makes it the head, as an earlier PUT would have.
        fn seeded(size: u64) -> Self {
            let fake = Self::new();
            fake.reserved.set(size);
            fake.versions.borrow_mut().push(Blob { key: "old".into(), version_id: "v0".into(), size });
            *fake.head.borrow_mut() = Some(Head { etag: "\"old\"".into(), key: "old".into(), version_id: "v0".into(), size });
            fake
        }

        fn stored_bytes(&self) -> u64 {
            self.versions.borrow().iter().map(|b| b.size).sum()
        }
    }

    impl ObjectStore for Fake {
        async fn read_head(&self) -> Result<Option<Head>, StoreErr> {
            Ok(self.head.borrow().clone())
        }

        async fn reserve(&self, bytes: u64) -> Result<bool, StoreErr> {
            if self.reserved.get() + bytes > self.cap {
                return Ok(false);
            }
            self.reserved.set(self.reserved.get() + bytes);
            Ok(true)
        }

        async fn upload(&self, key: &str) -> Result<String, StoreErr> {
            if let Some(err) = self.upload_fault.borrow_mut().take() {
                return Err(err);
            }
            self.counter.set(self.counter.get() + 1);
            let version_id = format!("v{}", self.counter.get());
            // The size is the one the caller reserved: the tests upload `UPLOAD_SIZE` bytes.
            self.versions.borrow_mut().push(Blob { key: key.to_owned(), version_id: version_id.clone(), size: UPLOAD_SIZE });
            Ok(version_id)
        }

        async fn switch_head(&self, pre: &Precondition, next: &Head) -> Result<bool, StoreErr> {
            if self.switch_fault.get() == Fault::Before {
                return Err(StoreErr::Other("Error".into()));
            }
            if let Some(rival) = self.rival.borrow_mut().take() {
                self.versions.borrow_mut().push(Blob { key: rival.key.clone(), version_id: rival.version_id.clone(), size: rival.size });
                self.reserved.set(self.reserved.get() + rival.size);
                *self.head.borrow_mut() = Some(rival);
            }
            let won = match (pre, self.head.borrow().as_ref()) {
                (Precondition::Replace(etag), Some(head)) => &head.etag == etag,
                (Precondition::Create, None) => true,
                _ => false,
            };
            if won {
                *self.head.borrow_mut() = Some(next.clone());
            }
            if self.switch_fault.get() == Fault::After {
                return Err(StoreErr::Other("Error".into()));
            }
            Ok(won)
        }

        async fn is_head(&self, etag: &str) -> Option<bool> {
            if self.reread_fails.get() {
                return None;
            }
            Some(self.head.borrow().as_ref().is_some_and(|h| h.etag == etag))
        }

        async fn discard(&self, blobs: &[Blob]) {
            for blob in blobs {
                self.deletes.set(self.deletes.get() + 1);
                let mut versions = self.versions.borrow_mut();
                if let Some(at) = versions.iter().position(|v| v.version_id == blob.version_id) {
                    versions.remove(at);
                    self.reserved.set(self.reserved.get().saturating_sub(blob.size));
                }
            }
        }

        async fn release(&self, bytes: u64) -> Result<(), StoreErr> {
            self.reserved.set(self.reserved.get().saturating_sub(bytes));
            Ok(())
        }
    }

    const UPLOAD_SIZE: u64 = 7;

    fn put(fake: &Fake, pre: &Precondition) -> Result<PutOutcome, StoreErr> {
        block_on(store_object(fake, pre, UPLOAD_SIZE, 1, "o/r", "catalog", "uuid-1"))
    }

    #[test]
    fn a_commit_that_threw_but_landed_keeps_the_live_version_and_its_bytes() {
        let fake = Fake::new();
        fake.switch_fault.set(Fault::After);
        assert_eq!(put(&fake, &Precondition::Create), Ok(PutOutcome::Stored { etag: "\"uuid-1\"".into() }));
        assert_eq!(fake.versions.borrow().len(), 1);
        assert_eq!(fake.reserved.get(), UPLOAD_SIZE);
        assert_eq!(fake.deletes.get(), 0);
        assert_eq!(fake.head.borrow().as_ref().map(|h| h.etag.clone()), Some("\"uuid-1\"".to_owned()));
    }

    #[test]
    fn a_commit_that_threw_and_did_not_land_deletes_the_new_version_and_releases_its_bytes() {
        let fake = Fake::new();
        fake.switch_fault.set(Fault::Before);
        assert_eq!(put(&fake, &Precondition::Create), Err(StoreErr::Other("Error".into())));
        assert!(fake.versions.borrow().is_empty());
        assert_eq!(fake.reserved.get(), 0);
    }

    #[test]
    fn when_the_commit_outcome_cannot_be_read_nothing_is_deleted_and_the_bytes_stay_charged() {
        let fake = Fake::new();
        fake.switch_fault.set(Fault::After);
        fake.reread_fails.set(true);
        assert_eq!(put(&fake, &Precondition::Create), Err(StoreErr::Other("Error".into())));
        assert_eq!(fake.versions.borrow().len(), 1);
        assert_eq!(fake.deletes.get(), 0);
        assert_eq!(fake.reserved.get(), UPLOAD_SIZE);
    }

    #[test]
    fn a_lost_race_deletes_the_new_version_and_releases_its_bytes() {
        let fake = Fake::seeded(10);
        *fake.rival.borrow_mut() = Some(Head { etag: "\"rival\"".into(), key: "rival".into(), version_id: "vr".into(), size: 5 });
        assert_eq!(put(&fake, &Precondition::Replace("\"old\"".into())), Ok(PutOutcome::PreconditionFailed));
        // The new version is gone. The old one and the rival's stay. Nothing is charged twice.
        assert_eq!(fake.versions.borrow().len(), 2);
        assert!(fake.versions.borrow().iter().all(|v| v.key != "catalog"));
        assert_eq!(fake.reserved.get(), fake.stored_bytes());
        assert_eq!(fake.reserved.get(), 15);
    }

    #[test]
    fn a_replace_deletes_the_old_version_and_keeps_one_stored_object() {
        let fake = Fake::seeded(10);
        assert_eq!(put(&fake, &Precondition::Replace("\"old\"".into())), Ok(PutOutcome::Stored { etag: "\"uuid-1\"".into() }));
        assert_eq!(fake.versions.borrow().len(), 1);
        assert_eq!(fake.reserved.get(), UPLOAD_SIZE);
        assert_eq!(fake.reserved.get(), fake.stored_bytes());
    }

    #[test]
    fn a_stale_precondition_stops_before_anything_is_reserved() {
        let fake = Fake::seeded(10);
        assert_eq!(put(&fake, &Precondition::Replace("\"other\"".into())), Ok(PutOutcome::PreconditionFailed));
        assert_eq!(put(&fake, &Precondition::Create), Ok(PutOutcome::PreconditionFailed));
        let empty = Fake::new();
        assert_eq!(put(&empty, &Precondition::Replace("\"old\"".into())), Ok(PutOutcome::PreconditionFailed));
        assert_eq!(fake.reserved.get(), 10);
        assert_eq!(empty.reserved.get(), 0);
        assert_eq!(fake.counter.get() + empty.counter.get(), 0);
    }

    #[test]
    fn the_cap_stops_the_upload() {
        let mut fake = Fake::new();
        fake.cap = UPLOAD_SIZE - 1;
        assert_eq!(put(&fake, &Precondition::Create), Ok(PutOutcome::StorageFull));
        assert_eq!(fake.counter.get(), 0);
        assert_eq!(fake.reserved.get(), 0);
    }

    #[test]
    fn an_upload_refused_by_b2_releases_its_bytes_and_a_lost_one_does_not() {
        let refused = Fake::new();
        *refused.upload_fault.borrow_mut() = Some(StoreErr::Refused("b2 put 503".into()));
        assert_eq!(put(&refused, &Precondition::Create), Err(StoreErr::Refused("b2 put 503".into())));
        assert_eq!(refused.reserved.get(), 0);

        let lost = Fake::new();
        *lost.upload_fault.borrow_mut() = Some(StoreErr::Other("TypeError".into()));
        assert_eq!(put(&lost, &Precondition::Create), Err(StoreErr::Other("TypeError".into())));
        assert_eq!(lost.reserved.get(), UPLOAD_SIZE);
    }
}
