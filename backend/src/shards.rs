//! Thumbnail bytes in eight D1 shard databases (`THUMBS_0` to `THUMBS_7`). The main database keeps
//! the index: which generation of a name is current. A shard keeps immutable rows, so a reader
//! never sees bytes of a revision that is not committed.
//!
//! The statements here are plain SQL. The orchestration (reserve, insert, commit, clean up) lives
//! in `thumbs.rs` behind the `ThumbStore` trait. This file is the D1 side of that trait.

use js_sys::Array;
use serde::Deserialize;
use wasm_bindgen::{JsCast, JsValue};
use worker::{D1Database, D1PreparedStatement, Env};

use crate::ctx::RepoCtx;
use crate::hex;
use crate::js::now_ms;
use crate::storage::{changes, number};
use crate::thumbs::{GenRef, IndexEntry, NewGen, ThumbErr, ThumbStore};
use crate::validate::is_generation_id;

pub use asset_hub_pack::{shard_of, SHARDS};

/// A shard refuses a reservation above 300 MiB of payload. The Free plan allows 500 MB per
/// database, and SQLite pages add up to 1.4 times the payload for thumbnails of about 6 KB.
pub const SHARD_CAP_BYTES: u64 = 300 * 1024 * 1024;

/// The SQL condition on `version_id` for a generation id that this Worker writes: the shape of a
/// lower-case UUID. Index rows with any other id (the B2 version ids of the TypeScript Worker)
/// name bytes that no shard holds. D1 allows patterns of 50 bytes at most, so the shape is a `LIKE`
/// of 36 characters with a `GLOB` that leaves only lower-case hex digits and dashes.
pub const GENERATION_SQL: &str = "version_id LIKE '________-____-____-____-____________' AND version_id NOT GLOB '*[^0-9a-f-]*'";

/// `?,?` once for each of `count` slots, for an `IN` list.
pub fn marks(count: usize) -> String {
    vec!["?"; count].join(",")
}

/// `(?,?),(?,?)` for `count` pairs, for `(name, gen) IN (VALUES ...)`.
pub fn pair_marks(count: usize) -> String {
    vec!["(?,?)"; count].join(",")
}

/// Raises the byte counter of a shard when the result stays under the cap. One conditional
/// statement, so writers at the same time cannot pass the cap together.
fn reserve_sql() -> String {
    format!("UPDATE usage SET bytes = bytes + ?1 WHERE id = 1 AND bytes + ?1 <= {SHARD_CAP_BYTES}")
}

const RELEASE_SQL: &str = "UPDATE usage SET bytes = MAX(0, bytes - ?1) WHERE id = 1";

/// D1 allows 100 bound values in one statement.
pub const D1_MAX_PARAMS: usize = 100;

/// Rows of one multi-row shard insert. The repo, the schema and the time are bound once and shared
/// by every row (`?1` to `?3`), so a row adds three values: name, generation and bytes.
const INSERT_ROWS: usize = (D1_MAX_PARAMS - 3) / 3;

/// A shard insert of `count` rows. Bound values: repo, schema, created, then name, gen and the bytes
/// as a hex string for each row. `unhex` turns the string into the BLOB that the column stores. It
/// gives NULL for text that is not valid hex, and the NOT NULL column then fails the statement, so
/// wrong bytes never land. `hex::encode` always gives valid hex.
fn insert_sql(count: usize) -> String {
    let rows: Vec<String> = (0..count).map(|i| format!("(?1,?2,?{},?{},unhex(?{}),?3)", 4 + 3 * i, 5 + 3 * i, 6 + 3 * i)).collect();
    format!("INSERT INTO blobs (repo, schema, name, gen, bytes, created) VALUES {}", rows.join(","))
}

/// Rows of one multi-row index upsert. The repo and the schema are bound once (`?1`, `?2`), so a
/// row adds three values: name, size and generation.
const UPSERT_ROWS: usize = (D1_MAX_PARAMS - 2) / 3;

/// An index upsert of `count` rows. Every row takes the revision that the same batch just wrote.
fn upsert_sql(count: usize) -> String {
    let rows: Vec<String> = (0..count)
        .map(|i| format!("(?1,?2,?{},?{},(SELECT rev FROM thumb_rev WHERE repo = ?1 AND schema = ?2),?{})", 3 + 3 * i, 4 + 3 * i, 5 + 3 * i))
        .collect();
    format!(
        "INSERT INTO thumbs (repo, schema, name, size, rev, version_id) VALUES {} \
         ON CONFLICT (repo, schema, name) DO UPDATE SET size = excluded.size, rev = excluded.rev, version_id = excluded.version_id",
        rows.join(",")
    )
}

