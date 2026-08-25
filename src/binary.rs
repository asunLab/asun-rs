//! ASUN binary format (ASUN-BIN).
//!
//! Wire compatibility is preserved: unsigned integers use LEB128, signed
//! integers use zigzag + LEB128, floats are fixed-width little-endian, and
//! strings/bytes are length-prefixed. The implementation uses a pointer cursor
//! for one bounds check per primitive and validates malformed integer encodings.

use crate::error::{Error, Result};
use crate::simd;
use crate::traits::{AsunDecodeBinary, AsunEncodeBinary};
use core::marker::PhantomData;
use core::{mem, ptr, slice};

/// Default guard for attacker-controlled sequence lengths.
///
/// Applications with a different protocol limit can construct a
/// [`BinaryDecoder`] with [`BinaryDecoder::with_max_sequence_len`].
pub const DEFAULT_MAX_SEQUENCE_LEN: usize = 16 * 1024 * 1024;

#[inline(always)]
fn zigzag_encode(v: i64) -> u64 {
    ((v as u64) << 1) ^ ((v >> 63) as u64)
}

#[inline(always)]
fn zigzag_decode(v: u64) -> i64 {
    ((v >> 1) as i64) ^ -((v & 1) as i64)
}

/// Encode `value` into a newly allocated byte vector.
#[inline]
pub fn encode_binary<T: AsunEncodeBinary + ?Sized>(value: &T) -> Result<Vec<u8>> {
    let mut enc = BinaryEncoder::with_capacity(256);
    value.encode_binary(&mut enc)?;
    Ok(enc.buf)
}

/// Encode into a caller-owned vector while retaining its allocation.
///
/// `out` is cleared before encoding. On error it contains the valid encoded
/// prefix, and can still be reused by the next call.
#[inline]
pub fn encode_binary_into<T: AsunEncodeBinary + ?Sized>(
    value: &T,
    out: &mut Vec<u8>,
) -> Result<()> {
    out.clear();
    let mut enc = BinaryEncoder::from_vec(mem::take(out));
    let result = value.encode_binary(&mut enc);
    *out = enc.buf;
    result
}

/// Decode one value. Trailing bytes are accepted for compatibility and for
/// callers that place several values in one framed buffer.
#[inline]
pub fn decode_binary<'de, T: AsunDecodeBinary<'de>>(data: &'de [u8]) -> Result<T> {
    let mut dec = BinaryDecoder::new(data);
    T::decode_binary(&mut dec)
}

/// Decode exactly one value and reject trailing bytes.
#[inline]
pub fn decode_binary_exact<'de, T: AsunDecodeBinary<'de>>(data: &'de [u8]) -> Result<T> {
    let mut dec = BinaryDecoder::new(data);
    let value = T::decode_binary(&mut dec)?;
    dec.finish()?;
    Ok(value)
}

/// Binary encode sink used by derive-generated implementations.
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

    /// Reuse an existing vector allocation as an encoder sink.
    #[inline]
    pub fn from_vec(buf: Vec<u8>) -> Self {
        Self { buf }
    }

    #[inline]
    pub fn clear(&mut self) {
        self.buf.clear();
    }

    #[inline]
    pub fn reserve(&mut self, additional: usize) {
        self.buf.reserve(additional);
    }

    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

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

    /// Write unsigned LEB128 directly into spare vector capacity. The common
    /// one-byte value retains the tiny `Vec::push` fast path.
    #[inline(always)]
    fn write_uvarint(&mut self, mut v: u64) {
        if v < 0x80 {
            self.buf.push(v as u8);
            return;
        }

        self.buf.reserve(10);
        let start = self.buf.len();
        unsafe {
            let out = self.buf.as_mut_ptr().add(start);
            let mut n = 0usize;
            while v >= 0x80 {
                out.add(n).write((v as u8) | 0x80);
                n += 1;
                v >>= 7;
            }
            out.add(n).write(v as u8);
            self.buf.set_len(start + n + 1);
        }
    }

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
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
        Ok(())
    }

    #[inline(always)]
    pub fn write_f64(&mut self, v: f64) -> Result<()> {
        self.buf.extend_from_slice(&v.to_bits().to_le_bytes());
        Ok(())
    }

    #[inline(always)]
    pub fn write_char(&mut self, v: char) -> Result<()> {
        self.write_uvarint(v as u32 as u64);
        Ok(())
    }

    #[inline]
    fn write_bytes_raw(&mut self, data: &[u8]) {
        simd::simd_bulk_extend(&mut self.buf, data);
    }

    #[inline]
    pub fn write_str(&mut self, s: &str) -> Result<()> {
        self.write_uvarint(s.len() as u64);
        self.write_bytes_raw(s.as_bytes());
        Ok(())
    }

    #[inline]
    pub fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        self.write_uvarint(data.len() as u64);
        self.write_bytes_raw(data);
        Ok(())
    }

    #[inline]
    pub fn write_seq<T: AsunEncodeBinary>(&mut self, items: &[T]) -> Result<()> {
        self.write_uvarint(items.len() as u64);
        for item in items {
            item.encode_binary(self)?;
        }
        Ok(())
    }

    #[inline]
    pub fn write_variant_index(&mut self, index: u32) -> Result<()> {
        self.write_uvarint(index as u64);
        Ok(())
    }
}

