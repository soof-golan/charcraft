//! The event graph: what the chunks of an `.arf` file mean.
//!
//! ```text
//! EGRF
//!   VERS  VMAJ u32 (2)  VMIN u32 (0)
//!   FrmS                                   the recording cut into frames
//!     FrmL                                 one frame
//!       FrmA  str start  str end           seconds, as text
//!       FrCL                               the frame's curves
//!         FCIL  FCIA u32 channel  FCTA u8 kind (1)  CrvA u64 n, n segments of 8 f64le
//!   FtrL
//!     StTA str  EdTA str                   when the recording started and stopped
//!     FDTA u64 u64                         (1, 4) in every sample
//!     RSpA str                             "1" in every sample
//!   FrPL  FrPA u64 index  u64 offset ...   byte offset of each FrmL from the file start
//!   ChnL  (ChkA str name  ChkV u32 channel) ...
//! ```
//!
//! A `str` is a `u64be` byte count and the bytes. Counts and lengths are big-endian, the doubles little-endian.

use std::collections::BTreeMap;

use crate::channel::Role;
use crate::chunk::{self, Chunk};
use crate::curve::{Curve, Segment};
use crate::decimal::format_time;
use crate::error::{Error, Result};

/// The seconds one frame covers when the recorder cuts a recording.
pub const FRAME_SECONDS: f64 = 0.25;
/// The curve kind of every sample file: a run of cubic Bezier segments.
pub const KIND_BEZIER: u8 = 1;
/// Most frames read.
pub const MAX_FRAMES: usize = 4_000_000;

/// A channel of the table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelEntry {
    /// The path (`Viseme/Index`).
    pub name: String,
    /// The number frames refer to it by.
    pub index: u32,
}

/// A channel's curve inside one frame.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameCurve {
    /// The channel (a [`ChannelEntry::index`]).
    pub channel: u32,
    /// The curve kind ([`KIND_BEZIER`]).
    pub kind: u8,
    /// The segments inside this frame; none when the channel did not change.
    pub segments: Vec<Segment>,
}

/// A stretch of the recording.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// When the frame starts, seconds.
    pub start: f64,
    /// When it ends.
    pub end: f64,
    /// The curves, one per channel recorded in it.
    pub curves: Vec<FrameCurve>,
}

/// A decoded `.arf` file.
#[derive(Debug, Clone, PartialEq)]
pub struct EventGraph {
    /// `VMAJ` and `VMIN`.
    pub version: (u32, u32),
    /// The frames, in time order.
    pub frames: Vec<Frame>,
    /// When the recording started (`StTA`). Live recordings use the application clock, transcript recordings 0.
    pub start: f64,
    /// When it stopped (`EdTA`).
    pub end: f64,
    /// `FDTA`, (1, 4) in every sample.
    pub fdta: (u64, u64),
    /// `RSpA`, "1" in every sample.
    pub rspa: String,
    /// The channel table, in the order it is stored (a hash order, not sorted).
    pub channels: Vec<ChannelEntry>,
}

/// One channel with its whole curve.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// The number frames use.
    pub index: u32,
    /// The path.
    pub name: String,
    /// What the path says it is.
    pub role: Role,
    /// The curve over the whole recording, in the recording's own clock.
    pub curve: Curve,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n).ok_or_else(|| Error::structure("a chunk payload is shorter than its fields"))?;
        self.0 = tail;
        Ok(head)
    }

    fn u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self.take(8)?.try_into().map_err(|_| Error::structure("short integer"))?;
        Ok(u64::from_be_bytes(bytes))
    }

    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().map_err(|_| Error::structure("short integer"))?;
        Ok(u32::from_be_bytes(bytes))
    }

    fn string(&mut self) -> Result<String> {
        let len = usize::try_from(self.u64()?).map_err(|_| Error::Limit("string length"))?;
        String::from_utf8(self.take(len)?.to_vec()).map_err(|_| Error::structure("a string is not UTF-8"))
    }

    fn time(&mut self) -> Result<f64> {
        let text = self.string()?;
        text.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or(Error::BadNumber(text))
    }

    fn finish(&self) -> Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(Error::structure("a chunk payload is longer than its fields")) }
    }
}

