//! The one-time code and device URL in the output of `gh auth login --web`.

use std::sync::OnceLock;

use regex::Regex;

use crate::events::GhLoginPrompt;

/// Finds the one-time code and device URL in `gh auth login --web` output.
///
/// gh has worded this differently across versions, e.g.
///   `! First copy your one-time code: 4915-D299`        (up to 2.100)
///   `! One-time code (2C04-3C10) copied to clipboard`   (2.103)
/// so the code is matched by its shape near "code" rather than by one exact sentence.
pub fn parse_gh_login_prompt(output: &str) -> Option<GhLoginPrompt> {
    static CODE: OnceLock<Regex> = OnceLock::new();
    static URL: OnceLock<Regex> = OnceLock::new();
    let code = CODE.get_or_init(|| Regex::new(r"(?i)code\W{0,4}([A-Z0-9]{4}-[A-Z0-9]{4})\b").expect("a valid pattern"));
    let url = URL.get_or_init(|| Regex::new(r"(https://github\.com/login/device\S*)").expect("a valid pattern"));
    let code = code.captures(output)?.get(1)?.as_str().to_uppercase();
    let url = url.captures(output)?.get(1)?.as_str().to_owned();
    Some(GhLoginPrompt { code, url })
}

#[cfg(test)]
mod tests {
    use super::*;

    const URL_LINE: &str = "Open this URL to continue in your web browser: https://github.com/login/device";

    #[test]
    fn reads_the_older_gh_wording() {
        let output = format!("! First copy your one-time code: 4915-D299\n{URL_LINE}\n");
        assert_eq!(parse_gh_login_prompt(&output).map(|p| (p.code, p.url)), Some(("4915-D299".into(), "https://github.com/login/device".into())));
    }

    #[test]
    fn reads_the_gh_2_103_wording() {
        let output = format!("\n! One-time code (2C04-3C10) copied to clipboard\n{URL_LINE}\n");
        assert_eq!(parse_gh_login_prompt(&output).map(|p| p.code), Some("2C04-3C10".to_owned()));
    }

    #[test]
    fn waits_until_both_the_code_and_the_url_have_arrived() {
        assert!(parse_gh_login_prompt("! One-time code (2C04-3C10) copied to clipboard\n").is_none());
        assert!(parse_gh_login_prompt(URL_LINE).is_none());
    }

    #[test]
    fn upper_cases_a_code_gh_printed_in_lower_case() {
        let output = format!("one-time code: ab12-cd34\n{URL_LINE}");
        assert_eq!(parse_gh_login_prompt(&output).map(|p| p.code), Some("AB12-CD34".to_owned()));
    }
}
