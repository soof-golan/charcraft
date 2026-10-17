//! The window: owns the host, the actors and the view, and runs one frame at a time.
//!
//! A frame is: take in what the world did (agent requests, live input, finished pictures), advance the clock and
//! the performance, ask the renderer for the picture of the current state, draw the UI as a function of that state,
//! and apply the actions it returned to the session. Nothing blocks on the actors except short tool calls.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anim_core::Tick;
use anim_mcp::{HostInfo, Reply, Request, SessionHost};
use charcraft_behaviors::api::{StackInfo, StackOp, stack_info};
use charcraft_behaviors::{BehaviorStack, Inputs};
use charcraft_puppet::PuppetPose;
use charcraft_record::{AudioTrack, track_stack};
use charcraft_stream::{Background, OutputHandle, Picture};
use crossbeam_channel::{Receiver, Sender, unbounded};
use egui::{Context, TextureHandle, TextureOptions};
use serde_json::{Value, json};

use crate::control::{Control, ControlArgs};
use crate::export::{self, Format};
use crate::hooks::{Hooks, PendingShot, ViewStatus, make_host};
use crate::live::{LiveEvent, LiveHandle, MicAction, behavior_body, behavior_face};
use crate::midi::MidiBank;
use crate::render::{RenderHandle, RenderRequest};
use crate::session::{AppDoc, Links, registry};
use crate::state::{Mode, UiSet};
use crate::view::{Action, ActiveTake, Ctx, Dialog, PreviewTexture, View};
use crate::world::{Opened, World};

mod start_thumbs;
mod workspace_ops;
use workspace_ops::WorkspaceState;

/// What the command line asked for.
#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools, reason = "one flag per command line switch")]
pub struct Options {
    /// Show the start workspace (`Some(true)`), never show it (`Some(false)`), or let the preferences decide.
    pub start: Option<bool>,
    /// Open the Preferences window.
    pub prefs_open: bool,
    /// The mode to start in.
    pub mode: Mode,
    /// Take a screenshot of the window to this file, then exit.
    pub screenshot: Option<PathBuf>,
    /// Record a scripted performance into the scene before showing it.
    pub demo_takes: bool,
    /// Start the demo face and the demo voice.
    pub live_demo: bool,
    /// Pretend a MIDI keyboard is plugged in and played (screenshots and demos): a device, a bound note, a bound knob.
    pub demo_midi: bool,
    /// Show the grid, two guides and turn snapping on (screenshots and demos).
    pub demo_grid: bool,
    /// Track the faces in this photo, animated GIF or folder of images instead of a camera.
    pub face_file: Option<PathBuf>,
    /// Dark theme.
    pub dark: bool,
    /// Start this stream output at launch.
    pub stream: Option<charcraft_stream::Kind>,
    /// Start with a transparent stream view (no backdrop in the window or the outputs).
    pub transparent: bool,
    /// A window to open at the start: `export` or `queue` (for screenshots).
    pub open: Option<String>,
    /// A floating window to open at the start (`warnings` or `maker`).
    pub dialog: Option<String>,
    /// The agent doors.
    pub control: ControlArgs,
}

/// How the run ended, reported to `main`.
pub type Outcome = Result<(), String>;

/// The token of the screenshot `--screenshot` asked for.
const CLI_SHOT: u64 = u64::MAX;
/// Frames the picture must stay unchanged before a command-line screenshot is taken.
const SETTLE_FRAMES: u32 = 24;
/// How long a command-line screenshot may take.
const SHOT_TIMEOUT: Duration = Duration::from_secs(40);

enum ExportEvent {
    Progress(f32, String),
    Done(Result<String, String>),
}

struct Shot {
    out: PathBuf,
    started: Instant,
    settled: u32,
    requested: bool,
}

/// The behaviors of the performed track, stepping once per frame.
struct Perform {
    key: String,
    track: usize,
    edits: Vec<StackOp>,
    generation: (u64, u64),
    stack: BehaviorStack,
    clock: Instant,
}

/// What was last sent to the renderer, to skip identical requests.
#[derive(PartialEq)]
struct Desc {
    record: bool,
    key: String,
    pose: PuppetPose,
    frame: i64,
    scale: i64,
    rev: u64,
    generation: (u64, u64),
}

/// The application.
pub struct CharCraftApp {
    host: SessionHost<AppDoc, Hooks>,
    control: Control,
    live: LiveHandle,
    render: RenderHandle,
    output: OutputHandle,
    ui_rx: Receiver<UiSet>,
    export_tx: Sender<ExportEvent>,
    export_rx: Receiver<ExportEvent>,
    done: Sender<Outcome>,
    view: View,
    doc: AppDoc,
    doc_rev: u64,
    rev: u64,
    stack: Option<StackInfo>,
    stack_key: Option<(u64, usize)>,
    texture: Option<(TextureHandle, [usize; 2])>,
    perform: Option<Perform>,
    pose: PuppetPose,
    seen: (u64, u64),
    pending: Option<u64>,
    next_id: u64,
    last: Option<Desc>,
    started: Instant,
    last_frame: Instant,
    last_status: Instant,
    shot: Option<Shot>,
    awaiting: Vec<(u64, PendingShot)>,
    next_shot: u64,
    title: String,
    theme_dirty: bool,
    ws: WorkspaceState,
    thumbs: Thumbs,
}

/// Ids of thumbnail renders start here, far above the ids of the main picture.
const THUMB_BASE: u64 = 1 << 62;
/// How long a thumbnail render may take before it is asked for again.
const THUMB_PATIENCE: Duration = Duration::from_secs(3);
/// Thumbnails are refreshed at most this often while the document changes.
const THUMB_INTERVAL: Duration = Duration::from_millis(700);

/// Small pictures of the puppets and scenes for the Project panel, drawn one at a time while the main view is idle.
#[derive(Default)]
struct Thumbs {
    textures: BTreeMap<String, (TextureHandle, [usize; 2])>,
    queue: VecDeque<String>,
    inflight: Option<(u64, String, Instant)>,
    next: u64,
    rev: u64,
    refreshed: Option<Instant>,
    /// Pictures of the starters and recent files arriving from the background thread, and what they were made for.
    start_rx: Option<Receiver<(String, Option<crate::compose::Picture>)>>,
    start_jobs: Vec<String>,
}

fn script_events() -> Vec<charcraft_behaviors::ScriptEvent> {
    use anim_lipsync::Viseme as V;
    use charcraft_behaviors::{FaceInput, ScriptEvent};
    let mut events = Vec::new();
    for step in 0_u32..12 {
        let t = f64::from(step) / 12.0 * std::f64::consts::TAU;
        events.push(ScriptEvent::Face {
            frame: step.saturating_mul(8),
            face: FaceInput {
                head_yaw: 14.0 * t.sin(),
                head_pitch: 6.0 * (t * 2.0).sin(),
                head_roll: 5.0 * t.cos(),
                eyebrow_left: 0.4 * t.sin(),
                eyebrow_right: 0.4 * t.sin(),
                smile: 0.5 + 0.4 * t.cos(),
                ..FaceInput::default()
            },
        });
    }
    let shapes = [V::Ah, V::Neutral, V::Oh, V::Ee, V::D, V::Neutral, V::Uh, V::Ah, V::M, V::S, V::Ah, V::Neutral, V::Oh, V::Ee, V::Neutral, V::D];
    for (i, viseme) in shapes.into_iter().enumerate() {
        events.push(ScriptEvent::Voice { frame: u32::try_from(i).unwrap_or(0).saturating_mul(6), viseme: Some(viseme) });
    }
    events
}

