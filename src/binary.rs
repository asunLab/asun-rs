//! ASON Binary Format (ASON-BIN)
//!
//! A high-performance binary encoding for ASON data structures.
//! Provides `encode_binary` and `decode_binary` for zero-overhead struct ↔ bytes conversion.
//!
//! ## Wire Format (all integers little-endian)
//!
//! ```text
//! bool      → 1 byte  (0x00=false, 0x01=true)
//! i8        → 1 byte (signed)
//! i16       → 2 bytes LE
//! i32       → 4 bytes LE
//! i64       → 8 bytes LE
//! u8        → 1 byte
//! u16       → 2 bytes LE
//! u32       → 4 bytes LE
//! u64       → 8 bytes LE
//! f32       → 4 bytes LE (IEEE 754 bit-cast)
//! f64       → 8 bytes LE (IEEE 754 bit-cast)
//! char      → 4 bytes LE (Unicode scalar as u32)
//! str       → u32 LE length + UTF-8 bytes  ← ZERO-COPY on decode (&'de str)
//! bytes     → u32 LE length + raw bytes
//! Option<T> → u8 tag (0=None, 1=Some) + [T payload if Some]
//! Vec<T>    → u32 LE count + [element × count]
//! Map       → u32 LE count + [(key, value) × count]
//! struct    → fields in declaration order (no length prefix — known from schema)
//! tuple     → elements in order (no length prefix)
//! enum      → u32 LE variant_index + [payload for non-unit variants]
//! unit      → 0 bytes
//! newtype   → inner value directly (no wrapper)
//! ```
//!
//! ## Key Features
//!
//! - **Zero-copy string decode**: borrowed `&'de str` slices directly reference input bytes.
//! - **No type tags** for struct fields: schema drives layout (like Protobuf binary, not CBOR).
//! - **SIMD-accelerated** bulk byte copy for large string payloads (≥ 32 bytes).
//! - All primitives written/read via `to_le_bytes` / `from_le_bytes` — compiler typically
//!   emits a single `STR`/`LDR` instruction on aarch64 / `MOV`/`MOV` on x86-64.

use core::mem;
use crate::error::{Error, Result};
use crate::simd;
use serde::de::{self, DeserializeSeed, EnumAccess, MapAccess, SeqAccess, VariantAccess, Visitor};
use serde::ser::{
    self, SerializeMap, SerializeSeq, SerializeStruct, SerializeStructVariant, SerializeTuple,
    SerializeTupleStruct, SerializeTupleVariant,
};
use serde::{Deserialize, Serialize};

// ============================================================================
// Public API
// ============================================================================

/// Serialize `value` to a `Vec<u8>` using the ASON binary format.
///
/// # Example
/// ```rust,ignore
/// let user = User { id: 1, name: "Alice".into(), active: true };
/// let bytes = ason::encode_binary(&user)?;
/// ```
#[inline]
pub fn encode_binary<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut ser = BinarySerializer::with_capacity(256);
    value.serialize(&mut ser)?;
    Ok(ser.buf)
}

/// Deserialize a value from ASON binary bytes.
///
/// The lifetime `'de` allows **zero-copy** deserialization: any `&'de str` fields
/// in the target type will borrow directly from `data` with no allocation.
///
/// # Example
/// ```rust,ignore
/// let user: User = ason::decode_binary(&bytes)?;
/// ```
#[inline]
pub fn decode_binary<'de, T: Deserialize<'de>>(data: &'de [u8]) -> Result<T> {
    let mut de = BinaryDeserializer::new(data);
    let v = T::deserialize(&mut de)?;
    Ok(v)
}

// ============================================================================
// BinarySerializer
// ============================================================================

pub struct BinarySerializer {
    pub(crate) buf: Vec<u8>,
}

