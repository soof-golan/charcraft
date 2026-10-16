//! `charcraft-cli`: headless `CharCraft`. Inspect, render, lip sync, animate, export and convert puppets and projects.
//!
//! ```text
//! charcraft-cli info PUPPET|CHPROJ|PSD|AI [--json] [--outline]
//! charcraft-cli render PUPPET|PSD|AI [--pose neutral|pose.json] [--viseme Ah] -o out.png
//! charcraft-cli lipsync speech.wav -o visemes.json
//! charcraft-cli animate PUPPET --audio speech.wav -o out.mp4
//! charcraft-cli export PROJECT --scene NAME -o out.mp4|frames/
//! charcraft-cli convert in.puppet out.puppet
//! charcraft-cli mcp [PUPPET|CHPROJ|...]        MCP server on stdio, headless (the demo puppet without a file)
//! charcraft-cli mcp --bridge PORT              MCP server on stdio for a running `charcraft --control PORT`
//! ```

mod cli;
mod commands;
mod frames;
mod perform;
mod source;
mod video;

use clap::Parser;
use cli::{Cli, Command};

fn main() -> color_eyre::eyre::Result<()> {
    color_eyre::install()?;
    match Cli::parse().command {
        Command::Info(args) => commands::info::run(&args),
        Command::Render(args) => commands::render::run(&args),
        Command::Lipsync(args) => commands::lipsync::run(&args),
        Command::Animate(args) => commands::animate::run(&args),
        Command::Export(args) => commands::export::run(&args),
        Command::Convert(args) => commands::convert::run(&args),
        Command::Mcp(args) => commands::mcp::run(&args),
    }
}
