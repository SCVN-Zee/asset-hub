//! Redis for derived data only, through the Upstash REST API. Each call gives `None` on an error
//! or a timeout, so a Redis outage never fails a request that can work without it.
//!
//! A single command is `POST <url>` with a JSON array. A pipeline is `POST <url>/pipeline` with an
//! array of such arrays. Each reply is `{"result": ...}` or `{"error": "..."}`.

use std::ops::Range;

use serde_json::Value;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use worker::console_error;

use crate::js::{fetch, fmt_number, js_number, with_timeout, FetchInit};

/// Every Redis call gives up after this long; callers then behave as if Redis were down.
pub const REDIS_TIMEOUT_MS: u64 = 300;
/// The cron rebuild is not latency sensitive; it gets a longer budget per pipeline.
pub const REDIS_REBUILD_TIMEOUT_MS: u64 = 5000;
/// Redis commands per ranking in `replace_rankings`.
pub const REPLACE_RANKING_COMMANDS: usize = 5;
/// Upstash accepts large pipelines; stay well below its request size and command limits.
pub const MAX_PIPELINE_COMMANDS: usize = 500;
pub const MAX_PIPELINE_BYTES: usize = 900 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub member: String,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RankingInput {
    pub key: String,
    pub members: Vec<Ranked>,
}

/// The top of a ranking and the time it was built (`None` if unknown).
#[derive(Debug, PartialEq)]
pub struct Ranking {
    pub items: Vec<Ranked>,
    pub built_at: Option<f64>,
}

/// One argument of a Redis command. A number keeps its JSON number form on the wire.
#[derive(Debug, Clone, PartialEq)]
pub enum Arg {
    Str(String),
    Num(f64),
}

pub type Command = Vec<Arg>;

fn s(text: &str) -> Arg {
    Arg::Str(text.to_owned())
}

fn command_json(command: &Command) -> String {
    let parts: Vec<String> = command
        .iter()
        .map(|arg| match arg {
            Arg::Str(text) => crate::http::quote(text),
            Arg::Num(number) => fmt_number(*number),
        })
        .collect();
    format!("[{}]", parts.join(","))
}

/// A list of commands as the JSON array that goes into a request body.
pub fn commands_json(commands: &[Command]) -> String {
    let parts: Vec<String> = commands.iter().map(command_json).collect();
    format!("[{}]", parts.join(","))
}

/// The commands of one ranking, as they are serialized into the pipeline request body.
pub fn ranking_commands(ranking: &RankingInput, built_at: f64, ttl_sec: u64) -> Vec<Command> {
    let tmp = format!("{}:new", ranking.key);
    let mut zadd = vec![s("ZADD"), s(&tmp)];
    for m in &ranking.members {
        zadd.push(Arg::Num(m.score));
        zadd.push(Arg::Str(m.member.clone()));
    }
    vec![
        vec![s("DEL"), s(&tmp)],
        zadd,
        vec![s("RENAME"), s(&tmp), s(&ranking.key)],
        vec![s("EXPIRE"), s(&ranking.key), Arg::Num(ttl_sec as f64)],
        vec![s("SET"), Arg::Str(format!("{}:at", ranking.key)), Arg::Str(fmt_number(built_at)), s("EX"), Arg::Num(ttl_sec as f64)],
    ]
}

/// Splits rankings into runs that each fit one pipeline, measuring the real serialized size (UTF-8
/// bytes of the JSON command array). A ranking is never split: one that alone exceeds the limit is
/// sent alone and counted in the second value. The runs are index ranges into `rankings`.
pub fn chunk_rankings(rankings: &[RankingInput], built_at: f64, ttl_sec: u64) -> (Vec<Range<usize>>, usize) {
    let mut chunks = Vec::new();
    let mut start = 0;
    let mut bytes = 2; // the enclosing "[" and "]"
    let mut oversized = 0;
    for (i, ranking) in rankings.iter().enumerate() {
        // Minus the outer brackets, plus a comma.
        let size = commands_json(&ranking_commands(ranking, built_at, ttl_sec)).len() - 1;
        if size > MAX_PIPELINE_BYTES {
            oversized += 1;
        }
        let count = i - start;
        let full = (count + 1) * REPLACE_RANKING_COMMANDS > MAX_PIPELINE_COMMANDS || bytes + size > MAX_PIPELINE_BYTES;
        if count > 0 && full {
            chunks.push(start..i);
            start = i;
            bytes = 2;
        }
        bytes += size;
    }
    if start < rankings.len() {
        chunks.push(start..rankings.len());
    }
    (chunks, oversized)
}

/// Commands that failed in one pipeline reply. A pipeline that failed as a whole counts every command.
pub fn failed_commands(commands: usize, reply: Option<&Value>) -> usize {
    match reply {
        Some(Value::Array(replies)) => replies.iter().filter(|r| r.get("error").is_some()).count(),
        _ => commands,
    }
}

