//! Popularity: `GET /popular` reads one Redis ranking per repo. The cron rebuilds every ranking from
//! the events in D1.

use std::collections::HashMap;

use js_sys::Array;
use wasm_bindgen::{JsCast, JsValue};
use worker::{console_error, D1Database, Env};

use crate::config::Config;
use crate::ctx::RepoCtx;
use crate::http::{json, quote, stop, Halt, Handled};
use crate::js::{fmt_number, now_ms};
use crate::redis::{Ranked, RankingInput, Redis};

const DAY_MS: f64 = 24.0 * 60.0 * 60.0 * 1000.0;
const TOP_N: usize = 200;
const WINDOW_DAYS: f64 = 60.0;
const RETENTION_DAYS: f64 = 90.0;
const HALF_LIFE_DAYS: f64 = 14.0;
const SEND_WEIGHT: f64 = 3.0;
const VIEW_WEIGHT: f64 = 1.0;
const RANKING_TTL_SEC: u64 = 7 * 24 * 60 * 60;

/// Redis budget (Upstash free tier: 500K commands a month). A rebuild runs 8 times a day and sends
/// `REPLACE_RANKING_COMMANDS` commands per repo, all in one pipeline request (so a single
/// subrequest, well inside Workers Free's 50), so 51 repos cost about 5 x 51 x 8 x 30 = 61K
/// commands a month. D1 reads are the event count of the last 60 days, 8 times a day.
#[cfg(test)]
pub fn monthly_redis_commands(repos: usize, rebuilds_per_day: usize, days: usize) -> usize {
    crate::redis::REPLACE_RANKING_COMMANDS * repos * rebuilds_per_day * days
}

pub fn ranking_key(slug: &str) -> String {
    format!("pop:{slug}")
}

/// `GET /popular`: one Redis request. With Redis down the answer is 503, never a D1 scan.
pub async fn get_popular(ctx: &RepoCtx<'_>) -> Handled<web_sys::Response> {
    let redis = Redis::new(&ctx.cfg.upstash_url, &ctx.cfg.upstash_token);
    let Some(ranking) = redis.ranking(&ranking_key(ctx.slug), TOP_N).await else {
        return Err(stop("redisDown", 503));
    };
    Ok(json(popular_body(&ranking.items, ranking.built_at), 200, &[])?)
}

/// `{"items":[{"sha":...,"score":...}],"builtAt":...}`, with numbers as JavaScript writes them.
pub fn popular_body(items: &[Ranked], built_at: Option<f64>) -> String {
    let items: Vec<String> = items.iter().map(|r| format!("{{\"sha\":{},\"score\":{}}}", quote(&r.member), fmt_number(r.score))).collect();
    format!("{{\"items\":[{}],\"builtAt\":{}}}", items.join(","), built_at.map_or_else(|| "null".to_owned(), fmt_number))
}

#[derive(Debug)]
pub struct EventRow {
    pub repo: String,
    pub sha: String,
    pub kind: String,
    pub ts: f64,
}

/// Score = sum of weight x 0.5^(ageDays / 14) per sha, per repo. Repos and shas keep the order in
/// which they first appear, so the sums add up in the same order as in the TypeScript Worker.
pub fn score_events(rows: &[EventRow], now: f64) -> Vec<(String, Vec<(String, f64)>)> {
    let mut repos: Vec<(String, Vec<(String, f64)>)> = Vec::new();
    let mut repo_at: HashMap<&str, usize> = HashMap::new();
    let mut sha_at: HashMap<(usize, &str), usize> = HashMap::new();
    for row in rows {
        let age_days = (now - row.ts).max(0.0) / DAY_MS;
        let weight = if row.kind == "send" { SEND_WEIGHT } else { VIEW_WEIGHT };
        let score = weight * 0.5_f64.powf(age_days / HALF_LIFE_DAYS);
        let repo = *repo_at.entry(&row.repo).or_insert_with(|| {
            repos.push((row.repo.clone(), Vec::new()));
            repos.len() - 1
        });
        match sha_at.get(&(repo, row.sha.as_str())) {
            Some(&at) => repos[repo].1[at].1 += score,
            None => {
                sha_at.insert((repo, row.sha.as_str()), repos[repo].1.len());
                repos[repo].1.push((row.sha.clone(), score));
            }
        }
    }
    repos
}

/// The best `TOP_N` members, by score and then by member in byte order (lower-case hex sorts as
/// `localeCompare` does).
pub fn top_members(shas: Vec<(String, f64)>) -> Vec<Ranked> {
    let mut members: Vec<Ranked> = shas.into_iter().map(|(member, score)| Ranked { member, score }).collect();
    members.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.member.cmp(&b.member)));
    members.truncate(TOP_N);
    members
}