/// The view with the window named by `--open` showing: `export` (the dialog) or `queue` (the queue with sample items).
fn opened_view(mut view: View, open: Option<&str>, scene_name: &str) -> View {
    match open {
        Some("export") => view.export.dialog = Some(export::Dialog::new(scene_name, "./")),
        Some("queue") => {
            view.export.queue_open = true;
            for (format, name) in [(Format::Mp4, "scene.mp4"), (Format::Gif, "scene.gif"), (Format::AfterEffects, "scene-ae")] {
                view.export.queue.push(format, PathBuf::from(name), export::Options::default());
            }
        }
        _ => {}
    }
    view
}

impl CharCraftApp {
    /// Builds the app: starts the actors and the agent doors around an opened document.
    ///
    /// # Errors
    /// A message when a door cannot start.
    #[expect(clippy::too_many_lines, reason = "starting every actor and door in one place")]
    pub fn new(cc: &eframe::CreationContext<'_>, opened: Opened, options: &Options, done: Sender<Outcome>) -> Result<Self, String> {
        let ctx = cc.egui_ctx.clone();
        if crate::ui::theme::install_fonts(&ctx) == 0 {
            eprintln!("charcraft: no system font covers Hebrew or Arabic; right-to-left text may show boxes");
        }
        let ws = WorkspaceState::open();
        let dark = options.dark && ws.prefs().dark;
        crate::ui::theme::apply(&ctx, dark);
        let wake = move || ctx.request_repaint();
        let live = LiveHandle::spawn(wake.clone());
        let render = RenderHandle::spawn(wake.clone());
        let output = OutputHandle::spawn();
        let (ui_tx, ui_rx) = unbounded();
        let links = Links::new(live.link(), ui_tx).with_output(output.link());
        let confinement = options.control.confinement(opened.world.source.as_deref())?;
        let registry = registry().map_err(|e| e.to_string())?;
        let info = HostInfo { name: "charcraft".into(), version: env!("CARGO_PKG_VERSION").into() };
        let mut host = make_host(opened, links, registry, confinement, info);
        host.hooks_mut().windowed = true;
        let midi_backend: Box<dyn charcraft_midi::Backend> =
            if options.demo_midi { Box::new(charcraft_midi::MockBackend::new(&["Demo Keyboard"])) } else { charcraft_midi::system_backend() };
        host.hooks_mut().midi = MidiBank::new(charcraft_midi::MidiHandle::spawn(midi_backend, wake.clone()));
        host.hooks_mut().midi.restore(&ws.prefs().midi);
        host.hooks_mut().grid = crate::grid::GridState::with_settings(ws.prefs().grid);
        let control = Control::start(&options.control, wake)?;
        let (export_tx, export_rx) = unbounded();
        let doc = host.session().doc().clone();
        let now = Instant::now();
        let mut app = Self {
            host,
            control,
            live,
            render,
            output,
            ui_rx,
            export_tx,
            export_rx,
            done,
            // Screenshots always show the built-in layouts: they neither read nor write the user's saved ones.
            view: opened_view(
                View {
                    mode: options.mode,
                    dark,
                    dialog: options.dialog.as_deref().and_then(Dialog::parse),
                    docks: crate::ui::dock::Docks::new(options.screenshot.is_none()),
                    ..View::default()
                },
                options.open.as_deref(),
                &doc.rec.scene.name,
            ),
            doc,
            doc_rev: 1,
            rev: 1,
            stack: None,
            stack_key: None,
            texture: None,
            perform: None,
            pose: PuppetPose::neutral(),
            seen: (0, 0),
            pending: None,
            next_id: 1,
            last: None,
            started: now,
            last_frame: now,
            last_status: now,
            shot: options.screenshot.clone().map(|out| Shot { out, started: now, settled: 0, requested: false }),
            awaiting: Vec::new(),
            next_shot: 1,
            title: String::new(),
            theme_dirty: false,
            ws,
            thumbs: Thumbs::default(),
        };
        app.init_workspace(&cc.egui_ctx, options.start);
        app.view.prefs_open = options.prefs_open;
        if options.demo_takes {
            app.demo_performance();
        }
        if options.demo_midi {
            app.demo_midi();
        }
        if options.demo_grid {
            use crate::grid::{GridOp, GridPatch, GridView};
            use anim_ui::stage::{Axis, Guide};
            let links = &app.host.session().doc().links;
            let patch = GridPatch { show_grid: Some(true), snap: Some(true), size: Some(40.0), ..GridPatch::default() };
            let _ = links.grid.send(GridOp::Set(patch));
            for (axis, pos) in [(Axis::Vertical, 320.0), (Axis::Horizontal, 460.0)] {
                let _ = links.grid.send(GridOp::AddGuide(GridView::Rig, Guide { axis, pos }));
                let _ = links.grid.send(GridOp::AddGuide(GridView::Scene, Guide { axis, pos }));
            }
        }
        if options.live_demo {
            let link = app.live.link();
            let _ = link.tool("tracking_start", &json!({ "demo": true }));
            let _ = link.tool("tracking_body_start", &json!({ "demo": true }));
            let _ = link.mic(MicAction::DemoVoice(true));
            app.view.live.demo_voice = true;
            app.view.live.tracking = true;
            app.view.live.body_tracking = true;
        }
        if let Some(file) = &options.face_file {
            let started = app.live.link().tool("tracking_start", &json!({ "file": file }));
            app.view.status = match started {
                Ok(_) => format!("tracking {}", file.display()),
                Err(e) => format!("tracking_start: {e}"),
            };
        }
        if !app.control.summary.is_empty() {
            app.view.status = app.control.summary.clone();
        }
        app.view.transparent = options.transparent;
        app.view.stream.spec.background = if options.transparent { Background::Transparent } else { Background::Colour(app.backdrop_colour()) };
        if let Some(kind) = options.stream {
            let spec = charcraft_stream::Spec { kind, ..app.view.stream.spec.clone() };
            app.view.status = match app.output.start(&spec) {
                Ok(status) => format!("Streaming {}: {}", kind.label(), status.detail),
                Err(e) => format!("{} did not start: {e}", kind.label()),
            };
            app.view.stream.spec.kind = kind;
        }
        app.refresh_inputs();
        app.refresh_outputs();
        Ok(app)
    }

