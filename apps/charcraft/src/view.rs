//! The window's own state (what is selected, where the playhead is, what the live input last said) and the
//! [`Action`]s the drawing code returns.
//!
//! The document lives in the session; this is everything else. Drawing is a function of `(&mut View, &Ctx)` that
//! returns actions; the app applies the actions to the session afterwards, so no widget changes the document
//! directly and everything a click can do is also a command an agent can send.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anim_lipsync::Viseme as VoiceViseme;
use anim_ui::{StageViewState, TimelineViewState};
use charcraft_behaviors::api::StackInfo;
use charcraft_puppet::PuppetPose;
use charcraft_record::Recorder;
use charcraft_tracking::body::BodyInput as TrackedBody;
use charcraft_tracking::input::FaceInput as TrackedFace;
use serde_json::Value;

use crate::export::{ExportUi, Format, Options};
use crate::live::MicAction;
use crate::projectio::PuppetRow;
use crate::session::AppDoc;
use crate::state::Mode;
use crate::workspace::{PanelState, Prefs, Starter};
use crate::world::World;

/// What the workspace menus and windows ask for. The app applies them (they change the preferences, the window or
/// the document's file, none of which the drawing code may touch).
#[derive(Debug, Clone)]
pub enum WorkspaceAction {
    /// Save: to the file the document has, or ask where.
    Save,
    /// Throw the changes away and read the document again.
    Revert,
    /// Show or hide the start workspace.
    Start(bool),
    /// The panels changed (open, closed, moved, resized).
    Panels(PanelState),
    /// Switch to a saved or built-in workspace.
    ApplyWorkspace(String),
    /// Save the current arrangement under a name.
    SaveWorkspace(String),
    /// Forget a saved workspace.
    DeleteWorkspace(String),
    /// New preferences.
    SetPrefs(Box<Prefs>),
    /// Forget the recent files.
    ClearRecent,
    /// Open an autosaved document.
    Recover(PathBuf),
    /// Delete an autosaved document.
    Discard(PathBuf),
    /// Start from a gallery tile.
    Starter(usize),
}

/// What the drawing code asks for.
#[derive(Debug, Clone)]
pub enum Action {
    /// Run a command of the registry.
    Run(&'static str, Value),
    /// Run a command that continues a drag or a typing run, folded into one undo step by the key.
    RunMerged(&'static str, Value, &'static str),
    /// Undo.
    Undo,
    /// Redo.
    Redo,
    /// Switch mode.
    Mode(Mode),
    /// Select a puppet track.
    SelectTrack(usize),
    /// Select a layer of the selected puppet.
    SelectLayer(Option<String>),
    /// Move the playhead (seconds).
    Seek(f64),
    /// Start or stop playback.
    Play(bool),
    /// Mirror the performer.
    Mirror(bool),
    /// Dark or light theme.
    Dark(bool),
    /// Open a file (puppet, project, artwork or saved document).
    Open(PathBuf),
    /// Save to a path.
    Save(PathBuf),
    /// Start a fresh demo document.
    New,
    /// Start a fresh document holding one Puppet Maker puppet.
    NewMade(Box<charcraft_maker::Spec>),
    /// Export the scene now (after the running export when there is one).
    Export(Format, PathBuf, Options),
    /// Add an export to the queue.
    Enqueue(Format, PathBuf, Options),
    /// Run the queued exports one after another.
    RunQueue,
    /// Stop the queue after the running export and drop the finished items.
    ClearQueue,
    /// Start recording a take on the selected track.
    StartRecord,
    /// Stop recording and keep the take.
    StopRecord,
    /// Compute a lip sync take on puppet track `track` from clip `clip` of audio track `audio_track`.
    LipsyncClip {
        /// The puppet track that gets the take.
        track: usize,
        /// The audio track holding the clip.
        audio_track: usize,
        /// The clip.
        clip: usize,
    },
    /// A `tracking_*` tool.
    Tracking(&'static str, Value),
    /// A microphone action.
    Mic(MicAction),
    /// Start a stream output.
    StreamStart(Box<charcraft_stream::Spec>),
    /// Stop the stream output.
    StreamStop,
    /// Ask what each stream output needs on this machine.
    StreamRefresh,
    /// Ask for the cameras and microphones the system has.
    InputsRefresh,
    /// Hold a trigger key down or let go of it.
    Trigger(String, bool),
    /// Pick a swap set member.
    Swap(String, String),
    /// Close the window.
    Quit,
    /// Something about the workspace, preferences or the document's file.
    Workspace(WorkspaceAction),
}

/// Which file prompt is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Open a puppet, project or artwork.
    Open,
    /// Save the document; the kind is chosen in the prompt.
    Save,
    /// Import a sound file onto the first audio track at the playhead.
    Audio,
    /// Import a `.puppet` or project into the open project.
    ImportPuppet,
}

/// What Save As writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveKind {
    /// A `.charcraft` document (the scene, takes, rig edits and sound in one file).
    Document,
    /// A Character Animator project folder.
    Project,
    /// The selected puppet as a `.puppet` file.
    Puppet,
}

