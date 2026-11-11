//! Search over asset names that understands groups (fish finds shark), typos and folder names
//! (`src/core/search/search.ts`). The groups are in [`concepts`].

pub mod concepts;

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use unicode_normalization::UnicodeNormalization;

use crate::js::{is_js_space, len16, locale_compare, trim, OrderedMap, OrderedSet};
use crate::types::Asset;
use concepts::CONCEPTS;

/// Lower case without diacritics, so "cá mập" and "ca map" are the same.
pub fn normalize(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    let stripped: String = text.nfd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).map(|c| if c == 'đ' || c == 'Đ' { 'd' } else { c }).collect();
    stripped.to_lowercase()
}

/// Words of a file or folder name: `Fish_TunaA02.fbx` gives fish, tuna, a.
///
/// The TypeScript matches `/[A-Z]?[a-z]+|[A-Z]+(?![a-z])|[a-z]+/g` over the name without its
/// extension; letters outside ASCII separate words. This is that scan written out (the third
/// alternative can never match where the first does not), checked against the expression in a test.
pub fn name_words(name: &str) -> Vec<String> {
    let base = match name.rfind('.') {
        Some(dot) if dot + 1 < name.len() && !name[dot + 1..].contains('/') => &name[..dot],
        _ => name,
    };
    let bytes = base.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        let run_end = |from: usize, pred: fn(u8) -> bool| from + bytes[from..].iter().take_while(|c| pred(**c)).count();
        let (start, end) = if b.is_ascii_lowercase() {
            (i, run_end(i, |c| c.is_ascii_lowercase()))
        } else if b.is_ascii_uppercase() {
            if bytes.get(i + 1).is_some_and(u8::is_ascii_lowercase) {
                (i, run_end(i + 1, |c| c.is_ascii_lowercase()))
            } else {
                let upper_end = run_end(i, |c| c.is_ascii_uppercase());
                // `[A-Z]+(?![a-z])` gives back the last capital when a lowercase letter follows.
                let end = if upper_end - i > 1 && bytes.get(upper_end).is_some_and(u8::is_ascii_lowercase) { upper_end - 1 } else { upper_end };
                (i, end)
            }
        } else {
            i += 1;
            continue;
        };
        out.push(normalize(&base[start..end]));
        i = end;
    }
    out
}

/// boxes gives box, rocks gives rock; leaves cactus, glass, axis alone.
fn singular(w: &str) -> &str {
    let n = len16(w);
    if n > 4 && (w.ends_with("xes") || w.ends_with("ches") || w.ends_with("shes")) {
        return &w[..w.len() - 2];
    }
    if n > 3 && w.ends_with('s') && !(w.ends_with("ss") || w.ends_with("us") || w.ends_with("is")) {
        return &w[..w.len() - 1];
    }
    w
}

/// Edit distance over UTF-16 units, stopping early once it exceeds `max`.
pub fn edit_distance(a: &str, b: &str, max: usize) -> usize {
    let a: Vec<u16> = a.encode_utf16().collect();
    let b: Vec<u16> = b.encode_utf16().collect();
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut best = i;
        for j in 1..=b.len() {
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + usize::from(a[i - 1] != b[j - 1]));
            best = best.min(cur[j]);
        }
        if best > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

fn typo_budget(word: &str) -> usize {
    match len16(word) {
        7.. => 2,
        4..=6 => 1,
        _ => 0,
    }
}

struct ConceptEntry {
    /// The group name as written, with diacritics ("cây").
    label: &'static str,
    /// Every word the concept stands for, groups inside it expanded.
    words: Arc<OrderedSet>,
}

struct ConceptIndex {
    by_name: OrderedMap<ConceptEntry>,
    longest_alias: usize,
}

static CONCEPT_INDEX: LazyLock<ConceptIndex> = LazyLock::new(build_concepts);

