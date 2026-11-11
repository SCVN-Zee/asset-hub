//! Blender on this Mac (`src/main/blender.ts`).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};

use tokio::process::Command;

const SPOTLIGHT_TIMEOUT: Duration = Duration::from_secs(5);

fn program_in(app: &Path) -> PathBuf {
    app.join("Contents/MacOS/Blender")
}

/// Blender's program on this Mac: /Applications, ~/Applications, else wherever Spotlight knows it.
pub async fn find_blender() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut homes = vec![PathBuf::from("/Applications/Blender.app")];
    if let Some(home) = std::env::var_os("HOME") {
        homes.push(PathBuf::from(home).join("Applications/Blender.app"));
    }
    if let Some(found) = homes.iter().map(|a| program_in(a)).find(|p| p.exists()) {
        return Some(found);
    }
    let mut spotlight = Command::new("mdfind");
    spotlight.arg("kMDItemCFBundleIdentifier == 'org.blenderfoundation.blender'").stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(SPOTLIGHT_TIMEOUT, spotlight.output()).await.ok()?.ok()?;
    String::from_utf8_lossy(&output.stdout).lines().filter(|l| !l.is_empty()).map(|l| program_in(Path::new(l))).find(|p| p.exists())
}

/// Starts a new Blender that runs `script` (which imports the model), left running on its own.
pub fn open_in_blender(blender: &Path, script: &Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new(blender);
    command.arg("--python").arg(script).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut child = command.spawn()?;
    // Reaped when it ends, so a closed Blender leaves no zombie behind.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Folders a Blender run left behind (it crashed, or was closed before the script finished): gone
/// after `max_age`.
pub fn sweep_blender_temp(dir: &Path, max_age: Duration) {
    let Ok(names) = std::fs::read_dir(dir) else { return };
    let now = SystemTime::now();
    for entry in names.filter_map(Result::ok) {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|m| now.duration_since(m).ok()).is_some_and(|age| age > max_age);
        if old {
            let path = entry.path();
            let _ = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
        }
    }
}

/// How long a Blender run's folder is kept.
pub const TEMP_MAX_AGE: Duration = Duration::from_secs(24 * 3600);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_only_folders_older_than_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["old", "fresh"] {
            std::fs::create_dir_all(dir.path().join(name)).unwrap();
            std::fs::write(dir.path().join(name).join("model.glb"), "x").unwrap();
        }
        let two_days_ago = SystemTime::now() - 2 * TEMP_MAX_AGE;
        std::fs::File::open(dir.path().join("old")).unwrap().set_modified(two_days_ago).unwrap();
        sweep_blender_temp(dir.path(), TEMP_MAX_AGE);
        let mut left: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["fresh"]);
        sweep_blender_temp(&dir.path().join("missing"), Duration::ZERO);
    }

    #[test]
    fn starts_a_program_without_waiting_for_it() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran.txt");
        let program = dir.path().join("blender");
        std::fs::write(&program, format!("#!/bin/sh\necho \"$1 $2\" > {}\n", marker.display())).unwrap();
        std::fs::set_permissions(&program, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        open_in_blender(&program, Path::new("/tmp/open_in_blender.py")).unwrap();
        for _ in 0..50 {
            if marker.exists() && std::fs::metadata(&marker).unwrap().len() > 0 {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "--python /tmp/open_in_blender.py");
    }
}