impl SaveKind {
    /// The chooser label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Document => ".charcraft document",
            Self::Project => "Project (.chproj)",
            Self::Puppet => "Puppet (.puppet)",
        }
    }

    /// The file name a path should end with.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Document => ".charcraft",
            Self::Project => ".chproj",
            Self::Puppet => ".puppet",
        }
    }
}

/// The Project panel's own state.
#[derive(Debug, Default)]
pub struct ProjectUi {
    /// The filter text.
    pub search: String,
    /// The puppet the properties section shows.
    pub selected: Option<String>,
    /// A rename in progress: the puppet and the text so far.
    pub renaming: Option<(String, String)>,
    /// A removal waiting for a click on Remove.
    pub confirm_remove: Option<String>,
}

/// Data for the start workspace that the app collects.
#[derive(Debug, Default)]
pub struct StartInfo {
    /// Autosaved documents with their age in seconds.
    pub recoverable: Vec<(PathBuf, u64)>,
    /// The gallery.
    pub starters: Vec<Starter>,
}

/// A window that floats over the mode view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    /// The import warnings list.
    Warnings,
    /// The Puppet Maker.
    Maker,
}

impl Dialog {
    /// Parses `warnings` or `maker` (any case).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_lowercase().as_str() {
            "warnings" => Some(Self::Warnings),
            "maker" => Some(Self::Maker),
            _ => None,
        }
    }
}

/// The Puppet Maker window's own state: the character being designed and the picture of it.
#[derive(Default)]
pub struct MakerState {
    /// The character being designed.
    pub spec: charcraft_maker::Spec,
    /// The picture of a character, with the spec it was drawn from.
    pub preview: Option<(charcraft_maker::Spec, egui::TextureHandle)>,
}

impl std::fmt::Debug for MakerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MakerState").field("spec", &self.spec).field("has_preview", &self.preview.is_some()).finish()
    }
}

/// A take being recorded.
#[derive(Debug)]
pub struct ActiveTake {
    /// Collects the armed groups.
    pub recorder: Recorder,
    /// The puppet track.
    pub track: usize,
    /// Scene time the take started at, seconds.
    pub start: f64,
    /// Whether microphone audio is being kept.
    pub audio: bool,
    /// Frames fed so far.
    pub frames: u32,
}

/// The texture of the camera preview (egui's handle has no `Debug`, which the view's derive needs).
#[derive(Clone)]
pub struct PreviewTexture(pub egui::TextureHandle);

impl std::fmt::Debug for PreviewTexture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreviewTexture").field("size", &self.0.size()).finish()
    }
}

impl std::ops::Deref for PreviewTexture {
    type Target = egui::TextureHandle;