fn build_concepts() -> ConceptIndex {
    let by_key: HashMap<&str, &concepts::Concept> = CONCEPTS.iter().map(|c| (c.key, c)).collect();
    fn expand(by_key: &HashMap<&str, &concepts::Concept>, key: &str, seen: &mut std::collections::HashSet<String>) -> OrderedSet {
        let mut out = OrderedSet::new();
        out.insert(key);
        if !seen.insert(key.to_string()) {
            return out;
        }
        if let Some(concept) = by_key.get(key) {
            for m in concept.members {
                for w in expand(by_key, m, seen).iter() {
                    out.insert(w);
                }
            }
            for a in concept.aliases {
                if !a.contains(' ') {
                    out.insert(normalize(a));
                }
            }
        }
        out
    }
    let mut by_name: OrderedMap<ConceptEntry> = OrderedMap::new();
    for concept in CONCEPTS {
        let words = expand(&by_key, concept.key, &mut Default::default());
        for label in std::iter::once(concept.key).chain(concept.aliases.iter().copied()) {
            let n = normalize(label);
            // A word that names two groups ("stone") means both; the first label stays.
            let entry = match by_name.get(&n) {
                Some(existing) => {
                    let mut union: OrderedSet = existing.words.iter().collect();
                    for w in words.iter() {
                        union.insert(w);
                    }
                    ConceptEntry { label: existing.label, words: Arc::new(union) }
                }
                None => ConceptEntry { label, words: Arc::new(words.clone()) },
            };
            by_name.insert(n, entry);
        }
    }
    let longest_alias = by_name.keys().map(|k| k.split(' ').count()).max().unwrap_or(1);
    ConceptIndex { by_name, longest_alias }
}

/// One thing the user asked for: a typed word, or a group name ("fish", "cá mập").
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QueryTerm {
    pub text: String,
    /// Words a matching name may contain instead of `text` (empty when it names no group).
    pub related: Arc<OrderedSet>,
}

fn is_query_separator(c: char) -> bool {
    is_js_space(c) || matches!(c, '_' | '-' | '.' | '/')
}

/// Splits a query into terms, joining words that together name a group ("ca map" gives shark).
pub fn parse_query(query: &str) -> Vec<QueryTerm> {
    let normalized = normalize(query);
    let words: Vec<&str> = normalized.split(is_query_separator).filter(|w| !w.is_empty()).collect();
    let index = &*CONCEPT_INDEX;
    let mut terms = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let mut taken = 0;
        let mut n = index.longest_alias.min(words.len() - i);
        while n >= 1 && taken == 0 {
            let phrase = words[i..i + n].join(" ");
            let concept = index.by_name.get(&phrase).or_else(|| index.by_name.get(singular(&phrase)));
            if let Some(concept) = concept {
                terms.push(QueryTerm { text: phrase, related: concept.words.clone() });
                taken = n;
            }
            n -= 1;
        }
        if taken == 0 {
            terms.push(QueryTerm { text: words[i].to_string(), related: Arc::default() });
            taken = 1;
        }
        i += taken;
    }
    terms
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    Name,
    Related,
    Folder,
    Typo,
}

impl MatchKind {
    fn score(self) -> i32 {
        match self {
            Self::Name => 100,
            Self::Related => 60,
            Self::Folder => 30,
            Self::Typo => 20,
        }
    }
}

struct Indexed {
    asset: Arc<Asset>,
    name: String,
    /// Other names the asset answers to (clips inside a model), normalised and joined.
    extra: String,
    words: Vec<String>,
    folders: Vec<String>,
}

/// What a search found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    /// Matching assets, best first.
    pub assets: Vec<Arc<Asset>>,
    /// Group members that produced matches, e.g. shark and tuna for "fish".
    pub related: Vec<String>,
    /// Name words matched despite a typo, e.g. "wolf" for "wlof".
    pub typos: Vec<String>,
    /// How well each asset (same order as `assets`) matched: the name itself scores highest, then
    /// group members, folders, typos. Empty for a query without terms, which lists everything.
    pub scores: Vec<i32>,
}

impl SearchResult {
    /// The score of the asset with this id, when the search scored it.
    pub fn score_of(&self, id: &str) -> Option<i32> {
        self.assets.iter().position(|a| a.id == id).and_then(|i| self.scores.get(i).copied())
    }
}

