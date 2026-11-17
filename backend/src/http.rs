//! JSON replies with a stable shape, the early-exit type, and request header helpers.

use wasm_bindgen::{JsCast, JsValue};
use worker::{Request, Response};

use crate::js::read_body_wasm;

pub const JSON_TYPE: &str = "application/json; charset=utf-8";

/// Why a handler stopped early.
pub enum Halt {
    /// A complete reply: a refusal such as 401, or a finished answer.
    Reply(web_sys::Response),
    /// An unexpected failure. The text is safe to log: it never holds a key, a token or a signature.
    Failed(String),
}

pub type Handled<T> = Result<T, Halt>;

impl Halt {
    pub fn failed(label: impl Into<String>) -> Self {
        Halt::Failed(label.into())
    }
}

impl From<worker::Error> for Halt {
    fn from(_: worker::Error) -> Self {
        // The error text can hold SQL or request details, so only its kind is kept.
        Halt::Failed("Error".to_owned())
    }
}

impl From<JsValue> for Halt {
    fn from(value: JsValue) -> Self {
        let name = value.dyn_ref::<js_sys::Error>().and_then(|e| e.name().as_string());
        Halt::Failed(name.unwrap_or_else(|| "unknown".to_owned()))
    }
}

/// A JSON string literal as `JSON.stringify` writes it.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        escape_into(c, &mut out);
    }
    out.push('"');
    out
}

/// A JSON string literal from UTF-16 code units. A lone surrogate becomes `\udXXX`, as in
/// `JSON.stringify`.
pub fn quote_utf16(units: &[u16]) -> String {
    let mut out = String::with_capacity(units.len() + 2);
    out.push('"');
    for piece in char::decode_utf16(units.iter().copied()) {
        match piece {
            Ok(c) => escape_into(c, &mut out),
            Err(lone) => out.push_str(&format!("\\u{:04x}", lone.unpaired_surrogate())),
        }
    }
    out.push('"');
    out
}

fn escape_into(c: char, out: &mut String) {
    match c {
        '"' => out.push_str("\\\""),
        '\\' => out.push_str("\\\\"),
        '\u{8}' => out.push_str("\\b"),
        '\u{c}' => out.push_str("\\f"),
        '\n' => out.push_str("\\n"),
        '\r' => out.push_str("\\r"),
        '\t' => out.push_str("\\t"),
        c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
        c => out.push(c),
    }
}

/// A JSON reply. `body` is already serialized, in the key order of the contract.
pub fn json(body: String, status: u16, extra: &[(&str, &str)]) -> worker::Result<web_sys::Response> {
    let mut res = Response::ok(body)?.with_status(status);
    let headers = res.headers_mut();
    headers.set("content-type", JSON_TYPE)?;
    for (name, value) in extra {
        headers.set(name, value)?;
    }
    Ok(res.into())
}

/// `{"error":"<code>"}` with `status`.
pub fn error_json(code: &str, status: u16) -> worker::Result<web_sys::Response> {
    json(format!("{{\"error\":{}}}", quote(code)), status, &[])
}

/// An early exit with `{"error":"<code>"}`.
pub fn stop(code: &str, status: u16) -> Halt {
    match error_json(code, status) {
        Ok(res) => Halt::Reply(res),
        Err(e) => e.into(),
    }
}

/// The cache namespace number the desktop sends: one to six ASCII digits.
pub fn parse_schema(raw: Option<&str>) -> Option<u32> {
    let raw = raw?;
    if (1..=6).contains(&raw.len()) && raw.bytes().all(|b| b.is_ascii_digit()) {
        raw.parse().ok()
    } else {
        None
    }
}

/// The schema of the request, or an early 400 `badSchema`.
pub fn read_schema(req: &Request) -> Handled<u32> {
    let raw = req.headers().get("x-asset-hub-schema")?;
    parse_schema(raw.as_deref()).ok_or_else(|| stop("badSchema", 400))
}

/// A header value, `None` when it is absent.
pub fn header(req: &Request, name: &str) -> Handled<Option<String>> {
    Ok(req.headers().get(name)?)
}

/// Reads a body of at most `max` bytes (4 MB or less) into Wasm memory. `None` means it is larger.
pub async fn read_body_small(req: &Request, max: usize) -> Handled<Option<Vec<u8>>> {
    Ok(read_body_wasm(req, max).await?)
}

/// Decodes a body as JavaScript's `TextDecoder` does: bad UTF-8 turns into U+FFFD and one leading
/// byte order mark is dropped.
pub fn decode_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').map_or_else(|| text.to_string(), str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_matches_json_stringify() {
        assert_eq!(quote("plain"), "\"plain\"");
        assert_eq!(quote("a\"b\\c"), "\"a\\\"b\\\\c\"");
        assert_eq!(quote("\n\r\t\u{8}\u{c}"), "\"\\n\\r\\t\\b\\f\"");
        assert_eq!(quote("\u{1}\u{1f}"), "\"\\u0001\\u001f\"");
        assert_eq!(quote("\u{7f}é€😀\u{2028}"), "\"\u{7f}é€😀\u{2028}\"");
        assert_eq!(quote("\"0b9a\""), "\"\\\"0b9a\\\"\"");
    }

    #[test]
    fn a_lone_surrogate_is_escaped() {
        let cut: Vec<u16> = "a😀".encode_utf16().take(2).collect();
        assert_eq!(quote_utf16(&cut), "\"a\\ud83d\"");
        let whole: Vec<u16> = "a😀".encode_utf16().collect();
        assert_eq!(quote_utf16(&whole), "\"a😀\"");
    }

    #[test]
    fn schema_is_one_to_six_digits() {
        assert_eq!(parse_schema(Some("1")), Some(1));
        assert_eq!(parse_schema(Some("000012")), Some(12));
        assert_eq!(parse_schema(Some("999999")), Some(999_999));
        for bad in [None, Some(""), Some("abc"), Some("1.5"), Some("1234567"), Some("-1"), Some(" 1"), Some("１")] {
            assert_eq!(parse_schema(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn text_decoding_follows_text_decoder() {
        assert_eq!(decode_text(b"\xef\xbb\xbf{\"a\":1}"), "{\"a\":1}");
        assert_eq!(decode_text(b"\xef\xbb\xbf\xef\xbb\xbfx"), "\u{feff}x");
        assert_eq!(decode_text(b"a\xffb"), "a\u{fffd}b");
        assert_eq!(decode_text(b""), "");
    }
}
