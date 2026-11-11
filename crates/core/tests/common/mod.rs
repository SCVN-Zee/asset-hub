//! Shared helpers of the golden tests: load the cases captured from the TypeScript implementation,
//! decode their tagged values and compare results.
#![allow(dead_code)]

use base64::Engine;
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

pub fn golden_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden")
}

/// The on-disk path of a file of the synthetic project. The project's `.gitattributes` is stored under
/// another name, so that it does not apply to this repository.
pub fn project_file(path: &str) -> PathBuf {
    let on_disk = if path == ".gitattributes" || path.ends_with("/.gitattributes") { path.replace(".gitattributes", "gitattributes.fixture") } else { path.to_string() };
    golden_root().join("inputs/repo").join(on_disk)
}

pub struct Case {
    pub name: String,
    pub args: Vec<Value>,
    pub args_after: Option<Vec<Value>>,
    pub result: Option<Value>,
    pub throws: Option<Value>,
    pub raw: Value,
}

/// Every case of function `func` under `expected/<dir>` (not its sub-folders), sorted by name.
pub fn cases(dir: &str, func: &str) -> Vec<Case> {
    let folder = golden_root().join("expected").join(dir);
    let mut out = Vec::new();
    let entries = std::fs::read_dir(&folder).unwrap_or_else(|e| panic!("read {}: {e}", folder.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw: Value = serde_json::from_str(&std::fs::read_to_string(&path).expect("read case")).expect("case json");
        if raw["fn"] != func {
            continue;
        }
        out.push(Case {
            name: raw["case"].as_str().unwrap_or("?").to_string(),
            args: raw["args"].as_array().cloned().unwrap_or_default().iter().map(norm).collect(),
            args_after: raw.get("argsAfter").and_then(|a| a.as_array()).map(|a| a.iter().map(norm).collect()),
            result: raw.get("result").map(norm),
            throws: raw.get("throws").map(norm),
            raw,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(!out.is_empty(), "no golden cases for {func} in {dir}");
    out
}

/// Decodes the bytes of a tagged `bytes` value.
pub fn tagged_bytes(v: &Value) -> Vec<u8> {
    if let Some(b64) = v.get("b64").and_then(|b| b.as_str()) {
        return base64::engine::general_purpose::STANDARD.decode(b64).expect("base64");
    }
    if let Some(name) = v.get("file").and_then(|f| f.as_str()) {
        return std::fs::read(golden_root().join("expected/blobs").join(name)).unwrap_or_else(|e| panic!("blob {name}: {e}"));
    }
    let hash = v["blob"].as_str().expect("bytes without data");
    std::fs::read(golden_root().join("expected/blobs").join(format!("{hash}.bin"))).expect("blob")
}

/// Turns the tagged values into plain JSON: bytes become an array of numbers (typed arrays become
/// arrays of their elements), a set or map becomes an array, `undefined` becomes null, a bigint a string.
pub fn norm(v: &Value) -> Value {
    match v {
        Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
        Value::Object(o) => match o.get("$").and_then(|t| t.as_str()) {
            Some("bytes") => typed_array(o),
            Some("undefined") => Value::Null,
            Some("set") => Value::Array(o["v"].as_array().expect("set").iter().map(norm).collect()),
            Some("map") => Value::Array(o["v"].as_array().expect("map").iter().map(norm).collect()),
            Some("bigint") => o["v"].clone(),
            Some("num") => o["v"].clone(),
            Some("file") => json!({ "$file": o["path"] }),
            _ => Value::Object(o.iter().map(|(k, v)| (k.clone(), norm(v))).collect::<Map<_, _>>()),
        },
        other => other.clone(),
    }
}

fn typed_array(o: &Map<String, Value>) -> Value {
    let bytes = tagged_bytes(&Value::Object(o.clone()));
    let kind = o.get("type").and_then(|t| t.as_str()).unwrap_or("Uint8Array");
    let chunks = |n: usize| bytes.chunks_exact(n).map(|c| c.to_vec()).collect::<Vec<_>>();
    let arr: Vec<Value> = match kind {
        "Float32Array" => chunks(4).iter().map(|c| json!(f32::from_le_bytes(c[..].try_into().unwrap()) as f64)).collect(),
        "Float64Array" => chunks(8).iter().map(|c| json!(f64::from_le_bytes(c[..].try_into().unwrap()))).collect(),
        "Uint16Array" => chunks(2).iter().map(|c| json!(u16::from_le_bytes(c[..].try_into().unwrap()))).collect(),
        "Int16Array" => chunks(2).iter().map(|c| json!(i16::from_le_bytes(c[..].try_into().unwrap()))).collect(),
        "Uint32Array" => chunks(4).iter().map(|c| json!(u32::from_le_bytes(c[..].try_into().unwrap()))).collect(),
        "Int32Array" => chunks(4).iter().map(|c| json!(i32::from_le_bytes(c[..].try_into().unwrap()))).collect(),
        _ => bytes.iter().map(|b| json!(*b)).collect(),
    };
    Value::Array(arr)
}

/// The text of a string argument, or of a file of the synthetic project.
pub fn text_of(v: &Value) -> String {
    match v.get("$file") {
        Some(p) => String::from_utf8_lossy(&file_bytes(p.as_str().unwrap())).into_owned(),
        None => s(v).to_string(),
    }
}

/// The bytes of a file argument (`inputs/repo/...`).
pub fn file_bytes(path: &str) -> Vec<u8> {
    let rel = path.strip_prefix("inputs/repo/").unwrap_or(path);
    std::fs::read(golden_root().join("inputs/repo").join(rel)).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// A normalized byte-array argument (or a file of the project) as bytes.
pub fn as_bytes(v: &Value) -> Vec<u8> {
    if let Some(p) = v.get("$file") {
        return file_bytes(p.as_str().unwrap());
    }
    v.as_array().expect("byte array").iter().map(|b| b.as_u64().expect("byte") as u8).collect()
}

/// Compares two JSON values; numbers within 1e-6 relative tolerance, a missing key equal to null.
pub fn json_diff(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
            let tolerance = 1e-6 * a.abs().max(b.abs()).max(1.0);
            ((a - b).abs() > tolerance).then(|| format!("{path}: expected {a}, got {b}"))
        }
        (Value::Array(a), Value::Array(b)) => {
            if a.len() != b.len() {
                return Some(format!("{path}: expected {} items, got {}: {} vs {}", a.len(), b.len(), short(expected), short(actual)));
            }
            a.iter().zip(b).enumerate().find_map(|(i, (x, y))| json_diff(x, y, &format!("{path}[{i}]")))
        }
        (Value::Object(a), Value::Object(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).collect();
            keys.into_iter().find_map(|k| json_diff(a.get(k).unwrap_or(&Value::Null), b.get(k).unwrap_or(&Value::Null), &format!("{path}.{k}")))
        }
        (a, b) if a == b => None,
        // JSON has no Infinity or NaN: serde_json writes them as null.
        (Value::String(a), Value::Null) if matches!(a.as_str(), "Infinity" | "-Infinity" | "NaN") => None,
        (a, b) => Some(format!("{path}: expected {}, got {}", short(a), short(b))),
    }
}

fn short(v: &Value) -> String {
    let s = v.to_string();
    if s.len() > 300 {
        format!("{}...", &s[..300])
    } else {
        s
    }
}

/// Runs every case of `func`: `f` maps the case's arguments to the actual result. A failing case
/// panics with its name. Returns the number of cases.
pub fn run(dir: &str, func: &str, f: impl Fn(&[Value]) -> Value) -> usize {
    run_case(dir, func, |c| f(&c.args))
}

/// Same as [`run`], for a function that also needs the case itself (its name, its raw record).
pub fn run_case(dir: &str, func: &str, f: impl Fn(&Case) -> Value) -> usize {
    let all = cases(dir, func);
    let mut failures = Vec::new();
    for c in &all {
        let actual = f(c);
        let expected = c.result.clone().unwrap_or(Value::Null);
        if let Some(d) = json_diff(&expected, &actual, "result") {
            failures.push(format!("{func} {}: {d}", c.name));
        }
    }
    assert!(failures.is_empty(), "{} of {} golden cases differ:\n{}", failures.len(), all.len(), failures.join("\n"));
    println!("golden {dir} {func}: {} cases", all.len());
    all.len()
}

/// Same as [`run`], with each result serialized from a typed value.
pub fn run_typed<T: serde::Serialize>(dir: &str, func: &str, f: impl Fn(&[Value]) -> T) -> usize {
    run(dir, func, |a| serde_json::to_value(f(a)).expect("serialize"))
}

pub fn s(v: &Value) -> &str {
    v.as_str().unwrap_or_else(|| panic!("expected a string, got {v}"))
}

pub fn f(v: &Value) -> f64 {
    v.as_f64().unwrap_or_else(|| panic!("expected a number, got {v}"))
}

pub fn from<T: serde::de::DeserializeOwned>(v: &Value) -> T {
    serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("decode {v}: {e}"))
}