/// Serialized as the TypeScript result: `scores` is a map of asset id to score, as `[id, score]` pairs.
impl Serialize for SearchResult {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        struct Scores<'a>(&'a SearchResult);
        impl Serialize for Scores<'_> {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                s.collect_seq(self.0.assets.iter().zip(&self.0.scores).map(|(a, score)| (&a.id, score)))
            }
        }
        let mut st = s.serialize_struct("SearchResult", 4)?;
        st.serialize_field("assets", &self.assets)?;
        st.serialize_field("related", &self.related)?;
        st.serialize_field("typos", &self.typos)?;
        st.serialize_field("scores", &Scores(self))?;
        st.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    /// Text to put in the search box.
    pub text: String,
    /// How many assets it finds.
    pub count: usize,
    /// For a group: a few of the names it covers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WordCount {
    pub word: String,
    pub count: usize,
}

/// A query term with what the matching of every item needs, worked out once.
struct Prepared<'a> {
    term: &'a QueryTerm,
    /// Short group names ("ca", "xe") would match inside unrelated names (Cactus); for them only the
    /// group counts.
    literal: bool,
    squeezed: String,
    singular: &'a str,
    budget: usize,
}

const CACHE_LIMIT: usize = 200;

/// Search over asset names that understands groups (fish gives shark), typos and folder names.
pub struct AssetSearch {
    items: Vec<Indexed>,
    /// Every name word and how many assets use it, for suggestions.
    vocabulary: OrderedMap<usize>,
    /// A name word as it appears in file names (before singular), for display.
    spelling: HashMap<String, String>,
    /// Results by query: suggestions run many searches while the user types.
    cache: Mutex<HashMap<String, Arc<SearchResult>>>,
}

impl AssetSearch {
    /// `extra_names`: other names an asset answers to, e.g. the animation clips inside a model.
    pub fn new(assets: &[Asset], extra_names: Option<&dyn Fn(&Asset) -> Vec<String>>) -> Self {
        let mut vocabulary: OrderedMap<usize> = OrderedMap::new();
        let mut spelling: HashMap<String, String> = HashMap::new();
        let mut items = Vec::with_capacity(assets.len());
        for asset in assets {
            let extras = extra_names.map(|f| f(asset)).unwrap_or_default();
            let mut raw = name_words(&asset.name);
            raw.extend(extras.iter().flat_map(|e| name_words(e)));
            let words: Vec<String> = raw.iter().map(|w| singular(w).to_string()).collect();
            let mut folders: Vec<&str> = asset.path.split('/').collect();
            folders.pop();
            let folders: Vec<String> = folders.into_iter().flat_map(name_words).map(|w| singular(&w).to_string()).collect();
            for (w, r) in words.iter().zip(&raw) {
                spelling.entry(w.clone()).or_insert_with(|| r.clone());
            }
            let mut distinct: Vec<&String> = Vec::new();
            for w in &words {
                if !distinct.contains(&w) {
                    distinct.push(w);
                }
            }
            for w in distinct.into_iter().filter(|w| len16(w) > 1) {
                match vocabulary.get_mut(w) {
                    Some(n) => *n += 1,
                    None => vocabulary.insert(w.clone(), 1),
                }
            }
            let extra = extras.iter().map(|e| normalize(e)).collect::<Vec<_>>().join(" ");
            items.push(Indexed { asset: Arc::new(asset.clone()), name: normalize(&asset.name), extra, words, folders });
        }
        Self { items, vocabulary, spelling, cache: Mutex::new(HashMap::new()) }
    }

    fn match_term(item: &Indexed, p: &Prepared) -> Option<(MatchKind, Option<usize>)> {
        let term = p.term;
        if p.literal && [&item.name, &item.extra].iter().any(|n| n.contains(&term.text) || n.contains(&p.squeezed)) {
            return Some((MatchKind::Name, None));
        }
        if let Some(i) = item.words.iter().position(|w| term.related.contains(w)) {
            return Some((MatchKind::Related, Some(i)));
        }
        if item.folders.iter().any(|f| f == p.singular || term.related.contains(f)) {
            return Some((MatchKind::Folder, None));
        }
        if p.budget > 0 && term.related.is_empty() {
            if let Some(i) = item.words.iter().position(|w| len16(w) >= 3 && edit_distance(p.singular, w, p.budget) <= p.budget) {
                return Some((MatchKind::Typo, Some(i)));
            }
        }
        None
    }

    /// Searches the names; results are kept for the next time the same query comes.
    pub fn search(&self, query: &str) -> Arc<SearchResult> {
        let key = normalize(query);
        let key = trim(&key);
        if let Some(hit) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(key) {
            return hit.clone();
        }
        let result = Arc::new(self.run(key));
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() > CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(key.to_string(), result.clone());
        result
    }

