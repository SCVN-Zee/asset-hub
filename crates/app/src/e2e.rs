//! Test automation hook, compiled only with the `e2e` cargo feature and active only when
//! `ASSETHUB_E2E_SOCKET` names a socket path. A local test script uses it to drive the page and
//! take snapshots without the window ever taking focus: no mouse, no keyboard, no accessibility.
//!
//! The socket speaks one JSON object per line, one reply per request:
//!
//! - `{"cmd":"eval","js":"<script>"}`: runs the script in the page; the value of its last
//!   expression, when it is a string, comes back as `{"ok":true,"value":"..."}`. The page's CSP
//!   forbids `eval`, so a script is code, not text to evaluate. Return `JSON.stringify(...)`.
//! - `{"cmd":"snapshot","path":"/abs/file.png"}`: writes the web view as a PNG (works while the
//!   window is behind others or minimised, because WebKit draws from its own layers).
//! - `{"cmd":"window","op":"state|minimize|unminimize|hide|show|ghost|unghost|move","x":0,"y":0}`:
//!   `show` orders the window to the back without activating the app; `ghost` puts it on screen at
//!   1% opacity and ignoring the mouse, which is what makes WebKit draw while it stays unseen.
//! - `{"cmd":"quit"}`: exits the app.
//!
//! It refuses to start unless `ASSETHUB_USER_DATA` is set too, so a test run never touches the
//! real data folder.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSImage, NSWindow};
use objc2_foundation::{NSDictionary, NSError, NSString};
use objc2_web_kit::WKWebView;
use serde_json::{json, Value};
use tauri::{Manager, Runtime, WebviewWindow};

pub const SOCKET_ENV: &str = "ASSETHUB_E2E_SOCKET";

/// How long one request may wait for the page or WebKit to answer.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(120);

/// Keeps the page's console errors and warnings, uncaught errors and rejections in
/// `window.__e2e.logs` (at most 500), for the `eval` command to read.
pub const INIT_SCRIPT: &str = r#"(function () {
  var box = (window.__e2e = { logs: [] });
  function push(kind, parts) {
    var text = Array.prototype.map.call(parts, function (x) {
      try { return typeof x === 'string' ? x : (x && x.stack) || JSON.stringify(x); } catch (e) { return String(x); }
    }).join(' ');
    box.logs.push({ kind: kind, at: Date.now(), text: text.slice(0, 2000) });
    if (box.logs.length > 500) box.logs.shift();
  }
  ['error', 'warn'].forEach(function (kind) {
    var original = console[kind];
    console[kind] = function () { push(kind, arguments); return original.apply(console, arguments); };
  });
  window.addEventListener('error', function (e) { push('uncaught', [e.message, e.filename + ':' + e.lineno]); });
  window.addEventListener('unhandledrejection', function (e) { push('rejection', [e.reason]); });
})();"#;

/// The socket path when the hook is switched on at run time, and the data folder is a scratch one.
pub fn socket_path() -> Option<PathBuf> {
    let socket = std::env::var_os(SOCKET_ENV).filter(|value| !value.is_empty())?;
    if std::env::var_os("ASSETHUB_USER_DATA").is_none_or(|value| value.is_empty()) {
        eprintln!("e2e: refusing to start without ASSETHUB_USER_DATA");
        return None;
    }
    Some(PathBuf::from(socket))
}

/// Starts the listener for the main window; does nothing when the hook is not switched on.
pub fn start<R: Runtime>(window: &WebviewWindow<R>) {
    let Some(path) = socket_path() else { return };
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => return eprintln!("e2e: cannot bind {}: {error}", path.display()),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let window = window.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let window = window.clone();
            std::thread::spawn(move || serve(stream, &window));
        }
    });
}

fn serve<R: Runtime>(stream: UnixStream, window: &WebviewWindow<R>) {
    let Ok(mut out) = stream.try_clone() else { return };
    for line in BufReader::new(stream).lines().map_while(Result::ok) {
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => handle(window, &request).unwrap_or_else(|error| json!({ "ok": false, "error": error })),
            Err(error) => json!({ "ok": false, "error": format!("bad request: {error}") }),
        };
        if writeln!(out, "{reply}").is_err() {
            break;
        }
    }
}

fn text<'a>(request: &'a Value, key: &str) -> Result<&'a str, String> {
    request.get(key).and_then(Value::as_str).ok_or_else(|| format!("`{key}` is missing"))
}

