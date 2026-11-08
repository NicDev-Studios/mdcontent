mod build;
mod config;
mod output;
mod serve;
mod watch;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "A small Markdown documentation generator")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Create a starter documentation project in the current directory.
    Init,
    /// Generate a static site.
    Build {
        #[arg(short, long, default_value = "mdcontent.toml")]
        config: PathBuf,
    },
    /// Build and serve the site locally with automatic reload.
    Serve {
        #[arg(short, long, default_value = "mdcontent.toml")]
        config: PathBuf,
        #[arg(long, default_value = "127.0.0.1:3000")]
        address: String,
    },
}
fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Init => build::init(),
        Command::Build { config } => build::build(&config).map(|_| ()),
        Command::Serve { config, address } => serve::serve(&config, &address),
    }
}