fn as_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `[member, score, member, score, ...]` as ranked members.
fn pairs(flat: &[Value]) -> Vec<Ranked> {
    flat.as_chunks::<2>().0.iter().map(|pair| Ranked { member: as_text(&pair[0]), score: js_number(&as_text(&pair[1])) }).collect()
}

pub struct Redis {
    url: String,
    token: String,
}

impl Redis {
    pub fn new(url: &str, token: &str) -> Self {
        Self { url: url.trim_end_matches('/').to_owned(), token: token.to_owned() }
    }

    /// Sends one request and gives the parsed reply. `name` is the command name for the log: keys
    /// can hold hashes of tokens, so they are never logged.
    async fn send(&self, suffix: &str, body: String, name: &str, timeout_ms: u64) -> Option<Value> {
        let url = format!("{}{suffix}", self.url);
        let auth = format!("Bearer {}", self.token);
        let work = async {
            let headers = [("authorization", auth.as_str()), ("content-type", "application/json")];
            let init = FetchInit { method: "POST", headers: &headers, body: Some(JsValue::from_str(&body)), signal: None };
            let res = fetch(&url, init).await.ok()?;
            let ok = res.ok();
            let text = JsFuture::from(res.text().ok()?).await.ok()?.as_string()?;
            if !ok {
                return None;
            }
            serde_json::from_str::<Value>(&text).ok()
        };
        match with_timeout(timeout_ms, work).await {
            Some(Some(reply)) => Some(reply),
            Some(None) => {
                console_error!("redis.error {name}");
                None
            }
            None => {
                console_error!("redis.error {name} timeout");
                None
            }
        }
    }

    /// The `result` of a single-command reply, `None` for an error reply.
    async fn command(&self, command: Command, name: &str) -> Option<Value> {
        let reply = self.send("", command_json(&command), name, REDIS_TIMEOUT_MS).await?;
        if reply.get("error").is_some() {
            console_error!("redis.error {name}");
            return None;
        }
        reply.get("result").cloned()
    }

    /// `None` is a failure. `Some(None)` is a miss.
    pub async fn get(&self, key: &str) -> Option<Option<String>> {
        match self.command(vec![s("GET"), s(key)], "get").await? {
            Value::Null => Some(None),
            other => Some(Some(as_text(&other))),
        }
    }

    pub async fn set(&self, key: &str, value: &str, ttl_sec: u64) -> Option<bool> {
        let command = vec![s("SET"), s(key), s(value), s("EX"), Arg::Num(ttl_sec as f64)];
        Some(self.command(command, "set").await? == Value::String("OK".to_owned()))
    }

    /// The top `count` members of a ranking plus the time it was built, in one request.
    pub async fn ranking(&self, key: &str, count: usize) -> Option<Ranking> {
        let commands = vec![
            vec![s("ZRANGE"), s(key), Arg::Num(0.0), Arg::Num(count as f64 - 1.0), s("REV"), s("WITHSCORES")],
            vec![s("GET"), Arg::Str(format!("{key}:at"))],
        ];
        let reply = self.send("/pipeline", commands_json(&commands), "ranking", REDIS_TIMEOUT_MS).await?;
        let replies = reply.as_array().filter(|r| r.len() == 2 && r.iter().all(|x| x.get("error").is_none()))?;
        let flat = replies[0].get("result")?.as_array()?;
        let built_at = match replies[1].get("result")? {
            Value::Null => None,
            at => Some(js_number(&as_text(at))).filter(|n| n.is_finite()),
        };
        Some(Ranking { items: pairs(flat), built_at })
    }

