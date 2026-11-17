//! Which assets of a repo the team uses most, as scores by blob SHA.
//!
//! The scores come from one cheap request per repo, are kept for ten minutes, and are never worth
//! a wait: with the Worker out of reach or slow, the answer is an empty map at once and the caller
//! sorts the way it did before.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::{Area, TeamClient};

/// How long the scores of a repo stay fresh.
const KEEP: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
struct Ranking {
    items: Vec<Item>,
}

#[derive(Deserialize)]
struct Item {
    sha: String,
    score: f64,
}

struct Cached {
    at: Instant,
    scores: Arc<HashMap<String, f64>>,
}

pub struct Popularity {
    client: Arc<TeamClient>,
    keep: Duration,
    cache: Mutex<HashMap<String, Cached>>,
    /// One request per repo at a time: the others wait for it, then read the cache.
    flights: Mutex<HashMap<String, Arc<Flight>>>,
}

/// The request of one repo that may be in the air.
#[derive(Default)]
struct Flight {
    gate: tokio::sync::Mutex<()>,
    /// How many requests of this repo failed. A caller that waited while one failed does not ask
    /// again: the Worker just showed that it does not answer, and the caller has waited enough.
    failures: AtomicU64,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn is_sha(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

impl Popularity {
    pub fn new(client: Arc<TeamClient>) -> Self {
        Self { client, keep: KEEP, cache: Mutex::default(), flights: Mutex::default() }
    }

    /// Tests: scores stay fresh this long.
    #[cfg(test)]
    fn with_keep(mut self, keep: Duration) -> Self {
        self.keep = keep;
        self
    }

    /// Tests: keeps these scores for `repo`.
    #[cfg(test)]
    pub(super) fn seed(&self, repo: &str, scores: HashMap<String, f64>) {
        locked(&self.cache).insert(repo.to_owned(), Cached { at: Instant::now(), scores: Arc::new(scores) });
    }

    fn fresh(&self, repo: &str) -> Option<Arc<HashMap<String, f64>>> {
        locked(&self.cache).get(repo).filter(|cached| cached.at.elapsed() < self.keep).map(|cached| cached.scores.clone())
    }

    /// The team's score of each blob SHA of `repo` that has one. Empty when the Worker has nothing,
    /// does not answer within the interactive budget, or the client is off. A failure is not kept.
    pub async fn scores(&self, repo: &str) -> Arc<HashMap<String, f64>> {
        if let Some(scores) = self.fresh(repo) {
            return scores;
        }
        let flight = locked(&self.flights).entry(repo.to_owned()).or_default().clone();
        let failures_before = flight.failures.load(Ordering::SeqCst);
        let _one_request = flight.gate.lock().await;
        // Another call may have filled the cache while this one waited.
        if let Some(scores) = self.fresh(repo) {
            return scores;
        }
        // Or its request failed: share that answer, so no call waits for more than one budget.
        if flight.failures.load(Ordering::SeqCst) != failures_before {
            return Arc::default();
        }
        let reply = self.client.get(repo, "/popular", Area::Popular, self.client.options().interactive, None).await;
        let Some(ranking) = reply.ok().and_then(|reply| serde_json::from_slice::<Ranking>(&reply.body).ok()) else {
            flight.failures.fetch_add(1, Ordering::SeqCst);
            return Arc::default();
        };
        let scores: HashMap<String, f64> = ranking.items.into_iter().filter(|item| is_sha(&item.sha) && item.score.is_finite() && item.score > 0.0).map(|item| (item.sha, item.score)).collect();
        let scores = Arc::new(scores);
        locked(&self.cache).insert(repo.to_owned(), Cached { at: Instant::now(), scores: scores.clone() });
        scores
    }

    /// The `n` highest-scored SHAs of `repo`, from what is kept (however old). Nothing is requested.
    pub fn top(&self, repo: &str, n: usize) -> Vec<String> {
        let cache = locked(&self.cache);
        let Some(cached) = cache.get(repo) else { return Vec::new() };
        let mut ranked: Vec<(&String, &f64)> = cached.scores.iter().collect();
        ranked.sort_by(|a, b| b.1.total_cmp(a.1).then_with(|| a.0.cmp(b.0)));
        ranked.into_iter().take(n).map(|(sha, _)| sha.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::team::TeamOptions;
    use crate::test_server::{serve_full, Reply};
    use asset_hub_core::github::client::AccountTokens;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn sha(n: u8) -> String {
        format!("{n:02x}").repeat(20)
    }

    fn client(url: &str) -> Arc<TeamClient> {
        let options = TeamOptions { jitter: false, ..TeamOptions::default() };
        Arc::new(TeamClient::new(Some(url.to_owned()), Arc::new(AccountTokens::new("t", Vec::new())), options, Arc::new(|_| {})))
    }

    fn body() -> String {
        format!(r#"{{"items":[{{"sha":"{}","score":2.5}},{{"sha":"{}","score":9}}],"builtAt":1}}"#, sha(1), sha(2))
    }

    async fn counting(reply: impl Fn() -> Reply + Send + Sync + 'static) -> (String, Arc<AtomicUsize>) {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let url = serve_full(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            reply()
        })
        .await;
        (url, count)
    }

    #[tokio::test]
    async fn scores_cached_10_min() {
        let (url, count) = counting(|| Reply::json(200, &body())).await;
        let popular = Popularity::new(client(&url));
        assert_eq!(popular.scores("o/r").await[&sha(2)], 9.0);
        assert_eq!(popular.scores("o/r").await.len(), 2);
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert_eq!(popular.top("o/r", 1), [sha(2)]);
        assert!(popular.top("o/other", 5).is_empty());
        // Past its time, the scores are read again.
        let stale = Popularity::new(client(&url)).with_keep(Duration::ZERO);
        stale.scores("o/r").await;
        stale.scores("o/r").await;
        assert_eq!(count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn redis_down_is_empty_and_not_cached() {
        let (url, count) = counting(|| Reply::json(503, r#"{"error":"redisDown"}"#)).await;
        let popular = Popularity::new(client(&url));
        assert!(popular.scores("o/r").await.is_empty());
        assert!(popular.scores("o/r").await.is_empty());
        assert_eq!(count.load(Ordering::SeqCst), 2);
        assert!(popular.top("o/r", 5).is_empty());
    }

    #[tokio::test]
    async fn concurrent_calls_share_one_request() {
        let count = Arc::new(AtomicUsize::new(0));
        let seen = count.clone();
        let url = serve_full(move |_| {
            seen.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(150));
            Reply::json(200, &body())
        })
        .await;
        let popular = Arc::new(Popularity::new(client(&url)));
        let calls: Vec<_> = (0..5).map(|_| tokio::spawn({
            let popular = popular.clone();
            async move { popular.scores("o/r").await.len() }
        })).collect();
        for call in calls {
            assert_eq!(call.await.unwrap(), 2);
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn hang_returns_empty_within_budget() {
        // A server that takes the connection and never answers.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((stream, _)) = listener.accept().await {
                held.push(stream);
            }
        });
        let popular = Popularity::new(client(&url));
        let begin = Instant::now();
        assert!(popular.scores("o/r").await.is_empty());
        assert!(begin.elapsed() < Duration::from_millis(1500) + Duration::from_millis(500), "took {:?}", begin.elapsed());
    }

    #[tokio::test]
    async fn concurrent_calls_on_a_hanging_worker_all_return_within_one_budget() {
        let accepted = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn({
            let accepted = accepted.clone();
            async move {
                let mut held = Vec::new();
                while let Ok((stream, _)) = listener.accept().await {
                    accepted.fetch_add(1, Ordering::SeqCst);
                    held.push(stream);
                }
            }
        });
        let popular = Arc::new(Popularity::new(client(&url)));
        let begin = Instant::now();
        let calls: Vec<_> = (0..3).map(|_| tokio::spawn({
            let popular = popular.clone();
            async move { popular.scores("o/r").await.len() }
        })).collect();
        for call in calls {
            assert_eq!(call.await.unwrap(), 0);
        }
        assert!(begin.elapsed() < Duration::from_millis(1500) + Duration::from_millis(500), "took {:?}", begin.elapsed());
        assert_eq!(accepted.load(Ordering::SeqCst), 1, "one request for all of them");
    }

    #[tokio::test]
    async fn invalid_items_dropped() {
        let text = format!(
            r#"{{"items":[{{"sha":"{}","score":1}},{{"sha":"nope","score":1}},{{"sha":"{}","score":0}},{{"sha":"{}","score":-2}},{{"sha":"{}","score":3}}]}}"#,
            sha(1),
            sha(2),
            sha(3),
            "AB".repeat(20)
        );
        let (url, _) = counting(move || Reply::json(200, &text)).await;
        let scores = Popularity::new(client(&url)).scores("o/r").await;
        assert_eq!(scores.keys().collect::<Vec<_>>(), [&sha(1)]);
    }

    #[tokio::test]
    async fn a_client_that_is_off_answers_empty() {
        let popular = Popularity::new(Arc::new(TeamClient::new(None, Arc::new(AccountTokens::new("t", Vec::new())), TeamOptions::default(), Arc::new(|_| {}))));
        assert!(popular.scores("o/r").await.is_empty());
    }
}
