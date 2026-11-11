//! HTTP to GitHub: one shared client, and the mapping of failed responses to the error codes the
//! page translates (`toSourceError` in `src/core/github/client.ts`).

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use reqwest::header::{HeaderMap, ACCEPT, AUTHORIZATION, USER_AGENT};
use reqwest::Client;

use crate::ipc::{AppErrorCode, IpcError};

pub const API: &str = "https://api.github.com";

/// The client every request uses. It follows the macOS system proxy settings and trusts the system
/// certificate store, like the 1.x app did.
pub fn client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .user_agent(concat!("AssetHub/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(15))
            .build()
            .expect("the HTTP client")
    })
}

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as f64).unwrap_or(0.0)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The error for a response that is not a success. `message` is the `message` of GitHub's JSON body.
pub fn failure(status: u16, headers: &HeaderMap, message: Option<String>, now_ms: f64) -> IpcError {
    let message = message.filter(|m| !m.is_empty()).unwrap_or_else(|| format!("GitHub responded {status}"));
    let remaining = header(headers, "x-ratelimit-remaining");
    let reset = header(headers, "x-ratelimit-reset").and_then(|v| v.parse::<f64>().ok()).filter(|v| v.is_finite() && *v > 0.0);
    let retry_after = header(headers, "retry-after").and_then(|v| v.parse::<f64>().ok()).filter(|v| *v > 0.0);
    let sso = header(headers, "x-github-sso");
    static SSO_URL: OnceLock<Regex> = OnceLock::new();
    static RATE_WORDS: OnceLock<Regex> = OnceLock::new();

    match status {
        401 => IpcError::new(AppErrorCode::Unauthorized, message),
        403 if sso.is_some() => {
            let url = SSO_URL.get_or_init(|| Regex::new(r"url=(\S+)").expect("a valid pattern"));
            let mut error = IpcError::new(AppErrorCode::Sso, message);
            error.sso_url = sso.and_then(|s| url.captures(s)).map(|c| c[1].to_owned());
            error
        }
        // Secondary (abuse) limits come as 403 with Retry-After or a message, not remaining=0.
        _ if status == 429
            || (status == 403
                && (remaining == Some("0")
                    || retry_after.is_some()
                    || RATE_WORDS.get_or_init(|| Regex::new(r"(?i)rate limit").expect("a valid pattern")).is_match(&message))) =>
        {
            let mut error = IpcError::new(AppErrorCode::RateLimited, message);
            error.reset_at = reset.map(|r| r * 1000.0).or(retry_after.map(|s| now_ms + s * 1000.0));
            error
        }
        403 => IpcError::new(AppErrorCode::Forbidden, message),
        404 => IpcError::new(AppErrorCode::NotFound, message),
        _ => IpcError::new(AppErrorCode::Unknown, message),
    }
}