    fn deref(&self) -> &egui::TextureHandle {
        &self.0
    }
}

/// What the live actor last reported.
#[derive(Debug, Clone)]
#[expect(clippy::struct_excessive_bools, reason = "independent flags of the live inputs")]
pub struct LiveSnapshot {
    /// The newest tracked face.
    pub face: Option<TrackedFace>,
    /// Faces received.
    pub faces: u64,
    /// The mouth shape the voice makes.
    pub viseme: Option<VoiceViseme>,
    /// The microphone level 0 to 1.
    pub level: f32,
    /// Whether the camera (or the demo face) runs.
    pub tracking: bool,
    /// Whether the microphone runs.
    pub mic: bool,
    /// The demo voice plays.
    pub demo_voice: bool,
    /// A line about the last problem.
    pub problem: Option<String>,
    /// What feeds the tracker while it runs: the camera's name, `file photo.jpg` or `demo face`.
    pub source: Option<String>,
    /// Calibration progress `(done, wanted)`.
    pub calibrating: Option<(usize, usize)>,
    /// The cameras the system reports: index and name.
    pub cameras: Vec<(u32, String)>,
    /// The camera list is the single scripted demo face (no camera support in this build).
    pub camera_demo: bool,
    /// A camera search is running ("looking for cameras...").
    pub cameras_searching: bool,
    /// Why the last camera search found nothing (a timeout, access denied).
    pub cameras_error: Option<String>,
    /// The camera chosen in the picker.
    pub camera_index: u32,
    /// The microphones the system reports.
    pub mic_devices: Vec<String>,
    /// The microphone chosen in the picker (`None` is the system default).
    pub mic_device: Option<String>,
    /// The microphone input sensitivity (gain).
    pub mic_gain: f32,
    /// The camera image of the live preview (absent for a scripted source).
    pub preview: Option<PreviewTexture>,
    /// The tracked landmark points over the preview, 0 to 1 across the image.
    pub preview_points: Vec<[f32; 2]>,
    /// Preview pictures received.
    pub previews: u64,
    /// The newest tracked body.
    pub body: Option<TrackedBody>,
    /// Bodies received.
    pub bodies: u64,
    /// Whether body tracking runs.
    pub body_tracking: bool,
    /// Whether the body's rest pose was captured from the performer.
    pub rest_pose_set: bool,
    /// Rest-pose capture progress `(done, wanted)`.
    pub setting_rest_pose: Option<(usize, usize)>,
    /// A line about the last body tracking problem.
    pub body_problem: Option<String>,
}

impl Default for LiveSnapshot {
    fn default() -> Self {
        Self {
            face: None,
            faces: 0,
            viseme: None,
            level: 0.0,
            tracking: false,
            mic: false,
            demo_voice: false,
            problem: None,
            source: None,
            calibrating: None,
            cameras: Vec::new(),
            camera_demo: false,
            cameras_searching: false,
            cameras_error: None,
            camera_index: 0,
            mic_devices: Vec::new(),
            mic_device: None,
            mic_gain: 1.0,
            preview: None,
            preview_points: Vec::new(),
            previews: 0,
            body: None,
            bodies: 0,
            body_tracking: false,
            rest_pose_set: false,
            setting_rest_pose: None,
            body_problem: None,
        }
    }
}

/// The stream output the window drives: what the Controls panel edits and what the output thread reports.
#[derive(Debug, Clone)]
pub struct StreamUi {
    /// The Controls, Triggers and scene panels are open.
    pub panels: bool,
    /// What to start.
    pub spec: charcraft_stream::Spec,
    /// The newest report of the output thread.
    pub status: charcraft_stream::Status,
    /// What each output needs on this machine.
    pub outputs: Vec<charcraft_stream::OutputInfo>,
    /// Trigger keys held down by the Triggers panel.
    pub held: std::collections::BTreeSet<String>,
    /// Swap set members chosen in the Triggers panel (set name to layer label).
    pub swaps: BTreeMap<String, String>,
}

