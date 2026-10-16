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
use charcraft_tracking::input::FaceInput as TrackedFace;
use serde_json::Value;

use crate::export::Format;
use crate::live::MicAction;
use crate::session::AppDoc;
use crate::state::Mode;
use crate::world::World;

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
    /// Export the scene.
    Export(Format, PathBuf),
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
}

/// Which file prompt is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// Open a puppet, project or artwork.
    Open,
    /// Save the document.
    Save,
    /// Export; the format is chosen in the prompt.
    Export,
    /// Import a sound file onto the first audio track at the playhead.
    Audio,
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
            calibrating: None,
            cameras: Vec::new(),
            camera_demo: false,
            camera_index: 0,
            mic_devices: Vec::new(),
            mic_device: None,
            mic_gain: 1.0,
            preview: None,
            preview_points: Vec::new(),
            previews: 0,
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
    /// The export format chosen in the prompt.
    pub export_format: Format,
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
            export_format: Format::Mp4,
            exporting: None,
            last_export: None,
            peaks: BTreeMap::new(),
            pointer: None,
            collapsed: std::collections::BTreeSet::new(),
            rig: crate::ui::rigtools::RigUi::default(),
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
}
