//! The GitHub client (`src/core/github/client.ts`). Asset Hub only ever reads from GitHub, though the
//! token it holds could write: every request passes [`assert_read_only`] inside [`GitHubClient`]
//! before it reaches the [`HttpFetch`] the app supplies, so no request path can skip it.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use base64::Engine;
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};

use crate::errors::{SourceError, SourceErrorCode, SourceErrorDetail};
use crate::js::{decode_utf8, to_number, OrderedMap};
use crate::re;
use crate::types::{EntryType, RepoRef, TreeEntry};

/// A request as the app's HTTP stack sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

/// A response with its body read in full.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// A header by name, ignoring case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
}

/// The HTTP stack and the clock the client needs, supplied by the app (so tests need no network).
/// A transport failure is `Err` with its message; any answer, an error status too, is `Ok`.
pub trait HttpFetch: Send + Sync {
    fn fetch(&self, request: HttpRequest) -> impl Future<Output = Result<HttpResponse, String>> + Send;
    /// Waits `ms` milliseconds (backoff between retries).
    fn sleep(&self, ms: u64) -> impl Future<Output = ()> + Send;
    /// Milliseconds since the epoch.
    fn now_ms(&self) -> i64;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchHead {
    pub name: String,
    pub commit: String,
    /// ISO date of the tip commit.
    pub date: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitHubUser {
    pub login: String,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comparison {
    pub files: Vec<ChangedFile>,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeListing {
    pub entries: Vec<TreeEntry>,
    /// Set when GitHub cut the listing short.
    pub truncated: bool,
}

const API: &str = "https://api.github.com";
/// Max pages followed for list endpoints (100 items each).
const MAX_PAGES: usize = 20;
/// Aliases per GraphQL query when reading many small blobs.
const GRAPHQL_BATCH: usize = 40;
const MAX_RETRIES: u32 = 4;
/// GitHub lists up to this many files in a comparison.
const COMPARE_LIMIT: usize = 300;

/// Refuses every request that could write. GET and HEAD, GraphQL queries (never mutations) and Git
/// LFS downloads go through; anything else is refused before it leaves the machine, whatever asked
/// for it.
pub fn assert_read_only(url: &str, method: &str, body: Option<&str>) -> Result<(), SourceError> {
    let verb = method.to_uppercase();
    if verb == "GET" || verb == "HEAD" {
        return Ok(());
    }
    if verb == "POST" {
        if let Some(body) = body {
            // A body that is not a JSON object has neither a query nor an operation.
            let payload: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
            let path = url_path(url);
            // A GraphQL document is a query when it starts with `query` or a bare selection set.
            let is_query = payload.get("query").and_then(|q| q.as_str()).is_some_and(|q| re!(r"^\s*(query\b|\{)").test(q));
            if path == Some("/graphql") && is_query {
                return Ok(());
            }
            if path.is_some_and(|p| p.ends_with("/info/lfs/objects/batch")) && payload.get("operation").and_then(|o| o.as_str()) == Some("download") {
                return Ok(());
            }
        }
    }
    Err(SourceError::read_only(format!("Asset Hub is read-only: {verb} {url} was not sent")))
}

/// The path of an absolute URL (`new URL(url).pathname`, without dot-segment normalisation: an odd
/// path is refused, never let through). `None` for a text that is no absolute URL.
fn url_path(url: &str) -> Option<&str> {
    let rest = url.split_once("://")?.1;
    let from_path = &rest[rest.find(['/', '?', '#']).unwrap_or(rest.len())..];
    let path = &from_path[..from_path.find(['?', '#']).unwrap_or(from_path.len())];
    Some(if path.is_empty() { "/" } else { path })
}

/// `encodeURIComponent`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn split_repo(repo: &str) -> Result<(&str, &str), SourceError> {
    let mut parts = repo.split('/');
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None) if !owner.is_empty() && !name.is_empty() => Ok((owner, name)),
        _ => Err(SourceError::new(SourceErrorCode::NotFound, format!("Invalid repository name \"{repo}\""))),
    }
}

fn repo_path(repo: &str) -> Result<String, SourceError> {
    let (owner, name) = split_repo(repo)?;
    Ok(format!("{}/{}", encode_uri_component(owner), encode_uri_component(name)))
}

fn ref_path(r: &str) -> String {
    r.split('/').map(encode_uri_component).collect::<Vec<_>>().join("/")
}

/// Gateway timeouts, cut-off JSON and dropped connections are worth retrying.
fn is_transient(err: &SourceError) -> bool {
    err.code == SourceErrorCode::Network || matches!(err.detail.status, Some(500 | 502 | 503 | 504))
}

/// Extracts `rel="next"` from a GitHub `Link` header.
pub fn next_link(header: Option<&str>) -> Option<String> {
    for part in header?.split(',') {
        if let Some(m) = re!(r#"<([^>]+)>;\s*rel="next""#).exec(part) {
            return m.get(1).map(str::to_string);
        }
    }
    None
}

/// Maps a failed response to the error the UI shows.
pub fn to_source_error(res: &HttpResponse, now_ms: i64) -> SourceError {
    let mut message = format!("GitHub responded {}", res.status);
    if let Some(m) = serde_json::from_slice::<serde_json::Value>(&res.body).ok().and_then(|b| b.get("message").and_then(|m| m.as_str().map(str::to_string))).filter(|m| !m.is_empty()) {
        message = m;
    }
    let number = |name: &str| res.header(name).map_or(0.0, to_number);
    let sso = res.header("x-github-sso").filter(|s| !s.is_empty());
    let remaining = res.header("x-ratelimit-remaining");
    let reset = number("x-ratelimit-reset");
    let reset_at = (reset.is_finite() && reset > 0.0).then_some((reset * 1000.0) as i64);
    let status = res.status;
    let detail = |extra: SourceErrorDetail| SourceErrorDetail { status: Some(status), ..extra };
    if status == 401 {
        return SourceError::with_detail(SourceErrorCode::Unauthorized, message, detail(Default::default()));
    }
    if let (403, Some(sso)) = (status, sso) {
        let sso_url = re!(r"url=(\S+)").exec(sso).and_then(|m| m.get(1).map(str::to_string));
        return SourceError::with_detail(SourceErrorCode::Sso, message, detail(SourceErrorDetail { sso_url, ..Default::default() }));
    }
    // Secondary (abuse) limits come as 403 with Retry-After or a message, not remaining=0.
    let retry_after = number("retry-after");
    if status == 429 || (status == 403 && (remaining == Some("0") || retry_after > 0.0 || re!("rate limit", "i").test(&message))) {
        let reset_at = reset_at.or_else(|| (retry_after > 0.0).then(|| now_ms + (retry_after * 1000.0) as i64));
        return SourceError::with_detail(SourceErrorCode::RateLimited, message, detail(SourceErrorDetail { reset_at, ..Default::default() }));
    }
    let code = match status {
        403 => SourceErrorCode::Forbidden,
        404 => SourceErrorCode::NotFound,
        _ => SourceErrorCode::Unknown,
    };
    SourceError::with_detail(code, message, detail(Default::default()))
}

/// How a request is authorised; the token as a bearer when unset.
enum Auth {
    /// The token as HTTP Basic credentials (Git LFS).
    Basic,
    /// None: pre-signed URLs.
    Nothing,
}

#[derive(Default)]
struct Outgoing {
    method: Option<&'static str>,
    body: Option<String>,
    headers: Vec<(String, String)>,
    auth: Option<Auth>,
    /// The repo the request is about; the tokens of the accounts that read it are tried in order.
    repo: Option<String>,
    /// Use exactly this token (the listing of one account's repos).
    token: Option<String>,
}

/// The tokens of every signed-in account, the active one first, and which of them can read which
/// repo (learned from the repo listing). A request about a repo uses the first account that reads
/// it; the next one is the fallback.
pub struct AccountTokens {
    tokens: Vec<String>,
    /// Lowercased repo name -> indexes into `tokens`, in order.
    readers: Mutex<HashMap<String, Vec<usize>>>,
}

impl AccountTokens {
    pub fn new(active: impl Into<String>, others: Vec<String>) -> Self {
        let mut tokens = vec![active.into()];
        for token in others {
            if !tokens.contains(&token) {
                tokens.push(token);
            }
        }
        Self { tokens, readers: Mutex::default() }
    }

