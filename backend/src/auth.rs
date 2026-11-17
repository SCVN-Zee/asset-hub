//! Who is calling, and may they read this repo. GitHub answers both, and Redis caches the answers
//! for `ACCESS_TTL_SEC`. The cache never extends a lookup, so a revoked token or repo permission
//! stops working within that time.

use serde_json::Value;
use sha2::{Digest, Sha256};
use web_sys::AbortController;
use worker::Request;

use crate::config::Config;
use crate::http::{header, quote, stop, Handled};
use crate::js::{fetch, is_js_space, with_timeout, FetchInit};
use crate::redis::Redis;

/// How long a verified token or repo permission is trusted.
pub const ACCESS_TTL_SEC: u64 = 300;

const GITHUB_API: &str = "https://api.github.com";
const GITHUB_TIMEOUT_MS: u64 = 5000;

pub struct Caller {
    /// `hex(sha256(token))`: the only form of the token that is ever used as a key.
    pub token_key: String,
    /// The caller's token, kept in memory for this request only. Never stored or logged.
    pub token: String,
    /// Set by `assert_repo_access`: whether the caller has push access to the repo.
    pub can_push: bool,
}

struct GithubReply {
    status: u16,
    body: Value,
    rate_limited: bool,
}

pub fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// The token of `Authorization: Bearer <token>`: the scheme in any case, white space, then a token
/// with no white space in it.
pub fn bearer_token(value: &str) -> Option<&str> {
    let rest = value.get(..6).filter(|scheme| scheme.eq_ignore_ascii_case("bearer")).map(|_| &value[6..])?;
    let token = rest.trim_start_matches(is_js_space);
    let spaced = token.len() < rest.len();
    (spaced && !token.is_empty() && !token.chars().any(is_js_space)).then_some(token)
}

/// `None` when GitHub could not be reached in time.
async fn github(path: &str, token: &str) -> Option<GithubReply> {
    let controller = AbortController::new().ok()?;
    let signal = controller.signal();
    let auth = format!("Bearer {token}");
    let url = format!("{GITHUB_API}{path}");
    let work = async {
        let headers = [("authorization", auth.as_str()), ("user-agent", "asset-hub-api"), ("accept", "application/vnd.github+json")];
        let init = FetchInit { method: "GET", headers: &headers, body: None, signal: Some(&signal) };
        let res = fetch(&url, init).await.ok()?;
        let status = res.status();
        let remaining = res.headers().get("x-ratelimit-remaining").ok().flatten();
        let rate_limited = status == 429 || (status == 403 && remaining.as_deref() == Some("0"));
        let mut body = Value::Null;
        if res.ok() {
            let text = wasm_bindgen_futures::JsFuture::from(res.text().ok()?).await.ok()?.as_string()?;
            body = serde_json::from_str(&text).unwrap_or(Value::Null);
        } else if let Some(stream) = res.body() {
            crate::js::settle(stream.cancel()).await;
        }
        Some(GithubReply { status, body, rate_limited })
    };
    let reply = with_timeout(GITHUB_TIMEOUT_MS, work).await.flatten();
    if reply.is_none() {
        controller.abort();
    }
    reply
}

fn upstream() -> crate::http::Halt {
    stop("upstream", 503)
}

fn parse_cached(raw: Option<Option<String>>) -> Option<Value> {
    serde_json::from_str(&raw??).ok()
}

pub async fn authenticate(req: &Request, redis: &Redis) -> Handled<Caller> {
    let authorization = header(req, "authorization")?.unwrap_or_default();
    let Some(token) = bearer_token(&authorization) else {
        return Err(stop("auth", 401));
    };

    let token_key = sha256_hex(token);
    let cache_key = format!("auth:{token_key}");

    let cached = parse_cached(redis.get(&cache_key).await);
    if cached.as_ref().and_then(|c| c.get("login")).and_then(Value::as_str).is_some_and(|login| !login.is_empty()) {
        return Ok(Caller { token_key, token: token.to_owned(), can_push: false });
    }

    let Some(reply) = github("/user", token).await else {
        return Err(upstream());
    };
    if reply.rate_limited || reply.status >= 500 {
        return Err(upstream());
    }
    let login = reply.body.get("login").and_then(Value::as_str);
    let (200, Some(login)) = (reply.status, login) else {
        return Err(stop("auth", 401));
    };

    redis.set(&cache_key, &format!("{{\"login\":{}}}", quote(login)), ACCESS_TTL_SEC).await;
    Ok(Caller { token_key, token: token.to_owned(), can_push: false })
}

