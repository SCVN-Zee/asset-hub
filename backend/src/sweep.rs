//! The sweep of the thumbnail shards. A PUT that is cancelled or killed after it inserted rows can
//! leave rows that no index row names, and bytes that the shard counter still charges. Each cron
//! run sweeps one shard: it deletes the rows older than an hour that the main index does not name,
//! and it counts the shard's bytes again from the rows that remain.
//!
//! A run costs three D1 queries (a scan, an index check and a cleanup batch), far below the 50 of
//! one invocation, and the scan reads at most `SWEEP_ROWS` rows. A saved cursor in the shard moves
//! through the table over many runs.

use std::collections::HashSet;

use serde::Deserialize;
use wasm_bindgen::JsValue;
use worker::{D1Database, Env};

use crate::shards::{marks, SHARDS};
use crate::storage::number;
use crate::thumbs::ThumbErr;

/// Rows of a shard that one run looks at.
const SWEEP_ROWS: usize = 96;
/// Index rows asked for in one statement: four bound values each, at most 100 in all.
const INDEX_ROWS_PER_STATEMENT: usize = 24;
/// Rows deleted by one statement: one bound value each, and one for the age limit.
const DELETE_ROWS_PER_STATEMENT: usize = 48;
/// A row younger than this may belong to a PUT that is still running.
const MIN_AGE_MS: f64 = 60.0 * 60.0 * 1000.0;
/// The time between two cron runs. The shard of a run follows from the time of its trigger.
const RUN_MS: f64 = 3.0 * 60.0 * 60.0 * 1000.0;

/// The shard a cron run sweeps. Successive runs go through the shards in turn.
pub fn shard_for_run(scheduled_ms: f64) -> usize {
    (((scheduled_ms / RUN_MS).floor().max(0.0) as u64) % SHARDS as u64) as usize
}

/// What names a generation: the same four values in a shard and in the main index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub repo: String,
    pub schema: u32,
    pub name: String,
    pub gen: String,
}

/// A shard row that the sweep looks at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub id: u64,
    pub key: Key,
}

/// What a run did.
#[derive(Debug, PartialEq, Eq)]
pub struct Report {
    pub scanned: usize,
    pub removed: usize,
}

/// The three steps of a sweep. A test store can fail any of them.
pub trait SweepStore {
    /// Up to `limit` rows older than `cutoff` after the saved cursor of `shard`, in id order.
    async fn scan(&self, shard: usize, cutoff: f64, limit: usize) -> Result<Vec<Row>, ThumbErr>;
    /// The keys among `rows` that the main index names.
    async fn named(&self, rows: &[Row]) -> Result<Vec<Key>, ThumbErr>;
    /// Deletes the rows `orphans` that are older than `cutoff`, counts the bytes of the shard again
    /// from the rows that remain, and saves `cursor`.
    async fn clean(&self, shard: usize, orphans: &[u64], cutoff: f64, cursor: u64) -> Result<(), ThumbErr>;
}

/// Sweeps one shard. When the scan or the index check fails, nothing is deleted.
pub async fn sweep_shard<S: SweepStore>(store: &S, shard: usize, now: f64) -> Result<Report, ThumbErr> {
    let cutoff = now - MIN_AGE_MS;
    let rows = store.scan(shard, cutoff, SWEEP_ROWS).await?;
    let named: HashSet<Key> = if rows.is_empty() { HashSet::new() } else { store.named(&rows).await?.into_iter().collect() };
    let orphans: Vec<u64> = rows.iter().filter(|row| !named.contains(&row.key)).map(|row| row.id).collect();
    // A short page is the end of the table: the next run starts again from the first row.
    let cursor = if rows.len() < SWEEP_ROWS { 0 } else { rows.last().map_or(0, |row| row.id) };
    store.clean(shard, &orphans, cutoff, cursor).await?;
    Ok(Report { scanned: rows.len(), removed: orphans.len() })
}

#[derive(Deserialize)]
struct ScanRow {
    id: f64,
    repo: String,
    schema: f64,
    name: String,
    gen: String,
}

#[derive(Deserialize)]
struct IndexRow {
    repo: String,
    schema: f64,
    name: String,
    version_id: String,
}

/// The D1 side of the sweep.
pub struct D1Sweep {
    env: Env,
}

impl D1Sweep {
    pub fn new(env: &Env) -> Self {
        Self { env: env.clone() }
    }

    fn shard(&self, shard: usize) -> worker::Result<D1Database> {
        self.env.d1(&format!("THUMBS_{shard}"))
    }
}

const SCAN_SQL: &str = "SELECT rowid AS id, repo, schema, name, gen FROM blobs \
                        WHERE rowid > (SELECT sweep_after FROM usage WHERE id = 1) AND created < ? ORDER BY rowid LIMIT ?";

const RECOUNT_SQL: &str = "UPDATE usage SET bytes = (SELECT COALESCE(SUM(length(blobs.bytes)), 0) FROM blobs), sweep_after = ? WHERE id = 1";

