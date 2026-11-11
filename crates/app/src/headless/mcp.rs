//! The commands of `asset-hub-cli` as MCP tools over stdio.
//!
//! A tool call builds the argument list of the matching command and runs it through the CLI's own
//! parser and [`dispatch`], so validation, defaults and error codes are the CLI's. A success returns
//! the CLI's `data` value and an error the CLI's `{code,message}`. A malformed call fails before any
//! sign-in is looked for, like on the command line. The server starts while signed out: a call finds the sign-in when it runs. stdout carries protocol
//! frames only; diagnostics go to stderr.

use clap::Parser;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::common::schema_for_input;
use rmcp::model::{CallToolResult, ContentBlock, Implementation, JsonObject, ServerCapabilities, ServerConfig};
use rmcp::{tool, tool_handler, tool_router, ServerHandler, ServiceExt};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

use super::cli::{dispatch, usage_error, Cli};
use super::error::HeadlessError;
use crate::paths::AppPaths;

const PROGRAM: &str = "asset-hub-cli";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInput {
    /// Text to look for in asset names.
    query: String,
    /// Only this asset type, such as prefab or texture.
    #[serde(rename = "type")]
    kind: Option<String>,
    /// branch (needs repo and branch), repo (every branch of repo; slow on repos with many branches) or all (the app's catalog). Default: branch with repo and branch, all with neither.
    #[serde(rename = "where")]
    scope: Option<String>,
    /// `owner/repo`; needs branch, or where=repo to search every branch.
    repo: Option<String>,
    /// A branch of repo; needs repo.
    branch: Option<String>,
    /// How many hits to return (1-100, default 20).
    limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RefInput {
    /// The asset, as `owner/repo@branch:path` (the `ref` of a search hit).
    #[serde(rename = "ref")]
    reference: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DepsInput {
    /// The asset, as `owner/repo@branch:path`.
    #[serde(rename = "ref")]
    reference: String,
    /// instant, quick or full (default full).
    mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ExportInput {
    /// One or more `owner/repo@branch:path` from the same repo and branch.
    refs: Vec<String>,
    /// The .unitypackage file to write; it must not exist yet.
    output: String,
    /// A Unity project to compare with; it is only read.
    project: Option<String>,
    /// source, grouped or by-type.
    layout: Option<String>,
    /// Keep the scripts that the export would remove.
    #[serde(rename = "keepScripts")]
    keep_scripts: Option<bool>,
}

/// The MCP server. `paths` of `None` means the app's folders, found when a tool runs.
#[derive(Clone)]
pub struct AssetHubMcp {
    paths: Option<AppPaths>,
    api: String,
    env_token: Option<String>,
    tool_router: ToolRouter<Self>,
}

impl AssetHubMcp {
    pub fn new(paths: Option<AppPaths>, api: impl Into<String>, env_token: Option<String>) -> Self {
        Self { paths, api: api.into(), env_token, tool_router: Self::tool_router() }
    }

    /// Runs the command `args` (after the program name) like the command line would.
    async fn run(&self, args: Vec<String>) -> CallToolResult {
        match self.data(args).await {
            Ok(data) => CallToolResult::success(vec![ContentBlock::text(data.to_string())]),
            Err(error) => failure(error),
        }
    }

    async fn data(&self, args: Vec<String>) -> Result<Value, HeadlessError> {
        let cli = Cli::try_parse_from(std::iter::once(PROGRAM.to_owned()).chain(args)).map_err(|e| usage_error(&e))?;
        let paths = match &self.paths {
            Some(paths) => paths.clone(),
            None => super::app_paths()?,
        };
        dispatch(cli, paths, &self.api, self.env_token.clone()).await
    }
}

/// The tool result of a failed call: the CLI's error object as text, flagged as an error.
fn failure(error: HeadlessError) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(json!({ "code": error.code, "message": error.message }).to_string())])
}

/// Reads the arguments of a call as `T`. A wrong or missing field is a tool error, not a protocol error.
fn parse<T: DeserializeOwned>(arguments: JsonObject) -> Result<T, HeadlessError> {
    serde_json::from_value(Value::Object(arguments)).map_err(|e| HeadlessError::bad_args(format!("invalid tool arguments: {e}")))
}

/// `flag value` when `value` is given.
fn option(args: &mut Vec<String>, flag: &str, value: Option<impl ToString>) {
    if let Some(value) = value {
        args.push(flag.to_owned());
        args.push(value.to_string());
    }
}

