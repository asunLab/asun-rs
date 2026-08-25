//! ASUN Binary Format (ASUN-BIN)
//!
//! A high-performance binary encoding for ASUN data structures.
//! Provides `encode_binary` and `decode_binary` for zero-overhead struct ↔ bytes conversion.
//!
//! Integers are **LEB128 varint** encoded (with **zigzag** for signed types), which
//! shrinks integer-heavy payloads substantially when values are small — the common
//! case for IDs, counts, timestamps, and enum tags.
//!
//! ## Wire Format
//!
//! ```text
//! bool      → 1 byte  (0x00=false, 0x01=true)
//! i8        → 1 byte (signed; varint never helps for a single byte)
//! u8        → 1 byte
//! i16/i32/i64 → zigzag + LEB128 varint  (1..=10 bytes)
//! u16/u32/u64 → LEB128 varint           (1..=10 bytes)
//! f32       → 4 bytes LE (IEEE 754 bit-cast — float bits don't compress)
//! f64       → 8 bytes LE (IEEE 754 bit-cast)
//! char      → uvarint (Unicode scalar as u32)
//! str       → uvarint length + UTF-8 bytes  ← ZERO-COPY on decode (&'de str)
//! bytes     → uvarint length + raw bytes
//! Option<T> → u8 tag (0=None, 1=Some) + [T payload if Some]
//! Vec<T>    → uvarint count + [element × count]
//! struct    → fields in declaration order (no length prefix — known from schema)
//! tuple     → elements in order (no length prefix)
//! enum      → uvarint variant_index + [payload for non-unit variants]
//! unit      → 0 bytes
//! newtype   → inner value directly (no wrapper)
//! ```
//!
//! ## Key Features
//!
//! - **Compact integers**: LEB128 varint + zigzag; small values cost 1 byte.
//! - **Zero-copy string decode**: borrowed `&'de str` slices directly reference input bytes.
//! - **No type tags** for struct fields: schema drives layout (like Protobuf binary, not CBOR).
//! - **SIMD-accelerated** bulk byte copy for large string payloads (≥ 32 bytes).

use crate::error::{Error, Result};
use crate::simd;
use crate::traits::{AsunDecodeBinary, AsunEncodeBinary};
use core::mem;

// ============================================================================
// zigzag helpers — map signed integers to unsigned so small magnitudes
// (positive or negative) encode into few varint bytes.
// ============================================================================

#[inline(always)]
fn zigzag_encode(v: i64) -> u64 {
    ((v << 1) ^ (v >> 63)) as u64
}

#[inline(always)]
fn zigzag_decode(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

// ============================================================================
// Public API
// ============================================================================

/// Encode `value` to a `Vec<u8>` using the ASUN binary format.
///
/// # Example
/// ```rust,ignore
/// let user = User { id: 1, name: "Alice".into(), active: true };
/// let bytes = asun::encode_binary(&user)?;
/// ```
#[inline]
pub fn encode_binary<T: AsunEncodeBinary + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let mut enc = BinaryEncoder::with_capacity(256);
    value.encode_binary(&mut enc)?;
    Ok(enc.buf)
}

/// Decode a value from ASUN binary bytes.
///
/// The lifetime `'de` allows **zero-copy** decoding: any `&'de str` fields
/// in the target type will borrow directly from `data` with no allocation.
///
/// # Example
/// ```rust,ignore
/// let user: User = asun::decode_binary(&bytes)?;
/// ```
#[inline]
pub fn decode_binary<'de, T: AsunDecodeBinary<'de>>(data: &'de [u8]) -> Result<T> {
    let mut dec = BinaryDecoder::new(data);
    T::decode_binary(&mut dec)
}

// ============================================================================
// BinaryEncoder
// ============================================================================

/// The ASUN binary encode sink that derive-generated [`AsunEncodeBinary`] impls
/// write into. Prefer the [`encode_binary`] free function; this type is exposed
/// for the generated code.
///
/// [`AsunEncodeBinary`]: crate::AsunEncodeBinary
pub struct BinaryEncoder {
    pub(crate) buf: Vec<u8>,
}