impl BinarySerializer {
    #[inline]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self { buf: Vec::with_capacity(cap) }
    }

    // ------------------------------------------------------------------
    // Primitive writers — each emits fixed bytes, zero heap allocation
    // ------------------------------------------------------------------

    #[inline(always)]
    fn write_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    #[inline(always)]
    fn write_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_i8(&mut self, v: i8) {
        self.buf.push(v as u8);
    }

    #[inline(always)]
    fn write_i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    #[inline(always)]
    fn write_f32(&mut self, v: f32) {
        // Bit-cast: no conversion, just copy 4 IEEE-754 bytes
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    #[inline(always)]
    fn write_f64(&mut self, v: f64) {
        // Bit-cast: no conversion, just copy 8 IEEE-754 bytes
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
    }

    /// Write raw bytes with SIMD bulk copy for large payloads.
    #[inline]
    fn write_bytes_raw(&mut self, data: &[u8]) {
        simd::simd_bulk_extend(&mut self.buf, data);
    }

    /// Write a string: `u32 LE length` + UTF-8 bytes.
    #[inline]
    fn write_str(&mut self, s: &str) {
        let bytes = s.as_bytes();
        self.write_u32(bytes.len() as u32);
        self.write_bytes_raw(bytes);
    }
}

// ============================================================================
// serde::Serializer impl
// ============================================================================

impl<'a> ser::Serializer for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = BinSeqSer<'a>;
    type SerializeTuple = &'a mut BinarySerializer;
    type SerializeTupleStruct = &'a mut BinarySerializer;
    type SerializeTupleVariant = &'a mut BinarySerializer;
    type SerializeMap = BinSeqSer<'a>;
    type SerializeStruct = &'a mut BinarySerializer;
    type SerializeStructVariant = &'a mut BinarySerializer;

    #[inline]
    fn serialize_bool(self, v: bool) -> Result<()> {
        self.write_u8(v as u8);
        Ok(())
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<()> {
        self.write_i8(v);
        Ok(())
    }

    #[inline]
    fn serialize_i16(self, v: i16) -> Result<()> {
        self.write_i16(v);
        Ok(())
    }

    #[inline]
    fn serialize_i32(self, v: i32) -> Result<()> {
        self.write_i32(v);
        Ok(())
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<()> {
        self.write_i64(v);
        Ok(())
    }

    #[inline]
    fn serialize_u8(self, v: u8) -> Result<()> {
        self.write_u8(v);
        Ok(())
    }

    #[inline]
    fn serialize_u16(self, v: u16) -> Result<()> {
        self.write_u16(v);
        Ok(())
    }

    #[inline]
    fn serialize_u32(self, v: u32) -> Result<()> {
        self.write_u32(v);
        Ok(())
    }

    #[inline]
    fn serialize_u64(self, v: u64) -> Result<()> {
        self.write_u64(v);
        Ok(())
    }

    #[inline]
    fn serialize_f32(self, v: f32) -> Result<()> {
        self.write_f32(v);
        Ok(())
    }

    #[inline]
    fn serialize_f64(self, v: f64) -> Result<()> {
        self.write_f64(v);
        Ok(())
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<()> {
        self.write_u32(v as u32);
        Ok(())
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<()> {
        self.write_str(v);
        Ok(())
    }

    #[inline]
    fn serialize_bytes(self, v: &[u8]) -> Result<()> {
        self.write_u32(v.len() as u32);
        self.write_bytes_raw(v);
        Ok(())
    }

    #[inline]
    fn serialize_none(self) -> Result<()> {
        self.write_u8(0);
        Ok(())
    }

    #[inline]
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<()> {
        self.write_u8(1);
        value.serialize(self)
    }

    /// Unit / unit struct / unit variant → 0 bytes.
    #[inline]
    fn serialize_unit(self) -> Result<()> {
        Ok(())
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
        Ok(())
    }

    #[inline]
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
    ) -> Result<()> {
        self.write_u32(variant_index);
        Ok(())
    }

    #[inline]
    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<()> {
        value.serialize(self)
    }

    #[inline]
    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        value: &T,
    ) -> Result<()> {
        self.write_u32(variant_index);
        value.serialize(self)
    }

    /// Sequence: write placeholder u32 count (fixed up in `end()`).
    fn serialize_seq(self, len: Option<usize>) -> Result<BinSeqSer<'a>> {
        Ok(BinSeqSer::new(self, len))
    }

    /// Tuple: known length, no prefix needed.
    fn serialize_tuple(self, _len: usize) -> Result<&'a mut BinarySerializer> {
        Ok(self)
    }

    fn serialize_tuple_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<&'a mut BinarySerializer> {
        Ok(self)
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<&'a mut BinarySerializer> {
        self.write_u32(variant_index);
        Ok(self)
    }

    /// Map: write placeholder u32 count (fixed up in `end()`).
    fn serialize_map(self, len: Option<usize>) -> Result<BinSeqSer<'a>> {
        Ok(BinSeqSer::new(self, len))
    }

    /// Struct: fields written in order, no length prefix.
    fn serialize_struct(
        self,
        _name: &'static str,
        _len: usize,
    ) -> Result<&'a mut BinarySerializer> {
        Ok(self)
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        variant_index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<&'a mut BinarySerializer> {
        self.write_u32(variant_index);
        Ok(self)
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

