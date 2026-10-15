//! Typed data from an [`LValue`]: the reading half of the serde bridge.
//!
//! Tables become structs, maps, sequences or enums depending on what the target type asks for:
//!
//! - a struct or map reads the string keys (and integer keys, with positional entries numbered from 1);
//! - a `Vec`, tuple or array reads a Lua sequence (keys `1..=n`, positional or explicit, in any order);
//! - an enum reads a string (unit variant) or a one-entry table `{ Variant = payload }`;
//! - `nil` is `None`/`()`; an `Option` field that is absent is `None`.
//!
//! Integers accept whole-number floats (`5.0`, `1e3`), floats accept integers, and map keys written as strings are
//! parsed when the map is keyed by integers. Repeated keys are not merged: serde reports a duplicate field.

use std::fmt;
use std::vec;

use serde::de::{self, DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::forward_to_deserialize_any;

use crate::error::{Error, Result};
use crate::value::{Key, LValue};

/// Deserializes a `T` from a parsed document.
///
/// # Errors
///
/// [`Error::Custom`] with serde's message when the document does not have the shape `T` expects.
pub fn from_lvalue<'de, T: de::Deserialize<'de>>(value: &'de LValue) -> Result<T> {
    T::deserialize(value)
}

/// Parses `text` and deserializes a `T` from it.
///
/// # Errors
///
/// A syntax error from [`parse`](crate::parse), or [`Error::Custom`] when the document does not fit `T`.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T> {
    let value = crate::parse(text)?;
    from_lvalue(&value)
}

fn expected(what: &'static str, found: &LValue) -> Error {
    Error::Custom(format!("expected {what}, found {}", found.type_name()))
}

/// Visits a sequence and rejects tables that hold more elements than the visitor consumed.
fn visit_sequence<'de, V: Visitor<'de>>(items: Vec<&'de LValue>, visitor: V) -> Result<V::Value> {
    let len = items.len();
    let mut access = SeqDe { items: items.into_iter() };
    let value = visitor.visit_seq(&mut access)?;
    if access.items.as_slice().is_empty() { Ok(value) } else { Err(<Error as de::Error>::invalid_length(len, &"a shorter sequence")) }
}

macro_rules! integers {
    ($($method:ident)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
            match self.as_i64() {
                Some(int) => visitor.visit_i64(int),
                None => self.deserialize_any(visitor),
            }
        }
    )*};
}

impl<'de> de::Deserializer<'de> for &'de LValue {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self {
            LValue::Nil => visitor.visit_unit(),
            LValue::Bool(flag) => visitor.visit_bool(*flag),
            LValue::Int(int) => visitor.visit_i64(*int),
            LValue::Num { value, .. } => visitor.visit_f64(*value),
            LValue::Str(text) => visitor.visit_borrowed_str(text),
            LValue::Bytes(bytes) => visitor.visit_borrowed_bytes(bytes),
            LValue::Table(entries) if !entries.is_empty() && entries.iter().all(|(key, _)| matches!(key, Key::Pos)) => {
                visit_sequence(entries.iter().map(|(_, value)| value).collect(), visitor)
            }
            LValue::Table(entries) => visitor.visit_map(MapDe::new(entries)),
        }
    }

    integers!(deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_u8 deserialize_u16 deserialize_u32);

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        // Integers above i64::MAX are kept as numbers with their digits as text; read them back losslessly.
        if let LValue::Num { text, .. } = self
            && let Ok(big) = text.parse::<u64>()
        {
            return visitor.visit_u64(big);
        }
        match self.as_i64() {
            Some(int) => visitor.visit_i64(int),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if self.is_nil() { visitor.visit_none() } else { visitor.visit_some(self) }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        if !matches!(self, LValue::Table(_)) {
            return Err(expected("a table", self));
        }
        let items = self.sequence().ok_or_else(|| Error::Custom("expected a sequence: a table with keys 1..n and nothing else".to_owned()))?;
        visit_sequence(items, visitor)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(self, _name: &'static str, _len: usize, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self {
            LValue::Table(entries) => visitor.visit_map(MapDe::new(entries)),
            other => Err(expected("a table", other)),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(self, _name: &'static str, _fields: &'static [&'static str], visitor: V) -> Result<V::Value> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value> {
        match self {
            LValue::Str(variant) => visitor.visit_enum(variant.as_str().into_deserializer()),
            LValue::Table(entries) => match entries.as_slice() {
                [(Key::Str(variant), payload)] => visitor.visit_enum(EnumDe { variant, payload }),
                _ => Err(Error::Custom("expected an enum: a string, or a table with one `Variant = payload` entry".to_owned())),
            },
            other => Err(expected("an enum (a string or a one-entry table)", other)),
        }
    }

    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self.as_bytes() {
            Some(bytes) => visitor.visit_borrowed_bytes(bytes),
            None => self.deserialize_any(visitor),
        }
    }

    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_bytes(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    forward_to_deserialize_any! { bool f32 f64 i128 u128 char str string unit unit_struct identifier }
}

// ---- sequences and maps -------------------------------------------------------------------------------------------

struct SeqDe<'de> {
    items: vec::IntoIter<&'de LValue>,
}

impl<'de> SeqAccess<'de> for SeqDe<'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.items.next().map(|value| seed.deserialize(value)).transpose()
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.items.len())
    }
}

