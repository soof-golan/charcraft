//! The chunk container: a tree of tagged, length-prefixed blocks.
//!
//! ```text
//! group   "< TAG >"  u64be length  children...  "</TAG >"
//! data    "[ TAG ]"  u64be length  payload  0xAC padding to a multiple of 8  "[/TAG ]"
//! ```
//!
//! A tag is four ASCII bytes. A group's length is the size of its children (header and trailer excluded). A data
//! chunk's length is the payload size without the padding. Reading and writing are exact inverses.

use crate::error::{Error, Result};

/// Four ASCII bytes naming a chunk.
pub type Tag = [u8; 4];

/// The byte that pads a data chunk to a multiple of eight.
pub const PAD: u8 = 0xAC;
/// Deepest nesting read (real files nest four deep).
pub const MAX_DEPTH: usize = 8;
/// Largest file read.
pub const MAX_BYTES: usize = 1 << 30;

const HEADER: usize = 16;
const TRAILER: usize = 8;

/// A chunk: a group of chunks or a block of bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Chunk {
    /// `< TAG >` with children.
    Group {
        /// The tag.
        tag: Tag,
        /// The chunks inside.
        children: Vec<Self>,
    },
    /// `[ TAG ]` with a payload.
    Data {
        /// The tag.
        tag: Tag,
        /// The payload, without padding.
        data: Vec<u8>,
    },
}

impl Chunk {
    /// A group.
    #[must_use]
    pub const fn group(tag: &Tag, children: Vec<Self>) -> Self {
        Self::Group { tag: *tag, children }
    }

    /// A data chunk.
    #[must_use]
    pub const fn data(tag: &Tag, data: Vec<u8>) -> Self {
        Self::Data { tag: *tag, data }
    }

    /// The tag.
    #[must_use]
    pub const fn tag(&self) -> &Tag {
        match self {
            Self::Group { tag, .. } | Self::Data { tag, .. } => tag,
        }
    }

    /// Size in bytes once written.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        match self {
            Self::Group { children, .. } => children.iter().fold(HEADER.saturating_add(TRAILER), |n, c| n.saturating_add(c.encoded_len())),
            Self::Data { data, .. } => HEADER.saturating_add(padded(data.len())).saturating_add(TRAILER),
        }
    }

    /// Appends the chunk's bytes.
    pub fn write(&self, out: &mut Vec<u8>) {
        match self {
            Self::Group { tag, children } => {
                let inner = children.iter().fold(0_usize, |n, c| n.saturating_add(c.encoded_len()));
                out.extend_from_slice(b"< ");
                out.extend_from_slice(tag);
                out.extend_from_slice(b" >");
                out.extend_from_slice(&length_bytes(inner));
                for child in children {
                    child.write(out);
                }
                out.extend_from_slice(b"</");
                out.extend_from_slice(tag);
                out.extend_from_slice(b" >");
            }
            Self::Data { tag, data } => {
                out.extend_from_slice(b"[ ");
                out.extend_from_slice(tag);
                out.extend_from_slice(b" ]");
                out.extend_from_slice(&length_bytes(data.len()));
                out.extend_from_slice(data);
                out.resize(out.len().saturating_add(padded(data.len()).saturating_sub(data.len())), PAD);
                out.extend_from_slice(b"[/");
                out.extend_from_slice(tag);
                out.extend_from_slice(b" ]");
            }
        }
    }

    /// The children of a group (empty for data).
    #[must_use]
    pub fn children(&self) -> &[Self] {
        match self {
            Self::Group { children, .. } => children,
            Self::Data { .. } => &[],
        }
    }

    /// The payload of a data chunk.
    #[must_use]
    pub fn payload(&self) -> Option<&[u8]> {
        match self {
            Self::Data { data, .. } => Some(data),
            Self::Group { .. } => None,
        }
    }
}

fn length_bytes(n: usize) -> [u8; 8] {
    u64::try_from(n).unwrap_or(u64::MAX).to_be_bytes()
}

/// `n` rounded up to a multiple of eight.
const fn padded(n: usize) -> usize {
    n.saturating_add(7) & !7
}

/// Reads a whole file: a sequence of top-level chunks (a real file has exactly one, `EGRF`).
///
/// # Errors
/// [`Error`] for truncated or malformed data; nothing panics on hostile input.
pub fn parse(bytes: &[u8]) -> Result<Vec<Chunk>> {
    if bytes.len() > MAX_BYTES {
        return Err(Error::Limit("file size"));
    }
    parse_seq(bytes, 0, 0)
}

/// Writes top-level chunks.
#[must_use]
pub fn write(chunks: &[Chunk]) -> Vec<u8> {
    let mut out = Vec::with_capacity(chunks.iter().fold(0_usize, |n, c| n.saturating_add(c.encoded_len())));
    for chunk in chunks {
        chunk.write(&mut out);
    }
    out
}