/// Positional values go after `--`, so a value that starts with `-` is not read as an option.
fn positional(args: &mut Vec<String>, values: impl IntoIterator<Item = String>) {
    args.push("--".to_owned());
    args.extend(values);
}

#[tool_router]
impl AssetHubMcp {
    #[tool(
        description = "Find Unity assets by name in the team's GitHub repos: in one branch, in every branch of a repo, or in the app's catalog of all repos. Returns hits with their ref.",
        input_schema = schema_for_input::<SearchInput>().expect("a valid input schema"),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn search_assets(&self, arguments: JsonObject) -> CallToolResult {
        let input: SearchInput = match parse(arguments) {
            Ok(input) => input,
            Err(error) => return failure(error),
        };
        let mut args = vec!["search".to_owned()];
        option(&mut args, "--type", input.kind);
        option(&mut args, "--where", input.scope);
        option(&mut args, "--repo", input.repo);
        option(&mut args, "--branch", input.branch);
        option(&mut args, "--limit", input.limit);
        positional(&mut args, [input.query]);
        self.run(args).await
    }

    #[tool(
        description = "Show type, size, dependency count and measurements of one asset.",
        input_schema = schema_for_input::<RefInput>().expect("a valid input schema"),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn asset_info(&self, arguments: JsonObject) -> CallToolResult {
        let input: RefInput = match parse(arguments) {
            Ok(input) => input,
            Err(error) => return failure(error),
        };
        let mut args = vec!["info".to_owned()];
        positional(&mut args, [input.reference]);
        self.run(args).await
    }

    #[tool(
        description = "List the files an import of the asset copies.",
        input_schema = schema_for_input::<DepsInput>().expect("a valid input schema"),
        annotations(read_only_hint = true, open_world_hint = true)
    )]
    async fn asset_dependencies(&self, arguments: JsonObject) -> CallToolResult {
        let input: DepsInput = match parse(arguments) {
            Ok(input) => input,
            Err(error) => return failure(error),
        };
        let mut args = vec!["deps".to_owned()];
        option(&mut args, "--mode", input.mode);
        positional(&mut args, [input.reference]);
        self.run(args).await
    }

    #[tool(
        description = "Write assets and what they depend on to a .unitypackage file at `output`. Fails when the file exists. The Unity project, when given, is only read; nothing is imported into Unity.",
        input_schema = schema_for_input::<ExportInput>().expect("a valid input schema"),
        annotations(read_only_hint = false, destructive_hint = false, idempotent_hint = false, open_world_hint = true)
    )]
    async fn export_unitypackage(&self, arguments: JsonObject) -> CallToolResult {
        let input: ExportInput = match parse(arguments) {
            Ok(input) => input,
            Err(error) => return failure(error),
        };
        let mut args = vec!["export".to_owned(), "--output".to_owned(), input.output];
        option(&mut args, "--project", input.project);
        option(&mut args, "--layout", input.layout);
        if input.keep_scripts == Some(true) {
            args.push("--keep-scripts".to_owned());
        }
        positional(&mut args, input.refs);
        self.run(args).await
    }

    #[tool(
        description = "The link that opens an asset in the Asset Hub app.",
        input_schema = schema_for_input::<RefInput>().expect("a valid input schema"),
        annotations(read_only_hint = true, open_world_hint = false)
    )]
    async fn asset_link(&self, arguments: JsonObject) -> CallToolResult {
        let input: RefInput = match parse(arguments) {
            Ok(input) => input,
            Err(error) => return failure(error),
        };
        let mut args = vec!["link".to_owned()];
        positional(&mut args, [input.reference]);
        self.run(args).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for AssetHubMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("asset-hub", env!("CARGO_PKG_VERSION")))
            .with_instructions("Search, inspect and export Unity assets from the team's GitHub repos. Search first, then pass a hit's ref to the other tools.")
    }
}

/// Serves the tools on stdin and stdout until the client closes the connection.
pub async fn serve_stdio(api: &str, env_token: Option<String>) -> Result<(), HeadlessError> {
    let server = AssetHubMcp::new(None, api, env_token);
    let running = server.serve(rmcp::transport::stdio()).await.map_err(|e| HeadlessError::unknown(format!("could not start the MCP server: {e}")))?;
    running.waiting().await.map_err(|e| HeadlessError::unknown(format!("the MCP server stopped: {e}")))?;
    Ok(())
}
