//! Small helpers that reproduce JavaScript string, regular-expression and number behaviour, so the
//! ported code gives the same answers as the TypeScript it replaces.
//!
//! The `regress` crate implements ECMAScript regular expressions (lookahead, back-references, `\w`
//! and `\s` with their JS meaning, `.` that stops at every line terminator, multiline `^` and `$`),
//! which the Rust `regex` crate does not. All offsets are byte offsets into the UTF-8 text.

use regress::{Match, Regex};

/// A compiled ECMAScript regular expression.
pub struct Re(Regex);

/// One match with its capture groups.
pub struct Caps<'t> {
    text: &'t str,
    m: Match,
}

impl<'t> Caps<'t> {
    /// Group `i` (0 is the whole match); `None` when the group did not take part in the match.
    pub fn get(&self, i: usize) -> Option<&'t str> {
        if i == 0 {
            return Some(&self.text[self.m.range()]);
        }
        self.m.group(i).map(|r| &self.text[r])
    }

    /// The whole match.
    pub fn all(&self) -> &'t str {
        &self.text[self.m.range()]
    }

    pub fn start(&self) -> usize {
        self.m.start()
    }

    pub fn end(&self) -> usize {
        self.m.end()
    }
}

impl Re {
    /// Compiles a pattern that is a constant of the program; an invalid pattern is a programming error.
    pub fn new(pattern: &str) -> Re {
        Re::with_flags(pattern, "")
    }

    pub fn with_flags(pattern: &str, flags: &str) -> Re {
        match Regex::with_flags(pattern, flags) {
            Ok(r) => Re(r),
            Err(e) => panic!("invalid regular expression /{pattern}/{flags}: {e}"),
        }
    }

    /// `re.exec(text)` for a non-global expression.
    pub fn exec<'t>(&self, text: &'t str) -> Option<Caps<'t>> {
        self.0.find(text).map(|m| Caps { text, m })
    }

    /// `re.test(text)`.
    pub fn test(&self, text: &str) -> bool {
        self.0.find(text).is_some()
    }

    /// `text.search(re)`: the byte offset of the first match.
    pub fn search(&self, text: &str) -> Option<usize> {
        self.0.find(text).map(|m| m.start())
    }

    /// `text.matchAll(re)` (the expression is used as if it were global).
    pub fn find_all<'t>(&self, text: &'t str) -> Vec<Caps<'t>> {
        self.0.find_iter(text).map(|m| Caps { text, m }).collect()
    }

    /// `text.split(re)` for an expression without capture groups.
    pub fn split<'t>(&self, text: &'t str) -> Vec<&'t str> {
        let mut parts = Vec::new();
        let mut p = 0;
        for m in self.0.find_iter(text) {
            if m.start() >= text.len() {
                break;
            }
            // An empty match right at the start of the current piece does not split.
            if m.end() == p {
                continue;
            }
            parts.push(&text[p..m.start()]);
            p = m.end();
        }
        parts.push(&text[p..]);
        parts
    }

    /// `text.replace(re, replacement)` with a literal replacement, for a non-global expression.
    pub fn replace_first(&self, text: &str, replacement: &str) -> String {
        match self.0.find(text) {
            Some(m) => format!("{}{replacement}{}", &text[..m.start()], &text[m.end()..]),
            None => text.to_string(),
        }
    }

    /// `text.replace(re, replacement)` with a literal replacement, for a global expression.
    pub fn replace_all(&self, text: &str, replacement: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut last = 0;
        for m in self.0.find_iter(text) {
            out.push_str(&text[last..m.start()]);
            out.push_str(replacement);
            last = m.end();
        }
        out.push_str(&text[last..]);
        out
    }
}

/// Declares a regular expression compiled once.
#[macro_export]
macro_rules! re {
    ($pattern:expr) => {{
        static RE: ::std::sync::LazyLock<$crate::js::Re> = ::std::sync::LazyLock::new(|| $crate::js::Re::new($pattern));
        &*RE
    }};
    ($pattern:expr, $flags:expr) => {{
        static RE: ::std::sync::LazyLock<$crate::js::Re> = ::std::sync::LazyLock::new(|| $crate::js::Re::with_flags($pattern, $flags));
        &*RE
    }};
}

