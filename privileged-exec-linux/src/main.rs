//! `codex-privileged-exec-linux` — research-only structured exec MCP runner.
//!
//! Invoked by the Codex plugin host as: `codex-privileged-exec-linux mcp`
//! Serves the MCP protocol over stdio until the client disconnects.
//!
//! No sudo, no shell strings, no environment overrides, no arbitrary stdin,
//! no live credentials, no host sudo are accepted.

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "codex-privileged-exec-linux",
    about = "Research-only structured privileged-exec MCP runner for Zodex."
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Serve the MCP protocol over stdio.
    Mcp,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::Mcp => codex_privileged_exec_linux::mcp::serve_mcp().await,
    }
}