fn named_sql(count: usize) -> String {
    format!(
        "SELECT repo, schema, name, version_id FROM thumbs WHERE (repo, schema, name, version_id) IN (VALUES {})",
        vec!["(?,?,?,?)"; count].join(",")
    )
}

fn delete_sql(count: usize) -> String {
    format!("DELETE FROM blobs WHERE rowid IN ({}) AND created < ?", marks(count))
}

impl SweepStore for D1Sweep {
    async fn scan(&self, shard: usize, cutoff: f64, limit: usize) -> Result<Vec<Row>, ThumbErr> {
        let statement = self.shard(shard)?.prepare(SCAN_SQL).bind(&[JsValue::from_f64(cutoff), number(limit as u64)])?;
        let rows: Vec<ScanRow> = statement.all().await?.results()?;
        Ok(rows.into_iter().map(|r| Row { id: r.id as u64, key: Key { repo: r.repo, schema: r.schema as u32, name: r.name, gen: r.gen } }).collect())
    }

    async fn named(&self, rows: &[Row]) -> Result<Vec<Key>, ThumbErr> {
        let main = self.env.d1("DB")?;
        let mut statements = Vec::new();
        for chunk in rows.chunks(INDEX_ROWS_PER_STATEMENT) {
            let mut values: Vec<JsValue> = Vec::with_capacity(chunk.len() * 4);
            for row in chunk {
                values.push(JsValue::from_str(&row.key.repo));
                values.push(number(u64::from(row.key.schema)));
                values.push(JsValue::from_str(&row.key.name));
                values.push(JsValue::from_str(&row.key.gen));
            }
            statements.push(main.prepare(named_sql(chunk.len())).bind(&values)?);
        }
        let mut keys = Vec::new();
        for result in main.batch(statements).await? {
            let found: Vec<IndexRow> = result.results()?;
            keys.extend(found.into_iter().map(|r| Key { repo: r.repo, schema: r.schema as u32, name: r.name, gen: r.version_id }));
        }
        Ok(keys)
    }