/// `\s` in JavaScript, which is also what `String.prototype.trim` removes.
pub fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{a}' | '\u{b}' | '\u{c}' | '\u{d}' | ' ' | '\u{a0}' | '\u{1680}' | '\u{2000}'..='\u{200a}' | '\u{2028}' | '\u{2029}' | '\u{202f}' | '\u{205f}' | '\u{3000}' | '\u{feff}'
    )
}

/// `String.prototype.trim`.
pub fn trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// `Number(text)` for a string: trims, an empty string is 0, `NaN` when it is not a number.
pub fn to_number(text: &str) -> f64 {
    let t = trim(text);
    if t.is_empty() {
        return 0.0;
    }
    match t {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = t.strip_prefix(prefix) {
            return match u64::from_str_radix(digits, radix) {
                Ok(v) if !digits.is_empty() => v as f64,
                _ => f64::NAN,
            };
        }
    }
    static DECIMAL: std::sync::LazyLock<Re> = std::sync::LazyLock::new(|| Re::new(r"^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?$"));
    if !DECIMAL.test(t) {
        return f64::NAN;
    }
    t.parse::<f64>().unwrap_or(f64::NAN)
}

/// `Number.isFinite(Number(text))` as a parsed value.
pub fn finite_number(text: &str) -> Option<f64> {
    let n = to_number(text);
    n.is_finite().then_some(n)
}

/// `text.slice(start)` from a byte offset, clamped.
pub fn slice_from(text: &str, start: usize) -> &str {
    &text[start.min(text.len())..]
}

/// The next `units` UTF-16 code units of `text` from a byte offset (`text.slice(i, i + units)`).
pub fn slice_units(text: &str, start: usize, units: usize) -> &str {
    let rest = slice_from(text, start);
    let mut used = 0;
    for (i, c) in rest.char_indices() {
        if used >= units {
            return &rest[..i];
        }
        used += c.len_utf16();
    }
    rest
}

/// Decodes bytes as latin1, so that every byte is kept as one character.
pub fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

/// A number the way JSON carries it: an integral value is written without a fraction.
pub fn json_number(v: f64) -> serde_json::Value {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 9.007_199_254_740_992e15 {
        serde_json::Value::from(v as i64)
    } else {
        serde_json::Number::from_f64(v).map(serde_json::Value::Number).unwrap_or(serde_json::Value::Null)
    }
}

/// `a.localeCompare(b)` for the ASCII-heavy text the app compares (paths, names): letters ignore case first
/// (lowercase sorts before uppercase on a tie), then punctuation sorts before digits, digits before letters.
pub fn locale_compare(a: &str, b: &str) -> std::cmp::Ordering {
    fn class(c: char) -> u8 {
        if c.is_alphabetic() {
            2
        } else if c.is_numeric() {
            1
        } else {
            0
        }
    }
    // Compares (class, lowercase form) per character without building the lowercase strings.
    let mut left = a.chars();
    let mut right = b.chars();
    loop {
        match (left.next(), right.next()) {
            (None, None) => break,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                let order = class(x).cmp(&class(y)).then_with(|| x.to_lowercase().cmp(y.to_lowercase()));
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
        }
    }
    // Tertiary: lowercase before uppercase at the first difference.
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            return if x.is_lowercase() { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater };
        }
    }
    std::cmp::Ordering::Equal
}