impl Default for BinaryEncoder {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl BinaryEncoder {
    #[inline]
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    #[inline]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: Vec::with_capacity(cap),
        }
    }

    /// Consume the encoder, returning the accumulated bytes.
    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    // ------------------------------------------------------------------
    // Primitive writers — each emits fixed bytes, zero heap allocation.
    //
    // These are the direct sink methods the `AsunEncodeBinary` impls call.
    // ------------------------------------------------------------------

    #[inline(always)]
    pub fn write_bool(&mut self, v: bool) -> Result<()> {
        self.buf.push(v as u8);
        Ok(())
    }

    #[inline(always)]
    pub fn write_u8(&mut self, v: u8) -> Result<()> {
        self.buf.push(v);
        Ok(())
    }

    /// LEB128 unsigned varint.
    #[inline(always)]
    fn write_uvarint(&mut self, mut v: u64) {
        while v >= 0x80 {
            self.buf.push((v as u8) | 0x80);
            v >>= 7;
        }
        self.buf.push(v as u8);
    }

    /// zigzag + LEB128 signed varint.
    #[inline(always)]
    fn write_ivarint(&mut self, v: i64) {
        self.write_uvarint(zigzag_encode(v));
    }

    #[inline(always)]
    pub fn write_i8(&mut self, v: i8) -> Result<()> {
        self.buf.push(v as u8);
        Ok(())
    }

    #[inline(always)]
    pub fn write_i16(&mut self, v: i16) -> Result<()> {
        self.write_ivarint(v as i64);
        Ok(())
    }

    #[inline(always)]
    pub fn write_i32(&mut self, v: i32) -> Result<()> {
        self.write_ivarint(v as i64);
        Ok(())
    }

    #[inline(always)]
    pub fn write_i64(&mut self, v: i64) -> Result<()> {
        self.write_ivarint(v);
        Ok(())
    }

    #[inline(always)]
    pub fn write_u16(&mut self, v: u16) -> Result<()> {
        self.write_uvarint(v as u64);
        Ok(())
    }

    #[inline(always)]
    pub fn write_u32(&mut self, v: u32) -> Result<()> {
        self.write_uvarint(v as u64);
        Ok(())
    }

    #[inline(always)]
    pub fn write_u64(&mut self, v: u64) -> Result<()> {
        self.write_uvarint(v);
        Ok(())
    }

    #[inline(always)]
    pub fn write_f32(&mut self, v: f32) -> Result<()> {
        // Bit-cast: no conversion, just copy 4 IEEE-754 bytes
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
        Ok(())
    }

    #[inline(always)]
    pub fn write_f64(&mut self, v: f64) -> Result<()> {
        // Bit-cast: no conversion, just copy 8 IEEE-754 bytes
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
        Ok(())
    }

    #[inline(always)]
    pub fn write_char(&mut self, v: char) -> Result<()> {
        self.write_uvarint(v as u64);
        Ok(())
    }

    /// Write raw bytes with SIMD bulk copy for large payloads.
    #[inline]
    fn write_bytes_raw(&mut self, data: &[u8]) {
        simd::simd_bulk_extend(&mut self.buf, data);
    }

    /// Write a string: `uvarint length` + UTF-8 bytes.
    #[inline]
    pub fn write_str(&mut self, s: &str) -> Result<()> {
        let bytes = s.as_bytes();
        self.write_uvarint(bytes.len() as u64);
        self.write_bytes_raw(bytes);
        Ok(())
    }

    /// Write raw bytes: `uvarint length` + bytes.
    #[inline]
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        self.write_uvarint(data.len() as u64);
        self.write_bytes_raw(data);
        Ok(())
    }

    /// Write a sequence: `uvarint count` + each element in order.
    ///
    /// The length is always known here (a slice), so the count is written up
    /// front and elements stream directly into the buffer — no scratch buffer.
    #[inline]
    pub fn write_seq<T: AsunEncodeBinary>(&mut self, items: &[T]) -> Result<()> {
        self.write_uvarint(items.len() as u64);
        for item in items {
            item.encode_binary(self)?;
        }
        Ok(())
    }

    /// Write an enum variant index as a uvarint.
    #[inline]
    pub fn write_variant_index(&mut self, index: u32) -> Result<()> {
        self.write_uvarint(index as u64);
        Ok(())
    }
}

// ============================================================================
// BinaryDecoder
// ============================================================================

