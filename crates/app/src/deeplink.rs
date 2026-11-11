//! `assethub://` links. The full parse stays in `src/shared/deeplink.ts`; here only the scheme is
//! checked, so nothing else is kept as a link.

use tauri::{AppHandle, Manager, Runtime};
use tauri_specta::Event;
use url::Url;

use crate::events::DeepLink;
use crate::state::AppState;

pub const SCHEME: &str = "assethub";

/// True for a URL with the `assethub` scheme (any letter case, as macOS may hand it over).
pub fn is_link(raw: &str) -> bool {
    Url::parse(raw.trim()).is_ok_and(|url| url.scheme() == SCHEME)
}

/// The first link among command line arguments: Windows and Linux pass a clicked link that way.
pub fn link_in<I, S>(args: I) -> Option<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().map(|a| a.as_ref().to_owned()).find(|a| is_link(a))
}

/// Keeps `url` until the page takes it, and nudges a page that is open to take it at once.
/// Anything that is not an `assethub://` link is ignored.
pub fn deliver<R: Runtime>(app: &AppHandle<R>, url: &str) {
    if !is_link(url) {
        return;
    }
    crate::commands::trace(|| format!("deep link received: {url}"));
    app.state::<AppState>().set_pending_link(url.to_owned());
    // A nudge only: the link itself is fetched with `take_deep_link`.
    let _ = DeepLink.emit(app);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_scheme_in_any_letter_case() {
        assert!(is_link("assethub://open?repo=a/b&branch=main&path=Assets/x.fbx"));
        assert!(is_link("ASSETHUB://open?repo=a/b"));
        assert!(is_link("  assethub://open?x=1 "));
    }

    #[test]
    fn rejects_other_schemes_and_plain_text() {
        assert!(!is_link("https://example.com/assethub://"));
        assert!(!is_link("file:///etc/passwd"));
        assert!(!is_link("assethub"));
        assert!(!is_link(""));
        assert!(!is_link("javascript:alert(1)"));
    }

    #[test]
    fn finds_the_link_among_arguments() {
        let args = ["/Applications/Asset Hub.app/Contents/MacOS/asset-hub", "--flag", "assethub://open?repo=a/b&branch=m&path=p"];
        assert_eq!(link_in(args).as_deref(), Some("assethub://open?repo=a/b&branch=m&path=p"));
        assert_eq!(link_in(["a", "b"]), None);
    }
}