// ============================================================================
// BinSeqSer — handles sequences and maps with unknown-at-call-time lengths
// ============================================================================

pub struct BinSeqSer<'a> {
    ser: &'a mut BinarySerializer,
    /// Byte position in `buf` where the `u32` count placeholder lives.
    len_pos: usize,
    count: u32,
}

impl<'a> BinSeqSer<'a> {
    fn new(ser: &'a mut BinarySerializer, known_len: Option<usize>) -> Self {
        let len_pos = ser.buf.len();
        // Write placeholder — will be fixed up in end()
        let count = known_len.unwrap_or(0) as u32;
        ser.write_u32(count);
        BinSeqSer { ser, len_pos, count: 0 }
    }

    #[inline(always)]
    fn fix_len(&mut self) {
        let bytes = self.count.to_le_bytes();
        self.ser.buf[self.len_pos..self.len_pos + 4].copy_from_slice(&bytes);
    }
}

impl<'a> SerializeSeq for BinSeqSer<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        self.count += 1;
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(mut self) -> Result<()> {
        self.fix_len();
        Ok(())
    }
}

impl<'a> SerializeMap for BinSeqSer<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_key<T: ?Sized + Serialize>(&mut self, key: &T) -> Result<()> {
        self.count += 1;
        key.serialize(&mut *self.ser)
    }

    #[inline]
    fn serialize_value<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(mut self) -> Result<()> {
        self.fix_len();
        Ok(())
    }
}

impl<'a> SerializeTupleVariant for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// Tuple / Struct — use &mut BinarySerializer directly (no count prefix)
// ============================================================================

impl<'a> SerializeTuple for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a> SerializeTupleStruct for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a> SerializeStruct for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _key: &'static str, value: &T) -> Result<()> {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

impl<'a> SerializeStructVariant for &'a mut BinarySerializer {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, _key: &'static str, value: &T) -> Result<()> {
        value.serialize(&mut **self)
    }

    #[inline]
    fn end(self) -> Result<()> {
        Ok(())
    }
}

// ============================================================================
// BinaryDeserializer
// ============================================================================

pub struct BinaryDeserializer<'de> {
    data: &'de [u8],
    pos: usize,
}

impl<'de> BinaryDeserializer<'de> {
    #[inline]
    pub fn new(data: &'de [u8]) -> Self {
        Self { data, pos: 0 }
    }

    // ------------------------------------------------------------------
    // Primitive readers — all inline, zero allocation
    // ------------------------------------------------------------------

    #[inline(always)]
    fn ensure(&self, n: usize) -> Result<()> {
        if self.pos + n <= self.data.len() {
            Ok(())
        } else {
            Err(Error::Eof)
        }
    }

