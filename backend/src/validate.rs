//! The input patterns the TypeScript Worker checks with regular expressions. Hand-written, so the
//! Wasm stays small. Every class is ASCII, as the JavaScript originals are.

fn all_lower_hex(text: &str) -> bool {
    text.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Forty lower-case hex digits.
pub fn is_sha40(text: &str) -> bool {
    text.len() == 40 && all_lower_hex(text)
}

/// A UUID of 8-4-4-4-12 hex digits, in either case.
pub fn is_uuid(text: &str) -> bool {
    let groups: Vec<&str> = text.split('-').collect();
    groups.len() == 5
        && groups.iter().zip([8, 4, 4, 4, 12]).all(|(group, len)| group.len() == len && group.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// A generation id that this Worker writes: a UUID in lower case.
pub fn is_generation_id(text: &str) -> bool {
    is_uuid(text) && !text.bytes().any(|b| b.is_ascii_uppercase())
}

/// A `sha40` with an optional `.fx` suffix.
pub fn is_thumb_name(text: &str) -> bool {
    is_sha40(text.strip_suffix(".fx").unwrap_or(text))
}

/// `<1 to 16 digits>:<thumb name>`: the revision as text, and the thumbnail name.
pub fn parse_cursor(text: &str) -> Option<(&str, &str)> {
    let (rev, name) = text.split_once(':')?;
    let digits_ok = (1..=16).contains(&rev.len()) && rev.bytes().all(|b| b.is_ascii_digit());
    (digits_ok && is_thumb_name(name)).then_some((rev, name))
}

/// Word characters and dashes: the name part of `/cache/:name`.
pub fn is_cache_segment(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Word characters, dots and dashes: an owner or repo name in the path.
pub fn is_repo_segment(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    #[test]
    fn sha40_is_lower_hex_only() {
        assert!(is_sha40(SHA));
        assert!(!is_sha40(&SHA.to_uppercase()));
        assert!(!is_sha40(&SHA[1..]));
        assert!(!is_sha40(&format!("{SHA}0")));
        assert!(!is_sha40(&format!("{}g", &SHA[1..])));
        assert!(!is_sha40(&format!("{SHA}\n")));
    }

    #[test]
    fn a_generation_id_is_a_lower_case_uuid() {
        assert!(is_generation_id("00000000-0000-4000-8000-00000000000a"));
        assert!(!is_generation_id("00000000-0000-4000-8000-00000000000A"));
        for legacy in ["", "4_z0123456789abcdef_d20260101_m000000_c000_v0001_t0000", "g0", "zzzz-newer"] {
            assert!(!is_generation_id(legacy), "{legacy:?}");
        }
    }

    #[test]
    fn uuid_ignores_case() {
        assert!(is_uuid("00000000-0000-4000-8000-000000000001"));
        assert!(is_uuid("ABCDEF00-0000-4000-8000-00000000000A"));
        assert!(!is_uuid("not-a-uuid"));
        assert!(!is_uuid("00000000-0000-4000-8000-00000000000"));
        assert!(!is_uuid("00000000-0000-4000-8000-00000000000g"));
        assert!(!is_uuid("00000000000040008000000000000001"));
    }

    #[test]
    fn thumb_name_has_an_optional_fx_suffix() {
        assert!(is_thumb_name(SHA));
        assert!(is_thumb_name(&format!("{SHA}.fx")));
        assert!(!is_thumb_name(&format!("{SHA}.fy")));
        assert!(!is_thumb_name(&format!("{SHA}.fx.fx")));
        assert!(!is_thumb_name(".fx"));
    }

    #[test]
    fn cursor_is_rev_colon_name() {
        assert_eq!(parse_cursor(&format!("5:{SHA}")), Some(("5", SHA)));
        let fx = format!("{SHA}.fx");
        assert_eq!(parse_cursor(&format!("1234567890123456:{fx}")), Some(("1234567890123456", fx.as_str())));
        assert_eq!(parse_cursor(&format!("12345678901234567:{SHA}")), None);
        assert_eq!(parse_cursor(&format!(":{SHA}")), None);
        assert_eq!(parse_cursor(&format!("-1:{SHA}")), None);
        assert_eq!(parse_cursor("garbage"), None);
        assert_eq!(parse_cursor("5:abc"), None);
    }

    #[test]
    fn path_segments() {
        assert!(is_cache_segment("facts-prefabfx4"));
        assert!(!is_cache_segment("catalog.json"));
        assert!(!is_cache_segment(""));
        assert!(is_repo_segment("Hb.creative-bulldozer_2"));
        assert!(!is_repo_segment("a%2Fb"));
        assert!(!is_repo_segment(""));
    }
}
