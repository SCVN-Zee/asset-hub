//! Bindings to the JavaScript runtime, and helpers that reproduce JavaScript number and text rules.
//!
//! Pure helpers take plain values so they run in native tests. Functions that touch the runtime
//! (`fetch`, `crypto`, streams) only work inside the Worker.

use std::future::Future;
use std::time::Duration;

use futures_util::future::{select, Either};
use js_sys::{Function, Promise, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{AbortSignal, ReadableStreamDefaultReader};
use worker::Request;

/// `Date.now()`.
pub fn now_ms() -> f64 {
    js_sys::Date::now()
}

/// `crypto.randomUUID()`.
pub fn random_uuid() -> Result<String, JsValue> {
    let crypto = Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))?;
    let function: Function = Reflect::get(&crypto, &JsValue::from_str("randomUUID"))?
        .dyn_into()
        .map_err(|_| JsValue::from_str("crypto.randomUUID is missing"))?;
    function
        .call0(&crypto)?
        .as_string()
        .ok_or_else(|| JsValue::from_str("crypto.randomUUID gave no string"))
}

/// A version 4 UUID from 16 random bytes, in the lower-case form of `crypto.randomUUID()`.
pub fn uuid_v4(mut bytes: [u8; 16]) -> String {
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let mut out = String::with_capacity(36);
    for (i, byte) in bytes.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

const HEX: &[u8; 16] = b"0123456789abcdef";

/// The most UUIDs one `getRandomValues` call can fill (65536 bytes).
const MAX_UUIDS_PER_CALL: usize = 4096;

/// `count` version 4 UUIDs from one `crypto.getRandomValues` call, so the runtime is crossed three
/// times instead of four times for each UUID.
pub fn random_uuids(count: usize) -> Result<Vec<String>, JsValue> {
    if count > MAX_UUIDS_PER_CALL {
        return Err(JsValue::from_str("too many uuids"));
    }
    let crypto = Reflect::get(&js_sys::global(), &JsValue::from_str("crypto"))?;
    let fill: Function = Reflect::get(&crypto, &JsValue::from_str("getRandomValues"))?
        .dyn_into()
        .map_err(|_| JsValue::from_str("crypto.getRandomValues is missing"))?;
    let random = Uint8Array::new_with_length((count * 16) as u32);
    fill.call1(&crypto, &random)?;
    let mut raw = vec![0u8; count * 16];
    random.copy_to(&mut raw);
    let (chunks, _) = raw.as_chunks::<16>();
    Ok(chunks.iter().copied().map(uuid_v4).collect())
}

/// What `fetch` needs besides the URL.
pub struct FetchInit<'a> {
    pub method: &'a str,
    pub headers: &'a [(&'a str, &'a str)],
    pub body: Option<JsValue>,
    pub signal: Option<&'a AbortSignal>,
}

/// The global `fetch`. The reply keeps its body stream, so a caller can hand it on without a copy.
pub async fn fetch(url: &str, init: FetchInit<'_>) -> Result<web_sys::Response, JsValue> {
    let options = web_sys::RequestInit::new();
    options.set_method(init.method);
    let headers = web_sys::Headers::new()?;
    for (name, value) in init.headers {
        headers.set(name, value)?;
    }
    options.set_headers(&headers);
    if let Some(body) = &init.body {
        options.set_body(body);
    }
    if let Some(signal) = init.signal {
        options.set_signal(Some(signal));
    }
    let global = js_sys::global();
    let function: Function = Reflect::get(&global, &JsValue::from_str("fetch"))?
        .dyn_into()
        .map_err(|_| JsValue::from_str("fetch is missing"))?;
    let promise: Promise = function
        .call2(&global, &JsValue::from_str(url), &options)?
        .dyn_into()
        .map_err(|_| JsValue::from_str("fetch gave no promise"))?;
    JsFuture::from(promise)
        .await?
        .dyn_into::<web_sys::Response>()
        .map_err(|_| JsValue::from_str("fetch gave no response"))
}

