//! MCP: how agents reach the app.
//!
//! One `SessionHost` owns the document, the undo history and the hooks. Three doors lead to it, all speaking the
//! same control protocol of `anim-mcp`, so a click and a tool call are the same thing:
//!
//! * the window itself, through an in-process [`Backend`] (file operations, exports),
//! * `--mcp-stdio`: an MCP server on stdin and stdout, running on its own thread against that backend,
//! * `--control-port`: the loopback control server (token in a file) that the `anim-mcp` bridge binary connects to,
//!   for agents that start their own MCP process.
//!
//! The window pumps the inboxes once per frame, so requests are handled between frames on the UI thread; no locks.
//! `--headless` runs the same host on an actor thread without a window (screenshots are then unsupported).

use std::path::PathBuf;

use anim_mcp::{AnimMcp, Backend, Confinement, ControlConfig, ControlHost, ControlInbox, ControlServer, HostInfo, Incoming, SessionHost, spawn_headless};
use clap::Args;

use crate::hooks::{Hooks, make_host};
use crate::live::LiveHandle;
use crate::session::{AppDoc, Links};
use crate::world::Opened;

/// What agents are told about the tools when they connect.
pub const INSTRUCTIONS: &str = "CharCraft is a live character animation app (puppets from Character Animator files). \
Start with session_status and list_commands. Commands are grouped: scene_* (stage, frame rate, length), track_* (puppets on the stage \
and their behavior edits), record_* (arming, punch range, scripted or explicit input recording), take_* (list, rename, enable, move, \
delete takes), audio_* (clips), playback_* (poses at frames), rig_* (tags, handles, origins on the puppet), tracking_* and mic_* (live \
camera and microphone), ui_set (drive the window: mode, selection, playhead). Queries (query_*) describe puppets and behaviors without \
changing anything. render_frame returns a PNG of a scene frame (or one puppet with options {puppet, viseme, blink}); screenshot returns \
the real window; export writes mp4, mov, png-sequence or png. Every edit is one undo step; run_batch makes several edits atomic. \
Record a performance headlessly with record_script (events: face, voice, pointer, key per frame), then render or export it.";

/// Options of the control channels.
#[derive(Debug, Clone, Args, Default)]
pub struct ControlArgs {
    /// Serve MCP on stdin and stdout (nothing else is printed there).
    #[arg(long)]
    pub mcp_stdio: bool,
    /// Serve the loopback control channel on this port (0 picks one) for the `anim-mcp` bridge.
    #[arg(long, value_name = "PORT")]
    pub control_port: Option<u16>,
    /// Where the control server writes its token and address.
    #[arg(long, value_name = "FILE")]
    pub token_file: Option<PathBuf>,
    /// Folders agents may read (repeatable). The working directory and the folder of the opened file are always allowed.
    #[arg(long, value_name = "DIR")]
    pub allow_read: Vec<PathBuf>,
    /// Folders agents may write (repeatable). The working directory and the temp folder are always allowed.
    #[arg(long, value_name = "DIR")]
    pub allow_write: Vec<PathBuf>,
    /// Let agents read and write anywhere.
    #[arg(long)]
    pub unrestricted: bool,
}

impl ControlArgs {
    /// Whether any agent door is requested.
    #[must_use]
    pub const fn agents(&self) -> bool {
        self.mcp_stdio || self.control_port.is_some()
    }

    /// The path confinement: none without agents (the person at the window is trusted), otherwise the allowed folders.
    ///
    /// # Errors
    /// A message when an allowed folder does not exist.
    pub fn confinement(&self, opened_file: Option<&std::path::Path>) -> Result<Confinement, String> {
        if self.unrestricted || !self.agents() {
            return Ok(Confinement::unrestricted());
        }
        let mut read = self.allow_read.clone();
        let mut write = self.allow_write.clone();
        for dir in [std::env::current_dir().ok(), Some(std::env::temp_dir())].into_iter().flatten() {
            write.push(dir);
        }
        if let Some(parent) = opened_file.and_then(|f| f.parent()).filter(|p| p.is_dir()) {
            read.push(parent.to_path_buf());
        }
        Confinement::new(&read, &write).map_err(|e| e.to_string())
    }
}

