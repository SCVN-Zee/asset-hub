//! The main window and its guards (`src/main/index.ts`, `createWindow`).

use std::time::Duration;

use tauri::webview::{NewWindowResponse, PageLoadEvent};
use tauri::utils::config::BackgroundThrottlingPolicy;
use tauri::window::Color;
use tauri::{AppHandle, Manager, Runtime, Url, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::commands::app::open_https;

pub const MAIN_WINDOW: &str = "main";

/// Longest the window stays hidden when the page never reports that it has loaded.
const SHOW_AFTER: Duration = Duration::from_secs(5);

/// The page keeps drawing thumbnails while the window is hidden, covered, minimised or the screen
/// is locked. With WebKit's default the page stops within seconds: thumbnails fell from 63 a
/// minute to 3. Honoured on macOS 14 and later only; wry ignores it on macOS 13.
const BACKGROUND_THROTTLING: BackgroundThrottlingPolicy = BackgroundThrottlingPolicy::Disabled;

/// True when `target` is the page the window shows: the window only ever shows the app itself,
/// so a dropped file or a stray link must not replace it.
pub fn is_app_url(target: &Url, app: &Url) -> bool {
    target.scheme() == app.scheme()
        && target.host_str() == app.host_str()
        && target.port_or_known_default() == app.port_or_known_default()
        && page_path(target) == page_path(app)
}

/// The path of a page; a URL of a scheme without a path (`tauri://localhost`) is the root.
fn page_path(url: &Url) -> &str {
    if url.path().is_empty() { "/" } else { url.path() }
}

/// Creates the main window, hidden until the page has loaded.
pub fn create<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<WebviewWindow<R>> {
    let own_url: Url = match &app.config().build.dev_url {
        Some(dev) if cfg!(dev) => dev.clone(),
        _ => Url::parse("tauri://localhost").expect("a valid URL"),
    };
    let opener = app.clone();
    let e2e = e2e_active();
    let builder = WebviewWindowBuilder::new(app, MAIN_WINDOW, WebviewUrl::default())
        .title("Asset Hub")
        .inner_size(1440.0, 900.0)
        .min_inner_size(960.0, 600.0)
        .visible(false)
        .background_color(Color(0x0E, 0x13, 0x1B, 0xFF))
        .background_throttling(BACKGROUND_THROTTLING)
        .on_navigation(move |target| is_app_url(target, &own_url))
        // Never open new windows; send https links to the system browser.
        .on_new_window(move |url, _features| {
            let _ = open_https(&opener, url.as_str());
            NewWindowResponse::Deny
        })
        .on_page_load(move |window, payload| {
            if payload.event() == PageLoadEvent::Finished && !e2e {
                let _ = window.show();
            }
        });
    // The test hook: the window is on screen from the start, never key, and the page logs its errors.
    #[cfg(all(feature = "e2e", target_os = "macos"))]
    let builder = if e2e {
        builder.initialization_script(crate::e2e::INIT_SCRIPT).visible(true).focused(false)
    } else {
        builder
    };
    let window = builder.build()?;
    #[cfg(all(feature = "e2e", target_os = "macos"))]
    if e2e {
        crate::e2e::start(&window);
    }
    // A page that cannot load must not leave the app invisible.
    if !e2e {
        let late = window.clone();
        std::thread::spawn(move || {
            std::thread::sleep(SHOW_AFTER);
            let _ = late.show();
        });
    }
    Ok(window)
}

/// True in a build with the `e2e` feature when `ASSETHUB_E2E_SOCKET` switches the test hook on.
pub fn e2e_active() -> bool {
    #[cfg(all(feature = "e2e", target_os = "macos"))]
    return crate::e2e::socket_path().is_some();
    #[cfg(not(all(feature = "e2e", target_os = "macos")))]
    false
}

static LAST_CLOSED: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

/// Records that a window has just been destroyed.
pub fn note_closed() {
    if let Ok(mut at) = LAST_CLOSED.lock() {
        *at = Some(std::time::Instant::now());
    }
}

/// True when a window was destroyed an instant ago: the request to exit then comes from the last
/// window closing, not from the user quitting the app.
pub fn closed_just_now() -> bool {
    LAST_CLOSED.lock().ok().and_then(|at| *at).is_some_and(|at| at.elapsed() < Duration::from_secs(1))
}

/// Focuses the first window: a second launch shows the first (two would fight over the cache).
pub fn focus<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    #[test]
    fn the_window_is_built_with_background_throttling_disabled() {
        assert_eq!(BACKGROUND_THROTTLING, BackgroundThrottlingPolicy::Disabled);
        // The builder's attributes cannot be read back, so check that `create` passes the constant.
        let source = include_str!("window.rs");
        assert!(source.contains(concat!(".background_throttling(", "BACKGROUND_THROTTLING)")));
    }

    #[test]
    fn the_e2e_hook_is_not_a_default_feature() {
        let manifest = include_str!("../Cargo.toml");
        let features = manifest.split("\n[features]").nth(1).expect("a features table");
        let table = features.split("\n[").next().unwrap_or(features);
        assert!(!table.lines().any(|line| line.trim_start().starts_with("default")), "no default features");
        assert!(table.lines().any(|line| line.starts_with("e2e = ")), "the e2e feature exists");
        // Without the feature the hook is not compiled in, so nothing can switch it on.
        #[cfg(not(feature = "e2e"))]
        {
            std::env::set_var("ASSETHUB_E2E_SOCKET", "/nonexistent/hook.sock");
            assert!(!e2e_active());
        }
    }

    #[test]
    fn allows_only_the_page_of_the_app() {
        let app = url("http://localhost:5173/");
        assert!(is_app_url(&url("http://localhost:5173/"), &app));
        assert!(is_app_url(&url("http://localhost:5173/?x=1#frag"), &app));
        assert!(!is_app_url(&url("http://localhost:5173/other"), &app));
        assert!(!is_app_url(&url("http://localhost:5174/"), &app));
        assert!(!is_app_url(&url("https://example.com/"), &app));
        assert!(!is_app_url(&url("file:///etc/hosts"), &app));
    }

    #[test]
    fn allows_the_bundled_app_page() {
        let app = url("tauri://localhost");
        assert!(is_app_url(&url("tauri://localhost/"), &app));
        assert!(!is_app_url(&url("tauri://evil/"), &app));
        assert!(!is_app_url(&url("http://localhost/"), &app));
    }
}
