//! A minimal signed client for one Backblaze B2 bucket over its S3 API. It never lists the bucket
//! (D1 holds every index) and never retries: a hidden retry would multiply requests and mask failures.

use js_sys::Uint8Array;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use worker::Url;

use crate::config::B2Config;
use crate::http::Halt;
use crate::js::{fetch, now_ms, settle, FetchInit};
use crate::sigv4::{amz_datetime, authorization, canonical_path, encode_component, Credentials, Signable};

/// No payload hash: TLS protects the body, and hashing a 20 MB body in the isolate would cost
/// about 10 ms of CPU, the whole budget of a Workers Free request.
const UNSIGNED_PAYLOAD: &str = "UNSIGNED-PAYLOAD";

#[derive(Debug)]
pub enum B2Error {
    /// B2 answered with an error status, so the request is known not to have changed the bucket.
    /// `detail` is the operation, and for a read also the S3 code. It never holds a key.
    Http { status: u16, detail: String },
    /// No answer arrived, so the outcome is unknown. The text is the error name.
    Lost(String),
    /// A reply that cannot be used, for example a PUT without a version id.
    Unusable(&'static str),
}

impl B2Error {
    /// The log line: `b2 <operation> <code> (<message>) <status>`. It holds no key and no signature.
    pub fn log_line(&self) -> String {
        match self {
            B2Error::Http { status, detail } => format!("b2 {detail} {status}"),
            B2Error::Lost(name) => name.clone(),
            B2Error::Unusable(what) => (*what).to_owned(),
        }
    }
}

impl From<B2Error> for Halt {
    fn from(err: B2Error) -> Self {
        Halt::Failed(err.log_line())
    }
}

/// Text between `<tag>` and `</tag>`, for the first occurrence where `accept` takes the text.
fn tag_text<'a>(text: &'a str, tag: &str, accept: impl Fn(&str) -> bool) -> Option<&'a str> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    text.match_indices(&open).find_map(|(at, _)| {
        let rest = &text[at + open.len()..];
        let inner = &rest[..rest.find(&close)?];
        accept(inner).then_some(inner)
    })
}

/// The S3 `<Code>` of a B2 error reply, plus its message when there is one: `Code (message)`. The
/// message has at most 160 characters. A cap reply says so only in its message
/// ("Transaction cap exceeded").
pub fn error_code(text: &str) -> String {
    let code = tag_text(text, "Code", |c| (1..=64).contains(&c.len()) && c.bytes().all(|b| b.is_ascii_alphabetic())).unwrap_or("unknown");
    let message = tag_text(text, "Message", |m| !m.is_empty() && !m.contains('<') && m.encode_utf16().count() <= 160);
    match message {
        Some(message) => format!("{code} ({message})"),
        None => code.to_owned(),
    }
}

pub struct B2 {
    base: String,
    region: String,
    key_id: String,
    app_key: String,
}

impl B2 {
    pub fn new(cfg: &B2Config) -> Self {
        Self {
            base: format!("{}/{}", cfg.endpoint.trim_end_matches('/'), cfg.bucket),
            region: cfg.region.clone(),
            key_id: cfg.key_id.clone(),
            app_key: cfg.app_key.clone(),
        }
    }

    fn url(&self, key: &str, version_id: Option<&str>) -> String {
        let path: Vec<String> = key.split('/').map(encode_component).collect();
        let query = version_id.map(|v| format!("?versionId={}", encode_component(v))).unwrap_or_default();
        format!("{}/{}{query}", self.base, path.join("/"))
    }

    /// The headers that sign `method` on `url` at `datetime`. The host is signed but not returned:
    /// the runtime sets it.
    fn sign(&self, method: &str, url: &str, datetime: &str) -> Result<Vec<(String, String)>, String> {
        let parsed = Url::parse(url).map_err(|_| "b2 url".to_owned())?;
        let host = match (parsed.host_str(), parsed.port()) {
            (Some(host), Some(port)) => format!("{host}:{port}"),
            (Some(host), None) => host.to_owned(),
            (None, _) => return Err("b2 url has no host".to_owned()),
        };
        let query: Vec<(String, String)> = parsed.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect();
        let signed = [
            ("host".to_owned(), host),
            ("x-amz-content-sha256".to_owned(), UNSIGNED_PAYLOAD.to_owned()),
            ("x-amz-date".to_owned(), datetime.to_owned()),
        ];
        let creds = Credentials { access_key_id: &self.key_id, secret_access_key: &self.app_key, region: &self.region, service: "s3" };
        let path = canonical_path(parsed.path());
        let request = Signable { method, path: &path, query: &query, headers: &signed, payload_hash: UNSIGNED_PAYLOAD, datetime };
        let auth = authorization(&creds, &request)?;
        Ok(vec![
            ("authorization".to_owned(), auth),
            ("x-amz-content-sha256".to_owned(), UNSIGNED_PAYLOAD.to_owned()),
            ("x-amz-date".to_owned(), datetime.to_owned()),
        ])
    }

