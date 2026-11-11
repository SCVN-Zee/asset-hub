//! `asset-hub-cli`: Asset Hub's search, inspect and export commands for scripts and coding agents.
//! Every run prints one JSON document on stdout; the exit code tells how it ended.

use asset_hub_lib::headless::cli::{dispatch, render, usage_error, Cli, Command};
use asset_hub_lib::headless::{app_paths, auth::TOKEN_ENV};
use clap::error::ErrorKind;
use clap::Parser;

fn finish(text: String, code: i32) -> ! {
    println!("{text}");
    std::process::exit(code)
}

fn main() {
    // Library code spawns through Tauri's runtime handle; it is set here, once, before anything else.
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(e) => {
            let (text, code) = render(Err(asset_hub_lib::headless::HeadlessError::unknown(format!("could not start the runtime: {e}"))), false);
            finish(text, code)
        }
    };
    tauri::async_runtime::set(runtime.handle().clone());
    let _guard = runtime.enter();

    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            print!("{e}");
            std::process::exit(0)
        }
        Err(e) => {
            // A bare run: the help goes to stderr, stdout keeps the JSON envelope.
            if e.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                eprint!("{}", e.render());
            }
            let (text, code) = render(Err(usage_error(&e)), false);
            finish(text, code)
        }
    };
    // The MCP server answers in protocol frames and never prints an envelope or exits on its own.
    if matches!(cli.command, Command::Mcp) {
        let served = runtime.block_on(asset_hub_lib::headless::mcp::serve_stdio(asset_hub_lib::http::API, std::env::var(TOKEN_ENV).ok()));
        if let Err(e) = served {
            eprintln!("asset-hub-cli mcp: {e}");
            std::process::exit(e.exit_code());
        }
        return;
    }
    let pretty = cli.pretty;
    let result = match app_paths() {
        Ok(paths) => runtime.block_on(dispatch(cli, paths, asset_hub_lib::http::API, std::env::var(TOKEN_ENV).ok())),
        Err(e) => Err(e),
    };
    let (text, code) = render(result, pretty);
    finish(text, code)
}
