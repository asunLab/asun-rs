//! Core traits for the ASUN format: [`AsunEncode`] / [`AsunDecode`] (text) and
//! [`AsunEncodeBinary`] / [`AsunDecodeBinary`] (binary).
//!
//! Types implement these via `#[derive(AsunEncode, AsunDecode)]` — each derive
//! emits both the text and binary impls. The design is deliberately *direct*: a
//! value writes itself straight into an [`Encoder`] sink and reads itself
//! straight from a [`Decoder`], with no visitor / seed / access indirection.
//! The derive macro emits plain sequential method bodies — simpler than a
//! visitor-based design and free of dynamic dispatch.
//!
//! Blanket impls are provided for the common standard types (integers, floats,
//! `bool`, `char`, `String`/`&str`, `Option`, `Vec`, tuples, …), so most types
//! compose without any manual impls.

use crate::binary::{BinaryDecoder, BinaryEncoder};
use crate::decode::Decoder;
use crate::encode::Encoder;
use crate::error::Result;

/// A value that can be encoded to the asun text format.
///
/// Derive with `#[derive(AsunEncode)]`.
pub trait AsunEncode {
    /// Write `self` into the text encode sink.
    fn encode(&self, enc: &mut Encoder) -> Result<()>;
}

/// A value that can be decoded from the asun text format.
///
/// The `'de` lifetime allows zero-copy borrowing of `&'de str` / `&'de [u8]`
/// directly out of the input buffer.
pub trait AsunDecode<'de>: Sized {
    /// Pull a value of `Self` from the text decode source.
    fn decode(dec: &mut Decoder<'de>) -> Result<Self>;
}

/// A value that can be encoded to the asun binary format.
///
/// Derive with `#[derive(AsunEncode)]` (the derive emits both the text and
/// binary encode impls).
pub trait AsunEncodeBinary {
    /// Write `self` into the binary encode sink.
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()>;
}

/// A value that can be decoded from the asun binary format.
///
/// Derive with `#[derive(AsunDecode)]` (the derive emits both the text and
/// binary decode impls).
pub trait AsunDecodeBinary<'de>: Sized {
    /// Pull a value of `Self` from the binary decode source.
    fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self>;
}

// ---------------------------------------------------------------------------
// Built-in text encode impls
// ---------------------------------------------------------------------------

macro_rules! encode_via {
    ($ty:ty, $method:ident) => {
        impl AsunEncode for $ty {
            #[inline]
            fn encode(&self, enc: &mut Encoder) -> Result<()> {
                enc.$method(*self)
            }
        }
    };
}

encode_via!(bool, encode_bool);
encode_via!(i8, encode_i8);
encode_via!(i16, encode_i16);
encode_via!(i32, encode_i32);
encode_via!(i64, encode_i64);
encode_via!(u8, encode_u8);
encode_via!(u16, encode_u16);
encode_via!(u32, encode_u32);
encode_via!(u64, encode_u64);
encode_via!(f32, encode_f32);
encode_via!(f64, encode_f64);
encode_via!(char, encode_char);

impl AsunEncode for str {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_str(self)
    }
}

impl AsunEncode for String {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_str(self)
    }
}

impl<T: AsunEncode + ?Sized> AsunEncode for &T {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        (**self).encode(enc)
    }
}

impl<T: AsunEncode> AsunEncode for Option<T> {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        match self {
            Some(v) => enc.encode_some(v),
            None => enc.encode_none(),
        }
    }
}

impl<T: AsunEncode> AsunEncode for Vec<T> {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_seq(self.as_slice())
    }
}

impl<T: AsunEncode> AsunEncode for [T] {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_seq(self)
    }
}

impl<T: AsunEncode, const N: usize> AsunEncode for [T; N] {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_seq(self.as_slice())
    }
}

impl AsunEncode for () {
    #[inline]
    fn encode(&self, enc: &mut Encoder) -> Result<()> {
        enc.encode_unit()
    }
}

// ---------------------------------------------------------------------------
// Built-in text decode impls
// ---------------------------------------------------------------------------

macro_rules! decode_via {
    ($ty:ty, $method:ident) => {
        impl<'de> AsunDecode<'de> for $ty {
            #[inline]
            fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
                dec.$method()
            }
        }
    };
}

decode_via!(bool, decode_bool);
decode_via!(i8, decode_i8);
decode_via!(i16, decode_i16);
decode_via!(i32, decode_i32);
decode_via!(i64, decode_i64);
decode_via!(u8, decode_u8);
decode_via!(u16, decode_u16);
decode_via!(u32, decode_u32);
decode_via!(u64, decode_u64);
decode_via!(f32, decode_f32);
decode_via!(f64, decode_f64);
decode_via!(char, decode_char);

impl<'de> AsunDecode<'de> for String {
    #[inline]
    fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
        dec.decode_string()
    }
}

impl<'de> AsunDecode<'de> for &'de str {
    #[inline]
    fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
        dec.decode_borrowed_str()
    }
}

impl<'de, T: AsunDecode<'de>> AsunDecode<'de> for Option<T> {
    #[inline]
    fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
        dec.decode_option()
    }
}

impl<'de, T: AsunDecode<'de>> AsunDecode<'de> for Vec<T> {
    #[inline]
    fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
        dec.decode_vec()
    }
}

impl<'de> AsunDecode<'de> for () {
    #[inline]
    fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
        dec.decode_unit()
    }
}

// ---------------------------------------------------------------------------
// Built-in binary encode impls
// ---------------------------------------------------------------------------

macro_rules! bin_encode_via {
    ($ty:ty, $method:ident) => {
        impl AsunEncodeBinary for $ty {
            #[inline]
            fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
                enc.$method(*self)
            }
        }
    };
}