    async fn call(&self, method: &str, key: &str, version_id: Option<&str>, body: Option<JsValue>, content_type: Option<&str>) -> Result<web_sys::Response, B2Error> {
        let url = self.url(key, version_id);
        let signed = self.sign(method, &url, &amz_datetime(now_ms())).map_err(|_| B2Error::Unusable("b2 sign"))?;
        let mut headers: Vec<(&str, &str)> = signed.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        if let Some(content_type) = content_type {
            headers.push(("content-type", content_type));
        }
        let init = FetchInit { method, headers: &headers, body, signal: None };
        fetch(&url, init).await.map_err(|e| match Halt::from(e) {
            Halt::Failed(name) => B2Error::Lost(name),
            Halt::Reply(_) => B2Error::Lost("unknown".to_owned()),
        })
    }

    async fn drop_body(res: &web_sys::Response) {
        if let Some(stream) = res.body() {
            settle(stream.cancel()).await;
        }
    }

    /// The latest version of `key`, or `None` when B2 has none. The caller must consume or cancel
    /// the body.
    pub async fn get(&self, key: &str) -> Result<Option<web_sys::Response>, B2Error> {
        let res = self.call("GET", key, None, None, None).await?;
        if res.status() == 404 {
            Self::drop_body(&res).await;
            return Ok(None);
        }
        if !res.ok() {
            let text = match res.text() {
                Ok(promise) => JsFuture::from(promise).await.ok().and_then(|v| v.as_string()).unwrap_or_default(),
                Err(_) => String::new(),
            };
            return Err(B2Error::Http { status: res.status(), detail: format!("get {}", error_code(&text)) });
        }
        Ok(Some(res))
    }

    /// Stores `body` under `key` and gives the version id B2 assigned.
    pub async fn put(&self, key: &str, body: &Uint8Array, content_type: &str) -> Result<String, B2Error> {
        let res = self.call("PUT", key, None, Some(body.clone().into()), Some(content_type)).await?;
        Self::drop_body(&res).await;
        if !res.ok() {
            return Err(B2Error::Http { status: res.status(), detail: "put".to_owned() });
        }
        // Without the id the version could never be deleted, so its bytes could never be freed.
        match res.headers().get("x-amz-version-id") {
            Ok(Some(id)) if !id.is_empty() => Ok(id),
            _ => Err(B2Error::Unusable("b2 put returned no version id")),
        }
    }

