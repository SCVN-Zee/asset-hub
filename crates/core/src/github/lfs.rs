//! Git LFS pointers and `.gitattributes` rules (`src/core/github/lfs.ts`).

use serde::Serialize;

use crate::js::{is_js_space, trim, Re};
use crate::re;

/// What an LFS pointer file says about the real object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LfsPointer {
    pub oid: String,
    pub size: u64,
}

/// Parses a Git LFS pointer file; `None` when the text is not a pointer.
pub fn parse_lfs_pointer(text: &str) -> Option<LfsPointer> {
    if !text.starts_with("version https://git-lfs.github.com/spec/") {
        return None;
    }
    let oid = re!(r"^oid sha256:([0-9a-f]{64})$", "m").exec(text)?.get(1)?.to_string();
    let size = re!(r"^size (\d+)$", "m").exec(text)?.get(1)?;
    // `Number(size)`: a size beyond 2^53 loses digits there; here it saturates at `u64::MAX`.
    Some(LfsPointer { oid, size: size.parse::<u64>().unwrap_or(u64::MAX) })
}

/// One line of a `.gitattributes` file that routes files through LFS, or away from it.
pub struct LfsRule {
    /// The expression as JavaScript's `RegExp.source` shows it (`/` escaped).
    pub source: String,
    re: Re,
    pub lfs: bool,
}

impl std::fmt::Debug for LfsRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LfsRule({}, {})", self.source, self.lfs)
    }
}

impl LfsRule {
    fn new(body: &str, anchored: bool, lfs: bool) -> Self {
        let pattern = if anchored { format!("^{body}$") } else { format!("(?:^|/){body}$") };
        Self { source: js_source(&pattern), re: Re::new(&pattern), lfs }
    }

    /// A rule from the source of its expression (flags are never used).
    pub fn from_source(source: &str, lfs: bool) -> Self {
        Self { source: source.to_string(), re: Re::new(source), lfs }
    }

    pub fn matches(&self, path: &str) -> bool {
        self.re.test(path)
    }
}

/// `RegExp.prototype.source` writes a `/` outside a class as `\/`.
fn js_source(pattern: &str) -> String {
    let mut out = String::with_capacity(pattern.len() + 4);
    let (mut in_class, mut escaped) = (false, false);
    for c in pattern.chars() {
        match c {
            '/' if !in_class && !escaped => out.push_str("\\/"),
            _ => out.push(c),
        }
        escaped = c == '\\' && !escaped;
        if !escaped {
            match c {
                '[' => in_class = true,
                ']' => in_class = false,
                _ => {}
            }
        }
    }
    out
}

/// Returns the rules of a `.gitattributes` file that route files through LFS, as expressions over
/// repo-relative paths. Handles the common forms (`*.fbx`, `*.[fF][bB][xX]`, `Assets/**/*.png`,
/// `/Art/*.psd`); later lines that unset `filter` are honoured by [`is_lfs_path`].
pub fn parse_lfs_attributes(text: &str) -> Vec<LfsRule> {
    let mut rules = Vec::new();
    for raw in text.split('\n') {
        let line = trim(raw.strip_suffix('\r').unwrap_or(raw));
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split(is_js_space).filter(|p| !p.is_empty());
        let Some(pattern) = parts.next() else { continue };
        if pattern.starts_with('!') {
            continue;
        }
        let attrs: Vec<&str> = parts.collect();
        let lfs = if attrs.contains(&"filter=lfs") {
            true
        } else if attrs.iter().any(|a| *a == "-filter" || *a == "!filter" || (a.starts_with("filter=") && *a != "filter=lfs")) {
            false
        } else {
            continue;
        };
        let (body, anchored) = glob_to_regexp(pattern);
        rules.push(LfsRule::new(&body, anchored, lfs));
    }
    rules
}

/// Last matching rule wins, as in git.
pub fn is_lfs_path(path: &str, rules: &[LfsRule]) -> bool {
    let mut result = false;
    for rule in rules {
        if rule.matches(path) {
            result = rule.lfs;
        }
    }
    result
}

/// The expression body of a glob, and whether it is anchored (a pattern without a slash matches the
/// file name at any depth).
fn glob_to_regexp(pattern: &str) -> (String, bool) {
    let anchored = pattern.contains('/');
    let body: Vec<char> = pattern.strip_prefix('/').unwrap_or(pattern).chars().collect();
    let mut re = String::new();
    let mut i = 0;
    while i < body.len() {
        let c = body[i];
        match c {
            '*' => {
                if body.get(i + 1) == Some(&'*') {
                    let slash = body.get(i + 2) == Some(&'/');
                    re.push_str(if slash { "(?:.*/)?" } else { ".*" });
                    i += if slash { 2 } else { 1 };
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push_str("[^/]"),
            '[' => match body[i + 1..].iter().position(|c| *c == ']').map(|p| p + i + 1) {
                None => re.push_str("\\["),
                Some(end) => {
                    let class: String = body[i..=end].iter().collect();
                    re.push_str(&match class.strip_prefix("[!") {
                        Some(rest) => format!("[^{rest}"),
                        None => class,
                    });
                    i = end;
                }
            },
            '.' | '+' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            _ => re.push(c),
        }
        i += 1;
    }
    (re, anchored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unterminated_class_is_a_literal_bracket() {
        let rules = parse_lfs_attributes("a[b filter=lfs");
        assert!(is_lfs_path("a[b", &rules));
    }

    #[test]
    fn a_later_unset_wins() {
        let rules = parse_lfs_attributes("*.png filter=lfs\nkeep.png -filter\n");
        assert!(is_lfs_path("x/a.png", &rules));
        assert!(!is_lfs_path("keep.png", &rules));
    }

    #[test]
    fn crlf_lines_and_negated_patterns_are_read() {
        let rules = parse_lfs_attributes("!*.skip filter=lfs\r\n*.fbx filter=lfs\r\n");
        assert_eq!(rules.len(), 1);
    }

    #[test]
    fn a_huge_size_saturates() {
        let p = parse_lfs_pointer(&format!("version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 99999999999999999999999\n", "a".repeat(64))).expect("pointer");
        assert_eq!(p.size, u64::MAX);
    }
}