    fn run(&self, query: &str) -> SearchResult {
        // "harvester | tractor | combine": any of the alternatives, each asset at its best match.
        let alternatives: Vec<&str> = query.split('|').map(trim).filter(|q| !q.is_empty()).collect();
        if alternatives.len() > 1 {
            return self.any(&alternatives);
        }
        let terms = parse_query(query);
        if terms.is_empty() {
            return SearchResult { assets: self.items.iter().map(|i| i.asset.clone()).collect(), related: Vec::new(), typos: Vec::new(), scores: Vec::new() };
        }
        let prepared: Vec<Prepared> = terms
            .iter()
            .map(|term| Prepared {
                term,
                literal: len16(&term.text) >= 3 || term.related.is_empty(),
                squeezed: term.text.replace(' ', ""),
                singular: singular(&term.text),
                budget: typo_budget(&term.text),
            })
            .collect();
        let mut related: OrderedMap<usize> = OrderedMap::new();
        let mut typos = OrderedSet::new();
        let mut scored: Vec<(&Indexed, i32)> = Vec::new();
        'items: for item in &self.items {
            let mut score = 0;
            let mut found = Vec::with_capacity(prepared.len());
            for p in &prepared {
                let Some(m) = Self::match_term(item, p) else { continue 'items };
                score += m.0.score();
                found.push(m);
            }
            for (kind, word) in found {
                let Some(word) = word.map(|i| item.words[i].as_str()) else { continue };
                match kind {
                    MatchKind::Related => match related.get_mut(word) {
                        Some(n) => *n += 1,
                        None => related.insert(word, 1),
                    },
                    MatchKind::Typo => {
                        typos.insert(word);
                    }
                    _ => {}
                }
            }
            scored.push((item, score));
        }
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_compare(&a.0.asset.name, &b.0.asset.name)));
        let mut related: Vec<(&str, usize)> = related.iter().map(|(w, n)| (w, *n)).collect();
        related.sort_by_key(|a| std::cmp::Reverse(a.1));
        SearchResult {
            assets: scored.iter().map(|(i, _)| i.asset.clone()).collect(),
            related: related.into_iter().map(|(w, _)| w.to_string()).collect(),
            typos: typos.iter().map(str::to_string).collect(),
            scores: scored.iter().map(|(_, s)| *s).collect(),
        }
    }

    fn any(&self, alternatives: &[&str]) -> SearchResult {
        let mut best: OrderedMap<(Arc<Asset>, i32)> = OrderedMap::new();
        let mut related = OrderedSet::new();
        let mut typos = OrderedSet::new();
        for alternative in alternatives {
            let r = self.run(alternative);
            for (i, asset) in r.assets.iter().enumerate() {
                let score = r.scores.get(i).copied().unwrap_or(0);
                if best.get(&asset.id).map_or(-1, |b| b.1) < score {
                    best.insert(asset.id.clone(), (asset.clone(), score));
                }
            }
            r.related.iter().for_each(|w| {
                related.insert(w);
            });
            r.typos.iter().for_each(|w| {
                typos.insert(w);
            });
        }
        let mut scored: Vec<&(Arc<Asset>, i32)> = best.values().collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_compare(&a.0.name, &b.0.name)));
        SearchResult {
            assets: scored.iter().map(|s| s.0.clone()).collect(),
            related: related.iter().map(str::to_string).collect(),
            typos: typos.iter().map(str::to_string).collect(),
            scores: scored.iter().map(|s| s.1).collect(),
        }
    }

    fn spelled<'a>(&'a self, word: &'a str) -> &'a str {
        self.spelling.get(word).map_or(word, String::as_str)
    }

    /// The words the names are made of, as spelled in them, most used first; numbers and one-letter
    /// parts left out. What the AI pet picks real search words from.
    pub fn words(&self) -> Vec<WordCount> {
        let mut all: Vec<(&str, usize)> = self.vocabulary.iter().filter(|(w, _)| !w.chars().any(|c| c.is_ascii_digit())).map(|(w, n)| (w, *n)).collect();
        all.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| locale_compare(a.0, b.0)));
        all.into_iter().map(|(w, count)| WordCount { word: self.spelled(w).to_string(), count }).collect()
    }

    /// Keywords for the last word being typed: groups first, then words used in names.
    pub fn suggest(&self, query: &str, limit: usize) -> Vec<Suggestion> {
        let head_end = query.char_indices().rev().find(|(_, c)| is_js_space(*c)).map_or(0, |(i, c)| i + c.len_utf8());
        let head = &query[..head_end];
        let last = normalize(&query[head_end..]);
        if last.is_empty() {
            return Vec::new();
        }
        let norm = normalize(query);
        let mut out: Vec<Suggestion> = Vec::new();
        let mut seen: std::collections::HashSet<String> = Default::default();
        let mut add = |out: &mut Vec<Suggestion>, text: &str, examples: Option<Vec<String>>| {
            let full = format!("{head}{text}");
            let normalized = normalize(&full);
            if seen.contains(&normalized) || normalized == norm {
                return;
            }
            let count = self.search(&full).assets.len();
            if count == 0 {
                return;
            }
            seen.insert(normalized);
            out.push(Suggestion { text: full, count, examples });
        };
        let mut groups: Vec<(&str, &ConceptEntry)> = CONCEPT_INDEX.by_name.iter().filter(|(name, _)| name.starts_with(&last)).collect();
        groups.sort_by_key(|(name, _)| len16(name));
        for (name, entry) in groups {
            let examples: Vec<String> = self.vocabulary.keys().filter(|w| entry.words.contains(w) && *w != name).take(4).map(|w| self.spelled(w).to_string()).collect();
            add(&mut out, entry.label, Some(examples));
        }
        let mut words: Vec<(&str, usize)> = self.vocabulary.iter().filter(|(w, _)| w.starts_with(&last)).map(|(w, n)| (w, *n)).collect();
        words.sort_by_key(|a| std::cmp::Reverse(a.1));
        for (w, _) in words {
            if out.len() >= limit {
                break;
            }
            add(&mut out, self.spelled(w), None);
        }
        // Nothing starts with it: offer close spellings.
        let budget = typo_budget(&last);
        if out.is_empty() && budget > 0 {
            let mut close: Vec<(&str, usize)> = self.vocabulary.iter().filter(|(w, _)| edit_distance(&last, w, budget) <= budget).map(|(w, n)| (w, *n)).collect();
            close.sort_by_key(|a| std::cmp::Reverse(a.1));
            for (w, _) in close.into_iter().take(limit) {
                add(&mut out, self.spelled(w), None);
            }
        }
        out.truncate(limit);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::re;

    /// The scan in `name_words` against the expression it replaces.
    #[test]
    fn name_words_scan_agrees_with_the_expression() {
        let ext = re!(r"\.[^./]+$");
        let parts = re!(r"[A-Z]?[a-z]+|[A-Z]+(?![a-z])|[a-z]+");
        let alphabet: Vec<char> = "aAbBcZz01._/ -Đé".chars().collect();
        let mut seed = 0x2545_f491_4f6c_dd1d_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..20_000 {
            let len = (next() % 12) as usize;
            let text: String = (0..len).map(|_| alphabet[(next() % alphabet.len() as u64) as usize]).collect();
            let base = ext.replace_first(&text, "");
            let expected: Vec<String> = parts.find_all(&base).iter().map(|m| normalize(m.all())).collect();
            assert_eq!(name_words(&text), expected, "{text:?}");
        }
    }

    #[test]
    fn singular_keeps_cactus_glass_and_axis() {
        assert_eq!(singular("boxes"), "box");
        assert_eq!(singular("rocks"), "rock");
        assert_eq!(singular("cactus"), "cactus");
        assert_eq!(singular("glass"), "glass");
        assert_eq!(singular("axis"), "axis");
        assert_eq!(singular("bus"), "bus");
    }

    #[test]
    fn normalize_drops_diacritics_and_folds_d_stroke() {
        assert_eq!(normalize("Đường phố CÁ MẬP"), "duong pho ca map");
    }

    #[test]
    fn a_group_that_two_entries_name_means_both() {
        let q = parse_query("stone");
        assert!(q[0].related.contains("rock") && q[0].related.contains("crystal"));
    }
}
