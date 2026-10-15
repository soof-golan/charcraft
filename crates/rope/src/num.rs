//! The rope's variable-length integers.
//!
//! Forward form (lengths, sizes): a first byte of `0` or of `9..=255` is the value itself; a first byte `n` of
//! `1..=8` is followed by `n` big-endian bytes holding the value. The writer always picks the shortest form, so a
//! length of 4 is `01 04` and one of 0x142 is `02 01 42`. The reader accepts longer spellings too.
//!
//! Backward form (the record trailer): a single byte `9..=255` is the value; otherwise the value is stored as
//! `n` little-endian bytes followed by the byte `n`, so the last byte of a record says how to read it from the end.

/// Reads a forward number at `*pos` and advances past it.
pub fn read_num(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let first = *buf.get(*pos)?;
    if first == 0 || first > 8 {
        *pos = pos.checked_add(1)?;
        return Some(u64::from(first));
    }
    let count = usize::from(first);
    let start = pos.checked_add(1)?;
    let end = start.checked_add(count)?;
    let bytes = buf.get(start..end)?;
    let mut wide = [0_u8; 8];
    wide.get_mut(8_usize.checked_sub(count)?..)?.copy_from_slice(bytes);
    *pos = end;
    Some(u64::from_be_bytes(wide))
}

/// Number of significant bytes of a non-zero value.
fn significant_bytes(value: u64) -> usize {
    let zeros = usize::try_from(value.leading_zeros() / 8).unwrap_or(7);
    8_usize.saturating_sub(zeros).max(1)
}

/// Appends the shortest forward encoding of `value`.
pub fn write_num(out: &mut Vec<u8>, value: u64) {
    if let Ok(byte) = u8::try_from(value)
        && (byte == 0 || byte > 8)
    {
        out.push(byte);
        return;
    }
    let count = significant_bytes(value);
    let bytes = value.to_be_bytes();
    if let Some(tail) = bytes.get(8_usize.saturating_sub(count)..) {
        out.push(u8::try_from(count).unwrap_or(8));
        out.extend_from_slice(tail);
    }
}

/// Appends the backward (trailer) encoding of `value`.
pub fn write_trailer(out: &mut Vec<u8>, value: u64) {
    if let Ok(byte) = u8::try_from(value)
        && byte > 8
    {
        out.push(byte);
        return;
    }
    let count = significant_bytes(value);
    let bytes = value.to_le_bytes();
    if let Some(head) = bytes.get(..count) {
        out.extend_from_slice(head);
        out.push(u8::try_from(count).unwrap_or(8));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc(value: u64) -> Vec<u8> {
        let mut out = Vec::new();
        write_num(&mut out, value);
        out
    }

    #[test]
    fn forward_forms_match_observed_bytes() {
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(4), [0x01, 0x04]);
        assert_eq!(enc(15), [0x0f]);
        assert_eq!(enc(162), [0xa2]);
        assert_eq!(enc(255), [0xff]);
        assert_eq!(enc(256), [0x02, 0x01, 0x00]);
        assert_eq!(enc(0x142), [0x02, 0x01, 0x42]);
    }

    #[test]
    fn forward_round_trips() {
        for value in [0, 1, 8, 9, 255, 256, 65_535, 65_536, u64::from(u32::MAX), u64::MAX] {
            let bytes = enc(value);
            let mut pos = 0;
            assert_eq!(read_num(&bytes, &mut pos), Some(value));
            assert_eq!(pos, bytes.len());
        }
    }

    #[test]
    fn long_spellings_are_accepted() {
        let mut pos = 0;
        assert_eq!(read_num(&[0x02, 0x00, 0x0f], &mut pos), Some(15));
        assert_eq!(pos, 3);
    }

    #[test]
    fn truncated_numbers_are_none() {
        let mut pos = 0;
        assert_eq!(read_num(&[0x03, 0x01], &mut pos), None);
        assert_eq!(read_num(&[], &mut pos), None);
    }

    #[test]
    fn trailer_forms_match_observed_bytes() {
        let mut out = Vec::new();
        write_trailer(&mut out, 39);
        assert_eq!(out, [0x27]);
        out.clear();
        write_trailer(&mut out, 0x16f);
        assert_eq!(out, [0x6f, 0x01, 0x02]);
    }
}