fn handle<R: Runtime>(window: &WebviewWindow<R>, request: &Value) -> Result<Value, String> {
    match text(request, "cmd")? {
        "eval" => evaluate(window, text(request, "js")?.to_owned()).map(|value| json!({ "ok": true, "value": value })),
        "snapshot" => {
            snapshot(window, PathBuf::from(text(request, "path")?))?;
            Ok(json!({ "ok": true }))
        }
        "window" => window_op(window, request),
        "quit" => {
            window.app_handle().exit(0);
            Ok(json!({ "ok": true }))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// Runs `job` on the main thread with the web view, and waits for the answer it sends.
fn on_web_view<R: Runtime, T: Send + 'static>(
    window: &WebviewWindow<R>,
    job: impl FnOnce(&WKWebView, mpsc::Sender<Result<T, String>>) + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = mpsc::channel();
    let failed = tx.clone();
    window
        .with_webview(move |view| {
            // SAFETY: `inner` is the window's live WKWebView, and this closure runs on the main thread.
            let web_view = unsafe { &*(view.inner() as *const WKWebView) };
            job(web_view, tx);
        })
        .map_err(|error| {
            let _ = failed.send(Err(String::new()));
            error.to_string()
        })?;
    rx.recv_timeout(ANSWER_TIMEOUT).map_err(|_| "timed out waiting for the web view".to_owned())?
}

fn evaluate<R: Runtime>(window: &WebviewWindow<R>, script: String) -> Result<Option<String>, String> {
    on_web_view(window, move |web_view, tx| {
        let handler = RcBlock::new(move |result: *mut AnyObject, error: *mut NSError| {
            // SAFETY: WebKit passes either null or live objects for the duration of the callback.
            let answer = match unsafe { (result.as_ref(), error.as_ref()) } {
                (_, Some(error)) => Err(error.localizedDescription().to_string()),
                (Some(value), None) => Ok(value.downcast_ref::<NSString>().map(|string| string.to_string())),
                (None, None) => Ok(None),
            };
            let _ = tx.send(answer);
        });
        // SAFETY: a string and a block of the signature WebKit documents.
        unsafe { web_view.evaluateJavaScript_completionHandler(&NSString::from_str(&script), Some(&handler)) };
    })
}

fn snapshot<R: Runtime>(window: &WebviewWindow<R>, path: PathBuf) -> Result<(), String> {
    let png = on_web_view(window, move |web_view, tx| {
        let handler = RcBlock::new(move |image: *mut NSImage, error: *mut NSError| {
            // SAFETY: WebKit passes either null or live objects for the duration of the callback.
            let answer = match unsafe { (image.as_ref(), error.as_ref()) } {
                (_, Some(error)) => Err(error.localizedDescription().to_string()),
                (Some(image), None) => png_of(image).ok_or_else(|| "the snapshot could not be encoded".to_owned()),
                (None, None) => Err("WebKit returned no image".to_owned()),
            };
            let _ = tx.send(answer);
        });
        // SAFETY: a nil configuration snapshots the visible bounds of the web view.
        unsafe { web_view.takeSnapshotWithConfiguration_completionHandler(None, &handler) };
    })?;
    std::fs::write(&path, png).map_err(|error| format!("{}: {error}", path.display()))
}

fn png_of(image: &NSImage) -> Option<Vec<u8>> {
    let tiff = image.TIFFRepresentation()?;
    let rep = NSBitmapImageRep::imageRepWithData(&tiff)?;
    // SAFETY: an empty properties dictionary is valid for PNG.
    let data = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
    Some(data.to_vec())
}

fn window_op<R: Runtime>(window: &WebviewWindow<R>, request: &Value) -> Result<Value, String> {
    let failed = |error: tauri::Error| error.to_string();
    match text(request, "op")? {
        "state" => {
            let position = window.outer_position().map_err(failed)?;
            let size = window.inner_size().map_err(failed)?;
            Ok(json!({
                "ok": true,
                "minimized": window.is_minimized().map_err(failed)?,
                "visible": window.is_visible().map_err(failed)?,
                "focused": window.is_focused().map_err(failed)?,
                "x": position.x, "y": position.y, "width": size.width, "height": size.height,
            }))
        }
        "minimize" => window.minimize().map_err(failed).map(|()| json!({ "ok": true })),
        "unminimize" => window.unminimize().map_err(failed).map(|()| json!({ "ok": true })),
        "hide" => window.hide().map_err(failed).map(|()| json!({ "ok": true })),
        "show" => order_back(window, None).map(|()| json!({ "ok": true })),
        // WebKit draws no frames for a window that is covered or not on screen (`visibilityState` is
        // `hidden`, no animation frames, blank snapshots). A window that is on screen but nearly
        // transparent and deaf to the mouse counts as visible and costs the user nothing.
        "ghost" => order_back(window, Some(0.01)).map(|()| json!({ "ok": true })),
        "unghost" => order_back(window, Some(1.0)).map(|()| json!({ "ok": true })),
        "move" => {
            let coordinate = |key| request.get(key).and_then(Value::as_i64).ok_or(format!("`{key}` is missing"));
            let position = tauri::PhysicalPosition::new(coordinate("x")? as i32, coordinate("y")? as i32);
            window.set_position(position).map_err(failed).map(|()| json!({ "ok": true }))
        }
        other => Err(format!("unknown window op `{other}`")),
    }
}

/// Puts the window on screen, without making it key or activating the app: behind every other
/// window, or in front of them when `alpha` makes it see-through (and deaf to the mouse).
pub fn order_back<R: Runtime>(window: &WebviewWindow<R>, alpha: Option<f64>) -> Result<(), String> {
    let ns_window = window.ns_window().map_err(|error| error.to_string())? as usize;
    window
        .run_on_main_thread(move || {
            // SAFETY: the pointer is the live NSWindow of this window, used on the main thread.
            let ns_window: Retained<NSWindow> = unsafe { Retained::retain(ns_window as *mut NSWindow) }.expect("a window");
            match alpha {
                Some(alpha) => {
                    ns_window.setAlphaValue(alpha);
                    ns_window.setIgnoresMouseEvents(alpha < 1.0);
                    ns_window.orderFrontRegardless();
                }
                None => ns_window.orderBack(None),
            }
        })
        .map_err(|error| error.to_string())
}