    /// Deletes one version. A delete without `version_id` only adds a hide marker and frees no
    /// storage, so the id is required. Gives `false` when the version was already gone: this call
    /// freed nothing, so the caller must not release bytes for it.
    pub async fn delete(&self, key: &str, version_id: &str) -> Result<bool, B2Error> {
        if version_id.is_empty() {
            return Err(B2Error::Unusable("b2 delete needs a version id"));
        }
        let res = self.call("DELETE", key, Some(version_id), None, None).await?;
        Self::drop_body(&res).await;
        match res.status() {
            404 => Ok(false),
            _ if res.ok() => Ok(true),
            status => Err(B2Error::Http { status, detail: "delete".to_owned() }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> B2 {
        B2 {
            base: "https://s3.us-west-004.backblazeb2.com/asset-hub-cache".to_owned(),
            region: "us-west-004".to_owned(),
            key_id: "AKIDEXAMPLE".to_owned(),
            app_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".to_owned(),
        }
    }

    const KEY: &str = "repos/supercent-io/game/s1/catalog/0b9a4f1e-1111-4222-8333-444455556666.json.gz";
    const VERSION: &str = "4_zabc_d2026_m01";
    const DATETIME: &str = "20260101T000000Z";

    fn authorization_of(method: &str, version: Option<&str>) -> String {
        let b2 = client();
        let signed = b2.sign(method, &b2.url(KEY, version), DATETIME).unwrap_or_default();
        signed.into_iter().find(|(name, _)| name == "authorization").map(|(_, value)| value).unwrap_or_default()
    }

    fn header(signature: &str) -> String {
        format!("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20260101/us-west-004/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256;x-amz-date, Signature={signature}")
    }

    // The expected signatures come from `aws4fetch 1.0.20`, the production signer, through
    // `scripts/sigv4-vectors.mjs`.
    #[test]
    fn a_get_matches_aws4fetch() {
        assert_eq!(authorization_of("GET", None), header("aac2ad2939bceec7ecd3a975214259a9b10cde5d069d54d49f9341339ec8bb98"));
    }

    #[test]
    fn a_get_of_a_version_matches_aws4fetch() {
        assert_eq!(authorization_of("GET", Some(VERSION)), header("b7d528b2fa72fbbd1b2cf4fea138af5ebb1ce21b4df6ca7bb71ce23f4f167c0e"));
    }

    #[test]
    fn a_put_matches_aws4fetch() {
        // `aws4fetch` does not sign `content-type`, so the PUT signature equals the GET shape.
        assert_eq!(authorization_of("PUT", None), header("39e196a1d9cfc7f3300491ec577083d7d4595515682e5f449ce02043b0b774ea"));
    }

    #[test]
    fn a_delete_of_a_version_matches_aws4fetch() {
        assert_eq!(authorization_of("DELETE", Some(VERSION)), header("a591d95d3830321905b6244778d6704d296481f9a3cce7e097e0f565dd4debbc"));
    }

    #[test]
    fn a_key_with_special_characters_matches_aws4fetch() {
        let b2 = client();
        let url = b2.url("repos/o/g/s1/a b!'()*~\u{e9}/x.json.gz", None);
        let signed = b2.sign("PUT", &url, DATETIME).unwrap_or_default();
        let auth = signed.into_iter().find(|(name, _)| name == "authorization").map(|(_, value)| value).unwrap_or_default();
        assert_eq!(auth, header("318ae3250bf18070651ace7beecf67645467f1ee22cc514e4bfaf27e722fcb81"));
    }

    #[test]
    fn urls_encode_each_key_segment_and_the_version() {
        let b2 = client();
        assert_eq!(b2.url("a b/c", Some("v/1")), "https://s3.us-west-004.backblazeb2.com/asset-hub-cache/a%20b/c?versionId=v%2F1");
    }

    #[test]
    fn an_error_reply_gives_its_code_and_message() {
        let body = "<?xml version=\"1.0\"?><Error><Code>AccessDenied</Code><Message>Transaction cap exceeded</Message></Error>";
        assert_eq!(error_code(body), "AccessDenied (Transaction cap exceeded)");
        assert_eq!(error_code("<Error><Code>NoSuchKey</Code></Error>"), "NoSuchKey");
        assert_eq!(error_code("fail"), "unknown");
        assert_eq!(error_code(""), "unknown");
    }

    #[test]
    fn only_plain_words_are_kept() {
        // A code with digits does not match, so the next one is tried.
        assert_eq!(error_code("<Code>E404</Code><Code>Real</Code>"), "Real");
        assert_eq!(error_code("<Code>E404</Code>"), "unknown");
        let long = "x".repeat(161);
        assert_eq!(error_code(&format!("<Code>A</Code><Message>{long}</Message>")), "A");
        let fit = "x".repeat(160);
        assert_eq!(error_code(&format!("<Code>A</Code><Message>{fit}</Message>")), format!("A ({fit})"));
        assert_eq!(error_code("<Code>A</Code><Message></Message>"), "A");
    }

    #[test]
    fn the_log_line_names_the_operation_code_and_status() {
        let err = B2Error::Http { status: 403, detail: "get AccessDenied (Transaction cap exceeded)".to_owned() };
        assert_eq!(err.log_line(), "b2 get AccessDenied (Transaction cap exceeded) 403");
        assert_eq!(B2Error::Http { status: 503, detail: "put".to_owned() }.log_line(), "b2 put 503");
    }
}