    /// The active account's token.
    pub fn active(&self) -> &str {
        &self.tokens[0]
    }

    /// Every token, the active one first.
    pub fn all(&self) -> &[String] {
        &self.tokens
    }

    /// The tokens to try for `repo`, best first: the accounts known to read it, then every other
    /// account (the listing can miss a repo an account still reads).
    pub fn for_repo(&self, repo: &str) -> Vec<String> {
        let known = self.readers.lock().ok().and_then(|r| r.get(&repo.to_lowercase()).cloned()).unwrap_or_default();
        let rest = (0..self.tokens.len()).filter(|i| !known.contains(i));
        known.iter().copied().chain(rest).map(|i| self.tokens[i].clone()).collect()
    }

    /// Remembers that the account of `token` reads `repo`, and tries it first from now on.
    pub fn remember_reader(&self, repo: &str, token: &str) {
        let Some(index) = self.tokens.iter().position(|t| t == token) else { return };
        if let Ok(mut readers) = self.readers.lock() {
            let accounts = readers.entry(repo.to_lowercase()).or_default();
            accounts.retain(|i| *i != index);
            accounts.insert(0, index);
        }
    }

    /// Takes the readers of a new listing. An account already known to read a repo stays first: a
    /// listing can miss a repo, and a read may have found its reader while the listing loaded.
    fn set_readers(&self, mut readers: HashMap<String, Vec<usize>>) {
        if let Ok(mut kept) = self.readers.lock() {
            for (repo, accounts) in kept.drain() {
                let Some(&known) = accounts.first() else { continue };
                let listed = readers.entry(repo).or_default();
                listed.retain(|i| *i != known);
                listed.insert(0, known);
            }
            *kept = readers;
        }
    }
}

/// The repo a request URL is about: `/repos/{owner}/{name}/...`, or a Git LFS URL
/// (`https://github.com/{owner}/{name}.git/info/lfs/...`).
fn repo_of_url(url: &str) -> Option<String> {
    let path = url_path(url)?;
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["repos", owner, name, ..] => Some(format!("{owner}/{name}")),
        [owner, name, ..] if name.ends_with(".git") => Some(format!("{owner}/{}", name.trim_end_matches(".git"))),
        _ => None,
    }
}

#[derive(Deserialize)]
struct GqlError {
    #[serde(rename = "type")]
    kind: Option<String>,
    message: String,
}

#[derive(Deserialize)]
struct RefsResponse {
    data: Option<RefsData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize)]
struct RefsData {
    repository: Option<RefsRepository>,
}