    /// Records a scripted performance on the first track (a lively first look, and the content of screenshots).
    fn demo_performance(&mut self) {
        let session = self.host.session_mut();
        let setup =
            [("record.arm", json!({ "groups": ["head", "brows", "eyes", "mouth", "voice"], "only": true })), ("scene.set", json!({ "duration_seconds": 8.0 }))];
        for (id, params) in setup {
            if let Err(e) = session.execute(id, &params) {
                self.view.status = format!("{id}: {e}");
            }
        }
        let script = charcraft_behaviors::Script { fps: 30.0, frames: 120, events: script_events() };
        let script = serde_json::to_value(&script).unwrap_or(Value::Null);
        for start in [0.5, 4.0] {
            if let Err(e) = session.execute("record.script", &json!({ "track": 0, "script": script, "start_seconds": start })) {
                self.view.status = format!("record.script: {e}");
            }
        }
        let _ = session.execute("take.set", &json!({ "track": 0, "take": 1, "name": "Take 2" }));
        self.view.playhead = 1.6;
        self.rev = self.rev.saturating_add(1);
    }

    /// Plugs in a pretend keyboard, binds a note to the fringe and a knob to the microphone sensitivity, and holds the note.
    fn demo_midi(&mut self) {
        use crate::midi::MidiOp;
        use charcraft_midi::{CcBinding, MidiMessage};
        let session = self.host.session_mut();
        let edit = json!({ "op": "set_midi_trigger", "layer": "Head/Fringe", "note": 60, "channel": 1 });
        if let Err(e) = session.execute("rig.edit", &edit) {
            self.view.status = format!("rig.edit: {e}");
        }
        let links = &session.doc().links;
        let send = |bytes: [u8; 3]| links.midi.send(MidiOp::Send(bytes.to_vec()));
        let _ = links.midi.send(MidiOp::Refresh);
        let _ = links.midi.send(MidiOp::Connect(Some("Demo Keyboard".to_owned())));
        let _ = links.midi.send(MidiOp::Bind { control: "mic_gain".to_owned(), binding: CcBinding { channel: Some(1), controller: 74 } });
        let _ = send(MidiMessage::ControlChange { channel: 1, controller: 74, value: 80 }.to_bytes());
        let _ = send(MidiMessage::NoteOn { channel: 1, note: 60, velocity: 100 }.to_bytes());
        self.rev = self.rev.saturating_add(1);
    }

    // --- taking in the world --------------------------------------------------------------------------------------

    /// Copies the MIDI state to the view, saves the controller bindings when they changed and tells the window
    /// when a learned note edited the document.
    fn sync_midi(&mut self) {
        let hooks = self.host.hooks_mut();
        self.view.midi = hooks.midi.snapshot();
        let saved = hooks.midi.take_prefs();
        let grid = hooks.grid.take_prefs();
        self.view.grid = hooks.grid.clone();
        if std::mem::take(&mut hooks.doc_touched) {
            self.rev = self.rev.saturating_add(1);
        }
        if let Some(midi) = saved {
            self.ws.store.update(|p| p.midi = midi);
            self.view.prefs = self.ws.prefs().clone();
        }
        if let Some(settings) = grid {
            self.ws.store.update(|p| p.grid = settings);
            self.view.prefs = self.ws.prefs().clone();
        }
    }