/// The ASUN binary decode source that derive-generated [`AsunDecodeBinary`]
/// impls pull from. Prefer the [`decode_binary`] free function; this type is
/// exposed for the generated code. The `'de` lifetime enables zero-copy
/// `&'de str` / `&'de [u8]` fields borrowed from the input.
///
/// [`AsunDecodeBinary`]: crate::AsunDecodeBinary
pub struct BinaryDecoder<'de> {
    data: &'de [u8],
    pos: usize,
}

impl<'de> BinaryDecoder<'de> {
    #[inline]
    pub fn new(data: &'de [u8]) -> Self {
        Self { data, pos: 0 }
    }

    // ------------------------------------------------------------------
    // Primitive readers — all inline, zero allocation
    // ------------------------------------------------------------------

    #[inline(always)]
    fn ensure(&self, n: usize) -> Result<()> {
        // `self.pos + n` can overflow `usize` for an attacker-controlled varint
        // length, wrapping to a small value that wrongly passes the check and
        // then panics on the subsequent slice. Compare against remaining bytes
        // instead so the arithmetic can never overflow.
        if n <= self.data.len() - self.pos {
            Ok(())
        } else {
            Err(Error::Eof)
        }
    }

    #[inline(always)]
    pub fn read_u8(&mut self) -> Result<u8> {
        self.ensure(1)?;
        let v = self.data[self.pos];
        self.pos += 1;
        Ok(v)
    }

    /// LEB128 unsigned varint. Rejects overlong (> 64-bit) encodings.
    #[inline(always)]
    fn read_uvarint(&mut self) -> Result<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = self.read_u8()?;
            if shift >= 64 {
                return Err(Error::msg("varint overflow"));
            }
            result |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    /// zigzag + LEB128 signed varint.
    #[inline(always)]
    fn read_ivarint(&mut self) -> Result<i64> {
        Ok(zigzag_decode(self.read_uvarint()?))
    }

    #[inline(always)]
    pub fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    #[inline(always)]
    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    #[inline(always)]
    pub fn read_i16(&mut self) -> Result<i16> {
        Ok(self.read_ivarint()? as i16)
    }

    #[inline(always)]
    pub fn read_i32(&mut self) -> Result<i32> {
        Ok(self.read_ivarint()? as i32)
    }

    #[inline(always)]
    pub fn read_i64(&mut self) -> Result<i64> {
        self.read_ivarint()
    }

    #[inline(always)]
    pub fn read_u16(&mut self) -> Result<u16> {
        Ok(self.read_uvarint()? as u16)
    }

    #[inline(always)]
    pub fn read_u32(&mut self) -> Result<u32> {
        Ok(self.read_uvarint()? as u32)
    }

    #[inline(always)]
    pub fn read_u64(&mut self) -> Result<u64> {
        self.read_uvarint()
    }

    #[inline(always)]
    fn read_u32_le(&mut self) -> Result<u32> {
        self.ensure(4)?;
        let v = u32::from_le_bytes(self.data[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    #[inline(always)]
    fn read_u64_le(&mut self) -> Result<u64> {
        self.ensure(8)?;
        let v = u64::from_le_bytes(self.data[self.pos..self.pos + 8].try_into().unwrap());
        self.pos += 8;
        Ok(v)
    }

    #[inline(always)]
    pub fn read_f32(&mut self) -> Result<f32> {
        // Bit-cast: read 4 fixed LE bytes, interpret as IEEE-754 float32
        Ok(f32::from_bits(self.read_u32_le()?))
    }

    #[inline(always)]
    pub fn read_f64(&mut self) -> Result<f64> {
        // Bit-cast: read 8 fixed LE bytes, interpret as IEEE-754 float64
        Ok(f64::from_bits(self.read_u64_le()?))
    }

    #[inline(always)]
    pub fn read_char(&mut self) -> Result<char> {
        let cp = self.read_uvarint()? as u32;
        char::from_u32(cp).ok_or_else(|| Error::msg(format!("invalid char codepoint: {cp}")))
    }

    /// Read string **without allocation** — returns a `&'de str` borrowing `data`.
    ///
    /// This is the core zero-copy path: callers with `&'de str` fields pay
    /// only for the `u32` length read + a bounds check.
    #[inline]
    pub fn read_str_zerocopy(&mut self) -> Result<&'de str> {
        let len = self.read_uvarint()? as usize;
        self.ensure(len)?;
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        // The input may be untrusted binary, so validate rather than assume
        // well-formed UTF-8; an invalid slice would otherwise be UB.
        core::str::from_utf8(bytes).map_err(|_| Error::msg("invalid utf-8"))
    }

    /// Read an owned `String` — borrows the input then copies into a fresh alloc.
    #[inline]
    pub fn read_string(&mut self) -> Result<String> {
        Ok(self.read_str_zerocopy()?.to_owned())
    }

    /// Read raw bytes slice — zero-copy borrow of input.
    #[inline]
    pub fn read_bytes_zerocopy(&mut self) -> Result<&'de [u8]> {
        let len = self.read_uvarint()? as usize;
        self.ensure(len)?;
        let bytes = &self.data[self.pos..self.pos + len];
        self.pos += len;
        Ok(bytes)
    }

    /// Read a sequence: `uvarint count` + each element in order.
    #[inline]
    pub fn read_vec<T: AsunDecodeBinary<'de>>(&mut self) -> Result<Vec<T>> {
        let count = self.read_uvarint()? as usize;
        // `count` is attacker-controlled; guard the pre-allocation against a
        // bogus huge length by capping the reserve to what the input could
        // possibly contain (each element is ≥ 1 byte on the wire).
        let cap = count.min(self.data.len().saturating_sub(self.pos));
        let mut out = Vec::with_capacity(cap);
        for _ in 0..count {
            out.push(T::decode_binary(self)?);
        }
        Ok(out)
    }