/// Binary decode source used by derive-generated implementations.
///
/// The decoder stores a pointer plus remaining length rather than `&[u8] +
/// index`. All pointer movement is centralized in checked methods, while
/// returned zero-copy slices remain tied to the original `'de` input lifetime.
pub struct BinaryDecoder<'de> {
    ptr: *const u8,
    remaining: usize,
    max_sequence_len: usize,
    _marker: PhantomData<&'de [u8]>,
}

// SAFETY: the raw pointer represents an immutable `&'de [u8]`; cursor mutation
// requires `&mut self`, and shared methods never dereference mutable memory.
unsafe impl<'de> Send for BinaryDecoder<'de> {}
unsafe impl<'de> Sync for BinaryDecoder<'de> {}

impl<'de> BinaryDecoder<'de> {
    #[inline]
    pub fn new(data: &'de [u8]) -> Self {
        Self::with_max_sequence_len(data, DEFAULT_MAX_SEQUENCE_LEN)
    }

    #[inline]
    pub fn with_max_sequence_len(data: &'de [u8], max_sequence_len: usize) -> Self {
        Self {
            ptr: data.as_ptr(),
            remaining: data.len(),
            max_sequence_len,
            _marker: PhantomData,
        }
    }

    #[inline(always)]
    pub fn remaining(&self) -> usize {
        self.remaining
    }

    #[inline(always)]
    pub fn is_finished(&self) -> bool {
        self.remaining == 0
    }

    #[inline]
    pub fn finish(&self) -> Result<()> {
        if self.is_finished() {
            Ok(())
        } else {
            Err(Error::TrailingBytes)
        }
    }