    fn begin_frame(&mut self, ctx: &Context) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f64().min(0.25);
        self.last_frame = now;
        if self.control.pump(&mut self.host) > 0 {
            self.rev = self.rev.saturating_add(1);
        }
        while let Ok(set) = self.ui_rx.try_recv() {
            self.apply_ui_set(&set);
        }
        self.pump_live(ctx);
        self.pump_exports(ctx);
        self.advance_clock(dt);
        if now.duration_since(self.last_status) > Duration::from_millis(400) {
            self.last_status = now;
            self.refresh_live_status();
            self.watch_artwork();
            self.view.presets = self.host.session().doc().links.presets.all();
        }
        self.pump_render_results(ctx);
        self.pump_start_thumbs(ctx);
        self.pump_workspace();
        self.sync_midi();
        let view = &self.view;
        let hooks = self.host.hooks_mut();
        hooks.view = ViewStatus {
            mode: view.mode,
            track: view.track,
            layer: view.layer.clone(),
            playhead_seconds: view.playhead,
            playing: view.playing,
            recording: view.recording.is_some(),
            mirror: view.mirror,
        };
        let (epoch, generation) = (hooks.epoch, hooks.world.generation);
        let title = format!("{} - CharCraft", hooks.world.title);
        if title != self.title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title = title;
        }
        if (epoch, generation) != self.seen {
            self.document_changed(epoch, generation);
        }
        if self.rev != self.doc_rev {
            self.doc = self.host.session().doc().clone();
            self.doc_rev = self.rev;
        }
        let key = (self.rev, self.view.track);
        if self.stack_key != Some(key) {
            self.stack_key = Some(key);
            let scene = &self.doc.rec.scene;
            let templates = self.host.hooks_mut().world.cast.templates();
            self.stack = scene.tracks.get(self.view.track).and_then(|t| track_stack(scene, &templates, self.view.track, t).ok()).map(|s| stack_info(&s));
        }
    }

    fn document_changed(&mut self, epoch: u64, generation: u64) {
        let new_document = epoch != self.seen.0;
        self.seen = (epoch, generation);
        let cast = self.host.hooks_mut().world.cast.clone();
        self.render.send(RenderRequest::Cast(cast));
        self.last = None;
        self.rev = self.rev.saturating_add(1);
        if new_document {
            self.view.track = 0;
            self.view.layer = None;
            self.view.playhead = 0.0;
            self.view.playing = false;
            self.view.recording = None;
            self.view.rig_stage.fit_request = true;
            self.view.scene_stage.fit_request = true;
            self.view.peaks.clear();
            self.texture = None;
            self.perform = None;
        }
    }

    fn apply_ui_set(&mut self, set: &UiSet) {
        if let Some(mode) = set.mode {
            self.set_mode(mode);
        }
        if let Some(track) = set.track {
            self.view.track = track.min(self.doc.rec.scene.tracks.len().saturating_sub(1));
        }
        if let Some(layer) = &set.layer {
            self.view.layer = Some(layer.clone());
        }
        if let Some(s) = set.seek_seconds {
            self.seek(s);
        }
        if let Some(on) = set.playing {
            self.view.playing = on;
        }
        if let Some(on) = set.mirror {
            self.view.mirror = on;
        }
        if let Some(dark) = set.dark {
            self.view.dark = dark;
            self.theme_dirty = true;
        }
        if let Some(on) = set.demo_voice {
            let _ = self.live.link().mic(MicAction::DemoVoice(on));
            self.view.live.demo_voice = on;
        }
        if let Some(keys) = &set.triggers {
            self.view.stream.held = keys.iter().map(|k| k.to_lowercase()).collect();
        }
        if let Some(swaps) = &set.swaps {
            self.view.stream.swaps.extend(swaps.iter().map(|(set, member)| (set.clone(), member.clone())));
        }
        if let Some(on) = set.stream_panels {
            self.view.stream.panels = on;
        }
        if let Some(on) = set.transparent {
            self.view.transparent = on;
            self.view.stream.spec.background = if on { Background::Transparent } else { Background::Colour(self.backdrop_colour()) };
        }
        if let Some(on) = set.green_screen {
            self.view.chroma = on;
        }
        if let Some(name) = &set.dialog {
            self.view.dialog = Dialog::parse(name);
        }
    }

    /// Looks at the artwork file twice a second; reads it again at once when the person asked for that.
    fn watch_artwork(&mut self) {
        let hooks = self.host.hooks_mut();
        let was_stale = hooks.world.stale;
        if !hooks.world.check_source() || was_stale {
            return;
        }
        if self.view.auto_reimport {
            self.run("puppet.reimport", &json!({}), None);
            self.host.hooks_mut().world.acknowledge_source();
        }
    }

    fn new_made(&mut self, spec: charcraft_maker::Spec) {
        match World::from_spec(spec) {
            Ok(opened) => {
                let doc = self.host.hooks_mut().adopt(opened);
                self.host.session_mut().replace_document(doc, None);
                self.rev = self.rev.saturating_add(1);
            }
            Err(e) => self.view.status = format!("New puppet failed: {e}"),
        }
    }

    /// The colour behind the puppet in outputs when the view is not transparent: the green screen, or the dark backdrop.
    const fn backdrop_colour(&self) -> [u8; 3] {
        if self.view.chroma { charcraft_stream::KEY_GREEN } else { [0x15, 0x16, 0x1b] }
    }

    /// Asks the live actor what cameras and microphones there are.
    fn refresh_inputs(&mut self) {
        let link = self.live.link();
        // The camera search runs off the UI thread; its outcome arrives as an event (see `take_cameras`).
        self.view.live.cameras_searching = true;
        self.view.live.cameras_error = None;
        link.search_cameras();
        if let Ok(list) = link.mic(MicAction::Devices) {
            self.view.live.mic_devices =
                list.get("devices").and_then(Value::as_array).map(|all| all.iter().filter_map(|d| d.as_str().map(str::to_owned)).collect()).unwrap_or_default();
            self.view.live.mic_device = list.get("selected").and_then(Value::as_str).map(str::to_owned);
        }
    }

    /// Takes the outcome of a camera search into the view: the list, or the reason there is none.
    fn take_cameras(&mut self, outcome: Result<Value, String>) {
        let live = &mut self.view.live;
        live.cameras_searching = false;
        match outcome {
            Ok(list) => {
                live.cameras_error = None;
                live.camera_demo = list.get("demo").and_then(Value::as_bool).unwrap_or(false);
                live.cameras = list
                    .get("cameras")
                    .and_then(Value::as_array)
                    .map(|all| all.iter().filter_map(|c| Some((u32::try_from(c.get("index")?.as_u64()?).ok()?, c.get("name")?.as_str()?.to_owned()))).collect())
                    .unwrap_or_default();
            }
            Err(message) => {
                live.cameras.clear();
                live.cameras_error = Some(message);
            }
        }
    }

    /// Asks what each stream output needs here.
    fn refresh_outputs(&mut self) {
        self.view.stream.outputs = charcraft_stream::outputs();
        if let Ok(status) = self.output.status() {
            self.view.stream.status = status;
        }
    }

    /// Takes a camera preview into the view: the image as a texture and the tracked points.
    fn take_preview(&mut self, ctx: &Context, preview: &charcraft_tracking::actor::Preview) {
        let live = &mut self.view.live;
        live.previews = live.previews.saturating_add(1);
        live.preview_points.clone_from(&preview.points);
        match &preview.frame {
            Some(frame) => {
                let size = [usize::try_from(frame.width()).unwrap_or(1), usize::try_from(frame.height()).unwrap_or(1)];
                let image = egui::ColorImage::from_rgb(size, frame.rgb());
                match live.preview.as_mut() {
                    Some(texture) => texture.0.set(image, TextureOptions::LINEAR),
                    None => live.preview = Some(PreviewTexture(ctx.load_texture("charcraft-camera", image, TextureOptions::LINEAR))),
                }
            }
            None => live.preview = None,
        }
    }

    fn pump_live(&mut self, ctx: &Context) {
        while let Some(event) = self.live.try_event() {
            match event {
                LiveEvent::Preview(preview) => self.take_preview(ctx, &preview),
                LiveEvent::Cameras(outcome) => self.take_cameras(outcome),
                LiveEvent::Face(_, face) => {
                    self.view.live.face = Some(face);
                    self.view.live.faces = self.view.live.faces.saturating_add(1);
                }
                LiveEvent::Body(_, body) => {
                    self.view.live.body = Some(*body);
                    self.view.live.bodies = self.view.live.bodies.saturating_add(1);
                }
                LiveEvent::Voice { viseme, level } => {
                    self.view.live.viseme = viseme;
                    self.view.live.level = level;
                }
                LiveEvent::Audio(chunk) => {
                    let at = Tick::from_seconds_f64((self.view.playhead - chunk.seconds()).max(0.0));
                    if let Some(active) = self.view.recording.as_mut() {
                        active.recorder.push_audio(at, &chunk);
                    }
                }
            }
        }
    }

    fn refresh_live_status(&mut self) {
        let link = self.live.link();
        if let Ok(status) = link.tool("tracking_status", &Value::Null) {
            self.view.live.tracking = status.get("running").and_then(Value::as_bool).unwrap_or(false);
            self.view.live.calibrating = status.get("calibrating").and_then(|c| serde_json::from_value::<(usize, usize)>(c.clone()).ok());
            self.view.live.problem = status.get("last_error").and_then(Value::as_str).map(str::to_owned);
            self.view.live.source = status.get("source").and_then(Value::as_str).map(str::to_owned);
        }
        if let Ok(status) = link.tool("tracking_body_status", &Value::Null) {
            self.view.live.body_tracking = status.get("running").and_then(Value::as_bool).unwrap_or(false);
            self.view.live.setting_rest_pose = status.get("capturing").and_then(|c| serde_json::from_value::<(usize, usize)>(c.clone()).ok());
            self.view.live.rest_pose_set = status.pointer("/calibration/captured").and_then(Value::as_bool).unwrap_or(false);
            self.view.live.body_problem = status.get("last_error").and_then(Value::as_str).map(str::to_owned);
        }
        if let Ok(mic) = link.mic(MicAction::Status) {
            self.view.live.mic = mic.get("running").and_then(Value::as_bool).unwrap_or(false);
            self.view.live.demo_voice = mic.get("demo_voice").and_then(Value::as_bool).unwrap_or(false);
            if let Some(error) = mic.get("error").and_then(Value::as_str) {
                self.view.live.problem = Some(error.to_owned());
            }
            self.view.live.mic_device = mic.get("device").and_then(Value::as_str).map(str::to_owned);
            self.view.live.mic_gain = mic.get("gain").and_then(Value::as_f64).map_or(1.0, anim_core::num::f64_to_f32);
        }
        if let Ok(status) = self.output.status() {
            self.view.stream.status = status;
        }
    }

    fn pump_exports(&mut self, ctx: &Context) {
        while let Ok(event) = self.export_rx.try_recv() {
            match event {
                ExportEvent::Progress(f, message) => self.view.exporting = Some((f, message)),
                ExportEvent::Done(result) => {
                    self.view.exporting = None;
                    self.view.export.queue.finish(result.clone().map(|_| ()));
                    self.view.last_export = Some(match result {
                        Ok(summary) => format!("Exported {summary}"),
                        Err(e) => format!("Export failed: {e}"),
                    });
                }
            }
        }
        if self.view.exporting.is_none() {
            self.start_next_queued(ctx);
        }
    }

    /// Starts the next waiting item of the queue when it is running.
    fn start_next_queued(&mut self, ctx: &Context) {
        if let Some((format, path, options)) = self.view.export.queue.take_next() {
            self.export(ctx, format, &path, &options);
        }
    }

    fn advance_clock(&mut self, dt: f64) {
        if !self.view.playing && self.view.recording.is_none() {
            return;
        }
        let duration = self.doc.rec.scene.duration.seconds();
        self.view.playhead += dt;
        if self.view.playhead >= duration {
            if self.view.recording.is_some() {
                self.view.playhead = duration;
                self.stop_record();
            } else if self.view.looping {
                self.view.playhead = 0.0;
            } else {
                self.view.playhead = duration;
                self.view.playing = false;
            }
        }
    }

    fn pump_render_results(&mut self, ctx: &Context) {
        while let Some(done) = self.render.try_result() {
            if done.id >= THUMB_BASE {
                let name = self.thumbs.inflight.take_if(|(id, _, _)| *id == done.id).map(|(_, name, _)| name);
                if let (Some(name), Ok(picture)) = (name, done.picture) {
                    let size = [usize::try_from(picture.width).unwrap_or(1), usize::try_from(picture.height).unwrap_or(1)];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &picture.rgba);
                    if let Some((texture, current)) = self.thumbs.textures.get_mut(&name) {
                        texture.set(image, TextureOptions::LINEAR);
                        *current = size;
                    } else {
                        let texture = ctx.load_texture(format!("thumb-{name}"), image, TextureOptions::LINEAR);
                        self.thumbs.textures.insert(name, (texture, size));
                    }
                }
                continue;
            }
            if self.pending == Some(done.id) {
                self.pending = None;
            }
            match done.picture {
                Ok(picture) => {
                    let size = [usize::try_from(picture.width).unwrap_or(1), usize::try_from(picture.height).unwrap_or(1)];
                    let image = egui::ColorImage::from_rgba_unmultiplied(size, &picture.rgba);
                    match self.texture.as_mut() {
                        Some((texture, current)) => {
                            texture.set(image, TextureOptions::LINEAR);
                            *current = size;
                        }
                        None => self.texture = Some((ctx.load_texture("charcraft-main", image, TextureOptions::LINEAR), size)),
                    }
                    // What the window draws is what goes out: the same picture, with its transparency.
                    if self.view.stream.status.active
                        && let Ok(out) = Picture::new(picture.width, picture.height, picture.rgba)
                    {
                        self.output.send_picture(out);
                    }
                }
                Err(e) => self.view.status = format!("render: {e}"),
            }
        }
    }

    // --- performing and drawing -----------------------------------------------------------------------------------

    /// Steps the performed puppet's behaviors with the live input and returns its pose.
    fn perform(&mut self, ctx: &Context) -> PuppetPose {
        let hooks = self.host.hooks_mut();
        let generation = (hooks.epoch, hooks.world.generation);
        let Some(key) = hooks.world.key_for_track(&self.doc.rec, self.view.track) else { return PuppetPose::neutral() };
        let track = self.doc.rec.scene.tracks.get(self.view.track);
        let edits = track.map(|t| t.behaviors.clone()).unwrap_or_default();
        let rebuild = self.perform.as_ref().is_none_or(|p| p.key != key || p.track != self.view.track || p.edits != edits || p.generation != generation);
        if rebuild {
            let templates = hooks.world.cast.templates();
            let stack = track
                .and_then(|t| track_stack(&self.doc.rec.scene, &templates, self.view.track, t).ok())
                .or_else(|| templates.get(&key).cloned())
                .unwrap_or_else(|| BehaviorStack::standard(charcraft_behaviors::Rig::default()));
            self.perform = Some(Perform { key, track: self.view.track, edits, generation, stack, clock: Instant::now() });
        }
        let Some(perform) = self.perform.as_mut() else { return PuppetPose::neutral() };
        let mut inputs = Inputs::at(Tick::from_seconds_f64(perform.clock.elapsed().as_secs_f64()));
        let live = &self.view.live;
        if live.tracking
            && let Some(face) = live.face.as_ref().filter(|f| f.tracked)
        {
            inputs.face = Some(behavior_face(face, self.view.mirror));
        }
        if live.body_tracking
            && let Some(body) = live.body.as_ref().filter(|b| b.tracked)
        {
            inputs.body = Some(behavior_body(body, self.view.mirror));
        }
        if live.mic || live.demo_voice {
            inputs.viseme = live.viseme;
        }
        if let Some((x, y, down)) = self.view.pointer {
            inputs = inputs.with_pointer(0, x, y, down);
        }
        if !ctx.egui_wants_keyboard_input() {
            let keys: BTreeSet<String> = ctx.input(|i| i.keys_down.iter().map(|k| k.name().to_lowercase()).collect());
            inputs.keys = keys;
        }
        // Triggers held from the stream view's Triggers panel (or over MCP) count as keys held down.
        inputs.keys.extend(self.view.stream.held.iter().cloned());
        // Notes held on a MIDI keyboard are trigger tokens (`midi:60`, `midi:1:60`).
        inputs.keys.extend(self.view.midi.held.iter().cloned());
        if let Some(active) = self.view.recording.as_mut() {
            let mut recorded = inputs.clone();
            recorded.time = Tick::from_seconds_f64(self.view.playhead);
            active.recorder.feed(&recorded);
            active.frames = active.frames.saturating_add(1);
        }
        let mut pose = perform.stack.step(&inputs);
        for (set, member) in &self.view.stream.swaps {
            pose.swaps.insert(set.clone(), member.clone());
        }
        pose
    }

    fn request_render(&mut self, ctx: &Context, pose: &PuppetPose) {
        let screen = ctx.content_rect();
        let ppp = ctx.pixels_per_point();
        let hooks = self.host.hooks_mut();
        let Some(key) = hooks.world.key_for_track(&self.doc.rec, self.view.track) else { return };
        let generation = (hooks.epoch, hooks.world.generation);
        let Some(member) = hooks.world.cast.get(&key) else { return };
        let artwork = member.puppet.size;
        let quantise = |s: f64| (s.clamp(0.05, 4.0) * 50.0).round() / 50.0;
        let record = self.view.mode == Mode::Record;
        let (scale, pose, frame) = match self.view.mode {
            Mode::Rig | Mode::Start => {
                // Without live input the rig shows the rest pose: idle motion (breathing, physics) would redraw the
                // artwork every frame and make it impossible to judge the rig.
                let live_on = self.view.live.tracking || self.view.live.mic || self.view.live.demo_voice;
                let mut pose = if live_on { pose.clone() } else { PuppetPose::neutral() };
                if let Some(v) = self.view.preview_viseme {
                    pose.viseme = Some(v);
                }
                if self.view.preview_blink {
                    pose = pose.with_blink(1.0);
                }
                (quantise(1400.0 / artwork.width.max(artwork.height).max(1.0)).min(1.0), pose, 0)
            }
            Mode::Stream => (quantise(f64::from(screen.height() * ppp) / artwork.height.max(1.0)).min(2.0), pose.clone(), 0),
            Mode::Record => {
                let scene = &self.doc.rec.scene;
                let live_on = self.view.live.tracking || self.view.live.mic || self.view.live.demo_voice || self.view.recording.is_some();
                let pose = if self.view.monitor && live_on { pose.clone() } else { PuppetPose::neutral() };
                let width = f64::from((screen.width() - 320.0).max(200.0) * ppp);
                (quantise(width / scene.size.width.max(1.0)).min(1.0), pose, scene.frame_at(Tick::from_seconds_f64(self.view.playhead)).max(0))
            }
        };
        let desc =
            Desc { record, key: key.clone(), pose: pose.clone(), frame, scale: anim_core::num::round_f64_to_i64(scale * 1000.0), rev: self.rev, generation };
        if self.pending.is_some() || self.last.as_ref() == Some(&desc) {
            return;
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.pending = Some(id);
        if record {
            let live = if self.view.monitor && (self.view.live.tracking || self.view.live.mic || self.view.live.demo_voice || self.view.recording.is_some()) {
                vec![(self.view.track, pose)]
            } else {
                Vec::new()
            };
            self.render.send(RenderRequest::Scene { id, scene: Box::new(self.doc.rec.scene.clone()), frame, live, scale });
        } else {
            self.render.send(RenderRequest::Puppet { id, key, pose: Box::new(pose), scale, background: None });
        }
        self.last = Some(desc);
    }

    /// Draws the next thumbnail when nothing else is being drawn: the neutral pose of each puppet and the first frame
    /// of each scene. They are redrawn a little after the document changes.
    fn drive_thumbs(&mut self) {
        if let Some((_, name, since)) = self.thumbs.inflight.as_ref()
            && since.elapsed() > THUMB_PATIENCE
        {
            // The renderer keeps only the newest request of a kind, so a thumbnail can be overtaken: ask again.
            self.thumbs.queue.push_front(name.clone());
            self.thumbs.inflight = None;
        }
        if self.thumbs.inflight.is_some() || self.texture.is_none() {
            return;
        }
        // While live input redraws the main picture every frame there is never a quiet moment: a thumbnail then goes
        // first, and the main picture that it may have overtaken is asked for again.
        let busy = self.pending.is_some();
        if busy && !(self.view.live.tracking || self.view.live.mic || self.view.live.demo_voice) {
            return;
        }
        if self.thumbs.queue.is_empty() {
            let stale = self.thumbs.rev != self.rev || self.thumbs.textures.is_empty();
            if !stale || self.thumbs.refreshed.is_some_and(|t| t.elapsed() < THUMB_INTERVAL) {
                return;
            }
            self.thumbs.rev = self.rev;
            self.thumbs.refreshed = Some(Instant::now());
            let mut names: Vec<String> = self.host.hooks_mut().world.cast.keys().map(|k| format!("puppet:{k}")).collect();
            names.extend((0..self.doc.rec.scene_count()).map(|i| format!("scene:{i}")));
            self.thumbs.queue = names.into();
        }
        let Some(name) = self.thumbs.queue.pop_front() else { return };
        let id = THUMB_BASE.saturating_add(self.thumbs.next);
        self.thumbs.next = self.thumbs.next.saturating_add(1);
        let request = if let Some(key) = name.strip_prefix("puppet:") {
            let height = self.host.hooks_mut().world.cast.get(key).map_or(1.0, |m| m.puppet.size.height.max(1.0));
            Some(RenderRequest::Puppet { id, key: key.to_owned(), pose: Box::new(PuppetPose::neutral()), scale: (96.0 / height).min(1.0), background: None })
        } else {
            name.strip_prefix("scene:").and_then(|i| i.parse::<usize>().ok()).and_then(|i| self.doc.rec.scenes().nth(i)).map(|scene| RenderRequest::Scene {
                id,
                scene: Box::new(scene.clone()),
                frame: 0,
                live: Vec::new(),
                scale: (128.0 / scene.size.width.max(1.0)).min(1.0),
            })
        };
        if let Some(request) = request {
            if busy {
                self.pending = None;
                self.last = None;
            }
            self.render.send(request);
            self.thumbs.inflight = Some((id, name, Instant::now()));
        }
    }

    // --- applying actions -----------------------------------------------------------------------------------------

    fn run(&mut self, id: &str, params: &Value, merge: Option<&str>) {
        let session = self.host.session_mut();
        let result = if merge.is_some() { session.execute_merged(id, params, merge) } else { session.execute(id, params) };
        self.rev = self.rev.saturating_add(1);
        match result {
            Ok(_) => self.view.status = format!("{id} ok"),
            Err(e) => self.view.status = format!("{id}: {e}"),
        }
    }

    fn call(&mut self, request: Request) -> Result<Reply, anim_mcp::ControlError> {
        let pending = self.control.backend.submit(request);
        self.control.pump(&mut self.host);
        self.rev = self.rev.saturating_add(1);
        pending.wait(|_| {})
    }

    fn set_mode(&mut self, mode: Mode) {
        self.view.start = mode == Mode::Start;
        if mode == Mode::Start {
            self.refresh_start_info();
        }
        if self.view.mode != mode {
            self.view.mode = mode;
            self.texture = None;
            self.last = None;
            self.view.rig_stage.fit_request = true;
            self.view.scene_stage.fit_request = true;
        }
    }

    fn seek(&mut self, seconds: f64) {
        let duration = self.doc.rec.scene.duration.seconds();
        self.view.playhead = if seconds.is_finite() { seconds.clamp(0.0, duration) } else { 0.0 };
    }

    fn start_record(&mut self) {
        if self.doc.rec.scene.tracks.get(self.view.track).is_none() {
            "Add a puppet track first".clone_into(&mut self.view.status);
            return;
        }
        let mut recorder = self.doc.rec.recorder();
        if let Err(e) = recorder.start(Tick::from_seconds_f64(self.view.playhead)) {
            self.view.status = format!("Cannot record: {e}. Arm a part of the performance first.");
            return;
        }
        let audio = self.doc.rec.arming.audio && self.view.live.mic;
        if self.doc.rec.arming.audio && !self.view.live.mic {
            "The microphone is armed but off: no sound will be kept".clone_into(&mut self.view.status);
        }
        self.live.link().forward_audio(audio);
        self.view.recording = Some(ActiveTake { recorder, track: self.view.track, start: self.view.playhead, audio, frames: 0 });
        self.view.playing = true;
    }

    /// Finds the mouth shapes in the sound of a scene clip and makes a voice take from them.
    fn lipsync_clip(&mut self, track: usize, audio_track: usize, clip: usize) {
        let media = self.doc.rec.scene.audio.get(audio_track).and_then(|t| t.clips.get(clip)).map(|c| c.media.clone());
        let Some(media) = media else {
            "That clip does not exist".clone_into(&mut self.view.status);
            return;
        };
        let Some(buffer) = self.host.hooks_mut().world.media.get(&media) else {
            self.view.status = format!("The sound {media} is not loaded");
            return;
        };
        let keys = anim_lipsync::analyze(&buffer.to_mono().samples, buffer.sample_rate);
        let seconds = buffer.seconds();
        let keys: Vec<Value> = keys.iter().map(|k| json!({ "time_seconds": k.start.seconds(), "viseme": k.viseme.ca_layer_name() })).collect();
        self.run("lipsync.from_clip", &json!({ "track": track, "audio_track": audio_track, "clip": clip, "keys": keys, "media_seconds": seconds }), None);
    }

    fn stop_record(&mut self) {
        let Some(mut active) = self.view.recording.take() else { return };
        self.live.link().forward_audio(false);
        self.view.playing = false;
        let end = Tick::from_seconds_f64(self.view.playhead);
        match active.recorder.finish(end) {
            Ok(recording) => {
                let mut media = std::mem::take(&mut self.host.hooks_mut().world.media);
                let track = active.track;
                let committed = self.host.session_mut().edit("Record take", None, |doc| {
                    if doc.rec.scene.audio.is_empty() {
                        doc.rec.scene.audio.push(AudioTrack::new("Voice"));
                    }
                    doc.rec.commit(track, recording, 0, &mut media)
                });
                self.host.hooks_mut().world.media = media;
                self.rev = self.rev.saturating_add(1);
                self.view.status = match committed {
                    Ok(done) => format!("Recorded take {}{}", done.take.map_or(0, |t| t.saturating_add(1)), if active.audio { " with sound" } else { "" }),
                    Err(e) => format!("Could not keep the take: {e}"),
                };
            }
            Err(e) => self.view.status = format!("Recording failed: {e}"),
        }
    }

    fn export(&mut self, ctx: &Context, format: Format, path: &Path, options: &export::Options) {
        let request = Request::Export {
            format: format.id().to_owned(),
            path: path.display().to_string(),
            options: serde_json::to_value(options).unwrap_or_else(|_| json!({})),
        };
        let pending = self.control.backend.submit(request);
        let (tx, ctx) = (self.export_tx.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("charcraft-export-relay".into()).spawn(move || {
            let progress_tx = tx.clone();
            let progress_ctx = ctx.clone();
            let result = pending.wait(move |p| {
                let _ = progress_tx.send(ExportEvent::Progress(p.fraction, p.message.clone()));
                progress_ctx.request_repaint();
            });
            let _ = tx.send(ExportEvent::Done(result.map(|reply| reply.value.to_string()).map_err(|e| e.to_string())));
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => {
                self.view.exporting = Some((0.0, "starting".to_owned()));
                self.control.pump(&mut self.host);
            }
            Err(e) => {
                self.view.status = format!("Export: {e}");
                self.view.export.queue.finish(Err(e.to_string()));
            }
        }
    }

    fn open(&mut self, path: &Path) {
        match self.call(Request::Open { path: path.display().to_string() }) {
            Ok(_) => self.view.status = format!("Opened {}", path.display()),
            Err(e) => self.view.status = format!("Open failed: {e}"),
        }
    }

    #[expect(clippy::too_many_lines, reason = "one arm per action")]
    fn apply(&mut self, ctx: &Context, actions: Vec<Action>) {
        for action in actions {
            match action {
                Action::Run(id, params) => self.run(id, &params, None),
                Action::RunMerged(id, params, key) => self.run(id, &params, Some(key)),
                Action::Undo => {
                    if self.host.session_mut().undo().is_some() {
                        self.rev = self.rev.saturating_add(1);
                    }
                }
                Action::Redo => {
                    if self.host.session_mut().redo().is_some() {
                        self.rev = self.rev.saturating_add(1);
                    }
                }
                Action::Mode(mode) => self.set_mode(mode),
                Action::SelectTrack(track) => {
                    self.view.track = track.min(self.doc.rec.scene.tracks.len().saturating_sub(1));
                    self.rev = self.rev.saturating_add(1);
                }
                Action::SelectLayer(layer) => self.view.layer = layer,
                Action::Seek(seconds) => self.seek(seconds),
                Action::Play(on) => self.view.playing = on,
                Action::Mirror(on) => self.view.mirror = on,
                Action::Dark(dark) => {
                    self.view.dark = dark;
                    self.theme_dirty = true;
                }
                Action::Open(path) => self.open(&path),
                Action::Save(path) => {
                    let saved = self.call(Request::Save { path: Some(path.display().to_string()) });
                    self.view.status = match saved {
                        Ok(_) => format!("Saved {}", path.display()),
                        Err(e) => format!("Save failed: {e}"),
                    };
                }
                Action::New => {
                    if let Err(e) = self.call(Request::New {}) {
                        self.view.status = format!("New failed: {e}");
                    }
                }
                Action::NewMade(spec) => self.new_made(*spec),
                Action::Export(format, path, options) => {
                    if self.view.exporting.is_some() {
                        // One export at a time: this one waits its turn in the queue.
                        self.view.export.queue.push(format, path, options);
                        self.view.export.queue.running = true;
                    } else {
                        self.export(ctx, format, &path, &options);
                    }
                }
                Action::Enqueue(format, path, options) => {
                    self.view.export.queue.push(format, path, options);
                    self.view.export.queue_open = true;
                }
                Action::RunQueue => self.view.export.queue.running = true,
                Action::ClearQueue => {
                    self.view.export.queue.running = false;
                    self.view.export.queue.clear_finished();
                }
                Action::StartRecord => self.start_record(),
                Action::StopRecord => self.stop_record(),
                Action::LipsyncClip { track, audio_track, clip } => self.lipsync_clip(track, audio_track, clip),
                Action::Tracking(name, params) => {
                    self.view.status = match self.live.link().tool(name, &params) {
                        Ok(_) => name.to_owned(),
                        Err(e) => format!("{name}: {e}"),
                    };
                    self.last_status = Instant::now().checked_sub(Duration::from_secs(1)).unwrap_or_else(Instant::now);
                }
                Action::Mic(action) => {
                    if let Err(e) = self.live.link().mic(action) {
                        self.view.status = format!("microphone: {e}");
                    }
                    self.last_status = Instant::now().checked_sub(Duration::from_secs(1)).unwrap_or_else(Instant::now);
                }
                Action::StreamStart(spec) => {
                    self.view.status = match self.output.start(&spec) {
                        Ok(status) => format!("Streaming {}: {}", spec.kind.label(), status.detail),
                        Err(e) => format!("{} did not start: {e}", spec.kind.label()),
                    };
                    self.refresh_outputs();
                }
                Action::StreamStop => {
                    let _ = self.output.stop();
                    "Stream output stopped".clone_into(&mut self.view.status);
                    self.refresh_outputs();
                }
                Action::StreamRefresh => self.refresh_outputs(),
                Action::InputsRefresh => self.refresh_inputs(),
                Action::Trigger(key, on) => {
                    let key = key.to_lowercase();
                    if on {
                        self.view.stream.held.insert(key);
                    } else {
                        self.view.stream.held.remove(&key);
                    }
                }
                Action::Swap(set, member) => {
                    self.view.stream.swaps.insert(set, member);
                }
                Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
                Action::Workspace(action) => self.workspace_action(ctx, action),
            }
        }
    }

    /// The keyboard, with the chords from the preferences (see `workspace_ops`).
    fn shortcuts(&mut self, ctx: &Context) -> Vec<Action> {
        self.workspace_shortcuts(ctx)
    }

    // --- screenshots ------------------------------------------------------------------------------------------------

    fn screenshots(&mut self, ctx: &Context) {
        while let Some(shot) = self.host.hooks_mut().shots.pop_front() {
            let token = self.next_shot;
            self.next_shot = self.next_shot.saturating_add(1);
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(token)));
            self.awaiting.push((token, shot));
        }
        let mut arrived: Vec<(u64, [usize; 2], Vec<u8>)> = Vec::new();
        ctx.input(|input| {
            for event in &input.events {
                if let egui::Event::Screenshot { user_data, image, .. } = event {
                    let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied().unwrap_or(CLI_SHOT);
                    arrived.push((token, image.size, image.as_raw().to_vec()));
                }
            }
        });
        for (token, size, raw) in arrived {
            let (width, height) = (u32::try_from(size[0]).unwrap_or(1), u32::try_from(size[1]).unwrap_or(1));
            if token == CLI_SHOT {
                self.finish_cli_shot(ctx, width, height, &raw);
                continue;
            }
            let Some(at) = self.awaiting.iter().position(|(t, _)| *t == token) else { continue };
            let (_, pending) = self.awaiting.remove(at);
            let reply = Reply::value(json!({ "width": width, "height": height, "note": "the whole window; panel crops are not supported" }))
                .with_png_rgba(width, height, &raw);
            match reply {
                Ok(reply) => {
                    if let (Some(path), Some(image)) = (pending.request.save_to.as_ref(), reply.images.first())
                        && let Err(e) = anim_mcp::save_image(path, image)
                    {
                        pending.ticket.finish(Err(e));
                        continue;
                    }
                    pending.ticket.finish(Ok(reply));
                }
                Err(e) => pending.ticket.finish(Err(e)),
            }
        }
    }

    fn finish_cli_shot(&mut self, ctx: &Context, width: u32, height: u32, raw: &[u8]) {
        let Some(shot) = self.shot.take() else { return };
        let result = anim_mcp::encode_png_rgba(width, height, raw).map_err(|e| e.to_string()).and_then(|png| {
            if let Some(parent) = shot.out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            std::fs::write(&shot.out, png).map_err(|e| format!("{}: {e}", shot.out.display()))
        });
        let _ = self.done.send(result);
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Drives the command-line screenshot: wait until the picture and the live input have settled, then ask for it.
    fn drive_cli_shot(&mut self, ctx: &Context) {
        // A live demo face changes the pose every frame, so a render is always in flight: the newest finished
        // picture is as good as any then.
        let live = self.view.live.tracking || self.view.live.mic || self.view.live.demo_voice;
        let ready = (self.pending.is_none() || live) && self.texture.is_some() && self.last.is_some() && self.view.exporting.is_none();
        let Some(shot) = self.shot.as_mut() else { return };
        shot.settled = if ready { shot.settled.saturating_add(1) } else { 0 };
        if !shot.requested && shot.settled >= SETTLE_FRAMES && shot.started.elapsed() > Duration::from_millis(1500) {
            shot.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
        }
        if shot.started.elapsed() > SHOT_TIMEOUT {
            let why = format!(
                "pending render: {}, texture: {}, last pose: {}, exporting: {}, screenshot requested: {}, settled frames: {}",
                self.pending.is_some(),
                self.texture.is_some(),
                self.last.is_some(),
                self.view.exporting.is_some(),
                shot.requested,
                shot.settled
            );
            let _ = self.done.send(Err(format!("timed out waiting for the window to settle ({why})")));
            self.shot = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

impl eframe::App for CharCraftApp {
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        // The window is created transparent; only the transparent stream view lets that show.
        if self.view.transparent && self.view.mode == Mode::Stream { [0.0; 4] } else { visuals.panel_fill.to_normalized_gamma_f32() }
    }

    fn on_exit(&mut self) {
        self.finish_workspace();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.theme_dirty {
            self.theme_dirty = false;
            crate::ui::theme::apply(&ctx, self.view.dark);
        }
        self.begin_frame(&ctx);
        let mut actions = self.shortcuts(&ctx);
        let pose = self.perform(&ctx);
        self.pose.clone_from(&pose);
        self.request_render(&ctx, &pose);
        self.drive_thumbs();

        let frame = u32::try_from(self.doc.rec.scene.frame_at(Tick::from_seconds_f64(self.view.playhead)).max(0)).unwrap_or(0);
        let can_undo = self.host.session().history().can_undo();
        let can_redo = self.host.session().history().can_redo();
        let dirty = self.host.session().is_dirty();
        let time = self.started.elapsed().as_secs_f64();
        let key = self.host.hooks_mut().world.key_for_track(&self.doc.rec, self.view.track);
        let doc_path = self.host.session().path().map(Path::to_path_buf);
        let hooks = self.host.hooks_mut();
        let ctx_view = Ctx {
            doc: &self.doc,
            world: &hooks.world,
            texture: self.texture.as_ref().map(|(t, s)| (t, *s)),
            pose: &self.pose,
            stack: self.stack.as_ref(),
            can_undo,
            can_redo,
            dirty,
            time,
            windowed: true,
            puppet: key.as_deref(),
            frame,
            rows: self.ws.rows(),
            doc_path: doc_path.as_deref(),
            thumbs: &self.thumbs.textures,
        };
        actions.extend(crate::ui::draw(ui, &mut self.view, &ctx_view));
        self.apply(&ctx, actions);
        self.screenshots(&ctx);
        self.drive_cli_shot(&ctx);

        let busy = self.view.playing
            || self.view.recording.is_some()
            || self.pending.is_some()
            || self.view.live.tracking
            || self.view.live.mic
            || self.view.live.demo_voice
            || self.shot.is_some()
            || !self.awaiting.is_empty();
        if busy {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(120));
        }
    }
}