/// The rows of `count` generations of one repo and schema.
fn match_sql(count: usize) -> String {
    format!("repo = ? AND schema = ? AND (name, gen) IN (VALUES {})", pair_marks(count))
}

/// Lowers the counter by the bytes of the rows `delete_sql` removes next, so the release equals
/// what the delete frees.
fn release_matching_sql(count: usize) -> String {
    format!(
        "UPDATE usage SET bytes = MAX(0, usage.bytes - (SELECT COALESCE(SUM(length(blobs.bytes)), 0) FROM blobs WHERE {})) WHERE id = 1",
        match_sql(count)
    )
}

fn delete_sql(count: usize) -> String {
    format!("DELETE FROM blobs WHERE {}", match_sql(count))
}

/// Bytes of the current generations, read as raw rows `[name, gen, hex]`. `hex(bytes)` makes the
/// BLOB one string, which crosses the D1 binding without a per-byte conversion.
pub fn select_blobs_sql(count: usize) -> String {
    format!("SELECT name, gen, hex(bytes) FROM blobs WHERE {}", match_sql(count))
}

/// Bound values of a GET shard query: the repo, the schema and two for each generation.
#[cfg(test)]
pub const fn select_params(count: usize) -> usize {
    2 + 2 * count
}

fn text(value: &str) -> JsValue {
    JsValue::from_str(value)
}

/// The two leading values and the pairs of a `match_sql` clause.
fn match_values(slug: &str, schema: u32, gens: &[GenRef]) -> Vec<JsValue> {
    let mut values = vec![text(slug), number(u64::from(schema))];
    for g in gens {
        values.push(text(&g.name));
        values.push(text(&g.gen));
    }
    values
}

#[derive(Deserialize)]
struct IndexRow {
    name: String,
    version_id: String,
}

/// The D1 store of one request: the main index and the shards that the request touches. A shard
/// binding is looked up only when a statement needs it. It owns its handles, so work that must
/// outlive the reply can move it into `wait_until`.
pub struct D1Thumbs {
    env: Env,
    slug: String,
    main: D1Database,
    schema: u32,
}

impl D1Thumbs {
    pub fn new(ctx: &RepoCtx<'_>, schema: u32) -> worker::Result<Self> {
        Ok(Self { env: ctx.env.clone(), slug: ctx.slug.to_owned(), main: ctx.db()?, schema })
    }

    fn shard(&self, shard: usize) -> worker::Result<D1Database> {
        self.env.d1(&format!("THUMBS_{shard}"))
    }

    fn prepare_match(&self, db: &D1Database, sql: &str, gens: &[GenRef]) -> worker::Result<D1PreparedStatement> {
        db.prepare(sql).bind(&match_values(&self.slug, self.schema, gens))
    }

    /// The bytes of the listed generations of one shard, as `(name, bytes)`. The statement returns each
    /// BLOB as one hex string. Reading it takes one conversion per row, where a BLOB in its native
    /// form arrives as a JavaScript number array that costs one conversion per byte.
    pub async fn read_blobs(&self, shard: usize, gens: &[GenRef]) -> Result<Vec<(String, Vec<u8>)>, ThumbErr> {
        let db = self.shard(shard)?;
        let rows = self.prepare_match(&db, &select_blobs_sql(gens.len()), gens)?.raw_js_value().await?;
        let mut blobs = Vec::with_capacity(rows.len());
        for row in &rows {
            let row: &Array = row.dyn_ref().ok_or_else(|| ThumbErr::new("badBlobRow"))?;
            let name = row.get(0).as_string().ok_or_else(|| ThumbErr::new("badBlobRow"))?;
            let hex = row.get(2).as_string().ok_or_else(|| ThumbErr::new("badBlobRow"))?;
            let mut bytes = Vec::new();
            hex::decode_into(&hex, &mut bytes).ok_or_else(|| ThumbErr::new("badBlobHex"))?;
            blobs.push((name, bytes));
        }
        Ok(blobs)
    }
}

impl ThumbStore for D1Thumbs {
    async fn reserve_shard(&self, shard: usize, bytes: u64) -> Result<bool, ThumbErr> {
        let statement = self.shard(shard)?.prepare(reserve_sql()).bind(&[number(bytes)])?;
        Ok(changes(&statement.run().await?)? > 0)
    }

    async fn release_shard(&self, shard: usize, bytes: u64, gens: &[GenRef]) -> Result<(), ThumbErr> {
        let db = self.shard(shard)?;
        let release = db.prepare(RELEASE_SQL).bind(&[number(bytes)])?;
        let mut statements = vec![release];
        if !gens.is_empty() {
            statements.push(self.prepare_match(&db, &delete_sql(gens.len()), gens)?);
        }
        db.batch(statements).await?;
        Ok(())
    }