fn put_u64(out: &mut Vec<u8>, v: u64) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn put_string(out: &mut Vec<u8>, s: &str) {
    put_u64(out, u64::try_from(s.len()).unwrap_or(u64::MAX));
    out.extend_from_slice(s.as_bytes());
}

#[allow(clippy::trivially_copy_pass_by_ref, reason = "tags are written as byte-string literals")]
fn want<'a>(children: &'a [Chunk], at: usize, tag: &[u8; 4]) -> Result<&'a Chunk> {
    children.get(at).filter(|c| c.tag() == tag).ok_or_else(|| Error::structure(format!("expected a {} chunk at position {at}", String::from_utf8_lossy(tag))))
}

fn payload(chunk: &Chunk) -> Result<Reader<'_>> {
    chunk.payload().map(Reader).ok_or_else(|| Error::structure(format!("{} should hold data", String::from_utf8_lossy(chunk.tag()))))
}

fn frame_curve(chunk: &Chunk) -> Result<FrameCurve> {
    let parts = chunk.children();
    let mut id = payload(want(parts, 0, b"FCIA")?)?;
    let channel = id.u32()?;
    id.finish()?;
    let mut kind = payload(want(parts, 1, b"FCTA")?)?;
    let kind_byte = *kind.take(1)?.first().ok_or_else(|| Error::structure("empty curve kind"))?;
    kind.finish()?;
    if kind_byte != KIND_BEZIER {
        return Err(Error::structure(format!("curve kind {kind_byte} is not known")));
    }
    let mut data = payload(want(parts, 2, b"CrvA")?)?;
    let count = usize::try_from(data.u64()?).map_err(|_| Error::Limit("segment count"))?;
    let expect = count.checked_mul(64).ok_or(Error::Limit("segment count"))?;
    if data.0.len() != expect {
        return Err(Error::structure("a curve's size does not match its segment count"));
    }
    let mut segments = Vec::with_capacity(count);
    for _ in 0..count {
        let mut t = [0.0; 4];
        let mut v = [0.0; 4];
        for (ti, vi) in t.iter_mut().zip(&mut v) {
            let a: [u8; 8] = data.take(8)?.try_into().map_err(|_| Error::structure("short double"))?;
            let b: [u8; 8] = data.take(8)?.try_into().map_err(|_| Error::structure("short double"))?;
            *ti = f64::from_le_bytes(a);
            *vi = f64::from_le_bytes(b);
        }
        segments.push(Segment { t, v });
    }
    if parts.len() != 3 {
        return Err(Error::structure("a frame curve has extra chunks"));
    }
    Ok(FrameCurve { channel, kind: kind_byte, segments })
}

fn curve_chunk(curve: &FrameCurve) -> Chunk {
    let mut data = Vec::with_capacity(curve.segments.len().saturating_mul(64).saturating_add(8));
    put_u64(&mut data, u64::try_from(curve.segments.len()).unwrap_or(u64::MAX));
    for s in &curve.segments {
        for (t, v) in s.t.iter().zip(&s.v) {
            data.extend_from_slice(&t.to_le_bytes());
            data.extend_from_slice(&v.to_le_bytes());
        }
    }
    Chunk::group(b"FCIL", vec![Chunk::data(b"FCIA", curve.channel.to_be_bytes().to_vec()), Chunk::data(b"FCTA", vec![curve.kind]), Chunk::data(b"CrvA", data)])
}

fn frame_chunk(frame: &Frame) -> Chunk {
    let mut times = Vec::new();
    put_string(&mut times, &format_time(frame.start));
    put_string(&mut times, &format_time(frame.end));
    Chunk::group(b"FrmL", vec![Chunk::data(b"FrmA", times), Chunk::group(b"FrCL", frame.curves.iter().map(curve_chunk).collect())])
}

