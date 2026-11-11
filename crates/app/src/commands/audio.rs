//! Audio the web view cannot decode itself.

use tauri::State;

use crate::ipc::{Binary, IpcError};
use crate::state::AppState;

/// Decodes Ogg Vorbis bytes (sent with `stash_bytes`; `ogg` is their id) to a 16-bit PCM WAV, for
/// when `decodeAudioData` fails on an `.ogg`.
#[tauri::command]
#[specta::specta]
pub async fn audio_ogg_to_wav(state: State<'_, AppState>, ogg: String) -> Result<Binary, IpcError> {
    let bytes = state.stash.take(&ogg).ok_or_else(|| IpcError::unknown("the audio bytes are no longer available"))?;
    let wav = tauri::async_runtime::spawn_blocking(move || crate::ogg::ogg_to_wav(&bytes)).await.map_err(|e| IpcError::unknown(format!("the Ogg decoder stopped: {e}")))?;
    wav.map(Binary).map_err(IpcError::unknown)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::AppPaths;
    use crate::settings::{default_settings, SettingsStore};
    use tauri::Manager;

    fn app_in(dir: &std::path::Path) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        let paths = AppPaths::resolve(Some(dir.to_str().unwrap()), dir);
        let store = SettingsStore::new(&paths.settings_file, default_settings(&paths.cache_dir));
        app.manage(AppState::new(paths, store));
        app
    }

    #[test]
    fn decodes_the_stashed_bytes_once() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in(dir.path());
        let id = app.state::<AppState>().stash.put(include_bytes!("../testdata/tone-stereo.ogg").to_vec()).unwrap();
        let wav = tauri::async_runtime::block_on(audio_ogg_to_wav(app.state(), id.clone())).unwrap();
        assert_eq!(&wav.0[0..4], b"RIFF");
        let again = tauri::async_runtime::block_on(audio_ogg_to_wav(app.state(), id)).err().expect("an error");
        assert!(again.message.contains("no longer available"), "{}", again.message);
    }

    #[test]
    fn bytes_that_are_not_ogg_are_an_error_not_a_crash() {
        let dir = tempfile::tempdir().unwrap();
        let app = app_in(dir.path());
        let id = app.state::<AppState>().stash.put(b"OggS-not-really".to_vec()).unwrap();
        let error = tauri::async_runtime::block_on(audio_ogg_to_wav(app.state(), id)).err().expect("an error");
        assert!(error.message.contains("Ogg"), "{}", error.message);
    }
}
