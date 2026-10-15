//! Typed data to an [`LValue`]: the writing half of the serde bridge.
//!
//! Structs and maps become tables with string (or number) keys, sequences become positional entries, enums become a
//! string (unit variants) or a one-entry table `{ Variant = payload }`. Fields that are `None` or `()` are left out,
//! as Lua has no stored `nil`. Floats print the way Character Animator prints them (`0.2` as
//! `0.20000000000000001`, `100.0` as `100`).

use serde::ser::{self, Serialize};

use crate::error::{Error, Result};
use crate::number::Number;
use crate::value::{Key, LValue};

/// Converts any serializable value to a document, keeping field and map order.
///
/// # Errors
///
/// [`Error::Custom`] if the value has map keys that are not strings, numbers or booleans, or if its `Serialize`
/// implementation fails.
pub fn to_lvalue<T: Serialize + ?Sized>(value: &T) -> Result<LValue> {
    value.serialize(Serializer)
}

/// Serializes `value` to text in Character Animator's layout, with the keys of every table sorted the way it sorts
/// them (see [`LValue::sort_keys`]) so output is stable whatever the field or map order.
///
/// # Errors
///
/// As [`to_lvalue`].
pub fn to_string<T: Serialize + ?Sized>(value: &T) -> Result<String> {
    let mut document = to_lvalue(value)?;
    document.sort_keys();
    Ok(crate::to_string_pretty(&document))
}

/// An integer literal too large for `i64`: a float carrying its digits as text, like the parser makes.
#[must_use]
pub fn big_number(digits: String) -> Number {
    Number::Float { value: digits.parse::<f64>().unwrap_or(f64::INFINITY), text: digits }
}

/// [`big_number`] as a value.
#[must_use]
pub fn big_integer(digits: String) -> LValue {
    LValue::from(big_number(digits))
}

struct Serializer;

impl ser::Serializer for Serializer {
    type Ok = LValue;
    type Error = Error;
    type SerializeSeq = SeqSerializer;
    type SerializeTuple = SeqSerializer;
    type SerializeTupleStruct = SeqSerializer;
    type SerializeTupleVariant = VariantSeqSerializer;
    type SerializeMap = MapSerializer;
    type SerializeStruct = StructSerializer;
    type SerializeStructVariant = VariantStructSerializer;

    fn serialize_bool(self, value: bool) -> Result<LValue> {
        Ok(LValue::Bool(value))
    }

    fn serialize_i8(self, value: i8) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_i16(self, value: i16) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_i32(self, value: i32) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_i64(self, value: i64) -> Result<LValue> {
        Ok(LValue::Int(value))
    }

    fn serialize_i128(self, value: i128) -> Result<LValue> {
        Ok(i64::try_from(value).map_or_else(|_| big_integer(value.to_string()), LValue::Int))
    }

    fn serialize_u8(self, value: u8) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_u16(self, value: u16) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_u32(self, value: u32) -> Result<LValue> {
        Ok(LValue::Int(i64::from(value)))
    }

    fn serialize_u64(self, value: u64) -> Result<LValue> {
        Ok(i64::try_from(value).map_or_else(|_| big_integer(value.to_string()), LValue::Int))
    }

    fn serialize_u128(self, value: u128) -> Result<LValue> {
        Ok(i64::try_from(value).map_or_else(|_| big_integer(value.to_string()), LValue::Int))
    }

    fn serialize_f32(self, value: f32) -> Result<LValue> {
        // The shortest text that reads back as the same f32 ("0.1"), not the noisy f64 expansion of it.
        Ok(LValue::from_number_text(&value.to_string()).unwrap_or_else(|| LValue::float(f64::from(value))))
    }

    fn serialize_f64(self, value: f64) -> Result<LValue> {
        Ok(LValue::float(value))
    }

    fn serialize_char(self, value: char) -> Result<LValue> {
        Ok(LValue::Str(value.to_string()))
    }

    fn serialize_str(self, value: &str) -> Result<LValue> {
        Ok(LValue::Str(value.to_owned()))
    }

    fn serialize_bytes(self, value: &[u8]) -> Result<LValue> {
        Ok(LValue::from_bytes(value.to_vec()))
    }

    fn serialize_none(self) -> Result<LValue> {
        Ok(LValue::Nil)
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<LValue> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<LValue> {
        Ok(LValue::Nil)
    }

    fn serialize_unit_struct(self, _name: &'static str) -> Result<LValue> {
        Ok(LValue::Nil)
    }

    fn serialize_unit_variant(self, _name: &'static str, _index: u32, variant: &'static str) -> Result<LValue> {
        Ok(LValue::Str(variant.to_owned()))
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(self, _name: &'static str, value: &T) -> Result<LValue> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(self, _name: &'static str, _index: u32, variant: &'static str, value: &T) -> Result<LValue> {
        Ok(LValue::Table(vec![(Key::Str(variant.to_owned()), value.serialize(self)?)]))
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqSerializer> {
        Ok(SeqSerializer::with_capacity(len))
    }

    fn serialize_tuple(self, len: usize) -> Result<SeqSerializer> {
        Ok(SeqSerializer::with_capacity(Some(len)))
    }

    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqSerializer> {
        Ok(SeqSerializer::with_capacity(Some(len)))
    }

    fn serialize_tuple_variant(self, _name: &'static str, _index: u32, variant: &'static str, len: usize) -> Result<VariantSeqSerializer> {
        Ok(VariantSeqSerializer { variant, items: SeqSerializer::with_capacity(Some(len)) })
    }

    fn serialize_map(self, len: Option<usize>) -> Result<MapSerializer> {
        Ok(MapSerializer { entries: Vec::with_capacity(preallocation(len)), pending_key: None })
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<StructSerializer> {
        Ok(StructSerializer { entries: Vec::with_capacity(len.min(MAX_PREALLOCATION)) })
    }

    fn serialize_struct_variant(self, _name: &'static str, _index: u32, variant: &'static str, len: usize) -> Result<VariantStructSerializer> {
        Ok(VariantStructSerializer { variant, fields: StructSerializer { entries: Vec::with_capacity(len.min(MAX_PREALLOCATION)) } })
    }
}

const MAX_PREALLOCATION: usize = 4096;

fn preallocation(len: Option<usize>) -> usize {
    len.unwrap_or(0).min(MAX_PREALLOCATION)
}

/// Positional entries.
struct SeqSerializer {
    items: Vec<(Key, LValue)>,
}

impl SeqSerializer {
    fn with_capacity(len: Option<usize>) -> Self {
        Self { items: Vec::with_capacity(preallocation(len)) }
    }

    fn push<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.items.push((Key::Pos, value.serialize(Serializer)?));
        Ok(())
    }
}

impl ser::SerializeSeq for SeqSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(self.items))
    }
}

