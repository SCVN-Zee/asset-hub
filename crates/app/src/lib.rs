//! Asset Hub: the Tauri shell and the typed contract between the page and the backend.

pub mod ai;
pub mod ai_tools;
pub mod auth;
pub mod blender;
pub mod bundle;
pub mod cache;
pub mod catalog_service;
pub mod codex_client;
pub mod codex_installer;
pub mod command_names;
pub mod commands;
pub mod constants;
pub mod deeplink;
pub mod dto;
#[cfg(all(feature = "e2e", target_os = "macos"))]
pub mod e2e;
pub mod events;
pub mod exporter;
pub mod gh_installer;
pub mod gh_prompt;
pub mod git_tree;
pub mod github_net;
#[cfg(test)]
mod golden_tests;
pub mod headless;
pub mod hub;
pub mod http;
pub mod http_cache;
pub mod index_repo;
pub mod ipc;
pub mod ogg;
pub mod paths;
pub mod settings;
pub mod shared_index;
pub mod shared_index_service;
pub mod state;
pub mod target_project;
#[cfg(test)]
mod test_server;
pub mod thumb_protocol;
pub mod unity_editors;
pub mod updater;
pub mod usage;
pub mod usage_env;
pub mod window;
pub mod zip;

use std::path::Path;

use tauri::{Manager, RunEvent};
use tauri_plugin_deep_link::DeepLinkExt;
use tauri_specta::{collect_commands, collect_events, Builder};

use crate::paths::AppPaths;
use crate::settings::{default_settings, fix_cache_dir, SettingsStore};
use crate::state::AppState;

/// Where the generated bindings live, relative to this crate.
pub const BINDINGS_FILE: &str = "../../ui/src/bindings.ts";

/// The events of the contract and the constants Rust owns, for any runtime.
pub fn event_contract<R: tauri::Runtime>() -> Builder<R> {
    use crate::{constants as c, events as e};

    Builder::<R>::new()
        .events(collect_events![
            e::DeepLink,
            e::GhLoginPrompt,
            e::ToolsInstallProgress,
            e::IndexProgress,
            e::PrefabProgress,
            e::KnownProgress,
            e::SharedIndexProgress,
            e::ExportProgress,
            e::CatalogProgress,
            e::UpdateProgress,
            e::AiStatusChanged,
            e::AiInstallProgress,
            e::AiChat,
            e::AiToolCallEvent,
        ])
        .constant("LANGUAGES", c::LANGUAGES)
        .constant("DEFAULT_EXCLUDED_FOLDERS", c::DEFAULT_EXCLUDED_FOLDERS)
        .constant("CACHE_LIMITS_GB", c::CACHE_LIMITS_GB)
        .constant("DEFAULT_CACHE_LIMIT_GB", c::DEFAULT_CACHE_LIMIT_GB)
        .constant("EXPORT_LAYOUTS", c::EXPORT_LAYOUTS)
        .constant("DEFAULT_EXPORT_LAYOUT", c::DEFAULT_EXPORT_LAYOUT)
        .constant("AI_TOOL_NAMES", c::AI_TOOL_NAMES)
        .constant("FEEDBACK_KINDS", c::FEEDBACK_KINDS)
        .constant("BUILTIN_SHADER_GUIDS", c::BUILTIN_SHADER_GUIDS)
        .constant("BUDGET_KEYS", c::BUDGET_KEYS)
        .constant("BUDGET_PRESETS", c::BUDGET_PRESETS)
        .constant("DEFAULT_BUDGET_PRESET", c::DEFAULT_BUDGET_PRESET)
        .constant("PLAYABLE_BUDGET", c::PLAYABLE_BUDGET)
        .constant("TIGHT_BUDGET", c::TIGHT_BUDGET)
}