/// `x.toFixed(digits)` for a finite, non-negative `x`: ties round up, as in JavaScript (Rust's own
/// formatting rounds ties to even).
pub fn to_fixed(x: f64, digits: usize) -> String {
    let wide = format!("{:.*}", digits + 30, x);
    let (int, frac) = wide.split_once('.').unwrap_or((&wide, ""));
    let keep = &frac[..digits];
    let round_up = frac.as_bytes()[digits] >= b'5';
    let mut number: Vec<u8> = int.bytes().chain(keep.bytes()).collect();
    if round_up {
        let mut i = number.len();
        loop {
            if i == 0 {
                number.insert(0, b'1');
                break;
            }
            i -= 1;
            if number[i] == b'9' {
                number[i] = b'0';
            } else {
                number[i] += 1;
                break;
            }
        }
    }
    let split = number.len() - digits;
    let (head, tail) = number.split_at(split);
    let head = String::from_utf8_lossy(head);
    if digits == 0 {
        head.into_owned()
    } else {
        format!("{head}.{}", String::from_utf8_lossy(tail))
    }
}

/// A compiled expression for a pattern built at run time, kept so the same key compiles once.
pub fn cached(pattern: &str, flags: &str) -> std::sync::Arc<Re> {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<(String, String), Arc<Re>>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    cache.entry((pattern.to_string(), flags.to_string())).or_insert_with(|| Arc::new(Re::with_flags(pattern, flags))).clone()
}

/// The first capture group of the first match of a run-time pattern: `new RegExp(pattern, flags).exec(text)?.[1]`.
pub fn group1<'t>(pattern: &str, flags: &str, text: &'t str) -> Option<&'t str> {
    cached(pattern, flags).exec(text).and_then(|c| c.get(1))
}

/// `text.length`: the number of UTF-16 code units.
pub fn len16(text: &str) -> usize {
    text.chars().map(char::len_utf16).sum()
}

/// A read outside a `DataView`: JavaScript throws a `RangeError`, so the readers return this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Offset is outside the bounds of the DataView")]
pub struct RangeError;

/// A `DataView` over bytes: bounds-checked reads in either byte order.
#[derive(Clone, Copy)]
pub struct View<'a> {
    bytes: &'a [u8],
    little: bool,
}

impl<'a> View<'a> {
    /// Big-endian, the `DataView` default.
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, little: false }
    }

    pub fn with_endian(bytes: &'a [u8], little: bool) -> Self {
        Self { bytes, little }
    }

    fn take<const N: usize>(&self, at: usize) -> Result<[u8; N], RangeError> {
        let end = at.checked_add(N).ok_or(RangeError)?;
        self.bytes.get(at..end).map(|s| <[u8; N]>::try_from(s).expect("slice length")).ok_or(RangeError)
    }

    pub fn u8(&self, at: usize) -> Result<u8, RangeError> {
        self.bytes.get(at).copied().ok_or(RangeError)
    }

    pub fn u16(&self, at: usize) -> Result<u16, RangeError> {
        let b = self.take::<2>(at)?;
        Ok(if self.little { u16::from_le_bytes(b) } else { u16::from_be_bytes(b) })
    }

    pub fn u32(&self, at: usize) -> Result<u32, RangeError> {
        let b = self.take::<4>(at)?;
        Ok(if self.little { u32::from_le_bytes(b) } else { u32::from_be_bytes(b) })
    }
}

/// `new TextDecoder().decode(bytes)`: UTF-8 with U+FFFD for invalid sequences, and a leading byte order
/// mark removed.
pub fn decode_utf8(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.strip_prefix('\u{feff}').map_or_else(|| text.to_string(), str::to_string)
}

/// `String(x)` for a number: the shortest digits that read back as `x`, in JavaScript's layout (no
/// exponent between 1e-6 and 1e21, `-0` as `0`).
pub fn number_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".into();
    }
    if x == 0.0 {
        return "0".into();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-Infinity" } else { "Infinity" }.into();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // `{:e}` prints the shortest round-trip digits as `d.ddde<exp>`.
    let sci = format!("{:e}", x.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().unwrap_or(0) + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sign_e = if e < 0 { '-' } else { '+' };
        if k == 1 {
            format!("{digits}e{sign_e}{}", e.abs())
        } else {
            format!("{}.{}e{sign_e}{}", &digits[..1], &digits[1..], e.abs())
        }
    };
    format!("{sign}{body}")
}

