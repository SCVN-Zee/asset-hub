//! Every command of the app and the legacy channel it stands for.
//!
//! `build.rs` includes this file to declare the app's command permissions, and the command wrapper
//! counts calls under the channel names of `src/shared/ipc.ts` (`IPC`), so the usage notes keep
//! their keys. A test checks the table against `ipc.ts` and against the generated bindings.

macro_rules! command_table {
    ($(($command:literal, $channel:literal)),* $(,)?) => {
        /// Commands of the contract, with the `IPC` channel each one replaces.
        pub const CHANNELS: &[(&str, &str)] = &[$(($command, $channel)),*];
        /// Every command callable from the page: the contract, the commands 2.x added and the raw
        /// byte upload.
        pub const COMMANDS: &[&str] = &[$($command,)* "audio_ogg_to_wav", "unity_mesh", "material", "shader_info", "embedded_texture", "decoded_image", "texture_info", "anim_clip", "prefab_scene", "fbx_info", "mesh_for_file_id", "audio_info", "audio_built_bytes", "font_info", "tmp_font", "tmp_font_atlas", "fbx_texture_names", "model_bundle_materials_for", "prefab_bundle_materials_for", "particle_totals", "handler_descriptors", "browse_assets", "search_build", "search_query", "search_words", "search_suggest", "search_folders", "agent_cli_install", STASH_COMMAND];
    };
}

/// Commands that have no channel in `src/shared/ipc.ts` because 1.x had no such call. They are in
/// the bindings and the capability like the others; calls to them are not counted in the usage notes.
pub const APP_ONLY_COMMANDS: &[&str] = &["audio_ogg_to_wav", "unity_mesh", "material", "shader_info", "embedded_texture", "decoded_image", "texture_info", "anim_clip", "prefab_scene", "fbx_info", "mesh_for_file_id", "audio_info", "audio_built_bytes", "font_info", "tmp_font", "tmp_font_atlas", "fbx_texture_names", "model_bundle_materials_for", "prefab_bundle_materials_for", "particle_totals", "handler_descriptors", "browse_assets", "search_build", "search_query", "search_words", "search_suggest", "search_folders", "agent_cli_install"];

/// Uploads raw bytes (previews, a GLB) that a later command takes by id.
pub const STASH_COMMAND: &str = "stash_bytes";

command_table! {
    ("get_settings", "settings:get"),
    ("set_settings", "settings:set"),
    ("choose_folder", "dialog:choose-folder"),
    ("is_unity_project", "fs:is-unity-project"),
    ("take_deep_link", "app:take-deep-link"),
    ("open_external", "shell:open-external"),
    ("auth_status", "auth:status"),
    ("auth_sign_in_with_token", "auth:sign-in-token"),
    ("auth_sign_in_with_gh", "auth:sign-in-gh"),
    ("auth_gh_web_login", "auth:gh-web-login"),
    ("auth_cancel_gh_web_login", "auth:gh-web-login-cancel"),
    ("auth_sign_out", "auth:sign-out"),
    ("tools_install_gh", "tools:install-gh"),
    ("source_list_repos", "source:list-repos"),
    ("source_list_branches", "source:list-branches"),
    ("source_list_tree", "source:list-tree"),
    ("source_watch_check", "source:watch-check"),
    ("assets_resolve", "assets:resolve"),
    ("assets_read_file", "assets:read-file"),
    ("assets_find_by_names", "assets:find-by-names"),
    ("assets_find_prefab_materials", "assets:find-prefab-materials"),
    ("assets_save_file", "assets:save-file"),
    ("assets_model_bundle", "assets:model-bundle"),
    ("assets_blender_installed", "blender:find"),
    ("assets_known_assets", "assets:known-assets"),
    ("catalog_scan", "catalog:scan"),
    ("catalog_pause", "catalog:pause"),
    ("catalog_resume", "catalog:resume"),
    ("catalog_status", "catalog:status"),
    ("catalog_list", "catalog:list"),
    ("catalog_where_is", "catalog:where"),
    ("thumbs_get", "thumbs:get"),
    ("thumbs_save", "thumbs:save"),
    ("shared_index_status", "sharedIndex:status"),
    ("shared_index_import", "sharedIndex:import"),
    ("shared_index_export", "sharedIndex:export"),
    ("cache_usage", "cache:usage"),
    ("cache_clear", "cache:clear"),
    ("stats_get", "stats:get"),
    ("stats_save", "stats:save"),
    ("exporter_unity_package", "export:unitypackage"),
    ("exporter_unity_editors", "unity:editors"),
    ("exporter_import_to_unity", "export:to-unity"),
    ("exporter_show_in_folder", "shell:show-item"),
    ("ai_status", "ai:status"),
    ("ai_install", "ai:install"),
    ("ai_connect", "ai:connect"),
    ("ai_cancel_connect", "ai:connect-cancel"),
    ("ai_disconnect", "ai:disconnect"),
    ("ai_chat_send", "ai:chat-send"),
    ("ai_chat_stop", "ai:chat-stop"),
    ("ai_chat_reset", "ai:chat-reset"),
    ("ai_tool_result", "ai:tool-result"),
    ("ai_feedback", "ai:feedback"),
    ("update_check", "update:check"),
    ("update_install", "update:install"),
    ("usage_track", "usage:track"),
    ("feedback_send", "feedback:send"),
}
