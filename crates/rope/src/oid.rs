//! Object ids: 20-byte SHA-1 digests of git-style objects.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::{Digest, Sha1};

use crate::error::Error;
use crate::record::ObjectKind;

/// A 20-byte object id: `sha1("<type> <len>\0<data>")`, as in git.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Oid(pub [u8; 20]);

impl Oid {
    /// The id of the empty tree (`4b825dc6...`), which Character Animator uses for its first commit.
    pub const EMPTY_TREE: Self = Self([0x4b, 0x82, 0x5d, 0xc6, 0x42, 0xcb, 0x6e, 0xb9, 0xa0, 0x60, 0xe5, 0x4b, 0xf8, 0xd6, 0x92, 0x88, 0xfb, 0xee, 0x49, 0x04]);

    /// Computes the id an object of `kind` with content `data` has.
    #[must_use]
    pub fn of(kind: ObjectKind, data: &[u8]) -> Self {
        let mut hasher = Sha1::new();
        hasher.update(kind.name().as_bytes());
        hasher.update(b" ");
        hasher.update(data.len().to_string().as_bytes());
        hasher.update([0]);
        hasher.update(data);
        let digest = hasher.finalize();
        let mut out = [0_u8; 20];
        if digest.len() == out.len() {
            out.copy_from_slice(&digest);
        }
        Self(out)
    }

    /// Reads an id from the first 20 bytes of `bytes`.
    #[must_use]
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let head = bytes.get(..20)?;
        let mut out = [0_u8; 20];
        out.copy_from_slice(head);
        Some(Self(out))
    }

    /// Parses 40 hex digits (either case).
    ///
    /// # Errors
    ///
    /// [`Error::BadHex`] when `text` is not exactly 40 hex digits.
    pub fn from_hex(text: &str) -> Result<Self, Error> {
        let bad = || Error::BadHex(text.to_owned());
        let bytes = text.as_bytes();
        if bytes.len() != 40 {
            return Err(bad());
        }
        let mut out = [0_u8; 20];
        let (pairs, _) = bytes.as_chunks::<2>();
        for (slot, [high, low]) in out.iter_mut().zip(pairs) {
            let hi = char::from(*high).to_digit(16).ok_or_else(bad)?;
            let lo = char::from(*low).to_digit(16).ok_or_else(bad)?;
            *slot = u8::try_from(hi.saturating_mul(16).saturating_add(lo)).map_err(|_| bad())?;
        }
        Ok(Self(out))
    }

    /// The raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// The lowercase hex form.
    #[must_use]
    pub fn to_hex(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({self})")
    }
}

impl FromStr for Oid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        Self::from_hex(s)
    }
}

impl Serialize for Oid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Oid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_tree_and_blob_ids_match_git() {
        assert_eq!(Oid::of(ObjectKind::Tree, b""), Oid::EMPTY_TREE);
        assert_eq!(Oid::of(ObjectKind::Blob, b"").to_hex(), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(Oid::of(ObjectKind::Blob, b"hello\n").to_hex(), "ce013625030ba8dba906f756967f9e9ca394464a");
    }

    #[test]
    fn hex_round_trips() {
        let id = Oid::of(ObjectKind::Blob, b"x");
        assert_eq!(Oid::from_hex(&id.to_hex()).ok(), Some(id));
        assert_eq!(Oid::from_hex(&id.to_hex().to_uppercase()).ok(), Some(id));
        assert!(Oid::from_hex("zz").is_err());
        assert!(Oid::from_hex(&"g".repeat(40)).is_err());
    }
}