struct JsonNumbers;

impl serde_json::ser::Formatter for JsonNumbers {
    fn write_f64<W: ?Sized + std::io::Write>(&mut self, writer: &mut W, value: f64) -> std::io::Result<()> {
        writer.write_all(number_string(value).as_bytes())
    }

    fn write_f32<W: ?Sized + std::io::Write>(&mut self, writer: &mut W, value: f32) -> std::io::Result<()> {
        writer.write_all(number_string(value as f64).as_bytes())
    }
}

/// `JSON.stringify(value)` for a serializable value: struct fields keep their declaration order, and
/// numbers are written the way JavaScript writes them (`1`, not `1.0`). A non-finite number is `null`.
pub fn json_stringify<T: serde::Serialize + ?Sized>(value: &T) -> String {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, JsonNumbers);
    // Serializing plain data into memory cannot fail; a map with non-string keys is the only error source.
    value.serialize(&mut ser).expect("serialize to JSON");
    String::from_utf8(out).expect("JSON is UTF-8")
}

/// A string-keyed map that remembers insertion order, like a JavaScript `Map`: setting a key again
/// replaces its value in place. Serialized as an array of `[key, value]` pairs.
#[derive(Debug, Clone)]
pub struct OrderedMap<V> {
    items: Vec<(String, V)>,
    index: std::collections::HashMap<String, usize>,
}

impl<V: PartialEq> PartialEq for OrderedMap<V> {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl<V: Eq> Eq for OrderedMap<V> {}

impl<V> Default for OrderedMap<V> {
    fn default() -> Self {
        Self { items: Vec::new(), index: std::collections::HashMap::new() }
    }
}

impl<V> OrderedMap<V> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `map.set(key, value)`.
    pub fn insert(&mut self, key: impl Into<String>, value: V) {
        let key = key.into();
        match self.index.get(&key) {
            Some(&i) => self.items[i].1 = value,
            None => {
                self.index.insert(key.clone(), self.items.len());
                self.items.push((key, value));
            }
        }
    }

    pub fn get(&self, key: &str) -> Option<&V> {
        self.index.get(key).map(|&i| &self.items[i].1)
    }

    /// `delete map[key]`: the later entries keep their order.
    pub fn remove(&mut self, key: &str) -> Option<V> {
        let at = self.index.remove(key)?;
        let (_, value) = self.items.remove(at);
        for i in self.index.values_mut() {
            if *i > at {
                *i -= 1;
            }
        }
        Some(value)
    }

    pub fn get_mut(&mut self, key: &str) -> Option<&mut V> {
        self.index.get(key).map(|&i| &mut self.items[i].1)
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.index.contains_key(key)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &V)> {
        self.items.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(|(k, _)| k.as_str())
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.items.iter().map(|(_, v)| v)
    }
}

impl<V, K: Into<String>> FromIterator<(K, V)> for OrderedMap<V> {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        let mut out = Self::new();
        for (k, v) in iter {
            out.insert(k, v);
        }
        out
    }
}

impl<V: serde::Serialize> serde::Serialize for OrderedMap<V> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(self.items.iter().map(|(k, v)| (k, v)))
    }
}

impl<'de, V: serde::Deserialize<'de>> serde::Deserialize<'de> for OrderedMap<V> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let pairs = Vec::<(String, V)>::deserialize(d)?;
        Ok(pairs.into_iter().collect())
    }
}

/// A set of strings that remembers insertion order, like a JavaScript `Set`. Serialized as an array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OrderedSet {
    items: Vec<String>,
    seen: std::collections::HashSet<String>,
}

impl OrderedSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// `set.add(value)`: true when it was not there.
    pub fn insert(&mut self, value: impl Into<String>) -> bool {
        let value = value.into();
        if self.seen.contains(&value) {
            return false;
        }
        self.seen.insert(value.clone());
        self.items.push(value);
        true
    }

    pub fn contains(&self, value: &str) -> bool {
        self.seen.contains(value)
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.items.iter().map(String::as_str)
    }
}

