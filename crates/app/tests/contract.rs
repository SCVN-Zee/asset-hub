//! The contract between the page and the backend: the committed bindings, the command list, the
//! permissions of the main window stay in step.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use asset_hub_lib::command_names::{COMMANDS, STASH_COMMAND};
use asset_hub_lib::paths::AppPaths;
use asset_hub_lib::settings::{default_settings, SettingsStore};
use asset_hub_lib::state::AppState;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{mock_builder, INVOKE_KEY};
use tauri::webview::InvokeRequest;
use tauri::{Manager, WebviewWindowBuilder};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(relative: &str) -> String {
    let path = manifest_dir().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// `snake_case` command name as the generated bindings invoke it.
fn bindings_commands(bindings: &str) -> BTreeSet<String> {
    let marker = "TAURI_INVOKE(\"";
    bindings.match_indices(marker).map(|(at, _)| bindings[at + marker.len()..].split('"').next().unwrap().to_owned()).collect()
}

#[test]
fn the_committed_bindings_are_current() {
    let dir = tempfile::tempdir().unwrap();
    let fresh = dir.path().join("bindings.ts");
    asset_hub_lib::export_bindings(&fresh).expect("the bindings export");
    let committed = manifest_dir().join(asset_hub_lib::BINDINGS_FILE);
    let committed = fs::read_to_string(&committed).unwrap_or_else(|e| panic!("cannot read {}: {e}", committed.display()));
    assert!(
        fs::read_to_string(&fresh).unwrap() == committed,
        "ui/src/bindings.ts is out of date: run `bun run bindings` and commit it"
    );
}

#[test]
fn the_bindings_invoke_every_command_but_the_byte_upload() {
    let invoked = bindings_commands(&read(asset_hub_lib::BINDINGS_FILE));
    let expected: BTreeSet<String> = COMMANDS.iter().filter(|c| **c != STASH_COMMAND).map(|c| c.to_string()).collect();
    assert_eq!(invoked, expected);
}

#[test]
fn the_main_window_may_call_exactly_the_commands_of_the_app() {
    let capability: serde_json::Value = serde_json::from_str(&read("capabilities/main.json")).unwrap();
    let granted: BTreeSet<String> = capability["permissions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|p| p.as_str())
        .filter_map(|p| p.strip_prefix("allow-"))
        .map(|p| p.replace('-', "_"))
        .collect();
    let commands: BTreeSet<String> = COMMANDS.iter().map(|c| c.to_string()).collect();
    assert_eq!(granted, commands);
    let others: Vec<&str> = capability["permissions"].as_array().unwrap().iter().filter_map(|p| p.as_str()).filter(|p| !p.starts_with("allow-")).collect();
    assert_eq!(others, ["core:event:default"], "the page gets events and the app's commands, nothing else");
}

fn state_in(dir: &Path) -> AppState {
    let paths = AppPaths::resolve(Some(dir.to_str().unwrap()), dir);
    let store = SettingsStore::new(&paths.settings_file, default_settings(&paths.cache_dir));
    AppState::new(paths, store)
}

#[tauri::command]
fn probe_outside_the_capability() -> u8 {
    1
}

fn request(command: &str) -> InvokeRequest {
    InvokeRequest {
        cmd: command.into(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: "tauri://localhost".parse().unwrap(),
        body: InvokeBody::Json(serde_json::json!({})),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

#[test]
fn the_capability_lets_the_page_call_its_commands_and_nothing_else() {
    let dir = tempfile::tempdir().unwrap();
    // The access check does not depend on which handler answers, so the mock runtime runs
    // two real commands and a probe that no capability grants.
    let app = mock_builder()
        .invoke_handler(tauri::generate_handler![
            asset_hub_lib::commands::app::get_settings,
            asset_hub_lib::commands::app::take_deep_link,
            probe_outside_the_capability
        ])
        .build(tauri::generate_context!())
        .expect("the mock app");
    asset_hub_lib::event_contract::<tauri::test::MockRuntime>().mount_events(&app);
    app.manage(state_in(dir.path()));
    let window = WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    let allowed = tauri::test::get_ipc_response(&window, request("get_settings")).expect("a granted command runs");
    let settings: serde_json::Value = allowed.deserialize().unwrap();
    assert_eq!(settings["language"], "en");

    let denied = tauri::test::get_ipc_response(&window, request("probe_outside_the_capability"));
    let denied = denied.expect_err("a command outside the capability is rejected");
    assert!(denied.to_string().to_lowercase().contains("not allowed"), "{denied}");

    let link = "assethub://open?repo=a/b&branch=main&path=Assets/x.fbx";
    asset_hub_lib::deeplink::deliver(window.app_handle(), "https://example.com/");
    let none: Option<String> = tauri::test::get_ipc_response(&window, request("take_deep_link")).unwrap().deserialize().unwrap();
    assert_eq!(none, None, "only an assethub link is kept");
    asset_hub_lib::deeplink::deliver(window.app_handle(), link);
    let taken: Option<String> = tauri::test::get_ipc_response(&window, request("take_deep_link")).unwrap().deserialize().unwrap();
    assert_eq!(taken.as_deref(), Some(link));
    let again: Option<String> = tauri::test::get_ipc_response(&window, request("take_deep_link")).unwrap().deserialize().unwrap();
    assert_eq!(again, None, "a link is taken once");
}
