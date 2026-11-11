//! The signed-in GitHub session: the asset source over the account's token, and the work that
//! runs while the user looks at a branch.
//!
//! A session is made on first use after sign-in and replaced when the token changes. The token
//! stays in this process; the page never sees it.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use asset_hub_core::github::client::{AccountTokens, GitHubClient};
use asset_hub_core::github::source::{GitHubAssetSource, GitHubSourceOptions};
use asset_hub_core::handlers::create_default_registry;
use asset_hub_core::registry::HandlerRegistry;

use crate::cache::Caches;
use crate::catalog_service::{CatalogConfig, CatalogService};
use crate::dto::{CatalogStatus, SharedIndexStatus};
use crate::git_tree::GitTree;
use crate::github_net::{CacheBridge, EtagFetch, GitHooks};
use crate::http::API;
use crate::index_repo::{IndexRepo, INDEX_REPO};
use crate::ipc::{AppErrorCode, IpcError};
use crate::shared_index::{Deps, CHECK_EVERY};
use crate::shared_index_service::{SharedIndexConfig, SharedIndexService};

pub type Source = GitHubAssetSource<EtagFetch>;

/// The repo list is kept this long (catalog scans ask for it often).
const REPOS_KEPT: Duration = Duration::from_secs(10 * 60);

/// Where the services of a session tell the page about their progress.
#[derive(Clone)]
pub struct Emitters {
    pub catalog: Arc<dyn Fn(CatalogStatus) + Send + Sync>,
    pub shared_index: Arc<dyn Fn(SharedIndexStatus) + Send + Sync>,
}

pub struct Session {
    pub source: Arc<Source>,
    pub registry: Arc<HandlerRegistry>,
    /// The background index of every branch.
    pub catalog: Arc<CatalogService>,
    /// The team's shared index.
    pub shared_index: Arc<SharedIndexService>,
    /// The active account.
    pub login: String,
    /// Login and token of every signed-in account, the active one first.
    accounts: Vec<(String, String)>,
    repos: Mutex<Option<(Instant, Vec<asset_hub_core::types::RepoRef>)>>,
}

impl Session {
    /// The active account's token.
    pub fn token(&self) -> &str {
        &self.accounts[0].1
    }

    /// Ends the background work of the session (signed out, or replaced by another account's).
    fn end(&self) {
        self.catalog.stop();
        self.shared_index.stop();
    }

    /// The repos the account reads, from the last answer when it is recent.
    pub async fn repos(&self) -> Result<Vec<asset_hub_core::types::RepoRef>, IpcError> {
        if let Some((at, list)) = self.repos.lock().ok().and_then(|r| r.clone()) {
            if at.elapsed() < REPOS_KEPT {
                return Ok(list);
            }
        }
        let list = self.source.list_repos().await.map_err(crate::github_net::ipc_error)?;
        if let Ok(mut kept) = self.repos.lock() {
            *kept = Some((Instant::now(), list.clone()));
        }
        Ok(list)
    }
}

/// Counts the requests for what the user is looking at; background scans wait while any run.
#[derive(Default, Clone)]
pub struct Foreground(Arc<AtomicUsize>);

pub struct ForegroundGuard(Arc<AtomicUsize>);

impl Drop for ForegroundGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Foreground {
    pub fn enter(&self) -> ForegroundGuard {
        self.0.fetch_add(1, Ordering::SeqCst);
        ForegroundGuard(self.0.clone())
    }

    pub fn busy(&self) -> bool {
        self.0.load(Ordering::SeqCst) > 0
    }

    /// Waits while the user waits on something (grid thumbnails step aside).
    pub async fn wait_idle(&self) {
        while self.busy() {
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    }
}

pub struct Hub {
    current: Mutex<Option<Arc<Session>>>,
    pub foreground: Foreground,
    registry: Arc<HandlerRegistry>,
    git: Arc<GitTree>,
    api: String,
    emitters: OnceLock<Emitters>,
    /// Where the shared index repo is fetched from; the team's repo on GitHub unless a test says otherwise.
    index_remote: Option<String>,
    /// Whether the shared index may be written to its GitHub repo without being asked.
    auto_contribute: bool,
}

impl Hub {
    pub fn new(caches: &Caches) -> Self {
        Self::with_api(caches, API)
    }