    #[inline(always)]
    fn read_u8(&mut self) -> Result<u8> {
        self.ensure(1)?;
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    #[inline(always)]
    fn read_u16(&mut self) -> Result<u16> {
        self.ensure(2)?;
        let v = u16::from_le_bytes(
            self.data[self.pos..self.pos + 2].try_into().unwrap()
        );
        self.pos += 2;
        Ok(v)
    }

    #[inline(always)]
    fn read_u32(&mut self) -> Result<u32> {
        self.ensure(4)?;
        let v = u32::from_le_bytes(
            self.data[self.pos..self.pos + 4].try_into().unwrap()
        );
        self.pos += 4;
        Ok(v)
    }

    #[inline(always)]
    fn read_u64(&mut self) -> Result<u64> {
        self.ensure(8)?;
        let v = u64::from_le_bytes(
            self.data[self.pos..self.pos + 8].try_into().unwrap()
        );
        self.pos += 8;
        Ok(v)
    }

    #[inline(always)]
    fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    #[inline(always)]
    fn read_i16(&mut self) -> Result<i16> {
        self.ensure(2)?;
        let v = i16::from_le_bytes(
            self.data[self.pos..self.pos + 2].try_into().unwrap()
        );
        self.pos += 2;
        Ok(v)
    }

    #[inline(always)]
    fn read_i32(&mut self) -> Result<i32> {
        self.ensure(4)?;
        let v = i32::from_le_bytes(
            self.data[self.pos..self.pos + 4].try_into().unwrap()
        );
        self.pos += 4;
        Ok(v)
    }

    #[inline(always)]
    fn read_i64(&mut self) -> Result<i64> {
        self.ensure(8)?;
        let v = i64::from_le_bytes(
            self.data[self.pos..self.pos + 8].try_into().unwrap()
        );
        self.pos += 8;
        Ok(v)
    }

    #[inline(always)]
    fn read_f32(&mut self) -> Result<f32> {
        // Bit-cast: read 4 bytes, interpret as IEEE-754 float32
        let bits = self.read_u32()?;
        Ok(f32::from_bits(bits))
    }

    #[inline(always)]
    fn read_f64(&mut self) -> Result<f64> {
        // Bit-cast: read 8 bytes, interpret as IEEE-754 float64
        let bits = self.read_u64()?;
        Ok(f64::from_bits(bits))
    }

    /// Read string **without allocation** — returns a `&'de str` borrowing `data`.
    ///
    /// This is the core zero-copy path: callers with `&'de str` fields pay
    /// only for the `u32` length read + a bounds check.
    #[inline]
    fn read_str_zerocopy(&mut self) -> Result<&'de str> {
        let len = self.read_u32()? as usize;
        self.ensure(len)?;
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        // SAFETY: serializer always writes valid UTF-8 (from Rust &str/String)
        Ok(unsafe { core::str::from_utf8_unchecked(bytes) })
    }

    /// Read raw bytes slice — zero-copy borrow of input.
    #[inline]
    fn read_bytes_zerocopy(&mut self) -> Result<&'de [u8]> {
        let len = self.read_u32()? as usize;
        self.ensure(len)?;
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(bytes)
    }
}

// ============================================================================
// serde::Deserializer impl
// ============================================================================