fn parse_seq(bytes: &[u8], base: usize, depth: usize) -> Result<Vec<Chunk>> {
    let mut chunks = Vec::new();
    let mut at = 0_usize;
    while at < bytes.len() {
        let offset = base.saturating_add(at);
        let rest = bytes.get(at..).ok_or(Error::Truncated { offset })?;
        let head: [u8; HEADER] = rest.get(..HEADER).and_then(|h| h.try_into().ok()).ok_or(Error::Truncated { offset })?;
        let [open, sp1, t0, t1, t2, t3, sp2, close, l0, l1, l2, l3, l4, l5, l6, l7] = head;
        let tag: Tag = [t0, t1, t2, t3];
        let len = usize::try_from(u64::from_be_bytes([l0, l1, l2, l3, l4, l5, l6, l7])).map_err(|_| Error::Limit("chunk length"))?;
        if sp1 != b' ' || sp2 != b' ' || !tag.iter().all(u8::is_ascii_graphic) {
            return Err(Error::BadChunk { offset, reason: "header" });
        }
        match (open, close) {
            (b'<', b'>') => {
                if depth >= MAX_DEPTH {
                    return Err(Error::TooDeep { offset });
                }
                let end = HEADER.checked_add(len).ok_or(Error::Limit("chunk length"))?;
                let body = rest.get(HEADER..end).ok_or(Error::Truncated { offset })?;
                let trailer_end = end.checked_add(TRAILER).ok_or(Error::Limit("chunk length"))?;
                let trailer = rest.get(end..trailer_end).ok_or(Error::Truncated { offset })?;
                if trailer != [b'<', b'/', t0, t1, t2, t3, b' ', b'>'] {
                    return Err(Error::BadChunk { offset, reason: "group trailer" });
                }
                let children = parse_seq(body, base.saturating_add(at).saturating_add(HEADER), depth.saturating_add(1))?;
                chunks.push(Chunk::Group { tag, children });
                at = at.saturating_add(trailer_end);
            }
            (b'[', b']') => {
                let space = padded(len);
                let end = HEADER.checked_add(space).ok_or(Error::Limit("chunk length"))?;
                let body = rest.get(HEADER..end).ok_or(Error::Truncated { offset })?;
                let trailer_end = end.checked_add(TRAILER).ok_or(Error::Limit("chunk length"))?;
                let trailer = rest.get(end..trailer_end).ok_or(Error::Truncated { offset })?;
                if trailer != [b'[', b'/', t0, t1, t2, t3, b' ', b']'] {
                    return Err(Error::BadChunk { offset, reason: "data trailer" });
                }
                let data = body.get(..len).ok_or(Error::Truncated { offset })?;
                if !body.get(len..).is_some_and(|pad| pad.iter().all(|&b| b == PAD)) {
                    return Err(Error::BadChunk { offset, reason: "padding" });
                }
                chunks.push(Chunk::Data { tag, data: data.to_vec() });
                at = at.saturating_add(trailer_end);
            }
            _ => return Err(Error::BadChunk { offset, reason: "header" }),
        }
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Chunk> {
        vec![Chunk::group(
            b"TOPA",
            vec![Chunk::data(b"ONEA", vec![1, 2, 3]), Chunk::group(b"INNR", vec![]), Chunk::data(b"EMPT", vec![]), Chunk::data(b"EIGH", vec![9; 8])],
        )]
    }

    #[test]
    fn chunks_round_trip() {
        let bytes = write(&sample());
        assert_eq!(parse(&bytes).unwrap(), sample());
        assert_eq!(write(&parse(&bytes).unwrap()), bytes);
    }

    #[test]
    fn data_is_padded_with_0xac_to_eight() {
        let bytes = write(&[Chunk::data(b"ONEA", vec![1, 2, 3])]);
        assert_eq!(&bytes[..8], b"[ ONEA ]");
        assert_eq!(&bytes[8..16], &3_u64.to_be_bytes());
        assert_eq!(&bytes[16..24], &[1, 2, 3, PAD, PAD, PAD, PAD, PAD]);
        assert_eq!(&bytes[24..], b"[/ONEA ]");
        assert_eq!(bytes.len(), sample()[0].children()[0].encoded_len());
    }

    #[test]
    fn damage_is_an_error_not_a_panic() {
        let bytes = write(&sample());
        for cut in 0..bytes.len() {
            let _ = parse(&bytes[..cut]);
        }
        for i in 0..bytes.len() {
            let mut damaged = bytes.clone();
            damaged[i] ^= 0x5a;
            let _ = parse(&damaged);
        }
        let mut huge = write(&[Chunk::data(b"ONEA", vec![1])]);
        huge[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
        assert!(parse(&huge).is_err());
    }

    #[test]
    fn wrong_padding_is_rejected_because_it_would_not_round_trip() {
        let mut bytes = write(&[Chunk::data(b"ONEA", vec![1, 2, 3])]);
        bytes[20] = 0;
        assert_eq!(parse(&bytes), Err(Error::BadChunk { offset: 0, reason: "padding" }));
    }

    #[test]
    fn nesting_is_bounded() {
        let mut chunk = Chunk::group(b"LEAF", vec![]);
        for _ in 0..20 {
            chunk = Chunk::group(b"DEEP", vec![chunk]);
        }
        assert!(matches!(parse(&write(&[chunk])), Err(Error::TooDeep { .. })));
    }
}