    pub fn with_api(caches: &Caches, api: &str) -> Self {
        Self {
            current: Mutex::new(None),
            foreground: Foreground::default(),
            registry: Arc::new(create_default_registry()),
            git: GitTree::new(caches.config.clone()),
            api: api.to_owned(),
            emitters: OnceLock::new(),
            index_remote: None,
            auto_contribute: crate::usage_env::auto_contribute(),
        }
    }

    /// Tests: the shared index repo is read from `url` and may be written to without being asked.
    #[cfg(test)]
    pub fn with_index_remote(mut self, url: String) -> Self {
        self.index_remote = Some(url);
        self.auto_contribute = true;
        self
    }

    /// Where the progress of the catalog and the shared index goes (the page, once the app has a window).
    pub fn set_emitters(&self, emitters: Emitters) {
        let _ = self.emitters.set(emitters);
    }

    pub fn registry(&self) -> &Arc<HandlerRegistry> {
        &self.registry
    }

    /// The session of a single signed-in account; `account` is its login and token.
    pub fn session(&self, account: Option<(String, String)>, caches: &Arc<Caches>) -> Result<Arc<Session>, IpcError> {
        self.session_with(account.into_iter().collect(), caches)
    }

    /// The session of the signed-in accounts (login and token, the active account first).
    pub fn session_with(&self, accounts: Vec<(String, String)>, caches: &Arc<Caches>) -> Result<Arc<Session>, IpcError> {
        let Some((login, token)) = accounts.first().cloned() else {
            self.clear();
            return Err(IpcError::new(AppErrorCode::Unauthorized, "Not signed in"));
        };
        let mut current = self.current.lock().map_err(|_| IpcError::unknown("the session is unavailable"))?;
        if let Some(session) = current.as_ref().filter(|s| s.accounts == accounts) {
            return Ok(session.clone());
        }
        let tokens = Arc::new(AccountTokens::new(token, accounts.iter().skip(1).map(|(_, t)| t.clone()).collect()));
        let hooks = GitHooks { git: self.git.clone(), registry: self.registry.clone(), tokens: tokens.clone() };
        let bridge = Arc::new(CacheBridge(caches.clone()));
        let options = GitHubSourceOptions { hooks: Arc::new(hooks), cache: Some(bridge.clone()), blob_cache: Some(bridge), registry: Some(self.registry.clone()) };
        let client = GitHubClient::with_tokens(tokens, EtagFetch::new(caches.clone(), self.api.clone())).with_api_base(self.api.clone());
        let source = Arc::new(GitHubAssetSource::new(client, options));
        let (catalog, shared_index) = self.services(&source, &accounts, caches);
        let session = Arc::new(Session { source, registry: self.registry.clone(), catalog, shared_index, login, accounts, repos: Mutex::new(None) });
        if let Some(old) = current.replace(session.clone()) {
            old.end();
        }
        Ok(session)
    }

    /// The catalog and the shared index of one session.
    fn services(&self, source: &Arc<Source>, accounts: &[(String, String)], caches: &Arc<Caches>) -> (Arc<CatalogService>, Arc<SharedIndexService>) {
        let emitters = self.emitters.get().cloned();
        let types = self.registry.all().iter().map(|h| h.id).collect::<Vec<_>>().join(",");
        let (is_asset, type_of) = (self.registry.clone(), self.registry.clone());
        let config = caches.config.clone();
        let foreground = self.foreground.clone();
        let catalog_sink = emitters.clone();
        let catalog = CatalogService::new(
            source.clone(),
            CatalogConfig::new(
                Box::new({
                    let config = config.clone();
                    move || config.dir().join("catalog")
                }),
                Arc::new(move |entry| is_asset.classify(entry).is_some()),
                Arc::new(move |path| {
                    let entry = asset_hub_core::types::TreeEntry { path: path.to_owned(), kind: asset_hub_core::types::EntryType::Blob, sha: String::new(), size: None, lfs: None, lfs_size: None, lfs_oid: None };
                    type_of.classify(&entry).map(|h| h.id.to_owned())
                }),
                Box::new(move || foreground.busy()),
                types.clone(),
                Box::new(move |status| {
                    if let Some(sink) = &catalog_sink {
                        (sink.catalog)(status);
                    }
                }),
            ),
        );
        let url = self.index_remote.clone().unwrap_or_else(|| format!("https://github.com/{INDEX_REPO}.git"));
        let repo = IndexRepo::with_accounts(config.dir().join("shared-index-repo"), accounts.to_vec(), INDEX_REPO.to_owned(), url, Some(self.api.clone()));
        let shared_index = SharedIndexService::new(
            Deps { catalog: catalog.clone(), facts: source.clone(), caches: caches.clone() },
            SharedIndexConfig {
                types,
                login: Some(accounts[0].0.clone()),
                dir: Box::new({
                    let config = config.clone();
                    move || config.shared_index_dir()
                }),
                seen_file: Box::new(move || config.dir().join("shared-index.json")),
                repo: Some(repo),
                emit: Box::new(move |status| {
                    if let Some(sink) = &emitters {
                        (sink.shared_index)(status);
                    }
                }),
                auto_contribute: self.auto_contribute,
            },
        );
        (catalog, shared_index)
    }