    /// Replaces many rankings, each as DEL tmp, ZADD tmp, RENAME tmp key, EXPIRE key, SET key:at.
    /// All groups share one pipeline; a new pipeline starts only above `MAX_PIPELINE_COMMANDS`
    /// commands or `MAX_PIPELINE_BYTES` serialized bytes. Gives the number of failed commands.
    /// Each ranking must have at least one member.
    pub async fn replace_rankings(&self, rankings: &[RankingInput], built_at: f64, ttl_sec: u64) -> usize {
        let (chunks, oversized) = chunk_rankings(rankings, built_at, ttl_sec);
        if oversized > 0 {
            console_error!("redis.pipeline.oversized {oversized}");
        }
        let mut failed = 0;
        for range in chunks {
            let commands: Vec<Command> = rankings[range.clone()].iter().flat_map(|r| ranking_commands(r, built_at, ttl_sec)).collect();
            let reply = self.send("/pipeline", commands_json(&commands), "replaceRankings", REDIS_REBUILD_TIMEOUT_MS).await;
            failed += failed_commands(range.len() * REPLACE_RANKING_COMMANDS, reply.as_ref());
        }
        failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rankings(n: usize) -> Vec<RankingInput> {
        (0..n).map(|i| RankingInput { key: format!("pop:o/r{i}"), members: vec![Ranked { member: "a".repeat(40), score: 1.0 }] }).collect()
    }

    fn wide(key: &str, members: usize, member_length: usize) -> RankingInput {
        RankingInput {
            key: key.to_owned(),
            // Multi-byte characters: the byte length is double the character count.
            members: (0..members).map(|j| Ranked { member: format!("{}{j}", "é".repeat(member_length)), score: 0.123456789 + j as f64 }).collect(),
        }
    }

    fn serialized(chunk: &[RankingInput]) -> usize {
        let commands: Vec<Command> = chunk.iter().flat_map(|r| ranking_commands(r, 1.0, 60)).collect();
        commands_json(&commands).len()
    }

    #[test]
    fn a_ranking_is_five_commands_with_javascript_numbers() {
        let ranking = RankingInput {
            key: "pop:o/r".into(),
            members: vec![Ranked { member: "aa".into(), score: 3.0 }, Ranked { member: "bb".into(), score: 0.5 }],
        };
        assert_eq!(
            commands_json(&ranking_commands(&ranking, 1_700_000_000_000.0, 604_800)),
            r#"[["DEL","pop:o/r:new"],["ZADD","pop:o/r:new",3,"aa",0.5,"bb"],["RENAME","pop:o/r:new","pop:o/r"],["EXPIRE","pop:o/r",604800],["SET","pop:o/r:at","1700000000000","EX",604800]]"#
        );
    }

    #[test]
    fn five_hundred_commands_fit_one_pipeline() {
        let input = rankings(100);
        let (chunks, oversized) = chunk_rankings(&input, 1.0, 60);
        assert_eq!(chunks, vec![0..100]);
        assert_eq!(oversized, 0);
        let commands: Vec<Command> = input.iter().flat_map(|r| ranking_commands(r, 1.0, 60)).collect();
        assert_eq!(commands.len(), 500);
        let names: Vec<&Arg> = commands[..5].iter().map(|c| &c[0]).collect();
        assert_eq!(names, vec![&s("DEL"), &s("ZADD"), &s("RENAME"), &s("EXPIRE"), &s("SET")]);
    }

    #[test]
    fn a_pipeline_splits_above_five_hundred_commands() {
        let (chunks, _) = chunk_rankings(&rankings(101), 1.0, 60);
        assert_eq!(chunks.iter().map(|c| c.len() * REPLACE_RANKING_COMMANDS).collect::<Vec<_>>(), vec![500, 5]);
    }

    #[test]
    fn pipelines_stay_under_900_kib_by_serialized_size() {
        // Each group is about 300 KiB serialized, though it has only 200 members.
        let groups: Vec<RankingInput> = (0..7).map(|i| wide(&format!("pop:o/w{i}"), 200, 700)).collect();
        let (chunks, oversized) = chunk_rankings(&groups, 1.0, 60);
        assert_eq!(oversized, 0);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(serialized(&groups[chunk.clone()]) <= MAX_PIPELINE_BYTES);
        }
        let covered: Vec<usize> = chunks.into_iter().flatten().collect();
        assert_eq!(covered, (0..7).collect::<Vec<_>>());
    }

    #[test]
    fn an_oversized_group_is_sent_alone() {
        let huge = wide("pop:o/huge", 200, 3000); // about 1.2 MB
        let rest = rankings(3);
        let input = vec![rest[0].clone(), huge.clone(), rest[1].clone(), rest[2].clone()];
        let (chunks, oversized) = chunk_rankings(&input, 1.0, 60);
        assert_eq!(oversized, 1);
        assert!(serialized(&[huge]) > MAX_PIPELINE_BYTES);
        assert_eq!(chunks.iter().map(|c| c.len() * REPLACE_RANKING_COMMANDS).collect::<Vec<_>>(), vec![5, 5, 10]);
    }

    #[test]
    fn no_rankings_make_no_chunks() {
        assert_eq!(chunk_rankings(&[], 1.0, 60), (vec![], 0));
    }

    #[test]
    fn failed_commands_are_counted_without_failing() {
        let one_error: Value = serde_json::from_str(r#"[{"result":"OK"},{"result":"OK"},{"error":"ERR"},{"result":"OK"}]"#).unwrap_or(Value::Null);
        assert_eq!(failed_commands(10, Some(&one_error)), 1);
        let clean: Value = serde_json::from_str(r#"[{"result":"OK"}]"#).unwrap_or(Value::Null);
        assert_eq!(failed_commands(10, Some(&clean)), 0);
    }

    #[test]
    fn a_failed_pipeline_counts_every_command() {
        assert_eq!(failed_commands(15, None), 15);
        assert_eq!(failed_commands(15, Some(&Value::Null)), 15);
    }

    #[test]
    fn pairs_read_member_and_score() {
        let flat = vec![Value::String("a".into()), Value::String("3".into()), Value::String("b".into()), Value::String("1.5".into())];
        assert_eq!(pairs(&flat), vec![Ranked { member: "a".into(), score: 3.0 }, Ranked { member: "b".into(), score: 1.5 }]);
        assert_eq!(pairs(&flat[..3]).len(), 1);
    }
}