impl<S: Into<String>> FromIterator<S> for OrderedSet {
    fn from_iter<I: IntoIterator<Item = S>>(iter: I) -> Self {
        let mut out = Self::new();
        for s in iter {
            out.insert(s);
        }
        out
    }
}

impl serde::Serialize for OrderedSet {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(&self.items)
    }
}

/// Serde for an [`OrderedMap`] kept as a JSON object, key order preserved (`#[serde(with = "...")]`).
pub mod ordered_object {
    use super::OrderedMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<V: Serialize, S: Serializer>(map: &OrderedMap<V>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(map.iter())
    }

    pub fn deserialize<'de, V: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<OrderedMap<V>, D::Error> {
        struct Visit<V>(std::marker::PhantomData<V>);
        impl<'de, V: Deserialize<'de>> serde::de::Visitor<'de> for Visit<V> {
            type Value = OrderedMap<V>;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("an object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut access: A) -> Result<Self::Value, A::Error> {
                let mut out = OrderedMap::new();
                while let Some((k, v)) = access.next_entry::<String, V>()? {
                    out.insert(k, v);
                }
                Ok(out)
            }
        }
        d.deserialize_map(Visit(std::marker::PhantomData))
    }
}

/// The same for an optional map: absent when `None`.
pub mod ordered_object_opt {
    use super::OrderedMap;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<V: Serialize, S: Serializer>(map: &Option<OrderedMap<V>>, s: S) -> Result<S::Ok, S::Error> {
        match map {
            Some(m) => super::ordered_object::serialize(m, s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, V: Deserialize<'de>, D: Deserializer<'de>>(d: D) -> Result<Option<OrderedMap<V>>, D::Error> {
        struct Wrapper<V>(OrderedMap<V>);
        impl<'de, V: Deserialize<'de>> Deserialize<'de> for Wrapper<V> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                super::ordered_object::deserialize(d).map(Wrapper)
            }
        }
        Ok(Option::<Wrapper<V>>::deserialize(d)?.map(|w| w.0))
    }
}

/// `Date.parse(text)` in milliseconds for the ISO forms GitHub and the catalog use
/// (`2026-11-08T10:11:12Z`, with fraction, with offset, or a date alone). A time without an offset
/// is read as UTC (JavaScript would use the machine's zone). `None` where JavaScript gives `NaN`.
pub fn parse_date(text: &str) -> Option<i64> {
    let m = re!(r"^(\d{4})(?:-(\d{2})(?:-(\d{2}))?)?(?:T(\d{2}):(\d{2})(?::(\d{2})(?:\.(\d+))?)?(Z|[+-]\d{2}:\d{2})?)?$").exec(text)?;
    let num = |i: usize, default: i64| m.get(i).map_or(default, |s| s.parse::<i64>().unwrap_or(default));
    let (year, month, day) = (num(1, 0), num(2, 1), num(3, 1));
    let (hour, minute, second) = (num(4, 0), num(5, 0), num(6, 0));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        _ => return None,
    };
    if day < 1 || day > days_in_month || hour > 24 || minute > 59 || second > 59 || (hour == 24 && (minute, second) != (0, 0)) {
        return None;
    }
    let millis = m.get(7).map_or(0, |f| format!("{:0<3}", &f[..f.len().min(3)]).parse::<i64>().unwrap_or(0));
    let offset = match m.get(8) {
        Some(z) if z != "Z" => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let (h, mi) = (z[1..3].parse::<i64>().ok()?, z[4..6].parse::<i64>().ok()?);
            if h > 23 || mi > 59 {
                return None;
            }
            sign * (h * 60 + mi)
        }
        _ => 0,
    };
    // Days from 1970-01-01 (the civil-calendar algorithm).
    let (y, mo) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * mo + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hour) * 60 + minute - offset) * 60_000 + second * 1000 + millis)
}