impl<'de, 'a> de::Deserializer<'de> for &'a mut BinaryDeserializer<'de> {
    type Error = Error;

    /// Binary format is NOT self-describing — type tags are absent.
    /// `deserialize_any` is only called by generic serde code that inspect values.
    fn deserialize_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value> {
        Err(Error::Message(
            "ASON binary format is not self-describing; use typed deserialization".into(),
        ))
    }

    #[inline]
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_bool(self.read_u8()? != 0)
    }

    #[inline]
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i8(self.read_i8()?)
    }

    #[inline]
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i16(self.read_i16()?)
    }

    #[inline]
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i32(self.read_i32()?)
    }

    #[inline]
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i64(self.read_i64()?)
    }

    #[inline]
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u8(self.read_u8()?)
    }

    #[inline]
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u16(self.read_u16()?)
    }

    #[inline]
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u32(self.read_u32()?)
    }

    #[inline]
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u64(self.read_u64()?)
    }

    #[inline]
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f32(self.read_f32()?)
    }

    #[inline]
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f64(self.read_f64()?)
    }

    #[inline]
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let cp = self.read_u32()?;
        let c = char::from_u32(cp)
            .ok_or_else(|| Error::Message(format!("invalid char codepoint: {cp}")))?;
        visitor.visit_char(c)
    }

    /// Zero-copy: returns `&'de str` borrowing directly from `data`.
    #[inline]
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_str_zerocopy()?;
        visitor.visit_borrowed_str(s)
    }

    /// For `String` fields: still zero-copy for the borrow; serde calls `visit_borrowed_str`
    /// and converts to `String` if needed. No intermediate `String::new()` call here.
    #[inline]
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let s = self.read_str_zerocopy()?;
        visitor.visit_borrowed_str(s)
    }

    #[inline]
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let bytes = self.read_bytes_zerocopy()?;
        visitor.visit_borrowed_bytes(bytes)
    }

    #[inline]
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let bytes = self.read_bytes_zerocopy()?;
        visitor.visit_borrowed_bytes(bytes)
    }

    #[inline]
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let tag = self.read_u8()?;
        if tag == 0 {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    #[inline]
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    #[inline]
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_unit()
    }

    #[inline]
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    /// Sequence: read `u32 LE` count, then deliver elements via `SeqAccess`.
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let count = self.read_u32()? as usize;
        visitor.visit_seq(BinSeqAccess::new(self, count))
    }

    /// Tuple: length known from schema, no prefix in data.
    fn deserialize_tuple<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(BinSeqAccess::new(self, len))
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(BinSeqAccess::new(self, len))
    }

    /// Map: read `u32 LE` count, then deliver pairs via `MapAccess`.
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        let count = self.read_u32()? as usize;
        visitor.visit_map(BinMapAccess::new(self, count))
    }

    /// Struct: fields are positional — no count prefix in data.
    /// The field count is supplied by the generated `Deserialize` impl via `fields`.
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(BinSeqAccess::new(self, fields.len()))
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_enum(BinEnumAccess { de: self })
    }

    /// Identifier: not used in binary (positional), but must be implemented.
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value> {
        Err(Error::Message("cannot ignore in binary format".into()))
    }

    fn is_human_readable(&self) -> bool {
        false
    }
}

// ============================================================================
// SeqAccess — drives struct and sequence deserialization
// ============================================================================

struct BinSeqAccess<'a, 'de: 'a> {
    de: &'a mut BinaryDeserializer<'de>,
    remaining: usize,
}

impl<'a, 'de> BinSeqAccess<'a, 'de> {
    #[inline]
    fn new(de: &'a mut BinaryDeserializer<'de>, remaining: usize) -> Self {
        Self { de, remaining }
    }
}

impl<'de, 'a> SeqAccess<'de> for BinSeqAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(&mut *self.de).map(Some)
    }

    #[inline]
    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

// ============================================================================
// MapAccess — drives HashMap deserialization
// ============================================================================

struct BinMapAccess<'a, 'de: 'a> {
    de: &'a mut BinaryDeserializer<'de>,
    remaining: usize,
}

impl<'a, 'de> BinMapAccess<'a, 'de> {
    #[inline]
    fn new(de: &'a mut BinaryDeserializer<'de>, remaining: usize) -> Self {
        Self { de, remaining }
    }
}

impl<'de, 'a> MapAccess<'de> for BinMapAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(&mut *self.de).map(Some)
    }

    #[inline]
    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value> {
        seed.deserialize(&mut *self.de)
    }

    #[inline]
    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

// ============================================================================
// EnumAccess + VariantAccess — drives enum deserialization
// ============================================================================

struct BinEnumAccess<'a, 'de: 'a> {
    de: &'a mut BinaryDeserializer<'de>,
}

impl<'de, 'a> EnumAccess<'de> for BinEnumAccess<'a, 'de> {
    type Error = Error;
    type Variant = BinVariantAccess<'a, 'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, BinVariantAccess<'a, 'de>)> {
        // variant index encoded as u32 LE
        let idx = self.de.read_u32()?;
        let val = seed.deserialize(de::value::U32Deserializer::new(idx))?;
        Ok((val, BinVariantAccess { de: self.de }))
    }
}

struct BinVariantAccess<'a, 'de: 'a> {
    de: &'a mut BinaryDeserializer<'de>,
}