impl Default for StreamUi {
    fn default() -> Self {
        Self {
            panels: true,
            spec: charcraft_stream::Spec::default(),
            status: charcraft_stream::Status::default(),
            outputs: Vec::new(),
            held: std::collections::BTreeSet::new(),
            swaps: BTreeMap::new(),
        }
    }
}

/// A cached waveform: the (frame rate, length, frames) it was made for, and the peaks.
pub type PeakEntry = ((i64, i64, usize), Vec<f32>);

/// Everything the window remembers between frames.
#[derive(Debug)]
#[expect(clippy::struct_excessive_bools, reason = "independent view toggles")]
pub struct View {
    /// The working mode.
    pub mode: Mode,
    /// Dark theme.
    pub dark: bool,
    /// Selected puppet track.
    pub track: usize,
    /// Selected take of the selected track.
    pub take: usize,
    /// Selected layer path.
    pub layer: Option<String>,
    /// Playhead, seconds.
    pub playhead: f64,
    /// Playing.
    pub playing: bool,
    /// Loop at the end.
    pub looping: bool,
    /// The puppet mirrors the performer.
    pub mirror: bool,
    /// Show the live performance on the selected track while in Record mode.
    pub monitor: bool,
    /// The artwork view of the Rig mode.
    pub rig_stage: StageViewState,
    /// The scene view of the Record mode.
    pub scene_stage: StageViewState,
    /// The timeline's scroll and zoom.
    pub timeline: TimelineViewState,
    /// A mouth shape to preview when nothing live drives it.
    pub preview_viseme: Option<charcraft_puppet::Viseme>,
    /// Preview a closed eye pair.
    pub preview_blink: bool,
    /// Text of the new-tag field.
    pub new_tag: String,
    /// Text of the new-handle field.
    pub new_handle: String,
    /// The kind of handle the next one is.
    pub handle_kind: &'static str,
    /// The stream view shows a green screen (otherwise a dark backdrop).
    pub chroma: bool,
    /// The stream view shows its overlay chips.
    pub hud: bool,
    /// The stream view has no backdrop at all: the window and the pictures sent to outputs are transparent.
    pub transparent: bool,
    /// The stream output and the panels of the stream view.
    pub stream: StreamUi,
    /// The newest live input.
    pub live: LiveSnapshot,
    /// A line for the status bar.
    pub status: String,
    /// The take being recorded.
    pub recording: Option<ActiveTake>,
    /// The open file prompt and its text.
    pub prompt: Option<(Prompt, String)>,
    /// The export dialog and the export queue.
    pub export: ExportUi,
    /// A running export: progress and what it is doing.
    pub exporting: Option<(f32, String)>,
    /// The last export result or error, shown in the status bar.
    pub last_export: Option<String>,
    /// Peaks of sounds per scene frame, by media name (and the frame rate and length they were made for).
    pub peaks: BTreeMap<String, PeakEntry>,
    /// The pointer over the artwork (artwork pixels) and whether it is down.
    pub pointer: Option<(f64, f64, bool)>,
    /// Layers whose children are collapsed in the tree.
    pub collapsed: std::collections::BTreeSet<String>,
    /// The rig tools.
    pub rig: crate::ui::rigtools::RigUi,
    /// The preferences (a copy; the app owns the stored ones).
    pub prefs: Prefs,
    /// The side panels.
    pub panels: PanelState,
    /// The Project panel.
    pub project: ProjectUi,
    /// The start workspace is showing.
    pub start: bool,
    /// What the start workspace lists.
    pub start_info: StartInfo,
    /// The Preferences window is open.
    pub prefs_open: bool,
    /// The shortcut waiting for a key press in the Preferences window.
    pub capturing: Option<&'static str>,
    /// Text of the new-workspace field.
    pub new_workspace: String,
    /// What the Save prompt writes.
    pub save_kind: SaveKind,
    /// The floating window that is open.
    pub dialog: Option<Dialog>,
    /// The Puppet Maker window's state.
    pub maker: MakerState,
    /// Read the artwork again as soon as its file changes.
    pub auto_reimport: bool,
    /// The docked layouts of the workspaces.
    pub docks: crate::ui::dock::Docks,
    /// Go back to the saved layout after this frame (asked from a panel menu while the dock is drawn).
    pub reset_layout: bool,
    /// Close this panel after this frame (asked from a panel menu while the dock is drawn).
    pub panel_close: Option<anim_ui::dock::PanelId>,
    /// The Keyboard Shortcuts window is open.
    pub show_shortcuts: bool,
    /// The About window is open.
    pub show_about: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            mode: Mode::Rig,
            dark: true,
            track: 0,
            take: 0,
            layer: None,
            playhead: 0.0,
            playing: false,
            looping: true,
            mirror: true,
            monitor: true,
            rig_stage: StageViewState::new(egui::vec2(640.0, 720.0)),
            scene_stage: StageViewState::new(egui::vec2(1280.0, 720.0)),
            timeline: TimelineViewState::default(),
            preview_viseme: None,
            preview_blink: false,
            new_tag: String::new(),
            new_handle: "Handle".to_owned(),
            handle_kind: "draggable",
            chroma: false,
            hud: true,
            transparent: false,
            stream: StreamUi::default(),
            live: LiveSnapshot::default(),
            status: String::new(),
            recording: None,
            prompt: None,
            export: ExportUi::default(),
            exporting: None,
            last_export: None,
            peaks: BTreeMap::new(),
            pointer: None,
            collapsed: std::collections::BTreeSet::new(),
            rig: crate::ui::rigtools::RigUi::default(),
            prefs: Prefs::default(),
            panels: PanelState::default(),
            project: ProjectUi::default(),
            start: false,
            start_info: StartInfo::default(),
            prefs_open: false,
            capturing: None,
            new_workspace: String::new(),
            save_kind: SaveKind::Document,
            dialog: None,
            maker: MakerState::default(),
            auto_reimport: false,
            docks: crate::ui::dock::Docks::new(false),
            reset_layout: false,
            panel_close: None,
            show_shortcuts: false,
            show_about: false,
        }
    }
}