/// The doors into the host that the window pumps.
pub struct Control {
    inboxes: Vec<ControlInbox>,
    server: Option<ControlServer>,
    /// The window's own backend into the host.
    pub backend: Backend,
    /// A line describing the doors, for the status bar and the log.
    pub summary: String,
}

impl Control {
    /// Opens the doors. `wake` asks the window to repaint when a request arrives.
    ///
    /// # Errors
    /// A message when a socket or thread cannot be started.
    pub fn start(args: &ControlArgs, wake: impl Fn() + Clone + Send + 'static) -> Result<Self, String> {
        let (own, backend) = ControlInbox::in_process(wake.clone()).map_err(|e| e.to_string())?;
        let mut inboxes = vec![own];
        let mut summary = Vec::new();
        if args.mcp_stdio {
            let (inbox, stdio_backend) = ControlInbox::in_process(wake.clone()).map_err(|e| e.to_string())?;
            inboxes.push(inbox);
            let mcp = AnimMcp::new(stdio_backend, "charcraft", env!("CARGO_PKG_VERSION")).with_instructions(INSTRUCTIONS);
            std::thread::Builder::new()
                .name("charcraft-mcp-stdio".into())
                .spawn(move || {
                    if let Err(e) = anim_mcp::run_stdio(mcp) {
                        eprintln!("charcraft: MCP on stdio ended: {e}");
                    }
                })
                .map_err(|e| e.to_string())?;
            summary.push("MCP on stdio".to_owned());
        }
        let server = match args.control_port {
            Some(port) => {
                let server = ControlServer::start(&ControlConfig { port, token_file: args.token_file.clone() }, wake).map_err(|e| format!("control server: {e}"))?;
                summary.push(format!("control channel on {}", server.addr()));
                Some(server)
            }
            None => None,
        };
        Ok(Self { inboxes, server, backend, summary: if summary.is_empty() { "no agents".to_owned() } else { summary.join(", ") } })
    }

    /// Hands every waiting request to the host; returns how many there were.
    pub fn pump(&self, host: &mut impl ControlHost) -> usize {
        let mut handled = self.inboxes.iter().map(|inbox| inbox.pump(host)).sum::<usize>();
        if let Some(server) = &self.server {
            handled = handled.saturating_add(server.inbox().pump(host));
        }
        handled
    }

    /// The control server's address and token, when it runs.
    #[must_use]
    pub const fn server(&self) -> Option<&ControlServer> {
        self.server.as_ref()
    }
}

/// The host plus the live actor it keeps alive, for the headless actor thread.
struct HeadlessHost {
    host: SessionHost<AppDoc, Hooks>,
    _live: LiveHandle,
}

impl ControlHost for HeadlessHost {
    fn handle(&mut self, incoming: Incoming) {
        self.host.handle(incoming);
    }

    fn tick(&mut self) {
        self.host.tick();
    }
}

/// Runs the app without a window until the MCP client on stdio disconnects (or the control channel is closed).
///
/// # Errors
/// A message when the host or a transport cannot start.
pub fn run_headless(opened: Opened, args: &ControlArgs) -> Result<(), String> {
    let confinement = args.confinement(opened.world.source.as_deref())?;
    let registry = crate::session::registry().map_err(|e| e.to_string())?;
    let info = HostInfo { name: "charcraft".into(), version: env!("CARGO_PKG_VERSION").into() };
    let session = spawn_headless(move || {
        let live = LiveHandle::spawn(|| {});
        // Nobody listens for view changes without a window: `ui_set` answers that there is no window.
        let (ui, _no_window) = crossbeam_channel::unbounded();
        let links = Links { live: live.link(), ui };
        HeadlessHost { host: make_host(opened, links, registry, confinement, info), _live: live }
    })
    .map_err(|e| e.to_string())?;
    let mcp = AnimMcp::new(session.backend().clone(), "charcraft", env!("CARGO_PKG_VERSION")).with_instructions(INSTRUCTIONS);
    let served = anim_mcp::run_stdio(mcp).map_err(|e| e.to_string());
    session.shutdown();
    served
}