#[derive(Deserialize)]
struct RefsRepository {
    refs: RefsPage,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefsPage {
    page_info: PageInfo,
    nodes: Vec<RefNode>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

#[derive(Deserialize)]
struct RefNode {
    name: String,
    target: Option<RefTarget>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RefTarget {
    oid: String,
    committed_date: Option<String>,
}

#[derive(Deserialize)]
struct BlobsResponse {
    data: Option<BlobsData>,
    errors: Option<Vec<GqlError>>,
}

#[derive(Deserialize)]
struct BlobsData {
    repository: Option<std::collections::HashMap<String, Option<BlobField>>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BlobField {
    text: Option<String>,
    is_truncated: Option<bool>,
    oid: Option<String>,
}

/// The error GraphQL answers with when it has no data.
fn graphql_error(errors: Option<Vec<GqlError>>) -> SourceError {
    let first = errors.and_then(|e| e.into_iter().next());
    let code = match first.as_ref().and_then(|e| e.kind.as_deref()) {
        Some("NOT_FOUND") => SourceErrorCode::NotFound,
        Some("RATE_LIMITED") => SourceErrorCode::RateLimited,
        _ => SourceErrorCode::Unknown,
    };
    SourceError::new(code, first.map_or_else(|| "GraphQL request failed".to_string(), |e| e.message))
}

/// GraphQL answers 200 when the account cannot see a repo: no repository and a NOT_FOUND error.
/// Returns that error's message, so the request can go out again with another account.
fn graphql_repo_missing(url: &str, res: &HttpResponse) -> Option<String> {
    if url_path(url) != Some("/graphql") {
        return None;
    }
    let body: serde_json::Value = serde_json::from_slice(&res.body).ok()?;
    if body.pointer("/data/repository").is_some_and(|r| !r.is_null()) {
        return None;
    }
    body.get("errors")?.as_array()?.iter().find_map(|e| {
        let message = e.get("message").and_then(|m| m.as_str()).unwrap_or_default();
        let missing = e.get("type").and_then(|t| t.as_str()) == Some("NOT_FOUND") || message.contains("Could not resolve to a Repository");
        missing.then(|| message.to_owned())
    })
}

pub struct GitHubClient<F: HttpFetch> {
    tokens: Arc<AccountTokens>,
    fetch: F,
    api_base: String,
}

impl<F: HttpFetch> GitHubClient<F> {
    pub fn new(token: impl Into<String>, fetch: F) -> Self {
        Self::with_tokens(Arc::new(AccountTokens::new(token, Vec::new())), fetch)
    }

    /// A client over every signed-in account.
    pub fn with_tokens(tokens: Arc<AccountTokens>, fetch: F) -> Self {
        Self { tokens, fetch, api_base: API.to_string() }
    }

    /// The clock of the HTTP stack, in milliseconds since the epoch.
    pub fn now_ms(&self) -> i64 {
        self.fetch.now_ms()
    }

    /// A client for another API root (GitHub Enterprise, or a test server).
    pub fn with_api_base(mut self, api_base: impl Into<String>) -> Self {
        self.api_base = api_base.into();
        self
    }

    pub async fn get_user(&self) -> Result<GitHubUser, SourceError> {
        #[derive(Deserialize)]
        struct User {
            login: String,
            name: Option<String>,
        }
        let (user, _): (User, _) = self.request("/user", None, None).await?;
        Ok(GitHubUser { login: user.login, name: user.name })
    }

    /// The repos every signed-in account reads, merged by name (the active account's first), and
    /// remembers which account reads which. Fails only when no account could list.
    pub async fn list_repos(&self) -> Result<Vec<RepoRef>, SourceError> {
        #[derive(Deserialize)]
        struct Repo {
            full_name: String,
            default_branch: String,
        }
        let mut merged: Vec<RepoRef> = Vec::new();
        let mut readers: HashMap<String, Vec<usize>> = HashMap::new();
        let mut failure = None;
        let mut listed = false;
        for (index, token) in self.tokens.all().iter().enumerate() {
            match self.paginate::<Repo>(Some(token), None, "/user/repos?per_page=100&sort=pushed&affiliation=owner,collaborator,organization_member").await {
                Ok(repos) => {
                    listed = true;
                    for repo in repos {
                        let accounts = readers.entry(repo.full_name.to_lowercase()).or_default();
                        if accounts.is_empty() {
                            merged.push(RepoRef { full_name: repo.full_name, default_branch: repo.default_branch });
                        }
                        accounts.push(index);
                    }
                }
                Err(error) => failure = failure.or(Some(error)),
            }
        }
        match failure {
            Some(error) if !listed => Err(error),
            _ => {
                self.tokens.set_readers(readers);
                Ok(merged)
            }
        }
    }

    pub async fn list_branches(&self, repo: &str) -> Result<Vec<String>, SourceError> {
        #[derive(Deserialize)]
        struct Branch {
            name: String,
        }
        let branches: Vec<Branch> = self.paginate(None, Some(repo), &format!("/repos/{}/branches?per_page=100", repo_path(repo)?)).await?;
        Ok(branches.into_iter().map(|b| b.name).collect())
    }

    /// Every branch with its tip commit and commit date, 100 per GraphQL request.
    pub async fn list_branch_heads(&self, repo: &str) -> Result<Vec<BranchHead>, SourceError> {
        #[derive(Serialize)]
        struct Variables<'a> {
            owner: &'a str,
            name: &'a str,
            after: Option<&'a str>,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            query: &'a str,
            variables: Variables<'a>,
        }
        const QUERY: &str = "query($owner: String!, $name: String!, $after: String) {
        repository(owner: $owner, name: $name) {
          refs(refPrefix: \"refs/heads/\", first: 100, after: $after) {
            pageInfo { hasNextPage endCursor }
            nodes { name target { oid ... on Commit { committedDate } } }
          }
        }
      }";
        let (owner, name) = split_repo(repo)?;
        let mut heads = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let body = json_body(&Body { query: QUERY, variables: Variables { owner, name, after: after.as_deref() } })?;
            let (data, _): (RefsResponse, _) = self.request_for(Some(repo), None, "/graphql", Some("POST"), Some(body)).await?;
            let Some(refs) = data.data.and_then(|d| d.repository).map(|r| r.refs) else { return Err(graphql_error(data.errors)) };
            for node in refs.nodes {
                if let Some(target) = node.target {
                    heads.push(BranchHead { name: node.name, commit: target.oid, date: target.committed_date });
                }
            }
            if !refs.page_info.has_next_page {
                break;
            }
            after = refs.page_info.end_cursor;
        }
        Ok(heads)
    }

    /// Commit SHA at the tip of a branch.
    pub async fn get_branch_head(&self, repo: &str, branch: &str) -> Result<String, SourceError> {
        #[derive(Deserialize)]
        struct Ref {
            object: Object,
        }
        #[derive(Deserialize)]
        struct Object {
            sha: String,
        }
        let (r, _): (Ref, _) = self.request(&format!("/repos/{}/git/ref/heads/{}", repo_path(repo)?, ref_path(branch)), None, None).await?;
        Ok(r.object.sha)
    }

    /// Files changed from `base` to `head` (GitHub lists up to 300; `more` when there were more).
    pub async fn compare_commits(&self, repo: &str, base: &str, head: &str) -> Result<Comparison, SourceError> {
        #[derive(Deserialize)]
        struct Compare {
            files: Option<Vec<File>>,
        }
        #[derive(Deserialize)]
        struct File {
            filename: String,
            status: String,
            sha: String,
        }
        let path = format!("/repos/{}/compare/{}...{}?per_page=300", repo_path(repo)?, encode_uri_component(base), encode_uri_component(head));
        let (data, _): (Compare, _) = self.request(&path, None, None).await?;
        let files: Vec<ChangedFile> = data.files.unwrap_or_default().into_iter().map(|f| ChangedFile { path: f.filename, status: f.status, sha: f.sha }).collect();
        let more = files.len() >= COMPARE_LIMIT;
        Ok(Comparison { files, more })
    }

    /// Full recursive tree of a commit. `truncated` is set when GitHub cut the listing short.
    pub async fn get_tree(&self, repo: &str, commit_sha: &str) -> Result<TreeListing, SourceError> {
        #[derive(Deserialize)]
        struct Tree {
            truncated: bool,
            tree: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            path: String,
            #[serde(rename = "type")]
            kind: String,
            sha: String,
            size: Option<u64>,
        }
        let (data, _): (Tree, _) = self.request(&format!("/repos/{}/git/trees/{}?recursive=1", repo_path(repo)?, encode_uri_component(commit_sha)), None, None).await?;
        let entries = data
            .tree
            .into_iter()
            .filter_map(|item| {
                // 'commit' entries are submodules; they are not part of this repo's files.
                let kind = match item.kind.as_str() {
                    "blob" => EntryType::Blob,
                    "tree" => EntryType::Tree,
                    _ => return None,
                };
                Some(TreeEntry { path: item.path, kind, sha: item.sha, size: item.size, lfs: None, lfs_size: None, lfs_oid: None })
            })
            .collect();
        Ok(TreeListing { entries, truncated: data.truncated })
    }

    /// Text of one file at a commit, or `None` when it does not exist.
    pub async fn get_file_text(&self, repo: &str, commit_sha: &str, path: &str) -> Result<Option<String>, SourceError> {
        let texts = self.get_blob_texts(repo, commit_sha, &[path.to_string()]).await?;
        Ok(texts.get(path).cloned())
    }

    /// Reads many small text blobs (LFS pointers, `.gitattributes`) with batched GraphQL queries:
    /// one request per 40 files instead of one each.
    pub async fn get_blob_texts(&self, repo: &str, commit_sha: &str, paths: &[String]) -> Result<OrderedMap<String>, SourceError> {
        let (owner, name) = split_repo(repo)?;
        let mut result = OrderedMap::new();
        for batch in paths.chunks(GRAPHQL_BATCH) {
            for (p, text) in self.blob_text_batch(owner, name, commit_sha, batch, 0).await?.iter() {
                result.insert(p, text.clone());
            }
        }
        Ok(result)
    }

    /// One GraphQL query; on a transient failure the batch is split in half and retried with backoff.
    fn blob_text_batch<'a>(&'a self, owner: &'a str, name: &'a str, commit_sha: &'a str, batch: &'a [String], attempt: u32) -> BoxFuture<'a, Result<OrderedMap<String>, SourceError>> {
        Box::pin(async move {
            let err = match self.read_blob_batch(owner, name, commit_sha, batch).await {
                Ok(result) => return Ok(result),
                Err(err) => err,
            };
            if !is_transient(&err) || attempt >= MAX_RETRIES {
                return Err(err);
            }
            self.fetch.sleep(400 * 2u64.pow(attempt)).await;
            if batch.len() <= 5 {
                return self.blob_text_batch(owner, name, commit_sha, batch, attempt + 1).await;
            }
            let (first, second) = batch.split_at(batch.len().div_ceil(2));
            let (a, b) = futures::try_join!(self.blob_text_batch(owner, name, commit_sha, first, attempt + 1), self.blob_text_batch(owner, name, commit_sha, second, attempt + 1))?;
            let mut merged = a;
            for (p, text) in b.iter() {
                merged.insert(p, text.clone());
            }
            Ok(merged)
        })
    }

    async fn read_blob_batch(&self, owner: &str, name: &str, commit_sha: &str, batch: &[String]) -> Result<OrderedMap<String>, SourceError> {
        #[derive(Serialize)]
        struct Body<'a> {
            query: &'a str,
        }
        let json = |s: &str| serde_json::to_string(s).map_err(|e| SourceError::new(SourceErrorCode::Unknown, e.to_string()));
        let mut fields = Vec::with_capacity(batch.len());
        for (i, p) in batch.iter().enumerate() {
            fields.push(format!("f{i}: object(expression: {}) {{ ... on Blob {{ text isTruncated oid }} }}", json(&format!("{commit_sha}:{p}"))?));
        }
        let query = format!("query {{ repository(owner: {}, name: {}) {{ {} }} }}", json(owner)?, json(name)?, fields.join("\n"));
        let (data, _): (BlobsResponse, _) = self.request_for(Some(&format!("{owner}/{name}")), None, "/graphql", Some("POST"), Some(json_body(&Body { query: &query })?)).await?;
        let Some(repo_data) = data.data.and_then(|d| d.repository) else { return Err(graphql_error(data.errors)) };
        let mut result = OrderedMap::new();
        let mut truncated: Vec<(&String, &str)> = Vec::new();
        for (i, p) in batch.iter().enumerate() {
            let blob = repo_data.get(&format!("f{i}")).and_then(|b| b.as_ref());
            match blob {
                Some(BlobField { is_truncated: Some(true), oid: Some(oid), .. }) if !oid.is_empty() => truncated.push((p, oid)),
                Some(BlobField { text: Some(text), .. }) => result.insert(p.as_str(), text.clone()),
                _ => {}
            }
        }
        // GraphQL cuts long texts (large prefabs, big .meta files); read those whole from the blob API.
        for (p, oid) in truncated {
            let bytes = self.get_blob_bytes(&format!("{owner}/{name}"), oid).await?;
            result.insert(p.as_str(), decode_utf8(&bytes));
        }
        Ok(result)
    }

    /// Raw contents of a git blob (up to 100 MB).
    pub async fn get_blob_bytes(&self, repo: &str, sha: &str) -> Result<Vec<u8>, SourceError> {
        let url = format!("{}/repos/{}/git/blobs/{}", self.api_base, repo_path(repo)?, encode_uri_component(sha));
        let res = self.send(&url, Outgoing { headers: vec![("Accept".into(), "application/vnd.github.raw".into())], ..Default::default() }).await?;
        Ok(res.body)
    }

    /// Real contents of a Git LFS object, through the LFS batch API.
    pub async fn get_lfs_object(&self, repo: &str, oid: &str, size: u64) -> Result<Vec<u8>, SourceError> {
        #[derive(Serialize)]
        struct Object<'a> {
            oid: &'a str,
            size: u64,
        }
        #[derive(Serialize)]
        struct Batch<'a> {
            operation: &'a str,
            transfers: [&'a str; 1],
            objects: [Object<'a>; 1],
        }
        #[derive(Deserialize)]
        struct Response {
            objects: Vec<LfsObject>,
        }
        #[derive(Deserialize)]
        struct LfsObject {
            error: Option<LfsError>,
            actions: Option<Actions>,
        }
        #[derive(Deserialize)]
        struct LfsError {
            message: String,
        }
        #[derive(Deserialize)]
        struct Actions {
            download: Option<Download>,
        }
        #[derive(Deserialize)]
        struct Download {
            href: String,
            header: Option<std::collections::BTreeMap<String, String>>,
        }
        let batch = json_body(&Batch { operation: "download", transfers: ["basic"], objects: [Object { oid, size }] })?;
        let res = self
            .send(
                &format!("https://github.com/{repo}.git/info/lfs/objects/batch"),
                Outgoing {
                    method: Some("POST"),
                    auth: Some(Auth::Basic),
                    repo: Some(repo.to_owned()),
                    headers: vec![("Accept".into(), "application/vnd.git-lfs+json".into()), ("Content-Type".into(), "application/vnd.git-lfs+json".into())],
                    body: Some(batch),
                    token: None,
                },
            )
            .await?;
        let data: Response = parse_json(&res)?;
        let obj = data.objects.into_iter().next();
        let download = obj.as_ref().and_then(|o| o.actions.as_ref()).and_then(|a| a.download.as_ref());
        let Some(download) = download else {
            let message = obj.as_ref().and_then(|o| o.error.as_ref()).map_or_else(|| format!("LFS object {oid} is not available"), |e| e.message.clone());
            return Err(SourceError::new(SourceErrorCode::NotFound, message));
        };
        let headers = download.header.iter().flatten().map(|(k, v)| (k.clone(), v.clone())).collect();
        Ok(self.send(&download.href, Outgoing { auth: Some(Auth::Nothing), headers, ..Default::default() }).await?.body)
    }

    /// Every page of a list, with exactly `token` or for `repo`. The pages after the first are linked
    /// by repo id (`/repositories/{id}/...`), so their URL does not name the repo.
    async fn paginate<T: for<'de> Deserialize<'de>>(&self, token: Option<&str>, repo: Option<&str>, path: &str) -> Result<Vec<T>, SourceError> {
        let mut items = Vec::new();
        let mut url = Some(path.to_string());
        let mut page = 0;
        while let Some(current) = url.take().filter(|_| page < MAX_PAGES) {
            let (data, link): (Vec<T>, _) = self.request_for(repo, token, &current, None, None).await?;
            items.extend(data);
            url = next_link(link.as_deref());
            page += 1;
        }
        Ok(items)
    }

    /// A JSON request to the API; also returns the `Link` header, for paging.
    async fn request<T: for<'de> Deserialize<'de>>(&self, path_or_url: &str, method: Option<&'static str>, body: Option<String>) -> Result<(T, Option<String>), SourceError> {
        self.request_for(None, None, path_or_url, method, body).await
    }

    /// [`Self::request`] about `repo` (else the repo in the URL), or with exactly `token`.
    async fn request_for<T: for<'de> Deserialize<'de>>(&self, repo: Option<&str>, token: Option<&str>, path_or_url: &str, method: Option<&'static str>, body: Option<String>) -> Result<(T, Option<String>), SourceError> {
        let url = if path_or_url.starts_with("http") { path_or_url.to_string() } else { format!("{}{path_or_url}", self.api_base) };
        let mut headers = vec![("Accept".to_string(), "application/vnd.github+json".to_string()), ("X-GitHub-Api-Version".to_string(), "2022-11-28".to_string())];
        if body.is_some() {
            headers.push(("Content-Type".into(), "application/json".into()));
        }
        let res = self.send(&url, Outgoing { method, body, headers, auth: None, repo: repo.map(str::to_owned), token: token.map(str::to_owned) }).await?;
        let data = parse_json(&res)?;
        Ok((data, res.header("link").map(str::to_string)))
    }

    /// Sends a request and maps failures to `SourceError`; the read-only guard runs first. A request
    /// about a repo that GitHub refuses (403) or does not show (404, or a GraphQL "not found") goes
    /// out again with each other account in turn. The account that gets through is remembered as a
    /// reader of the repo. When every account fails, the first account's answer is the error.
    async fn send(&self, url: &str, init: Outgoing) -> Result<HttpResponse, SourceError> {
        let method = init.method.unwrap_or("GET");
        assert_read_only(url, method, init.body.as_deref())?;
        let repo = if init.token.is_some() { None } else { init.repo.clone().or_else(|| repo_of_url(url)) };
        let candidates = match (&init.token, &repo) {
            (Some(token), _) => vec![token.clone()],
            (None, Some(repo)) => self.tokens.for_repo(repo),
            (None, None) => vec![self.tokens.active().to_owned()],
        };
        let mut first = None;
        for (position, token) in candidates.iter().enumerate() {
            match self.send_with(url, &init, token).await {
                Ok(res) => {
                    if let (true, Some(repo)) = (position > 0, &repo) {
                        self.tokens.remember_reader(repo, token);
                    }
                    return Ok(res);
                }
                Err(error) if matches!(error.code, SourceErrorCode::Forbidden | SourceErrorCode::NotFound) => {
                    first.get_or_insert(error);
                }
                Err(error) => return Err(first.unwrap_or(error)),
            }
        }
        Err(first.unwrap_or_else(|| SourceError::new(SourceErrorCode::Unknown, "No account to send the request")))
    }

    async fn send_with(&self, url: &str, init: &Outgoing, token: &str) -> Result<HttpResponse, SourceError> {
        let method = init.method.unwrap_or("GET");
        let auth = match init.auth {
            None => Some(format!("Bearer {token}")),
            Some(Auth::Basic) => Some(format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("x-access-token:{token}")))),
            Some(Auth::Nothing) => None,
        };
        let mut headers: Vec<(String, String)> = auth.into_iter().map(|a| ("Authorization".to_string(), a)).collect();
        headers.extend(init.headers.iter().cloned());
        let res = self.fetch.fetch(HttpRequest { url: url.to_string(), method: method.to_string(), headers, body: init.body.clone() }).await.map_err(|e| SourceError::new(SourceErrorCode::Network, e))?;
        if res.ok() {
            match graphql_repo_missing(url, &res) {
                Some(message) => Err(SourceError::with_detail(SourceErrorCode::NotFound, message, SourceErrorDetail { status: Some(res.status), ..Default::default() })),
                None => Ok(res),
            }
        } else {
            Err(to_source_error(&res, self.fetch.now_ms()))
        }
    }
}

