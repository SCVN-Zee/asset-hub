//! The Unity editors open on this Mac, and handing them a file (`src/main/unityEditors.ts`).

use std::path::Path;
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tokio::process::Command;

use crate::dto::RunningUnity;

const TIMEOUT: Duration = Duration::from_secs(15);

/// An editor found in the process list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnityProcess {
    pub pid: u32,
    pub project_path: String,
    pub version: Option<String>,
}

async fn run(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).stdin(Stdio::null()).kill_on_drop(true);
    let output = tokio::time::timeout(TIMEOUT, command.output()).await.map_err(|_| format!("{program} did not answer in time"))?.map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(if stderr.trim().is_empty() { format!("{program} failed") } else { stderr.trim().to_owned() })
    }
}

/// Editors from `ps` lines ("<pid> <command line>"). The project comes from `-projectPath`, which
/// Unity Hub always passes; a path may hold spaces, so it runs up to the next " -flag". Lines of
/// other programs are skipped, and so are Unity's own background workers (AssetImportWorker...,
/// which run `-batchMode` on the same project), so each open editor shows once.
pub fn parse_unity_processes(ps_output: &str) -> Vec<UnityProcess> {
    static LINE: OnceLock<Regex> = OnceLock::new();
    static BATCH: OnceLock<Regex> = OnceLock::new();
    static PROJECT: OnceLock<Regex> = OnceLock::new();
    static NEXT_FLAG: OnceLock<Regex> = OnceLock::new();
    static VERSION: OnceLock<Regex> = OnceLock::new();
    let line_re = LINE.get_or_init(|| Regex::new(r"^\s*(\d+)\s+(.*/Unity\.app/Contents/MacOS/Unity)(\s.*)?$").expect("a valid pattern"));
    let batch_re = BATCH.get_or_init(|| Regex::new(r"(?i)\s-batchmode(\s|$)").expect("a valid pattern"));
    let project_re = PROJECT.get_or_init(|| Regex::new(r"(?i)\s-projectpath\s+(.+)$").expect("a valid pattern"));
    let flag_re = NEXT_FLAG.get_or_init(|| Regex::new(r"\s+-[A-Za-z]").expect("a valid pattern"));
    let version_re = VERSION.get_or_init(|| Regex::new(r"/Editor/([^/]+)/Unity\.app/").expect("a valid pattern"));

    let mut found = Vec::new();
    for line in ps_output.lines() {
        let Some(m) = line_re.captures(line) else { continue };
        let rest = m.get(3).map_or("", |r| r.as_str());
        if batch_re.is_match(rest) {
            continue;
        }
        let Some(after) = project_re.captures(rest).map(|c| c[1].to_owned()) else { continue };
        let upto = flag_re.find(&after).map_or(after.len(), |f| f.start());
        let project = after[..upto].trim();
        let project = project.strip_prefix('"').and_then(|p| p.strip_suffix('"')).unwrap_or(project);
        if project.is_empty() {
            continue;
        }
        let Ok(pid) = m[1].parse() else { continue };
        found.push(UnityProcess {
            pid,
            project_path: project.trim_end_matches('/').to_owned(),
            version: version_re.captures(&m[2]).map(|v| v[1].to_owned()),
        });
    }
    found
}

/// productName from ProjectSettings, when readable and set.
fn product_name(project: &Path) -> Option<String> {
    let text = std::fs::read_to_string(project.join("ProjectSettings/ProjectSettings.asset")).ok()?;
    let name = Regex::new(r"(?m)^\s*productName:\s*(.+)$").expect("a valid pattern").captures(&text)?[1].trim().to_owned();
    (!name.is_empty() && name != "New Unity Project").then_some(name)
}

/// The Unity editors open right now, each with its project (only real Unity projects).
pub async fn list_running_unity() -> Vec<RunningUnity> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let Ok(ps) = run("ps", &["-axo", "pid=,args="]).await else { return Vec::new() };
    parse_unity_processes(&ps)
        .into_iter()
        .filter(|e| Path::new(&e.project_path).join("Assets").exists() && Path::new(&e.project_path).join("ProjectSettings").exists())
        .map(|e| {
            let folder = Path::new(&e.project_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let product = product_name(Path::new(&e.project_path)).filter(|p| *p != folder);
            RunningUnity { pid: e.pid, project_path: e.project_path, project: folder, product, version: e.version }
        })
        .collect()
}

