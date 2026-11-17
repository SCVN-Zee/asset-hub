//! AWS Signature Version 4, by hand: about 100 lines on `hmac` and `sha2`, so the Wasm stays small.
//! It signs the `Authorization` header only. The payload hash is an argument, so a caller can pass
//! `UNSIGNED-PAYLOAD` and never hash a large body.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

pub struct Credentials<'a> {
    pub access_key_id: &'a str,
    pub secret_access_key: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

/// A request to sign. Header names are lower-case. The path is the canonical URI, as
/// `canonical_path` makes it. The query holds decoded pairs.
pub struct Signable<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub query: &'a [(String, String)],
    pub headers: &'a [(String, String)],
    pub payload_hash: &'a str,
    /// `YYYYMMDDTHHMMSSZ`.
    pub datetime: &'a str,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Result<Vec<u8>, String> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|_| "hmac key".to_owned())?;
    mac.update(data.as_bytes());
    Ok(mac.finalize().into_bytes().to_vec())
}

/// `encodeURIComponent`: everything except letters, digits and `-_.!~*'()` is percent-encoded.
pub fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-encoding as SigV4 wants it: only letters, digits and `-_.~` stay.
fn encode_rfc3986(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Percent-decodes `text` (a `+` is a space, as `aws4fetch` reads it). `None` when the escapes do not
/// form UTF-8, where `decodeURIComponent` throws.
fn decode_component(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(&b) = bytes.get(at) {
        match b {
            b'+' => out.push(b' '),
            b'%' => {
                let pair = text.get(at + 1..at + 3)?;
                out.push(u8::from_str_radix(pair, 16).ok()?);
                at += 2;
            }
            _ => out.push(b),
        }
        at += 1;
    }
    String::from_utf8(out).ok()
}

/// The canonical URI of an S3 request: each segment of the wire path decoded and encoded again the
/// SigV4 way. `encodeURIComponent` keeps `!'()*`, SigV4 does not, so the two forms differ for a key
/// that holds one of them. A path that does not decode is used as it is.
pub fn canonical_path(wire_path: &str) -> String {
    match decode_component(wire_path) {
        Some(decoded) => decoded.split('/').map(encode_rfc3986).collect::<Vec<_>>().join("/"),
        None => wire_path.to_owned(),
    }
}

fn canonical_query(query: &[(String, String)]) -> String {
    let mut pairs: Vec<&(String, String)> = query.iter().collect();
    pairs.sort();
    let parts: Vec<String> = pairs.iter().map(|(k, v)| format!("{}={}", encode_rfc3986(k), encode_rfc3986(v))).collect();
    parts.join("&")
}

/// The `Authorization` header value.
pub fn authorization(creds: &Credentials<'_>, req: &Signable<'_>) -> Result<String, String> {
    let date = req.datetime.get(..8).ok_or_else(|| "datetime".to_owned())?;
    let mut headers: Vec<&(String, String)> = req.headers.iter().collect();
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed_headers: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let signed_headers = signed_headers.join(";");

    let canonical = format!(
        "{}\n{}\n{}\n{canonical_headers}\n{signed_headers}\n{}",
        req.method,
        req.path,
        canonical_query(req.query),
        req.payload_hash
    );
    let scope = format!("{date}/{}/{}/aws4_request", creds.region, creds.service);
    let to_sign = format!("AWS4-HMAC-SHA256\n{}\n{scope}\n{}", req.datetime, hex(&Sha256::digest(canonical.as_bytes())));

    let mut key = hmac(format!("AWS4{}", creds.secret_access_key).as_bytes(), date)?;
    for part in [creds.region, creds.service, "aws4_request"] {
        key = hmac(&key, part)?;
    }
    let signature = hex(&hmac(&key, &to_sign)?);
    Ok(format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        creds.access_key_id
    ))
}