    /// Starts the work that follows a sign-in: the shared index is looked at now and every 30 minutes
    /// (a newer one is taken in). Safe to call again; it starts once per session.
    pub fn begin(&self, session: &Session) {
        session.shared_index.start_checks(CHECK_EVERY);
    }

    /// Signed out: the session goes.
    pub fn clear(&self) {
        if let Some(old) = self.current.lock().ok().and_then(|mut current| current.take()) {
            old.end();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheConfig;
    use crate::test_server::{serve, Reply};
    use std::collections::HashMap;

    type Seen = Arc<Mutex<HashMap<String, Vec<Option<String>>>>>;

    async fn fake_github(seen: Seen) -> String {
        serve(move |(path, headers)| {
            seen.lock().unwrap().entry(path.clone()).or_default().push(headers.get("if-none-match").cloned());
            assert_eq!(headers.get("authorization").map(String::as_str), Some("Bearer dummy-token"));
            let etag = vec![("ETag".to_string(), "\"e\"".to_string()), ("Content-Type".to_string(), "application/json".to_string())];
            let tag = headers.get("if-none-match").is_some();
            let reply = |body: &str| if tag { Reply { status: 304, headers: etag.clone(), body: Vec::new() } } else { Reply { status: 200, headers: etag.clone(), body: body.as_bytes().to_vec() } };
            match path.as_str() {
                p if p.starts_with("/user/repos") => reply(r#"[{"full_name":"o/r","default_branch":"main"}]"#),
                "/repos/o/r/git/ref/heads/main" => reply(r#"{"object":{"sha":"c1"}}"#),
                p if p.starts_with("/repos/o/r/git/trees/c1") => reply(r#"{"truncated":false,"tree":[{"path":"Assets/a.fbx","type":"blob","sha":"b1","size":5},{"path":"Assets/a.fbx.meta","type":"blob","sha":"b2","size":9}]}"#),
                "/repos/o/r/git/blobs/b1" => Reply::bytes(b"hello".to_vec()),
                _ => Reply::json(404, r#"{"message":"Not Found"}"#),
            }
        })
        .await
    }

    #[tokio::test]
    async fn a_session_lists_repos_trees_and_files_and_revalidates_with_etags() {
        let dir = tempfile::tempdir().unwrap();
        let caches = Arc::new(Caches::new(CacheConfig::new(dir.path().to_owned(), 0)));
        let seen: Seen = Arc::default();
        let base = fake_github(seen.clone()).await;
        let hub = Hub::with_api(&caches, &base);
        let session = hub.session(Some(("octo".into(), "dummy-token".into())), &caches).unwrap();
        assert_eq!(session.repos().await.unwrap()[0].full_name, "o/r");
        // The kept list answers for ten minutes, and the source asks again with the ETag.
        session.repos().await.unwrap();
        assert_eq!(session.source.list_repos().await.unwrap().len(), 1);
        let listed = seen.lock().unwrap()["/user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member"].clone();
        assert_eq!(listed, [None, Some("\"e\"".to_string())]);

        let tree = session.source.list_tree("o/r", "main").await.unwrap();
        assert_eq!(tree.len(), 2);
        assert_eq!(&session.source.read_file("o/r", "main", "Assets/a.fbx", true).await.unwrap()[..], b"hello");
        // The second read comes from the blob cache.
        session.source.read_file("o/r", "main", "Assets/a.fbx", true).await.unwrap();
        assert_eq!(seen.lock().unwrap()["/repos/o/r/git/blobs/b1"].len(), 1);
        let missing = session.source.read_file("o/r", "main", "Assets/none.fbx", true).await.unwrap_err();
        assert_eq!(missing.code, asset_hub_core::errors::SourceErrorCode::NotFound);
    }

    #[tokio::test]
    async fn with_two_accounts_repos_are_merged_and_a_refused_repo_is_read_with_the_other_account() {
        let dir = tempfile::tempdir().unwrap();
        let caches = Arc::new(Caches::new(CacheConfig::new(dir.path().to_owned(), 0)));
        let seen: Arc<Mutex<Vec<(String, String)>>> = Arc::default();
        let log = seen.clone();
        let base = serve(move |(path, headers)| {
            let token = headers.get("authorization").and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("").to_owned();
            log.lock().unwrap().push((path.clone(), token.clone()));
            match (path.as_str(), token.as_str()) {
                (p, "t-octo") if p.starts_with("/user/repos") => Reply::json(200, r#"[{"full_name":"o/mine","default_branch":"main"},{"full_name":"o/shared","default_branch":"main"}]"#),
                (p, "t-work") if p.starts_with("/user/repos") => Reply::json(200, r#"[{"full_name":"o/shared","default_branch":"main"},{"full_name":"w/work","default_branch":"main"}]"#),
                ("/repos/w/work/git/ref/heads/main", "t-work") => Reply::json(200, r#"{"object":{"sha":"c1"}}"#),
                ("/repos/hidden/r/git/ref/heads/main", "t-work") => Reply::json(200, r#"{"object":{"sha":"c2"}}"#),
                _ => Reply::json(404, r#"{"message":"Not Found"}"#),
            }
        })
        .await;
        let hub = Hub::with_api(&caches, &base);
        let accounts = vec![("octo".to_owned(), "t-octo".to_owned()), ("work".to_owned(), "t-work".to_owned())];
        let session = hub.session_with(accounts.clone(), &caches).unwrap();
        assert!(Arc::ptr_eq(&session, &hub.session_with(accounts, &caches).unwrap()));
        assert_eq!((session.login.as_str(), session.token()), ("octo", "t-octo"));
        let names: Vec<String> = session.repos().await.unwrap().into_iter().map(|r| r.full_name).collect();
        assert_eq!(names, ["o/mine", "o/shared", "w/work"]);

        // Known to the second account only: it is used straight away.
        assert_eq!(session.source.added_since("w/work", "main", "").await.unwrap().head, "c1");
        // Not in any listing: the active account is refused (404) and the other is tried once.
        assert_eq!(session.source.added_since("hidden/r", "main", "").await.unwrap().head, "c2");
        let requests = seen.lock().unwrap();
        let on = |path: &str| requests.iter().filter(|(p, _)| p == path).map(|(_, t)| t.as_str()).collect::<Vec<_>>();
        assert_eq!(on("/repos/w/work/git/ref/heads/main"), ["t-work"]);
        assert_eq!(on("/repos/hidden/r/git/ref/heads/main"), ["t-octo", "t-work"]);
    }

    #[tokio::test]
    async fn a_new_token_makes_a_new_session_and_no_account_is_unauthorized() {
        let dir = tempfile::tempdir().unwrap();
        let caches = Arc::new(Caches::new(CacheConfig::new(dir.path().to_owned(), 0)));
        let hub = Hub::with_api(&caches, "http://127.0.0.1:1");
        let first = hub.session(Some(("a".into(), "t1".into())), &caches).unwrap();
        assert!(Arc::ptr_eq(&first, &hub.session(Some(("a".into(), "t1".into())), &caches).unwrap()));
        assert!(!Arc::ptr_eq(&first, &hub.session(Some(("a".into(), "t2".into())), &caches).unwrap()));
        assert_eq!(hub.session(None, &caches).err().unwrap().code, AppErrorCode::Unauthorized);
    }

    #[tokio::test]
    async fn the_foreground_count_makes_background_reads_wait() {
        let foreground = Foreground::default();
        let guard = foreground.enter();
        assert!(foreground.busy());
        let waiting = tokio::spawn({
            let foreground = foreground.clone();
            async move { foreground.wait_idle().await }
        });
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), waiting).await.unwrap().unwrap();
    }
}