    #[inline(always)]
    fn take(&mut self, n: usize) -> Result<&'de [u8]> {
        if n > self.remaining {
            return Err(Error::Eof);
        }
        let start = self.ptr;
        self.ptr = unsafe { start.add(n) };
        self.remaining -= n;
        // SAFETY: `n` was checked against the remaining input length, and the
        // backing allocation is alive for `'de`.
        Ok(unsafe { slice::from_raw_parts(start, n) })
    }

    #[inline(always)]
    pub fn read_u8(&mut self) -> Result<u8> {
        if self.remaining == 0 {
            return Err(Error::Eof);
        }
        let value = unsafe { *self.ptr };
        self.ptr = unsafe { self.ptr.add(1) };
        self.remaining -= 1;
        Ok(value)
    }

    /// Strict unsigned LEB128 decoder. The tenth byte of a `u64` may contain
    /// only one payload bit, so values greater than `1` are rejected.
    #[inline(always)]
    fn read_uvarint(&mut self) -> Result<u64> {
        let remaining = self.remaining();
        if remaining == 0 {
            return Err(Error::Eof);
        }

        let start = self.ptr;
        unsafe {
            let first = *start;
            if first < 0x80 {
                self.ptr = start.add(1);
                self.remaining -= 1;
                return Ok(first as u64);
            }

            let mut value = (first & 0x7f) as u64;
            let available = remaining.min(10);
            let mut i = 1usize;
            while i < available {
                let byte = *start.add(i);
                if i == 9 {
                    if byte > 1 {
                        return Err(Error::VarintOverflow);
                    }
                    value |= (byte as u64) << 63;
                    self.ptr = start.add(10);
                    self.remaining -= 10;
                    return Ok(value);
                }

                value |= ((byte & 0x7f) as u64) << (i * 7);
                if byte < 0x80 {
                    self.ptr = start.add(i + 1);
                    self.remaining -= i + 1;
                    return Ok(value);
                }
                i += 1;
            }
        }

        if remaining < 10 {
            Err(Error::Eof)
        } else {
            Err(Error::VarintOverflow)
        }
    }

    #[inline(always)]
    fn read_ivarint(&mut self) -> Result<i64> {
        Ok(zigzag_decode(self.read_uvarint()?))
    }

    #[inline(always)]
    fn read_len(&mut self) -> Result<usize> {
        usize::try_from(self.read_uvarint()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline(always)]
    pub fn read_bool(&mut self) -> Result<bool> {
        match self.read_u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::InvalidBool),
        }
    }

    #[inline(always)]
    pub fn read_i8(&mut self) -> Result<i8> {
        Ok(self.read_u8()? as i8)
    }

    #[inline(always)]
    pub fn read_i16(&mut self) -> Result<i16> {
        i16::try_from(self.read_ivarint()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline(always)]
    pub fn read_i32(&mut self) -> Result<i32> {
        i32::try_from(self.read_ivarint()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline(always)]
    pub fn read_i64(&mut self) -> Result<i64> {
        self.read_ivarint()
    }

    #[inline(always)]
    pub fn read_u16(&mut self) -> Result<u16> {
        u16::try_from(self.read_uvarint()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline(always)]
    pub fn read_u32(&mut self) -> Result<u32> {
        u32::try_from(self.read_uvarint()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline(always)]
    pub fn read_u64(&mut self) -> Result<u64> {
        self.read_uvarint()
    }

    #[inline(always)]
    fn read_u32_le(&mut self) -> Result<u32> {
        let bytes = self.take(4)?;
        let raw = unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<u32>()) };
        Ok(u32::from_le(raw))
    }

    #[inline(always)]
    fn read_u64_le(&mut self) -> Result<u64> {
        let bytes = self.take(8)?;
        let raw = unsafe { ptr::read_unaligned(bytes.as_ptr().cast::<u64>()) };
        Ok(u64::from_le(raw))
    }

    #[inline(always)]
    pub fn read_f32(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_u32_le()?))
    }

    #[inline(always)]
    pub fn read_f64(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_u64_le()?))
    }

    #[inline(always)]
    pub fn read_char(&mut self) -> Result<char> {
        let cp = u32::try_from(self.read_uvarint()?).map_err(|_| Error::IntegerOutOfRange)?;
        char::from_u32(cp).ok_or(Error::InvalidUnicodeEscape)
    }

    /// Read a valid UTF-8 string without allocating.
    #[inline]
    pub fn read_str_zerocopy(&mut self) -> Result<&'de str> {
        let bytes = self.read_bytes_zerocopy()?;
        core::str::from_utf8(bytes).map_err(|_| Error::InvalidUtf8)
    }

    #[inline]
    pub fn read_string(&mut self) -> Result<String> {
        Ok(self.read_str_zerocopy()?.to_owned())
    }

    /// Read raw bytes without allocating.
    #[inline]
    pub fn read_bytes_zerocopy(&mut self) -> Result<&'de [u8]> {
        let len = self.read_len()?;
        self.take(len)
    }

    #[inline]
    pub fn read_vec<T: AsunDecodeBinary<'de>>(&mut self) -> Result<Vec<T>> {
        let count = self.read_len()?;
        if count > self.max_sequence_len {
            return Err(Error::SequenceTooLong);
        }

        let mut out = Vec::new();
        // Do not preallocate solely from an untrusted count. Ordinary wire
        // elements consume at least one byte, so `remaining` is a useful upper
        // bound for the common case. ZSTs allocate nothing.
        let initial = if mem::size_of::<T>() == 0 {
            0
        } else {
            count.min(self.remaining())
        };
        out.try_reserve_exact(initial)
            .map_err(|_| Error::AllocationFailed)?;
        for _ in 0..count {
            out.push(T::decode_binary(self)?);
        }
        Ok(out)
    }

    #[inline]
    pub fn read_variant_index(&mut self) -> Result<u32> {
        u32::try_from(self.read_uvarint()?).map_err(|_| Error::IntegerOutOfRange)
    }
}

const _: () = {
    // Pointer + remaining length + one sequence limit. PhantomData is zero-sized.
    assert!(mem::size_of::<BinaryDecoder<'static>>() == 3 * mem::size_of::<usize>());
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

    #[test]
    fn rejects_invalid_binary_scalars() {
        let overflow = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
        let mut dec = BinaryDecoder::new(&overflow);
        assert!(matches!(dec.read_u64(), Err(Error::VarintOverflow)));

        let mut dec = BinaryDecoder::new(&[0x80, 0x80, 0x04]); // 65536
        assert!(matches!(dec.read_u16(), Err(Error::IntegerOutOfRange)));

        let mut dec = BinaryDecoder::new(&[2]);
        assert!(matches!(dec.read_bool(), Err(Error::InvalidBool)));
    }

    #[test]
    fn exact_decode_rejects_trailing_bytes() {
        assert!(matches!(
            decode_binary_exact::<u8>(&[1, 2]),
            Err(Error::TrailingBytes)
        ));
    }

    #[test]
    fn encode_into_reuses_capacity() {
        let mut out = Vec::with_capacity(1024);
        let cap = out.capacity();
        encode_binary_into(&123u64, &mut out).unwrap();
        assert_eq!(out.capacity(), cap);
        assert_eq!(decode_binary_exact::<u64>(&out).unwrap(), 123);
    }

}