impl<'de, 'a> VariantAccess<'de> for BinVariantAccess<'a, 'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<()> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(
        self,
        seed: T,
    ) -> Result<T::Value> {
        seed.deserialize(self.de)
    }

    fn tuple_variant<V: Visitor<'de>>(
        self,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(BinSeqAccess::new(self.de, len))
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(BinSeqAccess::new(self.de, fields.len()))
    }
}

// ============================================================================
// Compile-time size check
// ============================================================================

const _: () = {
    // BinaryDeserializer: &[u8] (2 usize fat ptr) + usize pos = 3 usize
    assert!(mem::size_of::<BinaryDeserializer<'_>>() == 3 * mem::size_of::<usize>());
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct User {
        id: i64,
        name: String,
        score: f64,
        active: bool,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct AllPrims {
        b: bool,
        i8v: i8,
        i16v: i16,
        i32v: i32,
        i64v: i64,
        u8v: u8,
        u16v: u16,
        u32v: u32,
        u64v: u64,
        f32v: f32,
        f64v: f64,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct WithOption {
        id: i64,
        label: Option<String>,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct WithVec {
        name: String,
        scores: Vec<i64>,
    }

    #[test]
    fn test_user_roundtrip() {
        let u = User { id: 42, name: "Alice".into(), score: 9.5, active: true };
        let bytes = encode_binary(&u).unwrap();
        let u2: User = decode_binary(&bytes).unwrap();
        assert_eq!(u, u2);
    }

    #[test]
    fn test_all_primitives() {
        let v = AllPrims {
            b: true,
            i8v: -1,
            i16v: -300,
            i32v: -70000,
            i64v: i64::MIN,
            u8v: 255,
            u16v: 65535,
            u32v: u32::MAX,
            u64v: u64::MAX,
            f32v: 3.15,
            f64v: 2.718281828,
        };
        let bytes = encode_binary(&v).unwrap();
        let v2: AllPrims = decode_binary(&bytes).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn test_option_some_none() {
        let a = WithOption { id: 1, label: Some("hello".into()) };
        let b = WithOption { id: 2, label: None };
        let b1 = encode_binary(&a).unwrap();
        let b2 = encode_binary(&b).unwrap();
        let a2: WithOption = decode_binary(&b1).unwrap();
        let b3: WithOption = decode_binary(&b2).unwrap();
        assert_eq!(a, a2);
        assert_eq!(b, b3);
    }

    #[test]
    fn test_vec_roundtrip() {
        let v = WithVec { name: "stats".into(), scores: vec![10, 20, 30, 40, 50] };
        let bytes = encode_binary(&v).unwrap();
        let v2: WithVec = decode_binary(&bytes).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn test_vec_of_structs() {
        let users = vec![
            User { id: 1, name: "Alice".into(), score: 9.0, active: true },
            User { id: 2, name: "Bob".into(), score: 7.5, active: false },
        ];
        let bytes = encode_binary(&users).unwrap();
        let users2: Vec<User> = decode_binary(&bytes).unwrap();
        assert_eq!(users, users2);
    }

    #[test]
    fn test_map_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct M {
            data: HashMap<String, i64>,
        }
        let mut m = M { data: HashMap::new() };
        m.data.insert("a".into(), 1);
        m.data.insert("b".into(), 2);
        let bytes = encode_binary(&m).unwrap();
        let m2: M = decode_binary(&bytes).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_enum_roundtrip() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        enum Color {
            Red,
            Green,
            Blue,
            Custom(u8, u8, u8),
        }
        for c in [Color::Red, Color::Green, Color::Blue, Color::Custom(10, 20, 30)] {
            let bytes = encode_binary(&c).unwrap();
            let c2: Color = decode_binary(&bytes).unwrap();
            assert_eq!(c, c2);
        }
    }

    #[test]
    fn test_binary_size_vs_text() {
        let users: Vec<User> = (0..100)
            .map(|i| User { id: i, name: format!("User_{}", i), score: i as f64 * 0.5, active: i % 2 == 0 })
            .collect();
        let bin = encode_binary(&users).unwrap();
        let json = serde_json::to_string(&users).unwrap();
        // Binary should be significantly smaller
        assert!(bin.len() < json.len(), "bin={} json={}", bin.len(), json.len());
    }
}
