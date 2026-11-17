//! The byte counter that keeps the Worker under the free tier of B2, and the cleanup of B2 versions.

use futures_util::future::join_all;
use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{console_error, D1Database, D1Result};

use crate::b2::B2;

/// Hard cap on bytes the Worker may hold in B2. The free tier ends at 10 GB.
pub const STORAGE_CAP_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// A stored B2 version and the bytes it charges against the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Blob {
    pub key: String,
    pub version_id: String,
    pub size: u64,
}

pub fn number(value: u64) -> JsValue {
    JsValue::from_f64(value as f64)
}

/// Rows changed by a statement.
pub fn changes(result: &D1Result) -> worker::Result<usize> {
    Ok(result.meta()?.and_then(|meta| meta.changes).unwrap_or(0))
}

/// Charges `bytes` against the cap before an upload. One conditional statement, so writers at the
/// same time cannot pass the cap together. `false` means the cap would be exceeded.
pub async fn reserve(db: &D1Database, bytes: u64) -> worker::Result<bool> {
    let statement = db
        .prepare("UPDATE storage SET reserved_bytes = reserved_bytes + ?1 WHERE id = 1 AND reserved_bytes + ?1 <= ?2")
        .bind(&[number(bytes), number(STORAGE_CAP_BYTES)])?;
    Ok(changes(&statement.run().await?)? > 0)
}

#[derive(Deserialize)]
struct Reserved {
    reserved_bytes: f64,
}

/// Gives back bytes that are known to be free: a confirmed delete or a known-failed upload. When
/// the outcome is unknown, do not call this: the bytes stay charged and the cap stays honest.
pub async fn release(db: &D1Database, bytes: u64) -> worker::Result<()> {
    if bytes == 0 {
        return Ok(());
    }
    // The read and the write run in one batch, so the value read is the one the write subtracts from.
    let before = db.prepare("SELECT reserved_bytes FROM storage WHERE id = 1");
    let update = db.prepare("UPDATE storage SET reserved_bytes = MAX(0, reserved_bytes - ?) WHERE id = 1").bind(&[number(bytes)])?;
    let results = db.batch(vec![before, update]).await?;
    let rows: Vec<Reserved> = results.first().map(|r| r.results()).transpose()?.unwrap_or_default();
    // Clamping hides a double release, so say so.
    if rows.first().map_or(0.0, |row| row.reserved_bytes) < bytes as f64 {
        console_error!("releaseUnderflow");
    }
    Ok(())
}

/// Deletes versions and releases the bytes of those this call deleted. A failed delete is logged
/// and its bytes stay charged. A version B2 no longer has frees nothing now, so it releases
/// nothing. Call it only for versions D1 does not reference. Never fails.
pub async fn discard(b2: &B2, db: &D1Database, blobs: &[Blob]) {
    if blobs.is_empty() {
        return;
    }
    let outcomes = join_all(blobs.iter().map(|b| b2.delete(&b.key, &b.version_id))).await;
    let mut freed = 0;
    for (blob, outcome) in blobs.iter().zip(outcomes) {
        match outcome {
            Ok(true) => freed += blob.size,
            Ok(false) => {}
            Err(err) => console_error!("b2DeleteFailed {}", err.log_line()),
        }
    }
    if release(db, freed).await.is_err() {
        console_error!("releaseFailed");
    }
}
