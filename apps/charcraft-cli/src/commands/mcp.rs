//! `mcp`: `CharCraft` as a Model Context Protocol server on stdin and stdout.
//!
//! Two ways to run, both the same tools (they come from `charcraft-app`, so a click in the window and a tool call
//! are the same command):
//!
//! * headless: the document, the undo history and the hooks live on an actor thread of this process
//!   (`charcraft_app::control::run_headless`). Nothing needs a display; `screenshot` answers that there is no window.
//! * `--bridge PORT`: this process only forwards to a running `charcraft --control PORT` over its loopback control
//!   channel (token in a file), so `screenshot` captures the real window.
//!
//! Nothing but protocol messages goes to stdout.

use std::path::{Path, PathBuf};

use anim_mcp::{AnimMcp, connect_bridge_from_file, run_stdio};
use charcraft_app::control::{ControlArgs, INSTRUCTIONS, default_token_file, run_headless};
use charcraft_app::world::World;
use color_eyre::eyre::{Result, WrapErr, eyre};

use crate::cli::McpArgs;

/// Runs the command.
pub fn run(args: &McpArgs) -> Result<()> {
    args.bridge.map_or_else(|| headless(args), |port| bridge(port, args.token_file.as_deref()))
}

fn headless(args: &McpArgs) -> Result<()> {
    let opened = args.file.as_ref().map_or_else(World::demo, |file| World::open(file)).map_err(|e| eyre!("{e}"))?;
    for warning in opened.world.warnings.iter().take(5) {
        eprintln!("charcraft-cli: {warning}");
    }
    let control = ControlArgs {
        mcp_stdio: true,
        allow_read: args.allow_read.clone(),
        allow_write: args.allow_write.clone(),
        unrestricted: args.unrestricted,
        ..ControlArgs::default()
    };
    run_headless(opened, &control).map_err(|e| eyre!("{e}"))
}

/// The token file of the app on `port`: the one given, the default for the port, or (for an app started with
/// `--control 0`, which picks the port) the default for 0 when its address names this port.
fn token_file_for(port: u16, given: Option<&Path>) -> PathBuf {
    if let Some(file) = given {
        return file.to_path_buf();
    }
    let by_port = default_token_file(port);
    if by_port.is_file() {
        return by_port;
    }
    let picked = default_token_file(0);
    match anim_mcp::read_token_file(&picked) {
        Ok((_, addr)) if addr.port() == port => picked,
        _ => by_port,
    }
}

fn bridge(port: u16, token_file: Option<&Path>) -> Result<()> {
    let file = token_file_for(port, token_file);
    let backend = connect_bridge_from_file(&file)
        .wrap_err_with(|| format!("could not connect to CharCraft on port {port} using {} (start it with `charcraft --control {port}`)", file.display()))?;
    eprintln!("charcraft-cli: connected to {}", backend.description());
    let server = AnimMcp::new(backend, "charcraft", env!("CARGO_PKG_VERSION")).with_instructions(INSTRUCTIONS);
    run_stdio(server).map_err(|e| eyre!("{e}"))
}