/// Starts a promise-returning call and ignores its result, as `promise.catch(() => {})` does.
pub async fn settle(promise: Promise) {
    let _ = JsFuture::from(promise).await;
}

/// Runs `future`, or gives `None` when it has not finished after `ms` milliseconds.
pub async fn with_timeout<F: Future>(ms: u64, future: F) -> Option<F::Output> {
    let future = std::pin::pin!(future);
    let delay = std::pin::pin!(worker::Delay::from(Duration::from_millis(ms)));
    match select(future, delay).await {
        Either::Left((value, _)) => Some(value),
        Either::Right(_) => None,
    }
}

/// The chunks of a request body of at most `max` bytes. `None` means the body is larger. A declared
/// `Content-Length` over the limit exits at once. Otherwise the stream is counted chunk by chunk
/// and cancelled as soon as the limit is passed, so an oversized body is never buffered.
async fn read_chunks(req: &Request, max: usize) -> Result<Option<Vec<Uint8Array>>, JsValue> {
    if let Ok(Some(declared)) = req.headers().get("content-length") {
        let declared = js_number(&declared);
        if declared.is_finite() && declared > max as f64 {
            return Ok(None);
        }
    }
    let Some(stream) = req.inner().body() else {
        return Ok(Some(Vec::new()));
    };
    let reader: ReadableStreamDefaultReader = stream.get_reader().unchecked_into();
    // The two keys are built once: each `JsValue::from_str` is a call into JavaScript.
    let (done_key, value_key) = (JsValue::from_str("done"), JsValue::from_str("value"));
    let mut chunks: Vec<Uint8Array> = Vec::new();
    let mut total: usize = 0;
    loop {
        let step = JsFuture::from(reader.read()).await?;
        if Reflect::get(&step, &done_key)?.as_bool() == Some(true) {
            break;
        }
        let chunk: Uint8Array = Reflect::get(&step, &value_key)?.dyn_into().map_err(|_| JsValue::from_str("a body chunk is not bytes"))?;
        total = total.saturating_add(chunk.length() as usize);
        if total > max {
            settle(reader.cancel()).await;
            return Ok(None);
        }
        chunks.push(chunk);
    }
    Ok(Some(chunks))
}

/// Reads a request body of at most `max` bytes into one `Uint8Array` that never enters Wasm
/// memory. `None` means the body is larger. A body that arrives as one chunk is handed on without
/// a copy.
pub async fn read_body_js(req: &Request, max: usize) -> Result<Option<Uint8Array>, JsValue> {
    let Some(mut chunks) = read_chunks(req, max).await? else {
        return Ok(None);
    };
    if chunks.len() == 1 {
        return Ok(chunks.pop());
    }
    let total: u32 = chunks.iter().map(Uint8Array::length).sum();
    let joined = Uint8Array::new_with_length(total);
    let mut at: u32 = 0;
    for chunk in &chunks {
        joined.set(chunk, at);
        at += chunk.length();
    }
    Ok(Some(joined))
}

/// Reads a request body of at most `max` bytes straight into Wasm memory, one copy per chunk and
/// no intermediate `Uint8Array`. Use it only when Rust must read the bytes. `None` means the body
/// is larger.
pub async fn read_body_wasm(req: &Request, max: usize) -> Result<Option<Vec<u8>>, JsValue> {
    let Some(chunks) = read_chunks(req, max).await? else {
        return Ok(None);
    };
    let mut out = vec![0u8; chunks.iter().map(|c| c.length() as usize).sum()];
    let mut at = 0usize;
    for chunk in &chunks {
        let end = at + chunk.length() as usize;
        chunk.copy_to(&mut out[at..end]);
        at = end;
    }
    Ok(Some(out))
}

