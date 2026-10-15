//! `lipsync`: speech audio to mouth shapes.

use std::path::Path;

use anim_audio::PcmBuffer;
use anim_lipsync::{VisemeKey, analyze};
use color_eyre::eyre::{Result, WrapErr};
use serde::{Deserialize, Serialize};

use crate::cli::LipsyncArgs;
use crate::frames::write_file;

/// One mouth shape from `start_ticks` (254 016 000 000 per second) until the next key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyOut {
    pub start_ticks: i64,
    pub time_seconds: f64,
    /// Character Animator's layer name for the shape (`Ah`, `W-Oo`, ...).
    pub viseme: String,
    /// The matching Animate lip-sync shape.
    pub animate: String,
}

/// The file `lipsync` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisemeFile {
    pub audio: String,
    pub sample_rate: u32,
    pub duration_seconds: f64,
    pub keys: Vec<KeyOut>,
}

/// Decodes an audio file.
pub fn decode(path: &Path) -> Result<PcmBuffer> {
    anim_audio::decode_path(path).wrap_err_with(|| format!("decoding {}", path.display()))
}

/// Mouth keys for a decoded buffer.
#[must_use]
pub fn keys_of(audio: &PcmBuffer) -> Vec<VisemeKey> {
    analyze(&audio.to_mono().samples, audio.sample_rate)
}

/// The JSON model of `keys`.
#[must_use]
pub fn file_of(audio_name: &str, audio: &PcmBuffer, keys: &[VisemeKey]) -> VisemeFile {
    VisemeFile {
        audio: audio_name.to_owned(),
        sample_rate: audio.sample_rate,
        duration_seconds: audio.seconds(),
        keys: keys
            .iter()
            .map(|k| KeyOut {
                start_ticks: k.start.0,
                time_seconds: k.start.seconds(),
                viseme: k.viseme.ca_layer_name().to_owned(),
                animate: k.viseme.to_animate().label().to_owned(),
            })
            .collect(),
    }
}

/// Runs the command.
pub fn run(args: &LipsyncArgs) -> Result<()> {
    let audio = decode(&args.audio)?;
    let keys = keys_of(&audio);
    let name = args.audio.file_name().and_then(|n| n.to_str()).unwrap_or("audio");
    let file = file_of(name, &audio, &keys);
    write_file(&args.output, serde_json::to_string_pretty(&file)?.as_bytes())?;
    println!("{}: {} keys over {:.1} s", args.output.display(), file.keys.len(), file.duration_seconds);
    Ok(())
}