fn frame_of(chunk: &Chunk) -> Result<Frame> {
    let parts = chunk.children();
    let mut times = payload(want(parts, 0, b"FrmA")?)?;
    let start = times.time()?;
    let end = times.time()?;
    times.finish()?;
    let list = want(parts, 1, b"FrCL")?;
    if parts.len() != 2 {
        return Err(Error::structure("a frame has extra chunks"));
    }
    let curves = list
        .children()
        .iter()
        .map(|c| if c.tag() == b"FCIL" { frame_curve(c) } else { Err(Error::structure("a frame's curve list holds something else")) })
        .collect::<Result<Vec<_>>>()?;
    Ok(Frame { start, end, curves })
}

impl EventGraph {
    /// Reads an `.arf` file.
    ///
    /// # Errors
    /// [`Error`] for damaged or unknown structure.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let top = chunk::parse(bytes)?;
        let [root] = top.as_slice() else { return Err(Error::structure("a file holds exactly one EGRF chunk")) };
        if root.tag() != b"EGRF" || root.payload().is_some() {
            return Err(Error::structure("the file does not start with EGRF"));
        }
        let parts = root.children();
        if parts.len() != 5 {
            return Err(Error::structure("EGRF should hold VERS, FrmS, FtrL, FrPL and ChnL"));
        }
        let version = want(parts, 0, b"VERS")?;
        let mut major = payload(want(version.children(), 0, b"VMAJ")?)?;
        let mut minor = payload(want(version.children(), 1, b"VMIN")?)?;
        let version = (major.u32()?, minor.u32()?);
        major.finish()?;
        minor.finish()?;

        let frame_list = want(parts, 1, b"FrmS")?.children();
        if frame_list.len() > MAX_FRAMES {
            return Err(Error::Limit("frame count"));
        }
        let frames = frame_list
            .iter()
            .map(|c| if c.tag() == b"FrmL" { frame_of(c) } else { Err(Error::structure("FrmS holds something besides frames")) })
            .collect::<Result<Vec<_>>>()?;

        let footer = want(parts, 2, b"FtrL")?.children();
        let mut start = payload(want(footer, 0, b"StTA")?)?;
        let mut end = payload(want(footer, 1, b"EdTA")?)?;
        let mut fdta = payload(want(footer, 2, b"FDTA")?)?;
        let mut rspa = payload(want(footer, 3, b"RSpA")?)?;
        if footer.len() != 4 {
            return Err(Error::structure("FtrL has extra chunks"));
        }
        let (start, end) = (start.time()?, end.time()?);
        let fdta_values = (fdta.u64()?, fdta.u64()?);
        let rspa_text = rspa.string()?;
        fdta.finish()?;
        rspa.finish()?;

        let offsets = want(parts, 3, b"FrPL")?.children();
        if offsets.len() != frames.len() {
            return Err(Error::structure("FrPL does not list every frame"));
        }
        for (i, c) in offsets.iter().enumerate() {
            let mut r = payload(c)?;
            let index = r.u64()?;
            r.u64()?;
            r.finish()?;
            if c.tag() != b"FrPA" || usize::try_from(index) != Ok(i) {
                return Err(Error::structure("FrPL is not the list of frame offsets in order"));
            }
        }