/// Whether JavaScript counts `c` as white space or a line terminator (`\s`, `String.prototype.trim`).
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{b}' | '\u{c}' | '\r' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}'
            | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn js_trim(text: &str) -> &str {
    text.trim_matches(is_js_space)
}

/// `Number(text)`: `NaN` when the text is not a number. Accepts the `0x`, `0o` and `0b` prefixes,
/// `Infinity`, and decimal forms such as `.5`, `5.` and `1e3`. An empty or blank text is 0.
pub fn js_number(text: &str) -> f64 {
    let text = js_trim(text);
    if text.is_empty() {
        return 0.0;
    }
    let radix = match text.get(..2) {
        Some("0x" | "0X") => Some(16),
        Some("0o" | "0O") => Some(8),
        Some("0b" | "0B") => Some(2),
        _ => None,
    };
    if let Some(radix) = radix {
        let digits = &text[2..];
        if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
            return f64::NAN;
        }
        return digits.chars().fold(0.0, |acc, c| acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0)));
    }
    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    // Rust also accepts `inf`, `nan` and `infinity`; JavaScript does not.
    if !text.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-')) {
        return f64::NAN;
    }
    text.parse::<f64>().unwrap_or(f64::NAN)
}

/// A number as `JSON.stringify` writes it: `3` stays `3`, and a non-finite value is `null`.
pub fn fmt_number(value: f64) -> String {
    if value.is_finite() {
        ryu_js::Buffer::new().format_finite(value).to_owned()
    } else {
        "null".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_print_like_json_stringify() {
        assert_eq!(fmt_number(3.0), "3");
        assert_eq!(fmt_number(0.5), "0.5");
        assert_eq!(fmt_number(1_700_000_000_000.0), "1700000000000");
        assert_eq!(fmt_number(-0.0), "0");
        assert_eq!(fmt_number(1e21), "1e+21");
        assert_eq!(fmt_number(1e-7), "1e-7");
        assert_eq!(fmt_number(0.1 + 0.2), "0.30000000000000004");
        assert_eq!(fmt_number(f64::NAN), "null");
        assert_eq!(fmt_number(f64::INFINITY), "null");
    }

    #[test]
    fn uuids_have_the_version_4_shape() {
        assert_eq!(uuid_v4([0; 16]), "00000000-0000-4000-8000-000000000000");
        assert_eq!(uuid_v4([0xff; 16]), "ffffffff-ffff-4fff-bfff-ffffffffffff");
        let sample = uuid_v4([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        assert_eq!(sample, "12345678-9abc-4ef0-9122-334455667788");
        assert!(crate::validate::is_uuid(&sample));
    }

    #[test]
    fn number_parsing_follows_javascript() {
        for (text, want) in [
            ("", 0.0),
            ("  \t ", 0.0),
            ("12", 12.0),
            (" 12 ", 12.0),
            ("+5", 5.0),
            ("-5", -5.0),
            ("1.0", 1.0),
            ("5.", 5.0),
            (".5", 0.5),
            ("1e3", 1000.0),
            ("0x10", 16.0),
            ("0X1f", 31.0),
            ("0o17", 15.0),
            ("0b101", 5.0),
            ("Infinity", f64::INFINITY),
            ("-Infinity", f64::NEG_INFINITY),
        ] {
            assert_eq!(js_number(text), want, "{text:?}");
        }
        for text in ["abc", "1 2", "1_0", "0x", "0xg", "0b2", "-0x10", "inf", "nan", "infinity", "1e", "e5", "+", ".", "--1", "12px", "\u{85}12"] {
            assert!(js_number(text).is_nan(), "{text:?}");
        }
    }

    #[test]
    fn trim_uses_javascript_white_space() {
        assert_eq!(js_trim("\u{feff} a \u{a0}"), "a");
        // U+0085 is white space for Rust, not for JavaScript.
        assert_eq!(js_trim("\u{85}a"), "\u{85}a");
    }
}