/// What the drawing code may read (never change).
#[expect(clippy::struct_excessive_bools, reason = "independent flags handed to the drawing code")]
pub struct Ctx<'a> {
    /// The document.
    pub doc: &'a AppDoc,
    /// The puppets and sound.
    pub world: &'a World,
    /// The texture of the newest render, with its pixel size.
    pub texture: Option<(&'a egui::TextureHandle, [usize; 2])>,
    /// The pose the performed puppet has now.
    pub pose: &'a PuppetPose,
    /// The behaviors of the selected track.
    pub stack: Option<&'a StackInfo>,
    /// Whether undo and redo have something to do, and the label of the next undo.
    pub can_undo: bool,
    /// Whether redo has something to do.
    pub can_redo: bool,
    /// The document has unsaved changes.
    pub dirty: bool,
    /// The seconds since the app started (for blinking cursors and similar).
    pub time: f64,
    /// A window exists (screenshots).
    pub windowed: bool,
    /// The cast key of the selected puppet.
    pub puppet: Option<&'a str>,
    /// The scene's current frame (from the playhead).
    pub frame: u32,
    /// The puppets of the project, for the Project panel.
    pub rows: &'a [PuppetRow],
    /// The `.charcraft` file the document is saved to, when it has one.
    pub doc_path: Option<&'a std::path::Path>,
    /// Small pictures for the Project panel, by `puppet:<cast key>` and `scene:<index>`.
    pub thumbs: &'a BTreeMap<String, (egui::TextureHandle, [usize; 2])>,
}