        let table = want(parts, 4, b"ChnL")?.children();
        if table.len() % 2 != 0 {
            return Err(Error::structure("ChnL should hold pairs of ChkA and ChkV"));
        }
        let mut channels = Vec::with_capacity(table.len() / 2);
        for pair in table.as_chunks::<2>().0 {
            let mut name = payload(want(pair, 0, b"ChkA")?)?;
            let mut index = payload(want(pair, 1, b"ChkV")?)?;
            let entry = ChannelEntry { name: name.string()?, index: index.u32()? };
            name.finish()?;
            index.finish()?;
            channels.push(entry);
        }
        Ok(Self { version, frames, start, end, fdta: fdta_values, rspa: rspa_text, channels })
    }

    /// The chunks of the file.
    #[must_use]
    pub fn to_chunk(&self) -> Chunk {
        let mut major = Vec::new();
        major.extend_from_slice(&self.version.0.to_be_bytes());
        let mut minor = Vec::new();
        minor.extend_from_slice(&self.version.1.to_be_bytes());
        let version = Chunk::group(b"VERS", vec![Chunk::data(b"VMAJ", major), Chunk::data(b"VMIN", minor)]);

        let frame_chunks: Vec<Chunk> = self.frames.iter().map(frame_chunk).collect();
        // The first frame starts after the EGRF header, VERS and the FrmS header.
        let mut offset = 16_usize.saturating_add(version.encoded_len()).saturating_add(16);
        let mut offsets = Vec::with_capacity(frame_chunks.len());
        for (i, chunk) in frame_chunks.iter().enumerate() {
            let mut entry = Vec::with_capacity(16);
            put_u64(&mut entry, u64::try_from(i).unwrap_or(u64::MAX));
            put_u64(&mut entry, u64::try_from(offset).unwrap_or(u64::MAX));
            offsets.push(Chunk::data(b"FrPA", entry));
            offset = offset.saturating_add(chunk.encoded_len());
        }

        let string_chunk = |tag: &[u8; 4], text: &str| {
            let mut data = Vec::new();
            put_string(&mut data, text);
            Chunk::data(tag, data)
        };
        let mut fdta = Vec::new();
        put_u64(&mut fdta, self.fdta.0);
        put_u64(&mut fdta, self.fdta.1);
        let footer = Chunk::group(
            b"FtrL",
            vec![
                string_chunk(b"StTA", &format_time(self.start)),
                string_chunk(b"EdTA", &format_time(self.end)),
                Chunk::data(b"FDTA", fdta),
                string_chunk(b"RSpA", &self.rspa),
            ],
        );

        let mut table = Vec::with_capacity(self.channels.len().saturating_mul(2));
        for entry in &self.channels {
            table.push(string_chunk(b"ChkA", &entry.name));
            table.push(Chunk::data(b"ChkV", entry.index.to_be_bytes().to_vec()));
        }
        Chunk::group(b"EGRF", vec![version, Chunk::group(b"FrmS", frame_chunks), footer, Chunk::group(b"FrPL", offsets), Chunk::group(b"ChnL", table)])
    }

    /// The bytes of the file.
    ///
    /// Times are written the way the recorder writes them (`%.36g`), so a file the recorder wrote comes back byte for
    /// byte; a file spelled differently (`0.1` for a time) comes back with the canonical spelling.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        chunk::write(&[self.to_chunk()])
    }

    /// Every channel with its curve joined across frames, sorted by channel number.
    #[must_use]
    pub fn tracks(&self) -> Vec<Track> {
        let mut curves: BTreeMap<u32, Vec<Segment>> = BTreeMap::new();
        for frame in &self.frames {
            for c in &frame.curves {
                curves.entry(c.channel).or_default().extend_from_slice(&c.segments);
            }
        }
        let mut tracks: Vec<Track> = self
            .channels
            .iter()
            .map(|e| Track { index: e.index, name: e.name.clone(), role: Role::parse(&e.name), curve: Curve::new(curves.remove(&e.index).unwrap_or_default()) })
            .collect();
        tracks.sort_by_key(|t| t.index);
        tracks
    }

    /// Cuts channels into frames the way the recorder does: windows of [`FRAME_SECONDS`] from `start`, the last one
    /// ending at `end`, plus one empty-length frame when `end` falls on a window edge.
    ///
    /// `channels` pairs each path with its curve; channel numbers follow the order given.
    #[must_use]
    pub fn from_curves(start: f64, end: f64, channels: &[(String, Curve)]) -> Self {
        let mut frames = Vec::new();
        let mut k = 0_u32;
        loop {
            let window_start = start + f64::from(k) * FRAME_SECONDS;
            if window_start > end || frames.len() >= MAX_FRAMES {
                break;
            }
            let window_end = (window_start + FRAME_SECONDS).min(end);
            let closed = window_end >= end;
            let curves = (0_u32..)
                .zip(channels)
                .map(|(channel, (_, curve))| FrameCurve { channel, kind: KIND_BEZIER, segments: curve.slice(window_start, window_end, closed) })
                .collect();
            frames.push(Frame { start: window_start, end: window_end, curves });
            k = k.saturating_add(1);
        }
        let entries = (0_u32..).zip(channels).map(|(index, (name, _))| ChannelEntry { name: name.clone(), index }).collect();
        Self { version: (2, 0), frames, start, end, fdta: (1, 4), rspa: "1".to_owned(), channels: entries }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny() -> EventGraph {
        let amplitude = Curve::from_samples(&[(10.0, 0.0), (10.4, 1.0), (10.9, 0.25)], false);
        let index = Curve::from_samples(&[(10.0, -2.0), (10.5, 3.0), (10.9, 3.0)], true);
        EventGraph::from_curves(10.0, 10.9, &[("Audio/Amplitude".to_owned(), amplitude), ("Viseme/Index".to_owned(), index)])
    }

    #[test]
    fn a_graph_round_trips() {
        let g = tiny();
        assert_eq!(g.frames.len(), 4, "10.0-10.25, 10.25-10.5, 10.5-10.75, 10.75-10.9");
        let bytes = g.to_bytes();
        let back = EventGraph::from_bytes(&bytes).unwrap();
        assert_eq!(back, g);
        assert_eq!(back.to_bytes(), bytes);
    }

    #[test]
    fn tracks_join_the_frames() {
        let g = tiny();
        let tracks = EventGraph::from_bytes(&g.to_bytes()).unwrap().tracks();
        assert_eq!(tracks.len(), 2);
        let amp = tracks.iter().find(|t| t.role == Role::AudioAmplitude).unwrap();
        for (t, v) in [(10.0, 0.0), (10.2, 0.5), (10.4, 1.0), (10.65, 0.625), (10.9, 0.25)] {
            let got = amp.curve.value_at(t).unwrap();
            assert!((got - v).abs() < 1e-6, "{t}: {got} vs {v}");
        }
        let idx = tracks.iter().find(|t| t.role == Role::VisemeIndex).unwrap();
        assert_eq!(idx.curve.value_at(10.2), Some(-2.0));
        assert_eq!(idx.curve.value_at(10.5), Some(3.0));
    }

    #[test]
    fn an_edge_on_the_window_adds_a_zero_length_frame() {
        let g = EventGraph::from_curves(0.0, 0.5, &[("Viseme/Index".to_owned(), Curve::from_samples(&[(0.0, 1.0), (0.5, 1.0)], true))]);
        assert_eq!(g.frames.len(), 3);
        assert_eq!((g.frames[2].start, g.frames[2].end), (0.5, 0.5));
    }

    #[test]
    fn structure_errors_are_named() {
        let mut g = tiny();
        g.frames.pop();
        let broken = g.to_chunk();
        assert!(EventGraph::from_bytes(&chunk::write(&[broken])).is_ok(), "FrPL is rewritten to match");
        assert!(EventGraph::from_bytes(b"").is_err());
        assert!(EventGraph::from_bytes(&chunk::write(&[Chunk::group(b"EGRF", vec![])])).is_err());
    }

    #[test]
    fn hostile_bytes_never_panic() {
        let bytes = tiny().to_bytes();
        for i in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[i] = damaged[i].wrapping_add(0x37);
            let _ = EventGraph::from_bytes(&damaged);
        }
        for cut in 0..bytes.len() {
            let _ = EventGraph::from_bytes(&bytes[..cut]);
        }
    }
}