impl ser::SerializeTuple for SeqSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(self.items))
    }
}

impl ser::SerializeTupleStruct for SeqSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.push(value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(self.items))
    }
}

/// `{ Variant = { positional... } }`.
struct VariantSeqSerializer {
    variant: &'static str,
    items: SeqSerializer,
}

impl ser::SerializeTupleVariant for VariantSeqSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        self.items.push(value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(vec![(Key::Str(self.variant.to_owned()), LValue::Table(self.items.items))]))
    }
}

/// Entries with serialized keys; entries whose value is `nil` are dropped.
struct MapSerializer {
    entries: Vec<(Key, LValue)>,
    pending_key: Option<Key>,
}

fn key_from_value(key: LValue) -> Result<Key> {
    match key {
        LValue::Str(text) => Ok(Key::Str(text)),
        LValue::Int(int) => Ok(Key::Int(int)),
        LValue::Num { value, text } => Ok(Key::Num { value, text }),
        LValue::Bool(flag) => Ok(Key::Bool(flag)),
        other => Err(Error::Custom(format!("map keys must be strings, numbers or booleans, not a {}", other.type_name()))),
    }
}

impl ser::SerializeMap for MapSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<()> {
        self.pending_key = Some(key_from_value(key.serialize(Serializer)?)?);
        Ok(())
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<()> {
        let key = self.pending_key.take().ok_or_else(|| Error::Custom("map value serialized before its key".to_owned()))?;
        let value = value.serialize(Serializer)?;
        if !value.is_nil() {
            self.entries.push((key, value));
        }
        Ok(())
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(self.entries))
    }
}

/// Named fields; fields whose value is `nil` are dropped.
struct StructSerializer {
    entries: Vec<(Key, LValue)>,
}

impl StructSerializer {
    fn push<T: Serialize + ?Sized>(&mut self, name: &'static str, value: &T) -> Result<()> {
        let value = value.serialize(Serializer)?;
        if !value.is_nil() {
            self.entries.push((Key::Str(name.to_owned()), value));
        }
        Ok(())
    }
}

impl ser::SerializeStruct for StructSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<()> {
        self.push(key, value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(self.entries))
    }
}

/// `{ Variant = { fields... } }`.
struct VariantStructSerializer {
    variant: &'static str,
    fields: StructSerializer,
}

impl ser::SerializeStructVariant for VariantStructSerializer {
    type Ok = LValue;
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, key: &'static str, value: &T) -> Result<()> {
        self.fields.push(key, value)
    }

    fn end(self) -> Result<LValue> {
        Ok(LValue::Table(vec![(Key::Str(self.variant.to_owned()), LValue::Table(self.fields.entries))]))
    }
}

// ---- LValue and Key as serde types ----------------------------------------------------------------------------------

/// Serializes a document as plain serde data: `nil` as unit, tables whose keys are all positional as sequences,
/// other tables as maps (number and boolean keys are written as their text, positional entries in a mixed table as
/// their index) so the result also works for `serde_json`. Number text beyond what an `f64` holds is not kept.
impl Serialize for LValue {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        use ser::{SerializeMap, SerializeSeq};

        match self {
            Self::Nil => serializer.serialize_unit(),
            Self::Bool(flag) => serializer.serialize_bool(*flag),
            Self::Int(int) => serializer.serialize_i64(*int),
            Self::Num { value, .. } => serializer.serialize_f64(*value),
            Self::Str(text) => serializer.serialize_str(text),
            Self::Bytes(bytes) => serializer.serialize_bytes(bytes),
            Self::Table(entries) if entries.iter().all(|(key, _)| matches!(key, Key::Pos)) => {
                let mut seq = serializer.serialize_seq(Some(entries.len()))?;
                for (_, value) in entries {
                    seq.serialize_element(value)?;
                }
                seq.end()
            }
            Self::Table(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                let mut next_positional = 1_i64;
                for (key, value) in entries {
                    match key {
                        Key::Str(name) => map.serialize_entry(name, value)?,
                        Key::Int(int) => map.serialize_entry(int, value)?,
                        Key::Num { text, .. } => map.serialize_entry(text, value)?,
                        Key::Bool(flag) => map.serialize_entry(if *flag { "true" } else { "false" }, value)?,
                        Key::Pos => {
                            map.serialize_entry(&next_positional, value)?;
                            next_positional = next_positional.saturating_add(1);
                        }
                    }
                }
                map.end()
            }
        }
    }
}
