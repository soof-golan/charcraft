//! Helpers shared by the integration tests: PNG decoding, synthesized speech and a small MP4 demuxer.
#![allow(dead_code, reason = "each test binary uses a different part")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    reason = "test code"
)]

use std::path::{Path, PathBuf};

/// A decoded RGBA8 image.
pub struct Pixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Pixels {
    /// How many pixels differ visibly from `other` (same size).
    pub fn differing(&self, other: &Self) -> usize {
        assert_eq!((self.width, self.height), (other.width, other.height), "sizes differ");
        self.rgba.chunks(4).zip(other.rgba.chunks(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 8)).count()
    }

    /// How many pixels are not (nearly) the colour of the top-left corner.
    pub fn drawn(&self) -> usize {
        let corner = &self.rgba[..4];
        self.rgba.chunks(4).filter(|p| p.iter().zip(corner).any(|(x, y)| x.abs_diff(*y) > 8)).count()
    }
}

/// Decodes PNG bytes into RGBA8.
pub fn decode_png(bytes: &[u8]) -> Pixels {
    let png = anim_testkit::decode_png(bytes).unwrap();
    Pixels { width: png.width, height: png.height, rgba: png.rgba }
}

pub fn read_png(path: &Path) -> Pixels {
    decode_png(&std::fs::read(path).unwrap())
}

/// A mono 16-bit WAV file.
pub fn wav_bytes(rate: u32, samples: &[i16]) -> Vec<u8> {
    let data_len = (samples.len() * 2) as u32;
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2_u16.to_le_bytes());
    out.extend_from_slice(&16_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Half a second of silence, a second of a vowel-like buzz, half a second of hiss: the mouth has to move.
pub fn speechlike_wav(dir: &Path) -> PathBuf {
    let rate = 16_000_u32;
    let mut samples = vec![0_i16; 8_000];
    for i in 0..16_000_u32 {
        let t = f64::from(i) / f64::from(rate);
        let voiced: f64 = (1..=20).map(|h| (std::f64::consts::TAU * 120.0 * f64::from(h) * t).sin() / f64::from(h)).sum();
        samples.push((voiced * 6_000.0) as i16);
    }
    let mut state = 7_u32;
    for _ in 0..8_000 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        samples.push(((i32::try_from(state >> 16).unwrap() - 32_768) / 3) as i16);
    }
    let path = dir.join("hello there.wav");
    std::fs::write(&path, wav_bytes(rate, &samples)).unwrap();
    path
}

/// What the demuxer found in one track of an MP4 or MOV file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    /// `vide` or `soun`.
    pub handler: String,
    /// Samples (frames for video).
    pub samples: u32,
    /// Picture size (0 for sound).
    pub width: u32,
    pub height: u32,
    /// Ticks per second and length in ticks.
    pub timescale: u32,
    pub duration: u64,
    /// The sample entry (`avc1`, `mp4a`, `ap4h`, ...).
    pub codec: String,
}

impl Track {
    pub fn seconds(&self) -> f64 {
        self.duration as f64 / f64::from(self.timescale.max(1))
    }
}

/// Splits a run of ISO-BMFF boxes into `(type, payload)`.
fn boxes(mut data: &[u8]) -> Vec<(String, &[u8])> {
    let mut out = Vec::new();
    while data.len() >= 8 {
        let size = u32::from_be_bytes(data[..4].try_into().unwrap()) as usize;
        let kind = String::from_utf8_lossy(&data[4..8]).into_owned();
        let (header, total) = match size {
            0 => (8, data.len()),
            1 => (16, u64::from_be_bytes(data[8..16].try_into().unwrap()) as usize),
            n => (8, n),
        };
        assert!(total >= header && total <= data.len(), "box {kind:?} claims {total} bytes of {}", data.len());
        out.push((kind, &data[header..total]));
        data = &data[total..];
    }
    assert!(data.is_empty(), "{} stray bytes after the last box", data.len());
    out
}

fn child<'a>(parent: &'a [u8], kind: &str) -> Option<&'a [u8]> {
    boxes(parent).into_iter().find(|(k, _)| k == kind).map(|(_, payload)| payload)
}

fn be32(data: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(data[at..at + 4].try_into().unwrap())
}

/// Reads the tracks of an MP4 or MOV file: handler, sample count, picture size, duration and codec.
pub fn demux(bytes: &[u8]) -> Vec<Track> {
    let top = boxes(bytes);
    assert_eq!(top.first().map(|(k, _)| k.as_str()), Some("ftyp"), "an ISO-BMFF file starts with ftyp");
    let moov = top.iter().find(|(k, _)| k == "moov").map(|(_, p)| *p).expect("a moov box");
    boxes(moov)
        .into_iter()
        .filter(|(k, _)| k == "trak")
        .map(|(_, trak)| {
            let tkhd = child(trak, "tkhd").unwrap();
            let version = tkhd[0];
            let size_at = if version == 1 { 88 } else { 76 };
            let (width, height) = (be32(tkhd, size_at) >> 16, be32(tkhd, size_at + 4) >> 16);
            let mdia = child(trak, "mdia").unwrap();
            let mdhd = child(mdia, "mdhd").unwrap();
            let (timescale, duration) =
                if mdhd[0] == 1 { (be32(mdhd, 20), u64::from_be_bytes(mdhd[24..32].try_into().unwrap())) } else { (be32(mdhd, 12), u64::from(be32(mdhd, 16))) };
            let hdlr = child(mdia, "hdlr").unwrap();
            let handler = String::from_utf8_lossy(&hdlr[8..12]).into_owned();
            let stbl = child(child(mdia, "minf").unwrap(), "stbl").unwrap();
            let stsz = child(stbl, "stsz").unwrap();
            let fixed = be32(stsz, 4);
            let count = be32(stsz, 8);
            if fixed == 0 {
                assert!(stsz.len() >= 12 + count as usize * 4, "stsz is cut short");
            }
            let entries = child(stbl, "stsd").unwrap();
            let codec = String::from_utf8_lossy(&entries[12..16]).into_owned();
            Track { handler, samples: count, width, height, timescale, duration, codec }
        })
        .collect()
}