bin_encode_via!(bool, write_bool);
bin_encode_via!(i8, write_i8);
bin_encode_via!(i16, write_i16);
bin_encode_via!(i32, write_i32);
bin_encode_via!(i64, write_i64);
bin_encode_via!(u8, write_u8);
bin_encode_via!(u16, write_u16);
bin_encode_via!(u32, write_u32);
bin_encode_via!(u64, write_u64);
bin_encode_via!(f32, write_f32);
bin_encode_via!(f64, write_f64);
bin_encode_via!(char, write_char);

impl AsunEncodeBinary for str {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        enc.write_str(self)
    }
}

impl AsunEncodeBinary for String {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        enc.write_str(self)
    }
}

impl<T: AsunEncodeBinary + ?Sized> AsunEncodeBinary for &T {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        (**self).encode_binary(enc)
    }
}

impl<T: AsunEncodeBinary> AsunEncodeBinary for Option<T> {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        match self {
            Some(v) => {
                enc.write_u8(1)?;
                v.encode_binary(enc)
            }
            None => enc.write_u8(0),
        }
    }
}

impl<T: AsunEncodeBinary> AsunEncodeBinary for Vec<T> {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        enc.write_seq(self.as_slice())
    }
}

impl<T: AsunEncodeBinary> AsunEncodeBinary for [T] {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        enc.write_seq(self)
    }
}

impl<T: AsunEncodeBinary, const N: usize> AsunEncodeBinary for [T; N] {
    #[inline]
    fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
        enc.write_seq(self.as_slice())
    }
}

impl AsunEncodeBinary for () {
    #[inline]
    fn encode_binary(&self, _enc: &mut BinaryEncoder) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Built-in binary decode impls
// ---------------------------------------------------------------------------

macro_rules! bin_decode_via {
    ($ty:ty, $method:ident) => {
        impl<'de> AsunDecodeBinary<'de> for $ty {
            #[inline]
            fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
                dec.$method()
            }
        }
    };
}

bin_decode_via!(bool, read_bool);
bin_decode_via!(i8, read_i8);
bin_decode_via!(i16, read_i16);
bin_decode_via!(i32, read_i32);
bin_decode_via!(i64, read_i64);
bin_decode_via!(u8, read_u8);
bin_decode_via!(u16, read_u16);
bin_decode_via!(u32, read_u32);
bin_decode_via!(u64, read_u64);
bin_decode_via!(f32, read_f32);
bin_decode_via!(f64, read_f64);
bin_decode_via!(char, read_char);

impl<'de> AsunDecodeBinary<'de> for String {
    #[inline]
    fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
        dec.read_string()
    }
}

impl<'de> AsunDecodeBinary<'de> for &'de str {
    #[inline]
    fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
        dec.read_str_zerocopy()
    }
}

impl<'de, T: AsunDecodeBinary<'de>> AsunDecodeBinary<'de> for Option<T> {
    #[inline]
    fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
        let tag = dec.read_u8()?;
        if tag == 0 {
            Ok(None)
        } else {
            Ok(Some(T::decode_binary(dec)?))
        }
    }
}

impl<'de, T: AsunDecodeBinary<'de>> AsunDecodeBinary<'de> for Vec<T> {
    #[inline]
    fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
        dec.read_vec()
    }
}

impl<'de> AsunDecodeBinary<'de> for () {
    #[inline]
    fn decode_binary(_dec: &mut BinaryDecoder<'de>) -> Result<Self> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tuples
// ---------------------------------------------------------------------------

macro_rules! tuple_impls {
    ($( $len:literal => ( $( $n:tt $T:ident ),+ ) ),+ $(,)?) => {
        $(
            impl<$($T: AsunEncode),+> AsunEncode for ($($T,)+) {
                #[inline]
                fn encode(&self, enc: &mut Encoder) -> Result<()> {
                    enc.begin_tuple()?;
                    $( enc.tuple_element(&self.$n)?; )+
                    enc.end_tuple()
                }
            }

            impl<'de, $($T: AsunDecode<'de>),+> AsunDecode<'de> for ($($T,)+) {
                #[inline]
                fn decode(dec: &mut Decoder<'de>) -> Result<Self> {
                    dec.begin_tuple()?;
                    let out = ( $( { let v = dec.tuple_element::<$T>()?; v }, )+ );
                    dec.end_tuple($len)?;
                    Ok(out)
                }
            }

            impl<$($T: AsunEncodeBinary),+> AsunEncodeBinary for ($($T,)+) {
                #[inline]
                fn encode_binary(&self, enc: &mut BinaryEncoder) -> Result<()> {
                    $( self.$n.encode_binary(enc)?; )+
                    Ok(())
                }
            }

            impl<'de, $($T: AsunDecodeBinary<'de>),+> AsunDecodeBinary<'de> for ($($T,)+) {
                #[inline]
                fn decode_binary(dec: &mut BinaryDecoder<'de>) -> Result<Self> {
                    Ok(( $( $T::decode_binary(dec)?, )+ ))
                }
            }
        )+
    };
}

tuple_impls! {
    1 => (0 T0),
    2 => (0 T0, 1 T1),
    3 => (0 T0, 1 T1, 2 T2),
    4 => (0 T0, 1 T1, 2 T2, 3 T3),
    5 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4),
    6 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5),
    7 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6),
    8 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7),
    9 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7, 8 T8),
    10 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7, 8 T8, 9 T9),
    11 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7, 8 T8, 9 T9, 10 T10),
    12 => (0 T0, 1 T1, 2 T2, 3 T3, 4 T4, 5 T5, 6 T6, 7 T7, 8 T8, 9 T9, 10 T10, 11 T11),
}