/// The contract: every command, every event and the constants Rust owns.
pub fn contract() -> Builder<tauri::Wry> {
    use crate::commands::{agent_cli, ai, app, assets, audio, auth, caches, catalog, exporter, parse, services, source};

    event_contract::<tauri::Wry>()
        .commands(collect_commands![
            app::get_settings,
            app::set_settings,
            app::choose_folder,
            app::is_unity_project,
            app::take_deep_link,
            app::open_external,
            auth::auth_status,
            auth::auth_sign_in_with_token,
            auth::auth_sign_in_with_gh,
            auth::auth_gh_web_login,
            auth::auth_cancel_gh_web_login,
            auth::auth_sign_out,
            services::tools_install_gh,
            source::source_list_repos,
            source::source_list_branches,
            source::source_list_tree,
            source::source_watch_check,
            assets::assets_resolve,
            assets::assets_read_file,
            assets::assets_find_by_names,
            assets::assets_find_prefab_materials,
            assets::assets_save_file,
            assets::assets_model_bundle,
            assets::assets_blender_installed,
            assets::assets_known_assets,
            audio::audio_ogg_to_wav,
            parse::unity_mesh,
            parse::material,
            parse::shader_info,
            parse::embedded_texture,
            parse::decoded_image,
            parse::texture_info,
            parse::anim_clip,
            parse::prefab_scene,
            parse::fbx_info,
            parse::mesh_for_file_id,
            parse::audio_info,
            parse::audio_built_bytes,
            parse::font_info,
            parse::tmp_font,
            parse::tmp_font_atlas,
            parse::fbx_texture_names,
            parse::model_bundle_materials_for,
            parse::prefab_bundle_materials_for,
            parse::particle_totals,
            parse::handler_descriptors,
            parse::browse_assets,
            parse::search_build,
            parse::search_query,
            parse::search_words,
            parse::search_suggest,
            parse::search_folders,
            agent_cli::agent_cli_install,
            catalog::catalog_scan,
            catalog::catalog_pause,
            catalog::catalog_resume,
            catalog::catalog_status,
            catalog::catalog_list,
            catalog::catalog_where_is,
            caches::thumbs_get,
            caches::thumbs_save,
            caches::shared_index_status,
            caches::shared_index_import,
            caches::shared_index_export,
            caches::cache_usage,
            caches::cache_clear,
            caches::stats_get,
            caches::stats_save,
            exporter::exporter_unity_package,
            exporter::exporter_unity_editors,
            exporter::exporter_import_to_unity,
            exporter::exporter_show_in_folder,
            ai::ai_status,
            ai::ai_install,
            ai::ai_connect,
            ai::ai_cancel_connect,
            ai::ai_disconnect,
            ai::ai_chat_send,
            ai::ai_chat_stop,
            ai::ai_chat_reset,
            ai::ai_tool_result,
            ai::ai_feedback,
            services::update_check,
            services::update_install,
            services::usage_track,
            services::feedback_send,
        ])
}

/// Writes the TypeScript bindings of the contract to `path`.
pub fn export_bindings(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    contract().export(specta_typescript::Typescript::default(), path)?;
    Ok(())
}

/// The folder the app's files go to: `ASSETHUB_USER_DATA`, or the app data folder of the bundle
/// identifier.
fn app_paths(app: &tauri::AppHandle) -> Result<AppPaths, Box<dyn std::error::Error>> {
    let default_dir = app.path().app_data_dir()?;
    Ok(AppPaths::resolve(std::env::var("ASSETHUB_USER_DATA").ok().as_deref(), &default_dir))
}