    async fn clean(&self, shard: usize, orphans: &[u64], cutoff: f64, cursor: u64) -> Result<(), ThumbErr> {
        let db = self.shard(shard)?;
        let mut statements = Vec::new();
        for chunk in orphans.chunks(DELETE_ROWS_PER_STATEMENT) {
            let mut values: Vec<JsValue> = chunk.iter().map(|id| number(*id)).collect();
            values.push(JsValue::from_f64(cutoff));
            statements.push(db.prepare(delete_sql(chunk.len())).bind(&values)?);
        }
        statements.push(db.prepare(RECOUNT_SQL).bind(&[number(cursor)])?);
        db.batch(statements).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_executor::block_on;
    use std::cell::{Cell, RefCell};

    struct Stored {
        id: u64,
        key: Key,
        created: f64,
        size: u64,
    }

    /// One shard and the main index, in memory.
    struct Fake {
        rows: RefCell<Vec<Stored>>,
        index: RefCell<HashSet<Key>>,
        usage: Cell<u64>,
        cursor: Cell<u64>,
        queries: Cell<usize>,
        index_fails: Cell<bool>,
    }

    fn key(n: u32) -> Key {
        Key { repo: "org/game".into(), schema: 1, name: format!("{n:040x}"), gen: format!("gen-{n}") }
    }

    const NOW: f64 = 10.0 * 3_600_000.0;

    impl Fake {
        fn new() -> Self {
            Self { rows: RefCell::default(), index: RefCell::default(), usage: Cell::new(0), cursor: Cell::new(0), queries: Cell::new(0), index_fails: Cell::new(false) }
        }

        /// A stored row of `size` bytes, `age_ms` old, named by the index when `live`.
        fn add(&self, n: u32, size: u64, age_ms: f64, live: bool) {
            self.rows.borrow_mut().push(Stored { id: u64::from(n), key: key(n), created: NOW - age_ms, size });
            if live {
                self.index.borrow_mut().insert(key(n));
            }
        }

        fn ids(&self) -> Vec<u64> {
            self.rows.borrow().iter().map(|r| r.id).collect()
        }
    }

    impl SweepStore for Fake {
        async fn scan(&self, _shard: usize, cutoff: f64, limit: usize) -> Result<Vec<Row>, ThumbErr> {
            self.queries.set(self.queries.get() + 1);
            let after = self.cursor.get();
            let mut rows: Vec<Row> = self.rows.borrow().iter().filter(|r| r.id > after && r.created < cutoff).map(|r| Row { id: r.id, key: r.key.clone() }).collect();
            rows.sort_by_key(|r| r.id);
            rows.truncate(limit);
            Ok(rows)
        }

        async fn named(&self, rows: &[Row]) -> Result<Vec<Key>, ThumbErr> {
            self.queries.set(self.queries.get() + 1);
            if self.index_fails.get() {
                return Err(ThumbErr::new("Error"));
            }
            Ok(rows.iter().filter(|r| self.index.borrow().contains(&r.key)).map(|r| r.key.clone()).collect())
        }

        async fn clean(&self, _shard: usize, orphans: &[u64], cutoff: f64, cursor: u64) -> Result<(), ThumbErr> {
            self.queries.set(self.queries.get() + 1);
            self.rows.borrow_mut().retain(|r| !(orphans.contains(&r.id) && r.created < cutoff));
            self.usage.set(self.rows.borrow().iter().map(|r| r.size).sum());
            self.cursor.set(cursor);
            Ok(())
        }
    }

    fn sweep(fake: &Fake) -> Result<Report, ThumbErr> {
        block_on(sweep_shard(fake, 0, NOW))
    }

    #[test]
    fn an_orphan_is_removed_and_a_live_row_stays() {
        let fake = Fake::new();
        fake.add(1, 10, 2.0 * MIN_AGE_MS, true);
        fake.add(2, 20, 2.0 * MIN_AGE_MS, false);
        assert_eq!(sweep(&fake), Ok(Report { scanned: 2, removed: 1 }));
        assert_eq!(fake.ids(), vec![1]);
    }

    #[test]
    fn a_young_orphan_stays() {
        let fake = Fake::new();
        fake.add(1, 10, MIN_AGE_MS / 2.0, false);
        assert_eq!(sweep(&fake), Ok(Report { scanned: 0, removed: 0 }));
        assert_eq!(fake.ids(), vec![1]);
    }

    #[test]
    fn the_usage_is_counted_again_from_the_rows_that_remain() {
        let fake = Fake::new();
        fake.usage.set(999);
        fake.add(1, 10, 2.0 * MIN_AGE_MS, true);
        fake.add(2, 20, 2.0 * MIN_AGE_MS, false);
        fake.add(3, 5, MIN_AGE_MS / 2.0, false);
        sweep(&fake).unwrap();
        assert_eq!(fake.usage.get(), 15, "the live row and the young row");
    }

    #[test]
    fn a_failed_index_check_deletes_nothing() {
        let fake = Fake::new();
        fake.add(1, 10, 2.0 * MIN_AGE_MS, false);
        fake.index_fails.set(true);
        assert_eq!(sweep(&fake), Err(ThumbErr::new("Error")));
        assert_eq!(fake.ids(), vec![1]);
    }

    #[test]
    fn runs_go_through_a_long_table_and_start_again_at_its_end() {
        let fake = Fake::new();
        let total = SWEEP_ROWS as u32 + 10;
        for n in 1..=total {
            fake.add(n, 1, 2.0 * MIN_AGE_MS, n % 2 == 0);
        }
        let first = sweep(&fake).unwrap();
        assert_eq!(first.scanned, SWEEP_ROWS);
        assert_eq!(fake.cursor.get(), SWEEP_ROWS as u64);
        let second = sweep(&fake).unwrap();
        assert_eq!(second.scanned, 10);
        assert_eq!(fake.cursor.get(), 0, "the end of the table resets the cursor");
        assert_eq!(first.removed + second.removed, (total as usize).div_ceil(2));
        assert!(fake.ids().iter().all(|id| id % 2 == 0));
    }

    #[test]
    fn a_run_costs_three_queries() {
        let fake = Fake::new();
        fake.add(1, 10, 2.0 * MIN_AGE_MS, false);
        sweep(&fake).unwrap();
        // The D1 side runs the scan as one query, the index check as one batch and the cleanup as one
        // batch, so the three steps are three of the 50 queries of an invocation.
        assert_eq!(fake.queries.get(), 3);
    }

    #[test]
    fn statements_stay_under_the_bound_value_limit() {
        const PARAM_LIMIT: usize = 100;
        let count = |sql: &str| sql.matches('?').count();
        assert_eq!(count(&named_sql(INDEX_ROWS_PER_STATEMENT)), INDEX_ROWS_PER_STATEMENT * 4);
        assert!(count(&named_sql(INDEX_ROWS_PER_STATEMENT)) <= PARAM_LIMIT);
        assert_eq!(count(&delete_sql(DELETE_ROWS_PER_STATEMENT)), DELETE_ROWS_PER_STATEMENT + 1);
        assert!(count(&delete_sql(DELETE_ROWS_PER_STATEMENT)) <= PARAM_LIMIT);
        assert_eq!(SWEEP_ROWS % INDEX_ROWS_PER_STATEMENT, 0);
    }

    #[test]
    fn successive_runs_take_the_shards_in_turn() {
        let shards: Vec<usize> = (0..10).map(|i| shard_for_run(f64::from(i) * RUN_MS + 17.0 * 60_000.0)).collect();
        assert_eq!(shards, vec![0, 1, 2, 3, 4, 5, 6, 7, 0, 1]);
        assert_eq!(shard_for_run(-5.0), 0);
    }
}
