//! The commands of `asset-hub-cli`, as data: parsing, validation, and one JSON result per command.
//!
//! Arguments are checked before any sign-in is looked for or any folder is made, so a malformed
//! command costs no request.

use std::path::{Path, PathBuf};

use asset_hub_core::export::unitypackage::PackageLayout;
use asset_hub_core::github::source::ResolveMode;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Serialize;
use serde_json::{json, Value};

use super::auth::account_with;
use super::error::HeadlessError;
use super::ops::{check_output, check_repo, parse_ref, AssetRef, SearchScope};
use super::{forbidden_roots, Headless};
use crate::paths::AppPaths;
use crate::target_project::is_unity_project;

/// The scheme and host of the link that opens an asset in the app (see `src/shared/deeplink.ts`).
const LINK_BASE: &str = "assethub://open";

#[derive(Debug, Parser)]
#[command(name = "asset-hub-cli", version, about = "Search, inspect and export Unity assets from GitHub, without the Asset Hub window. Prints one JSON document on stdout.")]
pub struct Cli {
    /// Indent the JSON.
    #[arg(long, global = true)]
    pub pretty: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Show the GitHub account in use and where its token came from.
    Whoami,
    /// List the repos the account can read.
    Repos,
    /// List the branches of a repo.
    Branches {
        /// `owner/repo`.
        repo: String,
    },
    /// Find assets by name.
    Search {
        query: String,
        /// Only this asset type, such as prefab or texture.
        #[arg(long = "type")]
        kind: Option<String>,
        /// How far to look: branch (needs --repo and --branch), repo (every branch of --repo; slow on repos with many branches) or all (the app's catalog).
        /// Default: branch with --repo and --branch, all with neither; --repo alone needs --branch or --where repo.
        #[arg(long = "where")]
        scope: Option<String>,
        /// `owner/repo`; needs --branch, or --where repo to search every branch.
        #[arg(long)]
        repo: Option<String>,
        /// A branch of --repo; needs --repo.
        #[arg(long)]
        branch: Option<String>,
        /// How many hits to return (1-100, default 20).
        #[arg(long)]
        limit: Option<usize>,
    },
    /// Show type, size, dependency count and measurements of one asset.
    Info {
        /// `owner/repo@branch:path`.
        reference: String,
    },
    /// List the files an import of the asset copies.
    Deps {
        /// `owner/repo@branch:path`.
        reference: String,
        #[arg(long, value_enum, default_value = "full")]
        mode: Mode,
    },
    /// Write assets and what they depend on to a .unitypackage.
    Export {
        /// One or more `owner/repo@branch:path` from the same repo and branch.
        #[arg(required = true, num_args = 1..)]
        references: Vec<String>,
        /// The .unitypackage file to write.
        #[arg(long)]
        output: PathBuf,
        /// A Unity project to compare with; it is only read.
        #[arg(long)]
        project: Option<PathBuf>,
        #[arg(long, value_enum)]
        layout: Option<Layout>,
        /// Keep the scripts that the export would remove.
        #[arg(long)]
        keep_scripts: bool,
        /// Replace the output file if it exists.
        #[arg(long)]
        force: bool,
    },
    /// Print the link that opens an asset in Asset Hub.
    Link {
        /// `owner/repo@branch:path`.
        reference: String,
    },
    /// Serve the commands as MCP tools over stdin and stdout, for coding agents.
    Mcp,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Mode {
    Instant,
    Quick,
    Full,
}

impl From<Mode> for ResolveMode {
    fn from(mode: Mode) -> Self {
        match mode {
            Mode::Instant => Self::Instant,
            Mode::Quick => Self::Quick,
            Mode::Full => Self::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Layout {
    Source,
    Grouped,
    ByType,
}

impl From<Layout> for PackageLayout {
    fn from(layout: Layout) -> Self {
        match layout {
            Layout::Source => Self::Source,
            Layout::Grouped => Self::Grouped,
            Layout::ByType => Self::ByType,
        }
    }
}

/// A command whose arguments passed every check that needs no network.
enum Plan {
    Whoami,
    Repos,
    Branches(String),
    Search { query: String, kind: Option<String>, scope: SearchScope, repo: Option<String>, branch: Option<String>, limit: Option<usize> },
    Info(String),
    Deps(String, ResolveMode),
    Export { references: Vec<String>, output: PathBuf, project: Option<PathBuf>, layout: Option<PackageLayout>, strip_scripts: Option<bool>, force: bool },
    Link(AssetRef),
}

fn plan(command: Command, paths: &AppPaths) -> Result<Plan, HeadlessError> {
    Ok(match command {
        Command::Whoami => Plan::Whoami,
        Command::Repos => Plan::Repos,
        Command::Branches { repo } => {
            check_repo(&repo)?;
            Plan::Branches(repo)
        }
        Command::Search { query, kind, scope, repo, branch, limit } => {
            let scope = match scope.as_deref() {
                Some(text) => SearchScope::parse(text)?,
                None if repo.is_some() && branch.is_some() => SearchScope::Branch,
                None if repo.is_some() => return Err(HeadlessError::bad_args("add --branch <name>, or pass --where repo to search every branch of the repo (slow on repos with many branches)")),
                None => SearchScope::All,
            };
            if branch.is_some() && repo.is_none() {
                return Err(HeadlessError::bad_args("--branch needs --repo"));
            }
            if scope == SearchScope::All && (repo.is_some() || branch.is_some()) {
                return Err(HeadlessError::bad_args("--where all searches every repo and branch; drop --repo and --branch, or use --where repo or --where branch"));
            }
            match scope {
                SearchScope::Branch if repo.is_none() || branch.is_none() => return Err(HeadlessError::bad_args("--where branch needs --repo and --branch")),
                SearchScope::Repo if repo.is_none() => return Err(HeadlessError::bad_args("--where repo needs --repo")),
                _ => {}
            }
            if let Some(repo) = &repo {
                check_repo(repo)?;
            }
            Plan::Search { query, kind, scope, repo, branch, limit }
        }
        Command::Info { reference } => {
            parse_ref(&reference)?;
            Plan::Info(reference)
        }
        Command::Deps { reference, mode } => {
            parse_ref(&reference)?;
            Plan::Deps(reference, mode.into())
        }
        Command::Export { references, output, project, layout, keep_scripts, force } => {
            let parsed = references.iter().map(|r| parse_ref(r)).collect::<Result<Vec<_>, _>>()?;
            if parsed.iter().any(|a| a.repo != parsed[0].repo || a.branch != parsed[0].branch) {
                return Err(HeadlessError::new("mixed_branches", "all assets of one export must come from the same repo and branch"));
            }
            if let Some(dir) = &project {
                if !is_unity_project(dir) {
                    return Err(HeadlessError::new("not_unity_project", format!("{} has no Assets and ProjectSettings folders", dir.display())));
                }
            }
            let forbidden = forbidden_roots(paths);
            check_output(&output, force, &forbidden.iter().map(PathBuf::as_path).collect::<Vec<&Path>>())?;
            Plan::Export { references, output, project, layout: layout.map(Into::into), strip_scripts: keep_scripts.then_some(false), force }
        }
        Command::Link { reference } => Plan::Link(parse_ref(&reference)?),
        Command::Mcp => return Err(HeadlessError::bad_args("mcp starts a server and prints no result; run it as `asset-hub-cli mcp`")),
    })
}

/// The link that opens `asset` in the app.
fn link(asset: &AssetRef) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new()).append_pair("repo", &asset.repo).append_pair("branch", &asset.branch).append_pair("path", &asset.path).finish();
    format!("{LINK_BASE}?{query}")
}

fn to_value<T: Serialize>(value: T) -> Result<Value, HeadlessError> {
    serde_json::to_value(value).map_err(|e| HeadlessError::unknown(format!("could not write the result: {e}")))
}

/// Runs one command and answers its `data`. `env_token` is the value of `ASSETHUB_GITHUB_TOKEN`;
/// `api` is the GitHub API root.
pub async fn dispatch(cli: Cli, paths: AppPaths, api: &str, env_token: Option<String>) -> Result<Value, HeadlessError> {
    let plan = plan(cli.command, &paths)?;
    if let Plan::Link(asset) = &plan {
        return Ok(json!({ "url": link(asset) }));
    }
    let account = account_with(&paths, api, env_token).await?;
    if matches!(plan, Plan::Whoami) {
        return Ok(json!({ "login": account.login, "source": account.source }));
    }
    let hub = Headless::with_account(paths, api, account)?;
    match plan {
        Plan::Whoami | Plan::Link(_) => unreachable!("answered above"),
        Plan::Repos => to_value(hub.session.repos().await.map_err(|e| HeadlessError::from(e).redact(hub.session.token()))?),
        Plan::Branches(repo) => to_value(hub.session.source.list_branches(&repo).await.map_err(|e| HeadlessError::from(e).redact(hub.session.token()))?),
        Plan::Search { query, kind, scope, repo, branch, limit } => to_value(hub.search(&query, kind.as_deref(), scope, repo.as_deref(), branch.as_deref(), limit).await?),
        Plan::Info(reference) => to_value(hub.info(&reference).await?),
        Plan::Deps(reference, mode) => to_value(hub.deps(&reference, mode).await?),
        Plan::Export { references, output, project, layout, strip_scripts, force } => to_value(hub.export(&references, &output, project.as_deref(), layout, strip_scripts, force).await?),
    }
}

/// The error for text clap rejected: its first line, without the `error:` prefix.
pub fn usage_error(error: &clap::Error) -> HeadlessError {
    if error.kind() == clap::error::ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
        return HeadlessError::bad_args("missing command; run asset-hub-cli --help");
    }
    let text = error.render().to_string();
    let first = text.lines().next().unwrap_or("invalid arguments");
    HeadlessError::bad_args(first.strip_prefix("error: ").unwrap_or(first))
}

/// The JSON document to print and the exit code of a finished command.
pub fn render(result: Result<Value, HeadlessError>, pretty: bool) -> (String, i32) {
    let (document, code) = match result {
        Ok(data) => (json!({ "ok": true, "data": data }), 0),
        Err(error) => {
            let code = error.exit_code();
            (json!({ "ok": false, "error": { "code": error.code, "message": error.message } }), code)
        }
    };
    let text = if pretty { serde_json::to_string_pretty(&document) } else { serde_json::to_string(&document) };
    (text.unwrap_or_else(|_| r#"{"ok":false,"error":{"code":"unknown","message":"could not write the result"}}"#.to_owned()), code)
}