struct MapDe<'de> {
    entries: std::slice::Iter<'de, (Key, LValue)>,
    next_positional: i64,
    value: Option<&'de LValue>,
}

impl<'de> MapDe<'de> {
    fn new(entries: &'de [(Key, LValue)]) -> Self {
        Self { entries: entries.iter(), next_positional: 1, value: None }
    }
}

impl<'de> MapAccess<'de> for MapDe<'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        let key = match key {
            Key::Str(name) => KeyDe::Str(name),
            Key::Int(int) => KeyDe::Int(*int),
            Key::Num { value, .. } => KeyDe::Num(*value),
            Key::Bool(flag) => KeyDe::Bool(*flag),
            Key::Pos => {
                let index = self.next_positional;
                self.next_positional = index.saturating_add(1);
                KeyDe::Int(index)
            }
        };
        seed.deserialize(key).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        let value = self.value.take().ok_or_else(|| Error::Custom("map value requested before its key".to_owned()))?;
        seed.deserialize(value)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.entries.len())
    }
}

// ---- keys ---------------------------------------------------------------------------------------------------------

/// A table key presented to serde as a string, integer, float or boolean.
#[derive(Clone, Copy)]
enum KeyDe<'de> {
    Str(&'de str),
    Int(i64),
    Num(f64),
    Bool(bool),
}

macro_rules! key_integers {
    ($($method:ident)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
            match self {
                // A map keyed by integers still reads keys written as strings ("12").
                Self::Str(text) => match text.parse::<i64>() {
                    Ok(int) => visitor.visit_i64(int),
                    Err(_) => visitor.visit_borrowed_str(text),
                },
                other => other.deserialize_any(visitor),
            }
        }
    )*};
}

impl<'de> de::Deserializer<'de> for KeyDe<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        match self {
            Self::Str(text) => visitor.visit_borrowed_str(text),
            Self::Int(int) => visitor.visit_i64(int),
            Self::Num(value) => visitor.visit_f64(value),
            Self::Bool(flag) => visitor.visit_bool(flag),
        }
    }

    key_integers!(deserialize_i8 deserialize_i16 deserialize_i32 deserialize_i64 deserialize_u8 deserialize_u16 deserialize_u32 deserialize_u64);

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_some(self)
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(self, _name: &'static str, visitor: V) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_enum<V: Visitor<'de>>(self, _name: &'static str, _variants: &'static [&'static str], visitor: V) -> Result<V::Value> {
        match self {
            Self::Str(variant) => visitor.visit_enum(variant.into_deserializer()),
            _ => Err(Error::Custom("an enum used as a map key must be a string".to_owned())),
        }
    }

    forward_to_deserialize_any! { bool f32 f64 i128 u128 char str string bytes byte_buf unit unit_struct seq tuple tuple_struct map struct identifier ignored_any }
}

// ---- enums --------------------------------------------------------------------------------------------------------

struct EnumDe<'de> {
    variant: &'de str,
    payload: &'de LValue,
}