    async fn insert_gens(&self, shard: usize, rows: &[NewGen<'_>]) -> Result<(), ThumbErr> {
        let db = self.shard(shard)?;
        let created = JsValue::from_f64(now_ms());
        let mut statements = Vec::with_capacity(rows.len().div_ceil(INSERT_ROWS));
        for group in rows.chunks(INSERT_ROWS) {
            let mut values = vec![text(&self.slug), number(u64::from(self.schema)), created.clone()];
            for row in group {
                // The statement turns the hex string into a BLOB with `unhex`.
                values.extend([text(row.name), text(row.gen), text(&hex::encode(row.bytes))]);
            }
            statements.push(db.prepare(insert_sql(group.len())).bind(&values)?);
        }
        if !statements.is_empty() {
            db.batch(statements).await?;
        }
        Ok(())
    }

    /// One commit per call: the revision moves up by one and every entry carries it. The batch is
    /// atomic and runs in order, so concurrent writers get distinct revisions and a reader that saw
    /// revision N never misses a later commit. Its first statement reads the rows the upserts then
    /// replace, so the returned generations are exactly the ones this commit made obsolete. Rows
    /// from before this Worker (other ids than a generation) are left out.
    async fn commit_index(&self, entries: &[IndexEntry<'_>]) -> Result<Vec<GenRef>, ThumbErr> {
        let slug = self.slug.as_str();
        let schema = number(u64::from(self.schema));
        let mut previous_values = vec![text(slug), schema.clone()];
        previous_values.extend(entries.iter().map(|e| text(e.name)));
        let previous = self
            .main
            .prepare(format!("SELECT name, version_id FROM thumbs WHERE repo = ? AND schema = ? AND name IN ({})", marks(entries.len())))
            .bind(&previous_values)?;
        let bump = self
            .main
            .prepare("INSERT INTO thumb_rev (repo, schema, rev) VALUES (?, ?, 1) ON CONFLICT (repo, schema) DO UPDATE SET rev = rev + 1")
            .bind(&[text(slug), schema.clone()])?;
        let mut statements = vec![previous, bump];
        for group in entries.chunks(UPSERT_ROWS) {
            let mut values = vec![text(slug), schema.clone()];
            for e in group {
                values.extend([text(e.name), number(e.size), text(e.gen)]);
            }
            statements.push(self.main.prepare(upsert_sql(group.len())).bind(&values)?);
        }
        let results = self.main.batch(statements).await?;
        let rows: Vec<IndexRow> = results.first().ok_or_else(|| ThumbErr::new("noBatchResult"))?.results()?;
        Ok(rows.into_iter().filter(|r| is_generation_id(&r.version_id)).map(|r| GenRef { name: r.name, gen: r.version_id }).collect())
    }

    async fn indexed_gens(&self, names: &[String]) -> Result<Vec<GenRef>, ThumbErr> {
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let mut values = vec![text(&self.slug), number(u64::from(self.schema))];
        values.extend(names.iter().map(|n| text(n)));
        let statement = self
            .main
            .prepare(format!("SELECT name, version_id FROM thumbs WHERE repo = ? AND schema = ? AND name IN ({})", marks(names.len())))
            .bind(&values)?;
        let rows: Vec<IndexRow> = statement.all().await?.results()?;
        Ok(rows.into_iter().filter(|r| is_generation_id(&r.version_id)).map(|r| GenRef { name: r.name, gen: r.version_id }).collect())
    }

    async fn delete_gens(&self, shard: usize, gens: &[GenRef]) -> Result<(), ThumbErr> {
        if gens.is_empty() {
            return Ok(());
        }
        let db = self.shard(shard)?;
        let release = self.prepare_match(&db, &release_matching_sql(gens.len()), gens)?;
        let delete = self.prepare_match(&db, &delete_sql(gens.len()), gens)?;
        db.batch(vec![release, delete]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shard_is_the_first_hex_digit_modulo_eight() {
        let digits = "0123456789abcdef";
        let shards: Vec<Option<usize>> = digits.chars().map(|c| shard_of(&format!("{c}{}", "0".repeat(39)))).collect();
        assert_eq!(shards, (0..16).map(|i| Some(i % 8)).collect::<Vec<_>>());
        let name = format!("c{}", "0".repeat(39));
        assert_eq!(shard_of(&format!("{name}.fx")), shard_of(&name));
    }

    #[test]
    fn a_name_without_a_hex_digit_has_no_shard() {
        assert_eq!(shard_of(""), None);
        assert_eq!(shard_of("zz"), None);
        assert_eq!(shard_of(".fx"), None);
    }

    #[test]
    fn the_cap_is_300_mib() {
        assert_eq!(SHARD_CAP_BYTES, 314_572_800);
        assert!(reserve_sql().ends_with("<= 314572800"));
    }

    #[test]
    fn the_generation_condition_keeps_its_patterns_under_the_d1_limit() {
        for pattern in ["________-____-____-____-____________", "*[^0-9a-f-]*"] {
            assert!(pattern.len() <= 50);
            assert!(GENERATION_SQL.contains(pattern));
        }
        assert_eq!("________-____-____-____-____________".len(), 36);
    }

    #[test]
    fn statements_bind_as_many_values_as_they_have_marks() {
        let count = |sql: &str| sql.matches('?').count();
        assert_eq!(count(&match_sql(3)), 2 + 6);
        assert_eq!(count(&select_blobs_sql(3)), select_params(3));
        assert_eq!(count(&delete_sql(3)), select_params(3));
        assert_eq!(count(&release_matching_sql(3)), select_params(3));
        assert_eq!(count(&insert_sql(1)), 3 + 3);
        assert_eq!(count(RELEASE_SQL), 1);
        assert_eq!(marks(3), "?,?,?");
        assert_eq!(pair_marks(2), "(?,?),(?,?)");
    }

    /// The highest `?N` of a statement: D1 counts the bound values this way.
    fn highest_param(sql: &str) -> usize {
        sql.split('?').skip(1).map(|rest| rest.chars().take_while(char::is_ascii_digit).collect::<String>().parse().unwrap_or(0)).max().unwrap_or(0)
    }

    #[test]
    fn multi_row_statements_stay_within_the_d1_parameter_limit() {
        assert_eq!(highest_param(&insert_sql(INSERT_ROWS)), 3 + 3 * INSERT_ROWS);
        assert_eq!(highest_param(&upsert_sql(UPSERT_ROWS)), 2 + 3 * UPSERT_ROWS);
        assert!(highest_param(&insert_sql(INSERT_ROWS)) <= D1_MAX_PARAMS);
        assert!(highest_param(&upsert_sql(UPSERT_ROWS)) <= D1_MAX_PARAMS);
        assert!(highest_param(&insert_sql(INSERT_ROWS + 1)) > D1_MAX_PARAMS);
        assert!(highest_param(&upsert_sql(UPSERT_ROWS + 1)) > D1_MAX_PARAMS);
    }

    #[test]
    fn multi_row_statements_bind_each_value_to_its_column() {
        assert_eq!(
            insert_sql(2),
            "INSERT INTO blobs (repo, schema, name, gen, bytes, created) VALUES (?1,?2,?4,?5,unhex(?6),?3),(?1,?2,?7,?8,unhex(?9),?3)"
        );
        let upsert = upsert_sql(2);
        assert!(upsert.contains("(?1,?2,?3,?4,(SELECT rev FROM thumb_rev WHERE repo = ?1 AND schema = ?2),?5),(?1,?2,?6,?7,"));
        assert!(upsert.ends_with("version_id = excluded.version_id"));
    }

    #[test]
    fn the_payload_binds_as_hex_and_is_read_back_as_hex() {
        assert!(insert_sql(3).matches("unhex(?").count() == 3);
        assert!(select_blobs_sql(1).starts_with("SELECT name, gen, hex(bytes) FROM blobs"));
        // `unhex` gives NULL for invalid hex, and the NOT NULL column then fails the statement. The
        // encoder only writes even-length lower-case hex, so a stored row never takes that path.
        let all: Vec<u8> = (0..=255).collect();
        let hex = hex::encode(&all);
        assert!(hex.len().is_multiple_of(2) && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)));
    }

    #[test]
    fn a_maximum_thumbnail_fits_one_bound_string_and_the_statement_text_stays_small() {
        const D1_MAX_STRING_BYTES: usize = 2 * 1024 * 1024;
        const D1_MAX_SQL_BYTES: usize = 100 * 1024;
        const { assert!(2 * crate::thumbs::MAX_THUMB_BYTES <= D1_MAX_STRING_BYTES) };
        assert!(insert_sql(INSERT_ROWS).len() < D1_MAX_SQL_BYTES);
    }

    #[test]
    fn the_release_runs_on_the_rows_the_delete_removes() {
        let release = release_matching_sql(2);
        let delete = delete_sql(2);
        let clause = match_sql(2);
        assert!(release.contains(&clause));
        assert!(delete.ends_with(&clause));
    }
}
