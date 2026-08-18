mod indexer;
mod languages;
mod lsp;
mod mcp;
mod model;
mod query;
mod store;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about)]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the MCP server over stdin/stdout (default).
    Serve,
    /// Build or replace the compact index.
    Index {
        path: PathBuf,
        /// Shell command used to start Eclipse JDT Language Server.
        #[arg(long)]
        jdtls_command: Option<String>,
        /// Shell command used to start clojure-lsp.
        #[arg(long)]
        clojure_lsp_command: Option<String>,
        /// Shell command used to start TypeScript Language Server.
        #[arg(long)]
        typescript_lsp_command: Option<String>,
        /// Shell command used to start rust-analyzer.
        #[arg(long)]
        rust_analyzer_command: Option<String>,
        /// Shell command used to start the Dart SDK language server.
        #[arg(long)]
        dart_lsp_command: Option<String>,
        /// Per-request LSP timeout in milliseconds.
        #[arg(long, default_value_t = 5000)]
        lsp_timeout_ms: u64,
        /// LSP strategy: auto discovers installed servers, on requires configured/available servers, off disables them.
        #[arg(long, default_value = "auto")]
        lsp_mode: String,
    },
    /// Search an existing index without MCP.
    Search {
        query: String,
        #[arg(long, default_value = ".")]
        path: PathBuf,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Print compact index statistics.
    Overview {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

fn main() -> Result<()> {
    match Args::parse().command.unwrap_or(Command::Serve) {
        Command::Serve => mcp::serve(),
        Command::Index {
            path,
            jdtls_command,
            clojure_lsp_command,
            typescript_lsp_command,
            rust_analyzer_command,
            dart_lsp_command,
            lsp_timeout_ms,
            lsp_mode,
        } => {
            let options = indexer::BuildOptions {
                jdtls_command,
                clojure_lsp_command,
                typescript_lsp_command,
                rust_analyzer_command,
                dart_lsp_command,
                lsp_timeout_ms,
                lsp_mode: lsp::parse_mode(&lsp_mode)?,
            };
            let index = indexer::build_with_options(&path, &options)?;
            let output = store::default_path(&path);
            store::save(&index, &output)?;
            println!("{}", serde_json::to_string(&query::overview(&index))?);
            Ok(())
        }
        Command::Search {
            query: value,
            path,
            limit,
        } => {
            let index = store::load(&store::default_path(&path))?;
            println!(
                "{}",
                serde_json::to_string(&query::search(&index, &value, None, limit))?
            );
            Ok(())
        }
        Command::Overview { path } => {
            let index = store::load(&store::default_path(&path))?;
            println!("{}", serde_json::to_string(&query::overview(&index))?);
            Ok(())
        }
    }
}