/// The script that sends `file` to the process `pid` as an "open documents" Apple Event.
fn open_script(pid: u32, file: &str) -> String {
    format!(
        r#"
ObjC.import('AppKit')
const pid = {pid}
const target = $.NSAppleEventDescriptor.descriptorWithProcessIdentifier(pid)
const event = $.NSAppleEventDescriptor.appleEventWithEventClassEventIDTargetDescriptorReturnIDTransactionID(0x61657674, 0x6f646f63, target, -1, 0)
const url = $.NSURL.fileURLWithPath({file})
const files = $.NSAppleEventDescriptor.listDescriptor
files.insertDescriptorAtIndex($.NSAppleEventDescriptor.descriptorWithDescriptorTypeData(0x6675726c, url.absoluteString.dataUsingEncoding($.NSUTF8StringEncoding)), 1)
event.setParamDescriptorForKeyword(files, 0x2d2d2d2d)
const error = Ref()
const reply = event.sendEventWithOptionsTimeoutError(1, 15, error)
if (reply.isNil()) throw new Error('Unity did not take the file')
const app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(pid)
if (!app.isNil()) app.activateWithOptions(2)
'ok'"#,
        file = serde_json::to_string(file).expect("a string is JSON")
    )
}

/// Hands `file` to the editor with process id `pid`, as double-clicking it would, but to that one
/// editor even when several are open (`open -a` picks any). Unity then shows its import dialog for
/// a .unitypackage, and the editor is brought to the front.
pub async fn open_in_process(pid: u32, file: &str) -> Result<(), String> {
    run("osascript", &["-l", "JavaScript", "-e", &open_script(pid, file)]).await.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(pid: u32, path: &str, version: &str) -> UnityProcess {
        UnityProcess { pid, project_path: path.into(), version: Some(version.into()) }
    }

    #[test]
    fn reads_each_editor_its_project_with_spaces_and_version() {
        let ps = [
            "  812 /Applications/Unity/Hub/Editor/2022.3.19f1/Unity.app/Contents/MacOS/Unity -projectpath /Users/a/Projects/Luna Pizza -useHub -hubIPC -cloudEnvironment production",
            " 9001 /Applications/Unity/Hub/Editor/6000.0.1f1/Unity.app/Contents/MacOS/Unity -projectPath \"/Users/a/Crab Life/\"",
            "  77 /Applications/Unity Hub.app/Contents/MacOS/Unity Hub",
            "  78 /Applications/Unity/Hub/Editor/2022.3.19f1/Unity.app/Contents/MacOS/Unity -batchmode",
            "  80 /Applications/Unity/Hub/Editor/2022.3.19f1/Unity.app/Contents/MacOS/Unity -adb2 -batchMode -noUpm -name AssetImportWorker0 -projectPath /Users/a/Projects/Luna Pizza -logFile Logs/AssetImportWorker0.log",
            "  79 /bin/zsh -c grep Unity.app/Contents/MacOS/Unity",
        ]
        .join("\n");
        assert_eq!(
            parse_unity_processes(&ps),
            [editor(812, "/Users/a/Projects/Luna Pizza", "2022.3.19f1"), editor(9001, "/Users/a/Crab Life", "6000.0.1f1")]
        );
    }

    #[test]
    fn an_editor_outside_the_hub_has_no_version() {
        let ps = "  5 /Applications/Unity.app/Contents/MacOS/Unity -projectpath /p/Game";
        assert_eq!(parse_unity_processes(ps), [UnityProcess { pid: 5, project_path: "/p/Game".into(), version: None }]);
    }

    #[test]
    fn the_product_name_shows_only_when_it_says_something_new() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(product_name(dir.path()), None);
        std::fs::create_dir_all(dir.path().join("ProjectSettings")).unwrap();
        let write = |text: &str| std::fs::write(dir.path().join("ProjectSettings/ProjectSettings.asset"), text).unwrap();
        write("PlayerSettings:\n  productName: Crab Life\n  companyName: x\n");
        assert_eq!(product_name(dir.path()).as_deref(), Some("Crab Life"));
        write("PlayerSettings:\n  productName: New Unity Project\n");
        assert_eq!(product_name(dir.path()), None);
    }

    #[test]
    fn the_open_script_names_the_process_and_quotes_the_file() {
        let script = open_script(812, r#"/tmp/a "b".unitypackage"#);
        assert!(script.contains("const pid = 812\n"));
        assert!(script.contains(r#"fileURLWithPath("/tmp/a \"b\".unitypackage")"#));
    }
}