pub fn run() {
    let bindings = Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS_FILE);
    // A development build keeps the bindings current; `ASSETHUB_EXPORT_ONLY` writes them and stops.
    let export_only = std::env::var_os("ASSETHUB_EXPORT_ONLY").is_some();
    if export_only || cfg!(all(debug_assertions, dev)) {
        if let Err(error) = export_bindings(&bindings) {
            eprintln!("could not write the bindings: {error}");
            if export_only {
                std::process::exit(1);
            }
        }
    }
    if export_only {
        return;
    }

    let builder = contract();
    let invoke = builder.invoke_handler();
    let upload: Box<dyn Fn(tauri::ipc::Invoke<tauri::Wry>) -> bool + Send + Sync> =
        Box::new(tauri::generate_handler![crate::commands::app::stash_bytes]);

    let tauri_builder = tauri::Builder::default();
    // The test hook must not take the focus when the app starts.
    #[cfg(all(feature = "e2e", target_os = "macos"))]
    let tauri_builder = if window::e2e_active() { tauri_builder.activate_ignoring_other_apps(false) } else { tauri_builder };
    let app = thumb_protocol::register(tauri_builder)
        // First: a second launch must reach the first instance before anything else starts.
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            window::focus(app);
            // Windows and Linux pass a clicked link as an argument of the second launch.
            if let Some(url) = deeplink::link_in(&argv) {
                deeplink::deliver(app, &url);
            }
        }))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(move |invoke_message| {
            if invoke_message.message.command() == command_names::STASH_COMMAND {
                upload(invoke_message)
            } else {
                invoke(invoke_message)
            }
        })
        .setup(move |app| {
            builder.mount_events(app);

            let paths = app_paths(app.handle())?;
            let mut store = SettingsStore::new(&paths.settings_file, default_settings(&paths.cache_dir));
            fix_cache_dir(&mut store, &paths.user_data, &paths.cache_dir)?;
            app.manage(AppState::new(paths, store));
            app.state::<AppState>().ai.set_sink(std::sync::Arc::new(commands::ai::PageSink::new(app.handle().clone())));
            app.state::<AppState>().hub.set_emitters(commands::caches::page_emitters(app.handle().clone()));
            // Leftovers and the size limit are dealt with once the app has settled.
            let caches = app.state::<AppState>().caches.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                caches.janitor.clean_stale().await;
                caches.janitor.prune().await;
            });
            let usage = app.state::<AppState>().usage.clone();
            crate::usage::install(usage.recorder().clone());
            usage.start(std::sync::Arc::new(usage_env::AppEnv::new(app.handle().clone())), usage_env::auto_send());

            // A link that launched the app: on macOS it arrives only through `on_open_url`, after
            // this setup; `get_current` is empty then, so it is no more than an extra source.
            let handle = app.handle().clone();
            app.deep_link().on_open_url(move |event| {
                for url in event.urls() {
                    deeplink::deliver(&handle, url.as_str());
                }
            });
            if let Ok(Some(urls)) = app.deep_link().get_current() {
                for url in urls {
                    deeplink::deliver(app.handle(), url.as_str());
                }
            }
            if let Some(url) = deeplink::link_in(std::env::args()) {
                deeplink::deliver(app.handle(), &url);
            }

            window::create(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("could not start Asset Hub");

    app.run(|handle, event| match event {
        // The Dock icon of an app without windows brings one back, as the app stays open.
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { has_visible_windows: false, .. } => {
            if handle.get_webview_window(window::MAIN_WINDOW).is_none() {
                let _ = window::create(handle);
            } else {
                window::focus(handle);
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::WindowEvent { event: tauri::WindowEvent::Destroyed, .. } => window::note_closed(),
        // Closing the last window does not quit a Mac app; Cmd+Q (no window closed just before) does.
        #[cfg(target_os = "macos")]
        RunEvent::ExitRequested { api, code: None, .. } if window::closed_just_now() => api.prevent_exit(),
        // The last session is saved, and sent, before the app goes; quitting waits a few seconds at most.
        RunEvent::ExitRequested { api, code, .. } => {
            if let Some(state) = handle.try_state::<AppState>() {
                if state.usage.begin_quit() {
                    api.prevent_exit();
                    let usage = state.usage.clone();
                    let handle = handle.clone();
                    tauri::async_runtime::spawn(async move {
                        usage.finish(&usage_env::AppEnv::new(handle.clone()), usage_env::auto_send()).await;
                        handle.exit(code.unwrap_or(0));
                    });
                }
            }
        }
        // No gh waiting for the browser, and no Codex, is left behind when the app quits.
        RunEvent::Exit => {
            if let Some(state) = handle.try_state::<AppState>() {
                tauri::async_runtime::block_on(async {
                    state.ai.shutdown().await;
                    state.auth.shutdown().await;
                    state.caches.stats.flush().await;
                });
            }
        }
        _ => {
            let _ = handle;
        }
    });
}
