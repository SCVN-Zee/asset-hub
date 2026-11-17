//! `POST /events`: anonymous usage events, de-duplicated by id. Nothing here touches Redis.

use serde_json::Value;
use worker::D1Type;

use crate::ctx::RepoCtx;
use crate::http::{decode_text, json, quote_utf16, read_body_small, stop, Handled};
use crate::js::now_ms;
use crate::validate::{is_sha40, is_uuid};

const MAX_BODY_BYTES: usize = 32 * 1024;
const MAX_EVENTS: usize = 100;
const MAX_AGE_MS: f64 = 7.0 * 24.0 * 60.0 * 60.0 * 1000.0;
const MAX_AHEAD_MS: f64 = 5.0 * 60.0 * 1000.0;
/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_991.0;
/// A rejected id is cut to this many UTF-16 code units.
const REJECTED_ID_UNITS: usize = 64;

#[derive(Debug, PartialEq)]
pub struct UsageEvent {
    pub id: String,
    pub sha: String,
    pub kind: &'static str,
    pub ts: f64,
}

/// `Number.isSafeInteger`: a number with no fractional part and an absolute value of at most 2^53 - 1.
fn safe_integer(value: &Value) -> Option<f64> {
    let n = value.as_f64()?;
    (n.fract() == 0.0 && n.abs() <= MAX_SAFE_INTEGER).then_some(n)
}

/// The event when every field is valid at time `now`.
pub fn valid_event(value: &Value, now: f64) -> Option<UsageEvent> {
    let object = value.as_object()?;
    let id = object.get("id")?.as_str().filter(|id| is_uuid(id))?;
    let sha = object.get("sha")?.as_str().filter(|sha| is_sha40(sha))?;
    let kind = match object.get("kind")?.as_str()? {
        "view" => "view",
        "send" => "send",
        _ => return None,
    };
    let ts = safe_integer(object.get("ts")?)?;
    (ts >= now - MAX_AGE_MS && ts <= now + MAX_AHEAD_MS).then(|| UsageEvent { id: id.to_ascii_lowercase(), sha: sha.to_owned(), kind, ts })
}

/// The id of an item that is not valid, cut to 64 UTF-16 code units, as a JSON string literal.
fn rejected_id(item: &Value) -> Option<String> {
    let id = item.as_object()?.get("id")?.as_str()?;
    let units: Vec<u16> = id.encode_utf16().take(REJECTED_ID_UNITS).collect();
    Some(quote_utf16(&units))
}

/// The accepted events and the JSON literals of the rejected ids, in the order of the batch.
pub fn sort_events(list: &[Value], now: f64) -> (Vec<UsageEvent>, Vec<String>) {
    let mut valid = Vec::new();
    let mut rejected = Vec::new();
    for item in list {
        match valid_event(item, now) {
            Some(event) => valid.push(event),
            None => rejected.extend(rejected_id(item)),
        }
    }
    (valid, rejected)
}

/// The `events` array of a parsed body, when it holds 1 to 100 items.
pub fn event_list(parsed: &Value) -> Option<&[Value]> {
    let list = parsed.get("events")?.as_array()?;
    (1..=MAX_EVENTS).contains(&list.len()).then_some(list.as_slice())
}

/// `{"accepted":N,"rejected":[...]}`.
pub fn reply_body(accepted: usize, rejected: &[String]) -> String {
    format!("{{\"accepted\":{accepted},\"rejected\":[{}]}}", rejected.join(","))
}