fn owner_allowed(owner: &str, allowed: &str) -> bool {
    let owner = owner.to_lowercase();
    allowed.split(',').map(|o| crate::js::js_trim(o).to_lowercase()).filter(|o| !o.is_empty()).any(|o| o == owner)
}

/// Proves the caller can read `owner/repo` and records whether they can push. `owner` and `repo`
/// are lower-case.
pub async fn assert_repo_access(caller: &mut Caller, owner: &str, repo: &str, cfg: &Config, redis: &Redis) -> Handled<()> {
    if !owner_allowed(owner, &cfg.allowed_owners) {
        return Err(stop("noAccess", 403));
    }

    let cache_key = format!("acl:{}:{owner}/{repo}", caller.token_key);
    let cached = parse_cached(redis.get(&cache_key).await);
    if cached.as_ref().and_then(|c| c.get("pull")).is_some_and(is_truthy) {
        caller.can_push = cached.as_ref().and_then(|c| c.get("push")) == Some(&Value::Bool(true));
        return Ok(());
    }

    // Owner and repo hold only word characters, dots and dashes, so no escaping is needed.
    let Some(reply) = github(&format!("/repos/{owner}/{repo}"), &caller.token).await else {
        return Err(upstream());
    };
    if reply.rate_limited || reply.status >= 500 {
        return Err(upstream());
    }
    let permissions = reply.body.get("permissions");
    let pull = permissions.and_then(|p| p.get("pull")) == Some(&Value::Bool(true));
    if reply.status != 200 || !pull {
        return Err(stop("noAccess", 403));
    }

    let push = permissions.and_then(|p| p.get("push")) == Some(&Value::Bool(true));
    redis.set(&cache_key, &format!("{{\"pull\":true,\"push\":{push}}}"), ACCESS_TTL_SEC).await;
    caller.can_push = push;
    Ok(())
}

/// JavaScript truthiness of a parsed JSON value.
fn is_truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_key_is_the_sha256_of_the_token() {
        assert_eq!(sha256_hex(""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(sha256_hex("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }

    #[test]
    fn bearer_tokens() {
        assert_eq!(bearer_token("Bearer abc"), Some("abc"));
        assert_eq!(bearer_token("bearer   abc"), Some("abc"));
        assert_eq!(bearer_token("BEARER\tabc"), Some("abc"));
        assert_eq!(bearer_token("Bearer ghp_a-b.c"), Some("ghp_a-b.c"));
        for bad in ["", "Bearer", "Bearer ", "Bearerabc", "Bearer a b", "Bearer abc ", "Basic abc", "Bear abc", "Bearer\u{e9}abc"] {
            assert_eq!(bearer_token(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn owners_are_matched_ignoring_case_and_spaces() {
        assert!(owner_allowed("supercent-io", "supercent-io,SCVN-Dan"));
        assert!(owner_allowed("scvn-dan", "supercent-io, SCVN-Dan "));
        assert!(!owner_allowed("someone-else", "supercent-io,SCVN-Dan"));
        assert!(!owner_allowed("", ""));
        assert!(!owner_allowed("", ",,"));
    }

    #[test]
    fn truthiness_follows_javascript() {
        let parse = |text: &str| serde_json::from_str::<Value>(text).unwrap_or(Value::Null);
        assert!(is_truthy(&parse("true")));
        assert!(is_truthy(&parse("1")));
        assert!(is_truthy(&parse("\"x\"")));
        assert!(is_truthy(&parse("{}")));
        assert!(!is_truthy(&parse("false")));
        assert!(!is_truthy(&parse("0")));
        assert!(!is_truthy(&parse("\"\"")));
        assert!(!is_truthy(&parse("null")));
    }
}
