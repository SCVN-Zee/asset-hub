#[allow(dead_code)]
#[path = "src/command_names.rs"]
mod command_names;

/// The repo that holds the 2.x releases is a single value, `releaseRepo` in `package.json`; the app
/// and the release scripts all read it from there.
fn release_repo() -> String {
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../package.json");
    println!("cargo:rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).expect("package.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("package.json is JSON");
    json["releaseRepo"].as_str().filter(|repo| repo.split('/').count() == 2).expect("package.json releaseRepo is owner/name").to_string()
}

fn main() {
    println!("cargo:rustc-env=ASSETHUB_RELEASE_REPO={}", release_repo());
    // Declaring the app's commands makes each one need its own permission in a capability;
    // without this, every command would be open to every window.
    let app = tauri_build::AppManifest::new().commands(command_names::COMMANDS);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(app)).expect("tauri build");
}