    /// Read an enum variant index (uvarint) as a `u32`.
    #[inline]
    pub fn read_variant_index(&mut self) -> Result<u32> {
        Ok(self.read_uvarint()? as u32)
    }
}

// ============================================================================
// Compile-time size check
// ============================================================================

const _: () = {
    // BinaryDecoder: &[u8] (2 usize fat ptr) + usize pos = 3 usize
    assert!(mem::size_of::<BinaryDecoder<'_>>() == 3 * mem::size_of::<usize>());
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AsunDecode, AsunEncode};

    #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
    struct User {
        id: i64,
        name: String,
        score: f64,
        active: bool,
    }

    #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
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

    #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
    struct WithOption {
        id: i64,
        label: Option<String>,
    }

    #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
    struct WithVec {
        name: String,
        scores: Vec<i64>,
    }

    #[test]
    fn test_user_roundtrip() {
        let u = User {
            id: 42,
            name: "Alice".into(),
            score: 9.5,
            active: true,
        };
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
        let a = WithOption {
            id: 1,
            label: Some("hello".into()),
        };
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
        let v = WithVec {
            name: "stats".into(),
            scores: vec![10, 20, 30, 40, 50],
        };
        let bytes = encode_binary(&v).unwrap();
        let v2: WithVec = decode_binary(&bytes).unwrap();
        assert_eq!(v, v2);
    }

    #[test]
    fn test_vec_of_structs() {
        let users = vec![
            User {
                id: 1,
                name: "Alice".into(),
                score: 9.0,
                active: true,
            },
            User {
                id: 2,
                name: "Bob".into(),
                score: 7.5,
                active: false,
            },
        ];
        let bytes = encode_binary(&users).unwrap();
        let users2: Vec<User> = decode_binary(&bytes).unwrap();
        assert_eq!(users, users2);
    }

    #[test]
    fn test_entry_list_roundtrip() {
        #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
        struct Entry {
            key: String,
            value: i64,
        }
        #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
        struct M {
            data: Vec<Entry>,
        }
        let m = M {
            data: vec![
                Entry {
                    key: "a".into(),
                    value: 1,
                },
                Entry {
                    key: "b".into(),
                    value: 2,
                },
            ],
        };
        let bytes = encode_binary(&m).unwrap();
        let m2: M = decode_binary(&bytes).unwrap();
        assert_eq!(m, m2);
    }

    #[test]
    fn test_enum_roundtrip() {
        #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
        enum Color {
            Red,
            Green,
            Blue,
            Custom(u8, u8, u8),
        }
        for c in [
            Color::Red,
            Color::Green,
            Color::Blue,
            Color::Custom(10, 20, 30),
        ] {
            let bytes = encode_binary(&c).unwrap();
            let c2: Color = decode_binary(&bytes).unwrap();
            assert_eq!(c, c2);
        }
    }
}