impl<'de> EnumAccess<'de> for EnumDe<'de> {
    type Error = Error;
    type Variant = VariantDe<'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        let variant = seed.deserialize(KeyDe::Str(self.variant))?;
        Ok((variant, VariantDe { payload: self.payload }))
    }
}

struct VariantDe<'de> {
    payload: &'de LValue,
}

impl<'de> VariantAccess<'de> for VariantDe<'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        match self.payload {
            LValue::Nil => Ok(()),
            LValue::Table(entries) if entries.is_empty() => Ok(()),
            other => Err(expected("nothing (nil or an empty table) for a unit variant", other)),
        }
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        seed.deserialize(self.payload)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        de::Deserializer::deserialize_seq(self.payload, visitor)
    }

    fn struct_variant<V: Visitor<'de>>(self, _fields: &'static [&'static str], visitor: V) -> Result<V::Value> {
        de::Deserializer::deserialize_map(self.payload, visitor)
    }
}

// ---- LValue and Key as serde types ----------------------------------------------------------------------------------

/// Reads any serde value into a Lua value. Numbers are rebuilt the way Character Animator prints them (so a
/// document read through serde keeps its number text when Character Animator wrote it); sequences become positional
/// entries. Round-tripping through `serde_json` or `from_lvalue` is lossless for documents Character Animator wrote.
impl<'de> de::Deserialize<'de> for LValue {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(LValueVisitor)
    }
}

/// Reads a serde map key (string, integer, float or boolean) into a table key.
impl<'de> de::Deserialize<'de> for Key {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> std::result::Result<Self, D::Error> {
        deserializer.deserialize_any(KeyVisitor)
    }
}

/// Largest capacity reserved from a size hint; hints from hostile input are not trusted.
const MAX_PREALLOCATION: usize = 4096;

struct LValueVisitor;

impl<'de> Visitor<'de> for LValueVisitor {
    type Value = LValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("any Lua value")
    }

    fn visit_unit<E: de::Error>(self) -> std::result::Result<LValue, E> {
        Ok(LValue::Nil)
    }

    fn visit_none<E: de::Error>(self) -> std::result::Result<LValue, E> {
        Ok(LValue::Nil)
    }

    fn visit_some<D: de::Deserializer<'de>>(self, deserializer: D) -> std::result::Result<LValue, D::Error> {
        de::Deserialize::deserialize(deserializer)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<LValue, E> {
        Ok(LValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<LValue, E> {
        Ok(LValue::Int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<LValue, E> {
        Ok(i64::try_from(value).map_or_else(|_| crate::ser::big_integer(value.to_string()), LValue::Int))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<LValue, E> {
        Ok(LValue::float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<LValue, E> {
        Ok(LValue::Str(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<LValue, E> {
        Ok(LValue::Str(value))
    }

    fn visit_bytes<E: de::Error>(self, value: &[u8]) -> std::result::Result<LValue, E> {
        Ok(LValue::from_bytes(value.to_vec()))
    }

    fn visit_byte_buf<E: de::Error>(self, value: Vec<u8>) -> std::result::Result<LValue, E> {
        Ok(LValue::from_bytes(value))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<LValue, A::Error> {
        let mut entries = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(MAX_PREALLOCATION));
        while let Some(value) = seq.next_element::<LValue>()? {
            entries.push((Key::Pos, value));
        }
        Ok(LValue::Table(entries))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<LValue, A::Error> {
        let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0).min(MAX_PREALLOCATION));
        while let Some((key, value)) = map.next_entry::<Key, LValue>()? {
            entries.push((key, value));
        }
        Ok(LValue::Table(entries))
    }
}

struct KeyVisitor;

impl Visitor<'_> for KeyVisitor {
    type Value = Key;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a string, number or boolean table key")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> std::result::Result<Key, E> {
        Ok(Key::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> std::result::Result<Key, E> {
        Ok(Key::Int(value))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> std::result::Result<Key, E> {
        Ok(i64::try_from(value).map_or_else(|_| Key::from(crate::ser::big_number(value.to_string())), Key::Int))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> std::result::Result<Key, E> {
        Ok(Key::float(value))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> std::result::Result<Key, E> {
        Ok(Key::Str(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> std::result::Result<Key, E> {
        Ok(Key::Str(value))
    }
}