pub async fn post_events(ctx: &RepoCtx<'_>) -> Handled<web_sys::Response> {
    let Some(body) = read_body_small(ctx.req, MAX_BODY_BYTES).await? else {
        return Err(stop("badRequest", 400));
    };
    let Ok(parsed) = serde_json::from_str::<Value>(&decode_text(&body)) else {
        return Err(stop("badRequest", 400));
    };
    let Some(list) = event_list(&parsed) else {
        return Err(stop("badRequest", 400));
    };

    let (valid, rejected) = sort_events(list, now_ms());
    if !valid.is_empty() {
        let db = ctx.db()?;
        // One prepared statement, bound once per event.
        let insert = db.prepare("INSERT OR IGNORE INTO events (id, repo, sha, kind, ts) VALUES (?, ?, ?, ?, ?)");
        let mut statements = Vec::with_capacity(valid.len());
        for event in &valid {
            let values = [D1Type::Text(&event.id), D1Type::Text(ctx.slug), D1Type::Text(&event.sha), D1Type::Text(event.kind), D1Type::Real(event.ts)];
            statements.push(insert.bind_refs(&values)?);
        }
        db.batch(statements).await?;
    }
    Ok(json(reply_body(valid.len(), &rejected), 200, &[])?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_800_000_000_000.0;
    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
    const ID: &str = "00000000-0000-4000-8000-000000000001";

    fn parse(text: &str) -> Value {
        serde_json::from_str(text).unwrap_or(Value::Null)
    }

    fn event(extra: &str) -> Value {
        parse(&format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view","ts":{NOW}{extra}}}"#))
    }

    #[test]
    fn a_valid_event_is_accepted_with_a_lower_case_id() {
        let upper = parse(&format!(r#"{{"id":"{}","sha":"{SHA}","kind":"send","ts":{NOW}}}"#, ID.replace("000001", "00000A").to_uppercase()));
        let got = valid_event(&upper, NOW);
        assert_eq!(got.map(|e| (e.id, e.kind)), Some(("00000000-0000-4000-8000-00000000000a".to_owned(), "send")));
        assert!(valid_event(&event(""), NOW).is_some());
    }

    #[test]
    fn each_bad_field_rejects_the_event() {
        for text in [
            r#"{"id":"x","sha":"s","kind":"view","ts":1}"#.to_owned(),
            format!(r#"{{"id":"{ID}","sha":"nope","kind":"view","ts":{NOW}}}"#),
            format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"delete","ts":{NOW}}}"#),
            format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view","ts":"{NOW}"}}"#),
            format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view","ts":{}}}"#, NOW + 0.5),
            format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view"}}"#),
            format!(r#"{{"id":7,"sha":"{SHA}","kind":"view","ts":{NOW}}}"#),
            "[]".to_owned(),
            "null".to_owned(),
            "\"text\"".to_owned(),
        ] {
            assert_eq!(valid_event(&parse(&text), NOW), None, "{text}");
        }
    }

    #[test]
    fn the_time_window_is_seven_days_back_and_five_minutes_ahead() {
        let at = |ts: f64| parse(&format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view","ts":{ts}}}"#));
        assert!(valid_event(&at(NOW - MAX_AGE_MS), NOW).is_some());
        assert!(valid_event(&at(NOW - MAX_AGE_MS - 1.0), NOW).is_none());
        assert!(valid_event(&at(NOW + MAX_AHEAD_MS), NOW).is_some());
        assert!(valid_event(&at(NOW + MAX_AHEAD_MS + 1.0), NOW).is_none());
    }

    #[test]
    fn a_whole_number_written_with_a_fraction_is_a_safe_integer() {
        // `Number.isSafeInteger(1.0)` is true, and `1.0` parses to the same number as `1`.
        let at = format!(r#"{{"id":"{ID}","sha":"{SHA}","kind":"view","ts":{NOW:.1}}}"#);
        assert!(valid_event(&parse(&at), NOW).is_some());
        assert_eq!(safe_integer(&parse("9007199254740991")), Some(MAX_SAFE_INTEGER));
        assert_eq!(safe_integer(&parse("9007199254740992")), None);
        assert_eq!(safe_integer(&parse("-3")), Some(-3.0));
        assert_eq!(safe_integer(&parse("1e3")), Some(1000.0));
    }

    #[test]
    fn rejected_ids_keep_the_batch_order_and_skip_items_without_a_string_id() {
        let list = vec![event(""), parse(r#"{"id":"b","kind":"x"}"#), parse(r#"{"id":5}"#), parse("null"), parse(r#"{"kind":"view"}"#), parse(r#"{"id":"a\"q"}"#)];
        let (valid, rejected) = sort_events(&list, NOW);
        assert_eq!(valid.len(), 1);
        assert_eq!(rejected, vec!["\"b\"".to_owned(), "\"a\\\"q\"".to_owned()]);
    }

    #[test]
    fn a_rejected_id_is_cut_to_64_utf16_units() {
        let long = "x".repeat(70);
        let item = parse(&format!(r#"{{"id":"{long}"}}"#));
        assert_eq!(rejected_id(&item), Some(format!("\"{}\"", "x".repeat(64))));
        // The emoji is two units. A cut inside it leaves a lone surrogate, which is escaped.
        let emoji = format!("{}😀tail", "y".repeat(63));
        let item = parse(&format!(r#"{{"id":"{emoji}"}}"#));
        assert_eq!(rejected_id(&item), Some(format!("\"{}\\ud83d\"", "y".repeat(63))));
    }

    #[test]
    fn the_batch_holds_one_to_one_hundred_events() {
        let batch = |n: usize| parse(&format!(r#"{{"events":[{}]}}"#, vec!["1"; n].join(",")));
        assert!(event_list(&batch(1)).is_some());
        assert!(event_list(&batch(100)).is_some());
        assert!(event_list(&batch(0)).is_none());
        assert!(event_list(&batch(101)).is_none());
        assert!(event_list(&parse(r#"{"events":"x"}"#)).is_none());
        assert!(event_list(&parse("[]")).is_none());
        assert!(event_list(&parse("null")).is_none());
    }

    #[test]
    fn the_reply_keeps_the_key_order() {
        assert_eq!(reply_body(1, &[]), r#"{"accepted":1,"rejected":[]}"#);
        assert_eq!(reply_body(0, &["\"a\"".into(), "\"b\"".into()]), "{\"accepted\":0,\"rejected\":[\"a\",\"b\"]}");
    }

    #[test]
    fn a_duplicate_key_keeps_the_last_value() {
        let value = parse(r#"{"events":[1],"events":[1,2]}"#);
        assert_eq!(event_list(&value).map(<[Value]>::len), Some(2));
    }
}