/// The three steps of the cron. A test store can fail any of them.
pub trait RebuildStore {
    /// Deletes events older than `before`.
    async fn prune(&self, before: f64) -> Result<(), String>;
    /// The events newer than `since`, in table order.
    async fn events_since(&self, since: f64) -> Result<Vec<EventRow>, String>;
    /// Replaces the rankings and gives the number of failed Redis commands.
    async fn replace(&self, rankings: &[RankingInput], now: f64) -> usize;
}

/// Cron: drop events older than 90 days, then rebuild every repo's ranking from the rest. The
/// delete runs first and does not depend on the scoring, so a rebuild that fails or runs out of
/// CPU never stops the table from shrinking. A failed delete does not stop the rebuild, and its
/// error is returned at the end.
pub async fn rebuild<S: RebuildStore>(store: &S, now: f64) -> Result<(), String> {
    let pruned = store.prune(now - RETENTION_DAYS * DAY_MS).await;
    let rows = store.events_since(now - WINDOW_DAYS * DAY_MS).await?;
    let rankings: Vec<RankingInput> =
        score_events(&rows, now).into_iter().map(|(repo, shas)| RankingInput { key: ranking_key(&repo), members: top_members(shas) }).collect();
    if !rankings.is_empty() {
        let failed = store.replace(&rankings, now).await;
        if failed > 0 {
            console_error!("popular.rebuild.failed {failed}");
        }
    }
    pruned
}

/// One event row from `raw()`: `[repo, sha, kind, ts]`. Reading by position skips the property
/// lookups and the object copy of `all()`.
fn event_row(row: &JsValue) -> Option<EventRow> {
    let row: &Array = row.dyn_ref()?;
    Some(EventRow { repo: row.get(0).as_string()?, sha: row.get(1).as_string()?, kind: row.get(2).as_string()?, ts: row.get(3).as_f64()? })
}

struct LiveRebuild {
    db: D1Database,
    redis: Redis,
}

impl RebuildStore for LiveRebuild {
    async fn prune(&self, before: f64) -> Result<(), String> {
        let statement = self.db.prepare("DELETE FROM events WHERE ts < ?").bind(&[JsValue::from_f64(before)]).map_err(|_| "Error".to_owned())?;
        statement.run().await.map(|_| ()).map_err(|_| "Error".to_owned())
    }

    async fn events_since(&self, since: f64) -> Result<Vec<EventRow>, String> {
        let statement = self.db.prepare("SELECT repo, sha, kind, ts FROM events WHERE ts > ?").bind(&[JsValue::from_f64(since)]).map_err(|_| "Error".to_owned())?;
        let rows = statement.raw_js_value().await.map_err(|_| "Error".to_owned())?;
        rows.iter().map(|row| event_row(row).ok_or_else(|| "badEventRow".to_owned())).collect()
    }

    async fn replace(&self, rankings: &[RankingInput], now: f64) -> usize {
        self.redis.replace_rankings(rankings, now, RANKING_TTL_SEC).await
    }
}

