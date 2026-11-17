//! The team backend: a small Cloudflare Worker that keeps the team's shared cache and counts which
//! assets the team uses.
//!
//! Nothing here may slow local work. Every call has a deadline, a run of failures opens a breaker,
//! and every caller has a local fallback, so team features switch off quietly while the Worker is
//! out of reach and come back on their own. The page sees only [`TeamStatus`].

mod client;
mod events;
pub mod merge;
pub mod pack;
pub mod popular;
pub mod prefetch;
pub mod remote_cache;

use std::path::PathBuf;
use std::sync::Arc;

use asset_hub_core::github::client::AccountTokens;

pub use client::{Area, Reply, TeamClient, TeamError, TeamOptions};
pub use events::{EventQueue, TeamEventKind};

pub use crate::dto::TeamStatus;

/// The Worker the app talks to.
pub const TEAM_API: &str = "https://asset-hub-api.asset-hub-team.workers.dev";

/// The cache namespace sent as `x-asset-hub-schema`.
pub const SCHEMA: u32 = 1;

/// The base URL of the team API, or none when the client is off. A release build uses `TEAM_API`
/// alone. A development build also reads `ASSETHUB_BACKEND_URL`: an empty value turns the client
/// off, and a loopback `http` address lets a test stand in for the Worker.
pub fn api_url() -> Option<String> {
    resolve_url(cfg!(debug_assertions), std::env::var("ASSETHUB_BACKEND_URL").ok().as_deref())
}

fn resolve_url(debug: bool, requested: Option<&str>) -> Option<String> {
    let url = match requested {
        Some(url) if debug => url.trim(),
        _ => TEAM_API,
    };
    if url.is_empty() {
        return None;
    }
    checked(url, debug)
}

/// The URL without a trailing slash when the token may be sent to it: `https`, or in a debug
/// build `http` to the loopback address. Nothing else gets a token.
fn checked(url: &str, debug: bool) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let allowed = match parsed.scheme() {
        "https" => true,
        "http" => debug && matches!(parsed.host_str(), Some("127.0.0.1" | "localhost")),
        _ => false,
    };
    let plain = parsed.username().is_empty() && parsed.password().is_none() && parsed.query().is_none() && parsed.fragment().is_none() && matches!(parsed.path(), "" | "/");
    (allowed && plain).then(|| url.trim_end_matches('/').to_owned())
}

/// Whether this build may write team data. A development build reads the team's data but writes
/// only when `ASSETHUB_INDEX_CONTRIBUTE` is set.
pub fn writes_enabled() -> bool {
    !cfg!(debug_assertions) || std::env::var_os("ASSETHUB_INDEX_CONTRIBUTE").is_some_and(|v| !v.is_empty())
}

/// The team work of one session: the client and the queue of events it sends.
pub struct Team {
    pub client: Arc<TeamClient>,
    pub events: Arc<EventQueue>,
    pub popular: Arc<popular::Popularity>,
}

impl Team {
    /// `events_file` is where unsent events are kept (it is asked for each time, as the cache folder can change).
    pub fn new(url: Option<String>, login: &str, tokens: Arc<AccountTokens>, events_file: Box<dyn Fn() -> PathBuf + Send + Sync>, on_status: Arc<dyn Fn(TeamStatus) + Send + Sync>) -> Self {
        let client = Arc::new(TeamClient::new(url, tokens, TeamOptions::default(), on_status));
        let events = Arc::new(EventQueue::new(client.clone(), login, events_file));
        let popular = Arc::new(popular::Popularity::new(client.clone()));
        Self { client, events, popular }
    }

    /// Starts the periodic send of events. Safe to call again.
    pub fn start(&self) {
        if self.client.status() != TeamStatus::Disabled {
            EventQueue::start(&self.events);
        }
    }

    /// The session ended (signed out, or another account's took over): no more requests.
    pub fn end(&self) {
        self.events.stop();
        self.client.end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_release_build_uses_only_the_https_constant() {
        assert_eq!(resolve_url(false, None).as_deref(), Some(TEAM_API));
        assert_eq!(resolve_url(false, Some("http://127.0.0.1:9")).as_deref(), Some(TEAM_API));
        assert_eq!(resolve_url(false, Some("")).as_deref(), Some(TEAM_API));
        assert_eq!(checked("http://127.0.0.1:9", false), None);
    }

    #[test]
    fn a_development_build_may_turn_the_client_off_or_use_loopback() {
        assert_eq!(resolve_url(true, None).as_deref(), Some(TEAM_API));
        assert_eq!(resolve_url(true, Some("")), None);
        assert_eq!(resolve_url(true, Some("  ")), None);
        assert_eq!(resolve_url(true, Some("http://127.0.0.1:8787/")).as_deref(), Some("http://127.0.0.1:8787"));
        assert_eq!(resolve_url(true, Some("http://localhost:8787")).as_deref(), Some("http://localhost:8787"));
    }

    #[test]
    fn no_other_address_gets_a_token() {
        for url in ["http://example.com", "http://127.0.0.1.evil.com", "ftp://127.0.0.1", "https://user:pw@example.com", "https://example.com/path", "https://example.com/?a=1", "not a url"] {
            assert_eq!(checked(url, true), None, "{url}");
        }
        assert_eq!(checked("https://example.com", false).as_deref(), Some("https://example.com"));
    }
}