/// `YYYYMMDDTHHMMSSZ` for a Unix time in milliseconds.
pub fn amz_datetime(ms: f64) -> String {
    let secs = (ms / 1000.0).floor() as i64;
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil date from a day count (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z", rest / 3600, rest % 3600 / 60, rest % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDS: Credentials<'static> = Credentials {
        access_key_id: "AKIDEXAMPLE",
        secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
        region: "us-east-1",
        service: "service",
    };
    const EMPTY_HASH: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    fn pairs(items: &[(&str, &str)]) -> Vec<(String, String)> {
        items.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect()
    }

    /// One case of AWS's published SigV4 test suite: host and date headers, an empty body.
    fn suite(method: &str, query: &[(&str, &str)]) -> String {
        let headers = pairs(&[("host", "example.amazonaws.com"), ("x-amz-date", "20150830T123600Z")]);
        let query = pairs(query);
        let req = Signable { method, path: "/", query: &query, headers: &headers, payload_hash: EMPTY_HASH, datetime: "20150830T123600Z" };
        authorization(&CREDS, &req).unwrap_or_default()
    }

    fn expected(signature: &str) -> String {
        format!("AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date, Signature={signature}")
    }

    #[test]
    fn suite_get_vanilla() {
        assert_eq!(suite("GET", &[]), expected("5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"));
    }

    #[test]
    fn suite_get_vanilla_query() {
        // The suite case has the query `Param1=value1`.
        assert_eq!(suite("GET", &[("Param1", "value1")]), expected("a67d582fa61cc504c4bae71f336f98b97f1ea3c7a6bfe1b6e45aec72011b9aeb"));
    }

    #[test]
    fn suite_get_vanilla_query_order_key_case() {
        assert_eq!(
            suite("GET", &[("Param2", "value2"), ("Param1", "value1")]),
            expected("b97d918cfa904a5beff61c982a1b6f458b799221646efd99d3219ec94cdf2500")
        );
    }

    #[test]
    fn suite_post_header_key_case() {
        assert_eq!(suite("POST", &[]), expected("5da7c1a2acd57cee7505fc6676e4e544621c30862966e37dddb68e92efbe5d6b"));
    }

    #[test]
    fn a_wrong_secret_gives_another_signature() {
        let headers = pairs(&[("host", "example.amazonaws.com"), ("x-amz-date", "20150830T123600Z")]);
        let req = Signable { method: "GET", path: "/", query: &[], headers: &headers, payload_hash: EMPTY_HASH, datetime: "20150830T123600Z" };
        let wrong = Credentials { secret_access_key: "wrong-secret", ..CREDS };
        assert_ne!(authorization(&wrong, &req), authorization(&CREDS, &req));
    }

    #[test]
    fn component_encoding_follows_encode_uri_component() {
        assert_eq!(encode_component("a b/c"), "a%20b%2Fc");
        assert_eq!(encode_component("A-z_0.9!~*'()"), "A-z_0.9!~*'()");
        assert_eq!(encode_component("é"), "%C3%A9");
        assert_eq!(encode_rfc3986("a!b*"), "a%21b%2A");
    }

    #[test]
    fn the_canonical_path_encodes_what_encode_uri_component_keeps() {
        assert_eq!(canonical_path("/b/a%20b!'()*~%C3%A9/x.json.gz"), "/b/a%20b%21%27%28%29%2A~%C3%A9/x.json.gz");
        assert_eq!(canonical_path("/b/plain-key_1.json"), "/b/plain-key_1.json");
        assert_eq!(canonical_path("/b/a+b"), "/b/a%20b");
        // Escapes that are not UTF-8 leave the path as it is.
        assert_eq!(canonical_path("/b/%FF"), "/b/%FF");
        assert_eq!(canonical_path("/b/%4"), "/b/%4");
    }

    #[test]
    fn datetime_is_utc_and_zero_padded() {
        assert_eq!(amz_datetime(0.0), "19700101T000000Z");
        assert_eq!(amz_datetime(1_767_225_600_000.0), "20260101T000000Z");
        assert_eq!(amz_datetime(1_709_251_199_999.0), "20240229T235959Z");
        assert_eq!(amz_datetime(1_709_251_200_000.0), "20240301T000000Z");
        assert_eq!(amz_datetime(1_441_024_560_000.0), "20150831T123600Z");
    }
}