fn json_body<T: Serialize>(value: &T) -> Result<String, SourceError> {
    serde_json::to_string(value).map_err(|e| SourceError::new(SourceErrorCode::Unknown, e.to_string()))
}

/// A body that is not the JSON expected was cut off or garbled in transit: a dropped connection, so
/// a transient network failure (the TypeScript lets the `SyntaxError` through and retries it).
fn parse_json<T: for<'de> Deserialize<'de>>(res: &HttpResponse) -> Result<T, SourceError> {
    serde_json::from_slice(&res.body).map_err(|e| SourceError::new(SourceErrorCode::Network, format!("Unexpected response from GitHub: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_or_odd_url_never_passes_a_post() {
        let body = r#"{"query":"query { a }"}"#;
        assert!(assert_read_only("/graphql", "POST", Some(body)).is_err());
        assert!(assert_read_only("https://api.github.com/graphql", "POST", Some(body)).is_ok());
        assert!(assert_read_only("https://api.github.com/graphql?x=1#y", "POST", Some(body)).is_ok());
        assert!(assert_read_only("https://api.github.com/graphql/", "POST", Some(body)).is_err());
        assert!(assert_read_only("https://api.github.com", "POST", Some(body)).is_err());
    }

    #[test]
    fn a_body_that_is_not_an_object_is_refused() {
        for body in ["null", "[]", "5", "\"query\"", "not json"] {
            let err = assert_read_only("https://api.github.com/graphql", "POST", Some(body)).expect_err("refused");
            assert_eq!(err.code, SourceErrorCode::ReadOnly);
        }
    }

    #[test]
    fn mutations_and_other_verbs_are_refused_with_the_read_only_code() {
        let mutation = r#"{"query":"mutation { addStar(input: {}) { clientMutationId } }"}"#;
        assert_eq!(assert_read_only("https://api.github.com/graphql", "POST", Some(mutation)).expect_err("refused").code, SourceErrorCode::ReadOnly);
        assert_eq!(assert_read_only("https://api.github.com/graphql", "POST", Some(r#"{"query":"queryX { a }"}"#)).expect_err("refused").code, SourceErrorCode::ReadOnly);
        for verb in ["PUT", "PATCH", "DELETE", "post", "OPTIONS"] {
            assert!(assert_read_only("https://api.github.com/repos/a/b", verb, Some("{}")).is_err(), "{verb}");
        }
        assert!(assert_read_only("https://github.com/a/b.git/info/lfs/objects/batch", "POST", Some(r#"{"operation":"upload"}"#)).is_err());
        assert!(assert_read_only("https://github.com/a/b.git/info/lfs/objects/batch", "POST", Some(r#"{"operation":"download"}"#)).is_ok());
    }

    #[test]
    fn uri_components_are_encoded_like_the_browser_does() {
        assert_eq!(encode_uri_component("feat/a b+é!'()*~"), "feat%2Fa%20b%2B%C3%A9!'()*~");
    }

    #[test]
    fn rate_limit_headers_give_the_reset_time() {
        let res = HttpResponse { status: 403, headers: vec![("Retry-After".into(), "30".into())], body: br#"{"message":"slow down"}"#.to_vec() };
        let err = to_source_error(&res, 1_000);
        assert_eq!(err.code, SourceErrorCode::RateLimited);
        assert_eq!(err.detail.reset_at, Some(31_000));
        assert_eq!(err.message, "slow down");
    }

    #[test]
    fn sso_responses_carry_the_authorisation_url() {
        let res = HttpResponse { status: 403, headers: vec![("X-GitHub-SSO".into(), "required; url=https://github.com/orgs/x/sso?token=1".into())], body: b"{}".to_vec() };
        let err = to_source_error(&res, 0);
        assert_eq!(err.code, SourceErrorCode::Sso);
        assert_eq!(err.detail.sso_url.as_deref(), Some("https://github.com/orgs/x/sso?token=1"));
    }

    /// A scripted HTTP stack: answers in order, records what was sent.
    struct Script {
        answers: std::sync::Mutex<std::collections::VecDeque<Result<HttpResponse, String>>>,
        sent: std::sync::Mutex<Vec<HttpRequest>>,
        slept: std::sync::Mutex<Vec<u64>>,
    }

    impl Script {
        fn new(answers: Vec<Result<HttpResponse, String>>) -> Self {
            Self { answers: std::sync::Mutex::new(answers.into()), sent: Default::default(), slept: Default::default() }
        }
    }

    impl HttpFetch for &Script {
        async fn fetch(&self, request: HttpRequest) -> Result<HttpResponse, String> {
            self.sent.lock().unwrap().push(request);
            self.answers.lock().unwrap().pop_front().expect("an answer is scripted")
        }

        async fn sleep(&self, ms: u64) {
            self.slept.lock().unwrap().push(ms);
        }

        fn now_ms(&self) -> i64 {
            0
        }
    }

    fn json(status: u16, body: &str) -> Result<HttpResponse, String> {
        Ok(HttpResponse { status, headers: Vec::new(), body: body.as_bytes().to_vec() })
    }

    #[test]
    fn a_request_carries_the_token_and_the_api_headers() {
        let script = Script::new(vec![json(200, r#"{"login":"zee","name":null}"#)]);
        let user = futures::executor::block_on(GitHubClient::new("tok", &script).get_user()).expect("user");
        assert_eq!(user, GitHubUser { login: "zee".into(), name: None });
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://api.github.com/user");
        assert_eq!(sent[0].method, "GET");
        assert!(sent[0].headers.contains(&("Authorization".into(), "Bearer tok".into())));
        assert!(sent[0].headers.contains(&("X-GitHub-Api-Version".into(), "2022-11-28".into())));
    }

    fn two_accounts(script: &Script) -> GitHubClient<&Script> {
        GitHubClient::with_tokens(Arc::new(AccountTokens::new("active", vec!["other".into(), "active".into()])), script)
    }

    fn bearer(request: &HttpRequest) -> &str {
        request.headers.iter().find(|(k, _)| k == "Authorization").map(|(_, v)| v.as_str()).unwrap_or("")
    }

    #[test]
    fn the_repos_of_every_account_are_merged_by_name_and_read_by_the_first_account_that_lists_them() {
        let script = Script::new(vec![
            json(200, r#"[{"full_name":"o/shared","default_branch":"main"},{"full_name":"o/mine","default_branch":"main"}]"#),
            json(200, r#"[{"full_name":"O/Shared","default_branch":"main"},{"full_name":"w/work","default_branch":"dev"}]"#),
            json(200, r#"{"object":{"sha":"c1"}}"#),
            json(200, r#"{"object":{"sha":"c2"}}"#),
        ]);
        let client = two_accounts(&script);
        let repos = futures::executor::block_on(client.list_repos()).expect("repos");
        assert_eq!(repos.iter().map(|r| r.full_name.as_str()).collect::<Vec<_>>(), ["o/shared", "o/mine", "w/work"]);
        {
            let sent = script.sent.lock().unwrap();
            assert_eq!((bearer(&sent[0]), bearer(&sent[1])), ("Bearer active", "Bearer other"));
        }
        // "w/work" is readable by the second account only, "o/mine" by the first only.
        futures::executor::block_on(client.get_branch_head("w/work", "main")).expect("head");
        futures::executor::block_on(client.get_branch_head("o/mine", "main")).expect("head");
        let sent = script.sent.lock().unwrap();
        assert_eq!((bearer(&sent[2]), bearer(&sent[3])), ("Bearer other", "Bearer active"));
    }

    #[test]
    fn a_repo_the_active_account_cannot_read_is_retried_with_the_next_account() {
        let script = Script::new(vec![json(404, r#"{"message":"Not Found"}"#), json(200, r#"{"object":{"sha":"c1"}}"#), json(403, r#"{"message":"Forbidden"}"#), json(404, r#"{"message":"Not Found"}"#)]);
        assert_eq!(futures::executor::block_on(two_accounts(&script).get_branch_head("o/r", "main")).expect("head"), "c1");
        let error = futures::executor::block_on(two_accounts(&script).get_branch_head("o/r", "main")).expect_err("both refuse");
        assert_eq!(error.code, SourceErrorCode::Forbidden, "the active account's answer is kept");
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent.iter().map(bearer).collect::<Vec<_>>(), ["Bearer active", "Bearer other", "Bearer active", "Bearer other"]);
    }

    fn three_accounts(script: &Script) -> GitHubClient<&Script> {
        GitHubClient::with_tokens(Arc::new(AccountTokens::new("first", vec!["second".into(), "third".into()])), script)
    }

    #[test]
    fn every_account_is_tried_until_one_reads_the_repo_and_it_is_remembered() {
        let script = Script::new(vec![
            json(404, r#"{"message":"Not Found"}"#),
            json(404, r#"{"message":"Not Found"}"#),
            json(200, r#"{"object":{"sha":"c1"}}"#),
            json(200, r#"{"object":{"sha":"c2"}}"#),
        ]);
        let client = three_accounts(&script);
        assert_eq!(futures::executor::block_on(client.get_branch_head("o/r", "dev")).expect("head"), "c1");
        assert_eq!(futures::executor::block_on(client.get_branch_head("O/R", "dev")).expect("head"), "c2");
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent.iter().map(bearer).collect::<Vec<_>>(), ["Bearer first", "Bearer second", "Bearer third", "Bearer third"]);
    }

    #[test]
    fn an_account_that_lists_a_repo_but_cannot_read_it_does_not_block_the_others() {
        // The listing shows the repo for the second account only; the first one is the one that reads it.
        let script = Script::new(vec![
            json(200, "[]"),
            json(200, r#"[{"full_name":"z/UnityVFX","default_branch":"dev"}]"#),
            json(404, r#"{"message":"Not Found"}"#),
            json(200, r#"{"object":{"sha":"c1"}}"#),
        ]);
        let client = two_accounts(&script);
        futures::executor::block_on(client.list_repos()).expect("repos");
        assert_eq!(futures::executor::block_on(client.get_branch_head("z/UnityVFX", "dev")).expect("head"), "c1");
        let sent = script.sent.lock().unwrap();
        assert_eq!((bearer(&sent[2]), bearer(&sent[3])), ("Bearer other", "Bearer active"));
    }

    #[test]
    fn a_graphql_not_found_goes_to_the_next_account() {
        let missing = r#"{"data":{"repository":null},"errors":[{"type":"NOT_FOUND","message":"Could not resolve to a Repository with the name 'o/r'."}]}"#;
        let found = r#"{"data":{"repository":{"refs":{"pageInfo":{"hasNextPage":false,"endCursor":null},"nodes":[{"name":"dev","target":{"oid":"c1","committedDate":"2024-01-01T00:00:00Z"}}]}}}}"#;
        let script = Script::new(vec![json(200, missing), json(200, missing), json(200, found)]);
        let heads = futures::executor::block_on(three_accounts(&script).list_branch_heads("o/r")).expect("heads");
        assert_eq!(heads.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["dev"]);
        assert_eq!(script.sent.lock().unwrap().iter().map(bearer).collect::<Vec<_>>(), ["Bearer first", "Bearer second", "Bearer third"]);
        let script = Script::new(vec![json(200, missing), json(200, missing), json(200, missing)]);
        let error = futures::executor::block_on(three_accounts(&script).list_branch_heads("o/r")).expect_err("no account reads it");
        assert_eq!(error.code, SourceErrorCode::NotFound);
    }

    #[test]
    fn a_single_account_gets_no_second_try() {
        let script = Script::new(vec![json(404, r#"{"message":"Not Found"}"#)]);
        let error = futures::executor::block_on(GitHubClient::new("only", &script).get_branch_head("o/r", "main")).expect_err("not found");
        assert_eq!(error.code, SourceErrorCode::NotFound);
        assert_eq!(script.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_listing_fails_only_when_no_account_could_list() {
        let script = Script::new(vec![json(401, r#"{"message":"Bad credentials"}"#), json(200, r#"[{"full_name":"o/r","default_branch":"main"}]"#)]);
        let repos = futures::executor::block_on(two_accounts(&script).list_repos()).expect("one account listed");
        assert_eq!(repos.len(), 1);
        let script = Script::new(vec![json(401, r#"{"message":"Bad credentials"}"#), json(401, r#"{"message":"Bad credentials"}"#)]);
        assert_eq!(futures::executor::block_on(two_accounts(&script).list_repos()).expect_err("none listed").code, SourceErrorCode::Unauthorized);
    }

    #[test]
    fn the_repo_of_a_url_is_read_for_the_api_and_for_lfs() {
        assert_eq!(repo_of_url("https://api.github.com/repos/o/r/git/trees/abc?recursive=1").as_deref(), Some("o/r"));
        assert_eq!(repo_of_url("https://github.com/o/r.git/info/lfs/objects/batch").as_deref(), Some("o/r"));
        assert_eq!(repo_of_url("https://api.github.com/user/repos?per_page=100"), None);
        assert_eq!(repo_of_url("https://api.github.com/graphql"), None);
    }

    #[test]
    fn the_guard_stops_a_write_before_it_reaches_the_network() {
        let script = Script::new(vec![]);
        let client = GitHubClient::new("tok", &script);
        let mutation = r#"{"query":"mutation { x }"}"#.to_string();
        let err = futures::executor::block_on(client.request::<serde_json::Value>("/graphql", Some("POST"), Some(mutation))).expect_err("refused");
        assert_eq!(err.code, SourceErrorCode::ReadOnly);
        let err = futures::executor::block_on(client.request::<serde_json::Value>("/repos/a/b", Some("DELETE"), None)).expect_err("refused");
        assert_eq!(err.code, SourceErrorCode::ReadOnly);
        assert!(script.sent.lock().unwrap().is_empty());
    }

    #[test]
    fn list_pages_follow_the_next_link() {
        let page = |body: &str, link: Option<&str>| Ok(HttpResponse { status: 200, headers: link.map(|l| vec![("Link".to_string(), l.to_string())]).unwrap_or_default(), body: body.as_bytes().to_vec() });
        let script = Script::new(vec![page(r#"[{"name":"main"}]"#, Some(r#"<https://api.github.com/p2>; rel="next""#)), page(r#"[{"name":"dev"}]"#, None)]);
        let branches = futures::executor::block_on(GitHubClient::new("t", &script).list_branches("a/b")).expect("branches");
        assert_eq!(branches, ["main", "dev"]);
        assert_eq!(script.sent.lock().unwrap()[1].url, "https://api.github.com/p2");
    }

    #[test]
    fn every_page_of_a_listing_goes_to_the_account_that_reads_the_repo() {
        // GitHub links the next page by repo id (`/repositories/{id}/...`), so the URL names no repo.
        let next = r#"<https://api.github.com/repositories/832437869/branches?per_page=100&page=2>; rel="next""#;
        let script = Script::new(vec![
            json(404, r#"{"message":"Not Found"}"#),
            Ok(HttpResponse { status: 200, headers: vec![("Link".into(), next.into())], body: br#"[{"name":"main"}]"#.to_vec() }),
            json(200, r#"[{"name":"dev"}]"#),
        ]);
        let branches = futures::executor::block_on(two_accounts(&script).list_branches("o/r")).expect("branches");
        assert_eq!(branches, ["main", "dev"]);
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent[2].url, "https://api.github.com/repositories/832437869/branches?per_page=100&page=2");
        assert_eq!(sent.iter().map(bearer).collect::<Vec<_>>(), ["Bearer active", "Bearer other", "Bearer other"]);
    }

    #[test]
    fn a_reader_found_while_the_repos_load_is_kept_after_the_listing() {
        let script = Script::new(vec![
            json(404, r#"{"message":"Not Found"}"#),
            json(200, r#"{"object":{"sha":"c1"}}"#),
            json(200, "[]"),
            json(200, "[]"),
            json(200, r#"{"object":{"sha":"c2"}}"#),
        ]);
        let client = two_accounts(&script);
        futures::executor::block_on(client.get_branch_head("h/r", "main")).expect("head");
        futures::executor::block_on(client.list_repos()).expect("repos");
        futures::executor::block_on(client.get_branch_head("h/r", "main")).expect("head");
        let sent = script.sent.lock().unwrap();
        assert_eq!(bearer(&sent[4]), "Bearer other", "the account that read it still goes first");
    }

    #[test]
    fn a_gateway_error_splits_the_blob_batch_and_retries_with_backoff() {
        let ok = |names: &[(&str, &str)]| {
            let fields: Vec<String> = names.iter().map(|(f, t)| format!(r#""{f}":{{"text":"{t}"}}"#)).collect();
            json(200, &format!(r#"{{"data":{{"repository":{{{}}}}}}}"#, fields.join(",")))
        };
        let script = Script::new(vec![json(502, "{}"), ok(&[("f0", "a"), ("f1", "b"), ("f2", "c")]), ok(&[("f0", "d"), ("f1", "e"), ("f2", "f")])]);
        let paths: Vec<String> = (0..6).map(|i| format!("p{i}")).collect();
        let texts = futures::executor::block_on(GitHubClient::new("t", &script).get_blob_texts("a/b", "abc", &paths)).expect("texts");
        assert_eq!(texts.iter().map(|(p, t)| format!("{p}={t}")).collect::<Vec<_>>(), ["p0=a", "p1=b", "p2=c", "p3=d", "p4=e", "p5=f"]);
        assert_eq!(*script.slept.lock().unwrap(), [400]);
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent.len(), 3);
        assert!(sent[1].body.as_deref().unwrap().contains("abc:p0") && !sent[1].body.as_deref().unwrap().contains("abc:p3"));
    }

    #[test]
    fn a_truncated_blob_is_read_whole_from_the_blob_api() {
        let script = Script::new(vec![json(200, r#"{"data":{"repository":{"f0":{"text":"cut","isTruncated":true,"oid":"o1"}}}}"#), Ok(HttpResponse { status: 200, headers: vec![], body: b"\xef\xbb\xbfwhole".to_vec() })]);
        let text = futures::executor::block_on(GitHubClient::new("t", &script).get_file_text("a/b", "c", "x.meta")).expect("text");
        assert_eq!(text.as_deref(), Some("whole"));
        assert_eq!(script.sent.lock().unwrap()[1].url, "https://api.github.com/repos/a/b/git/blobs/o1");
    }

    #[test]
    fn an_lfs_object_is_downloaded_through_the_batch_api_without_the_token_on_the_signed_url() {
        let batch = r#"{"objects":[{"actions":{"download":{"href":"https://cdn.example/obj","header":{"X-Sig":"s"}}}}]}"#;
        let script = Script::new(vec![json(200, batch), Ok(HttpResponse { status: 200, headers: vec![], body: vec![1, 2, 3] })]);
        let bytes = futures::executor::block_on(GitHubClient::new("tok", &script).get_lfs_object("a/b", "oid", 3)).expect("bytes");
        assert_eq!(bytes, [1, 2, 3]);
        let sent = script.sent.lock().unwrap();
        assert_eq!(sent[0].url, "https://github.com/a/b.git/info/lfs/objects/batch");
        assert!(sent[0].headers.iter().any(|(k, v)| k == "Authorization" && v.starts_with("Basic ")));
        assert!(!sent[1].headers.iter().any(|(k, _)| k == "Authorization"));
        assert!(sent[1].headers.contains(&("X-Sig".into(), "s".into())));
    }

    #[test]
    fn a_missing_lfs_object_is_not_found_with_its_message() {
        let script = Script::new(vec![json(200, r#"{"objects":[{"error":{"message":"gone"}}]}"#)]);
        let err = futures::executor::block_on(GitHubClient::new("t", &script).get_lfs_object("a/b", "oid", 3)).expect_err("missing");
        assert_eq!((err.code, err.message.as_str()), (SourceErrorCode::NotFound, "gone"));
    }

    #[test]
    fn a_dropped_connection_is_a_network_error() {
        let script = Script::new(vec![Err("reset".into())]);
        let err = futures::executor::block_on(GitHubClient::new("t", &script).get_user()).expect_err("network");
        assert_eq!(err.code, SourceErrorCode::Network);
    }

    #[test]
    fn a_bad_repository_name_is_not_found() {
        let script = Script::new(vec![]);
        let err = futures::executor::block_on(GitHubClient::new("t", &script).list_branches("nope")).expect_err("invalid");
        assert_eq!(err.code, SourceErrorCode::NotFound);
        assert_eq!(err.message, "Invalid repository name \"nope\"");
    }
}