/// The login of the account `token` belongs to. `api` is the API root (`API`; a test passes its own).
pub async fn get_user_login(client: &Client, api: &str, token: &str) -> Result<String, IpcError> {
    let response = client
        .get(format!("{api}/user"))
        .header(AUTHORIZATION, format!("Bearer {token}"))
        .header(ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header(USER_AGENT, concat!("AssetHub/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| IpcError::new(AppErrorCode::Network, e.without_url().to_string()))?;
    let status = response.status();
    let headers = response.headers().clone();
    if !status.is_success() {
        let message = response.json::<serde_json::Value>().await.ok().and_then(|b| b.get("message")?.as_str().map(str::to_owned));
        return Err(failure(status.as_u16(), &headers, message, now_ms()));
    }
    let body: serde_json::Value = response.json().await.map_err(|e| IpcError::new(AppErrorCode::Unknown, format!("GitHub sent an unreadable answer: {}", e.without_url())))?;
    body.get("login")
        .and_then(|l| l.as_str())
        .map(str::to_owned)
        .ok_or_else(|| IpcError::new(AppErrorCode::Unknown, "GitHub did not say who this token belongs to"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_server::{serve, Reply};
    use reqwest::header::HeaderValue;

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        pairs.iter().map(|(k, v)| (reqwest::header::HeaderName::from_static(k), HeaderValue::from_static(v))).collect()
    }

    #[test]
    fn a_rejected_token_is_unauthorized() {
        let error = failure(401, &headers(&[]), Some("Bad credentials".into()), 0.0);
        assert_eq!(error.code, AppErrorCode::Unauthorized);
        assert_eq!(error.message, "Bad credentials");
    }

    #[test]
    fn an_sso_organisation_carries_the_authorisation_url() {
        let error = failure(403, &headers(&[("x-github-sso", "required; url=https://github.com/orgs/x/sso?authorization_request=1")]), None, 0.0);
        assert_eq!(error.code, AppErrorCode::Sso);
        assert_eq!(error.sso_url.as_deref(), Some("https://github.com/orgs/x/sso?authorization_request=1"));
        assert_eq!(error.message, "GitHub responded 403");
    }

    #[test]
    fn an_exhausted_limit_says_when_it_resets() {
        let error = failure(403, &headers(&[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "1700000000")]), None, 0.0);
        assert_eq!(error.code, AppErrorCode::RateLimited);
        assert_eq!(error.reset_at, Some(1_700_000_000_000.0));
    }

    #[test]
    fn a_secondary_limit_uses_retry_after() {
        let error = failure(403, &headers(&[("retry-after", "30")]), Some("Slow down".into()), 1000.0);
        assert_eq!(error.code, AppErrorCode::RateLimited);
        assert_eq!(error.reset_at, Some(31_000.0));
        assert_eq!(failure(403, &headers(&[]), Some("You have exceeded a secondary rate limit".into()), 0.0).code, AppErrorCode::RateLimited);
        assert_eq!(failure(429, &headers(&[]), None, 0.0).code, AppErrorCode::RateLimited);
    }

    #[test]
    fn other_statuses_map_to_their_codes() {
        assert_eq!(failure(403, &headers(&[]), Some("Resource not accessible".into()), 0.0).code, AppErrorCode::Forbidden);
        assert_eq!(failure(404, &headers(&[]), None, 0.0).code, AppErrorCode::NotFound);
        assert_eq!(failure(500, &headers(&[]), None, 0.0).code, AppErrorCode::Unknown);
    }

    #[tokio::test]
    async fn the_login_comes_from_the_user_endpoint_with_the_token_in_the_header() {
        let base = serve(|(path, sent)| {
            if path == "/user" && sent.get("authorization").map(String::as_str) == Some("Bearer dummy-token") {
                Reply::json(200, r#"{"login":"octo","name":"Octo"}"#)
            } else {
                Reply::json(401, r#"{"message":"Bad credentials"}"#)
            }
        })
        .await;
        assert_eq!(get_user_login(client(), &base, "dummy-token").await.unwrap(), "octo");
        let error = get_user_login(client(), &base, "other").await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::Unauthorized);
        assert_eq!(error.message, "Bad credentials");
    }

    /// The API answers a large listing (the tree of a big repo is megabytes of JSON) about eight times
    /// smaller when it may compress it, so the client must ask for that and read the answer.
    #[tokio::test]
    async fn a_compressed_answer_is_asked_for_and_read() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(br#"{"login":"zipped"}"#).unwrap();
        let packed = gz.finish().unwrap();
        let base = serve(move |(_, sent)| {
            if sent.get("accept-encoding").is_some_and(|v| v.contains("gzip")) {
                let mut reply = Reply::bytes(packed.clone());
                reply.headers = vec![("Content-Type".into(), "application/json".into()), ("Content-Encoding".into(), "gzip".into())];
                reply
            } else {
                Reply::json(200, r#"{"login":"plain"}"#)
            }
        })
        .await;
        assert_eq!(get_user_login(client(), &base, "t").await.unwrap(), "zipped");
    }

    #[tokio::test]
    async fn an_unreachable_api_is_a_network_error() {
        let error = get_user_login(client(), "http://127.0.0.1:1", "dummy-token").await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::Network);
    }
}