pub async fn rebuild_popular(env: &Env) -> Handled<()> {
    let cfg = Config::load(env)?;
    let store = LiveRebuild { db: env.d1("DB")?, redis: Redis::new(&cfg.upstash_url, &cfg.upstash_token) };
    rebuild(&store, now_ms()).await.map_err(Halt::failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000_000.0;

    fn row(repo: &str, sha: &str, kind: &str, age_days: f64) -> EventRow {
        EventRow { repo: repo.into(), sha: sha.into(), kind: kind.into(), ts: NOW - age_days * DAY_MS }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_send_weighs_three_views_and_decay_halves_every_14_days() {
        let scored = score_events(&[row("o/r", "a", "send", 0.0), row("o/r", "b", "view", 0.0), row("o/r", "b", "view", 0.0), row("o/r", "c", "view", 14.0)], NOW);
        assert_eq!(scored.len(), 1);
        let shas = &scored[0].1;
        assert_eq!(shas.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert!(close(shas[0].1, 3.0));
        assert!(close(shas[1].1, 2.0));
        assert!(close(shas[2].1, 0.5));
    }

    #[test]
    fn an_event_in_the_future_counts_as_age_zero() {
        let scored = score_events(&[row("o/r", "a", "view", -1.0)], NOW);
        assert!(close(scored[0].1[0].1, 1.0));
    }

    #[test]
    fn repos_and_shas_keep_first_seen_order() {
        let scored = score_events(&[row("o/b", "x", "view", 0.0), row("o/a", "y", "view", 0.0), row("o/b", "z", "view", 0.0), row("o/b", "x", "view", 0.0)], NOW);
        assert_eq!(scored.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(), vec!["o/b", "o/a"]);
        assert_eq!(scored[0].1.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), vec!["x", "z"]);
        assert!(close(scored[0].1[0].1, 2.0));
    }

    #[test]
    fn ties_sort_by_member_and_the_list_keeps_the_top_200() {
        let tied = top_members(vec![("b".into(), 1.0), ("a".into(), 1.0), ("c".into(), 2.0)]);
        assert_eq!(tied.iter().map(|m| m.member.as_str()).collect::<Vec<_>>(), vec!["c", "a", "b"]);
        let many: Vec<(String, f64)> = (0..250).map(|i| (format!("{i:040x}"), 1.0)).collect();
        let top = top_members(many);
        assert_eq!(top.len(), TOP_N);
        assert_eq!(top[0].member, format!("{:040x}", 0));
    }

    #[test]
    fn the_rebuild_stays_inside_the_upstash_free_tier_with_51_repos() {
        // 5 commands x 51 repos x 8 rebuilds a day x 30 days.
        assert_eq!(monthly_redis_commands(51, 8, 30), 61_200);
        assert!(monthly_redis_commands(51, 8, 30) < 500_000);
    }

    #[test]
    fn the_reply_has_javascript_numbers_in_contract_order() {
        let items = vec![Ranked { member: "aa".into(), score: 3.0 }, Ranked { member: "bb".into(), score: 0.5 }];
        assert_eq!(popular_body(&items, Some(1_700_000_000_000.0)), r#"{"items":[{"sha":"aa","score":3},{"sha":"bb","score":0.5}],"builtAt":1700000000000}"#);
        assert_eq!(popular_body(&[], None), r#"{"items":[],"builtAt":null}"#);
    }

    use futures_executor::block_on;
    use std::cell::{Cell, RefCell};

    struct Fake {
        calls: RefCell<Vec<&'static str>>,
        pruned_before: Cell<f64>,
        prune_fails: bool,
        read_fails: bool,
        events: Vec<(&'static str, &'static str, &'static str, f64)>,
        replaced: RefCell<Vec<(String, usize)>>,
    }

    impl Fake {
        fn new(events: Vec<(&'static str, &'static str, &'static str, f64)>) -> Self {
            Self { calls: RefCell::new(Vec::new()), pruned_before: Cell::new(0.0), prune_fails: false, read_fails: false, events, replaced: RefCell::new(Vec::new()) }
        }
    }

    impl RebuildStore for Fake {
        async fn prune(&self, before: f64) -> Result<(), String> {
            self.calls.borrow_mut().push("prune");
            self.pruned_before.set(before);
            if self.prune_fails {
                return Err("Error".to_owned());
            }
            Ok(())
        }

        async fn events_since(&self, since: f64) -> Result<Vec<EventRow>, String> {
            self.calls.borrow_mut().push("read");
            if self.read_fails {
                return Err("Error".to_owned());
            }
            Ok(self.events.iter().filter(|e| e.3 > since).map(|e| EventRow { repo: e.0.into(), sha: e.1.into(), kind: e.2.into(), ts: e.3 }).collect())
        }

        async fn replace(&self, rankings: &[RankingInput], _now: f64) -> usize {
            self.calls.borrow_mut().push("replace");
            self.replaced.borrow_mut().extend(rankings.iter().map(|r| (r.key.clone(), r.members.len())));
            0
        }
    }

    #[test]
    fn the_old_events_are_deleted_before_anything_is_read() {
        let fake = Fake::new(vec![("o/r", "a", "view", NOW - DAY_MS)]);
        assert_eq!(block_on(rebuild(&fake, NOW)), Ok(()));
        assert_eq!(*fake.calls.borrow(), vec!["prune", "read", "replace"]);
        assert_eq!(fake.pruned_before.get(), NOW - RETENTION_DAYS * DAY_MS);
        assert_eq!(*fake.replaced.borrow(), vec![("pop:o/r".to_owned(), 1)]);
    }

    #[test]
    fn the_delete_has_run_when_the_rebuild_fails_afterwards() {
        let mut fake = Fake::new(Vec::new());
        fake.read_fails = true;
        assert_eq!(block_on(rebuild(&fake, NOW)), Err("Error".to_owned()));
        assert_eq!(*fake.calls.borrow(), vec!["prune", "read"]);
    }

    #[test]
    fn a_failed_delete_does_not_stop_the_rebuild_and_is_reported() {
        let mut fake = Fake::new(vec![("o/r", "a", "send", NOW)]);
        fake.prune_fails = true;
        assert_eq!(block_on(rebuild(&fake, NOW)), Err("Error".to_owned()));
        assert_eq!(*fake.calls.borrow(), vec!["prune", "read", "replace"]);
    }

    #[test]
    fn no_events_in_the_window_means_no_redis_call() {
        let fake = Fake::new(vec![("o/r", "a", "view", NOW - 70.0 * DAY_MS)]);
        assert_eq!(block_on(rebuild(&fake, NOW)), Ok(()));
        assert_eq!(*fake.calls.borrow(), vec!["prune", "read"]);
    }
}
