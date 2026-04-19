use crate::error::{Error, Result};
use crate::simd;
use serde::ser::{self, Serialize};

// ---------------------------------------------------------------------------
// Lookup tables
// ---------------------------------------------------------------------------

/// Two-digit lookup table for fast integer formatting (itoa-style).
static DEC_DIGITS: &[u8; 200] = b"0001020304050607080910111213141516171819\
2021222324252627282930313233343536373839\
4041424344454647484950515253545556575859\
6061626364656667686970717273747576777879\
8081828384858687888990919293949596979899";

// ---------------------------------------------------------------------------
// Stack-based number formatting (no heap allocation)
// ---------------------------------------------------------------------------

/// Write u64 — delegates to SIMD module's optimized version.
#[inline(always)]
fn write_u64(buf: &mut Vec<u8>, v: u64) {
    simd::fast_write_u64(buf, v);
}

/// Write i64 — delegates to SIMD module's optimized version.
#[inline(always)]
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    simd::fast_write_i64(buf, v);
}

/// Write f64 to buffer using `ryu` for fast float formatting.
/// - Integer-valued floats: fast path via write_i64 + ".0"
/// - One-decimal floats (e.g. 50.5): fast path via integer arithmetic
/// - General: ryu (Ryū algorithm) for fast, accurate float-to-string
#[inline]
fn write_f64(buf: &mut Vec<u8>, v: f64) {
    if v.is_finite() && v.fract() == 0.0 {
        if v >= i64::MIN as f64 && v <= i64::MAX as f64 {
            write_i64(buf, v as i64);
            buf.extend_from_slice(b".0");
        } else {
            ryu_f64(buf, v);
        }
        return;
    }
    if v.is_finite() {
        // Fast path: one decimal place (covers xx.5, xx.1, etc.)
        let v10 = v * 10.0;
        if v10.fract() == 0.0 && v10.abs() < 1e18 {
            let vi = v10 as i64;
            let (int_part, frac) = if vi < 0 {
                buf.push(b'-');
                let pos = (-vi) as u64;
                ((pos / 10), (pos % 10) as u8)
            } else {
                let pos = vi as u64;
                ((pos / 10), (pos % 10) as u8)
            };
            write_u64(buf, int_part);
            buf.push(b'.');
            buf.push(b'0' + frac);
            return;
        }
        // Fast path: two decimal places (covers xx.25, xx.75, etc.)
        let v100 = v * 100.0;
        if v100.fract() == 0.0 && v100.abs() < 1e18 {
            let vi = v100 as i64;
            let (int_part, frac) = if vi < 0 {
                buf.push(b'-');
                let pos = (-vi) as u64;
                ((pos / 100), (pos % 100) as usize)
            } else {
                let pos = vi as u64;
                ((pos / 100), (pos % 100) as usize)
            };
            write_u64(buf, int_part);
            buf.push(b'.');
            buf.push(DEC_DIGITS[frac * 2]);
            let d2 = DEC_DIGITS[frac * 2 + 1];
            if d2 != b'0' {
                buf.push(d2);
            }
            return;
        }
    }
    ryu_f64(buf, v);
}

/// Fast float formatting using the Ryū algorithm (via `ryu` crate).
#[inline]
fn ryu_f64(buf: &mut Vec<u8>, v: f64) {
    let mut b = ryu::Buffer::new();
    let s = b.format(v);
    buf.extend_from_slice(s.as_bytes());
}

// ---------------------------------------------------------------------------
// String quoting / escaping
// ---------------------------------------------------------------------------

/// Single-pass check: does `s` need to be wrapped in quotes?
/// Uses SIMD to scan for special chars in 16-byte chunks.
#[inline]
fn needs_quoting(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return true;
    }
    if bytes[0] == b' ' || bytes[bytes.len() - 1] == b' ' {
        return true;
    }
    if (bytes.len() == 4 && bytes == b"true") || (bytes.len() == 5 && bytes == b"false") {
        return true;
    }

    // SIMD fast-path: check for ASUN special chars in bulk
    if simd::simd_has_special_chars(bytes) {
        return true;
    }

    // Check if it looks like a number (would be ambiguous as a bare value)
    let num_start = if bytes[0] == b'-' { 1 } else { 0 };
    if num_start < bytes.len() {
        let mut could_be_number = true;
        for i in num_start..bytes.len() {
            if !bytes[i].is_ascii_digit() && bytes[i] != b'.' {
                could_be_number = false;
                break;
            }
        }
        if could_be_number {
            return true;
        }
    }
    false
}

/// Write `s` wrapped in quotes with escaping using SIMD-accelerated scanning.
#[inline]
fn write_escaped(buf: &mut Vec<u8>, s: &str) {
    simd::simd_write_escaped(buf, s.as_bytes());
}

// ---------------------------------------------------------------------------
// Serializer
// ---------------------------------------------------------------------------

pub struct Encoder {
    pub(crate) buf: Vec<u8>,
    in_tuple: bool,
    first: bool,
    /// When true, record type hints for top-level struct fields.
    typed: bool,
    /// Accumulates type hint for the current field being serialized.
    current_type_hint: Option<&'static str>,
    /// Top-level seq (Vec<Struct>) support
    in_top_seq: bool,
    top_seq_data_start: usize,
    top_seq_fields: Option<Vec<&'static str>>,
    top_seq_field_types: Option<Vec<Option<&'static str>>>,
    top_seq_field_schemas: Option<Vec<Option<Vec<u8>>>>,
    /// Schema fragment bubbled up from nested struct/seq-of-struct serializers.
    nested_schema: Option<Vec<u8>>,
}

pub fn encode<T: Serialize>(value: &T) -> Result<String> {
    let mut serializer = Encoder {
        buf: Vec::with_capacity(256),
        in_tuple: false,
        first: true,
        typed: false,
        current_type_hint: None,
        in_top_seq: false,
        top_seq_data_start: 0,
        top_seq_fields: None,
        top_seq_field_types: None,
        top_seq_field_schemas: None,
        nested_schema: None,
    };
    value.serialize(&mut serializer)?;
    Ok(unsafe { String::from_utf8_unchecked(serializer.buf) })
}

/// Serialize a single struct to ASUN string with type-annotated schema.
///
/// Output example: `{id@int,name@str,active@bool}:(1,Alice,true)`
pub fn encode_typed<T: Serialize>(value: &T) -> Result<String> {
    let mut serializer = Encoder {
        buf: Vec::with_capacity(256),
        in_tuple: false,
        first: true,
        typed: true,
        current_type_hint: None,
        in_top_seq: false,
        top_seq_data_start: 0,
        top_seq_fields: None,
        top_seq_field_types: None,
        top_seq_field_schemas: None,
        nested_schema: None,
    };
    value.serialize(&mut serializer)?;
    Ok(unsafe { String::from_utf8_unchecked(serializer.buf) })
}

fn schema_field_name_needs_quotes(name: &str) -> bool {
    if name.is_empty() {
        return true;
    }
    if name == "true" || name == "false" {
        return true;
    }
    if name.starts_with(' ') || name.ends_with(' ') {
        return true;
    }
    let mut could_be_number = true;
    let num_start = if name.as_bytes()[0] == b'-' { 1 } else { 0 };
    if num_start >= name.len() {
        could_be_number = false;
    }
    for (idx, &b) in name.as_bytes().iter().enumerate() {
        if b <= 0x20
            || matches!(
                b,
                b',' | b'@' | b':' | b'{' | b'}' | b'[' | b']' | b'(' | b')' | b'"' | b'\\'
            )
        {
            return true;
        }
        if could_be_number && idx >= num_start && !b.is_ascii_digit() && b != b'.' {
            could_be_number = false;
        }
    }
    could_be_number && name.len() > num_start
}

fn push_schema_field_name(buf: &mut Vec<u8>, name: &str) {
    if !schema_field_name_needs_quotes(name) {
        buf.extend_from_slice(name.as_bytes());
        return;
    }
    buf.push(b'"');
    for &b in name.as_bytes() {
        match b {
            b'"' => buf.extend_from_slice(br#"\""#),
            b'\\' => buf.extend_from_slice(br#"\\"#),
            b'\n' => buf.extend_from_slice(br#"\n"#),
            b'\r' => buf.extend_from_slice(br#"\r"#),
            b'\t' => buf.extend_from_slice(br#"\t"#),
            0x08 => buf.extend_from_slice(br#"\b"#),
            0x0c => buf.extend_from_slice(br#"\f"#),
            _ => buf.push(b),
        }
    }
    buf.push(b'"');
}

impl Encoder {
    #[inline(always)]
    fn push_separator(&mut self) {
        if !self.first {
            self.buf.push(b',');
        }
        self.first = false;
    }

    #[inline(always)]
    fn reserve_for_seq(&mut self, len: usize, top_level: bool) {
        let per_item = if top_level { 64 } else { 24 };
        self.buf.reserve(len.saturating_mul(per_item) + 8);
    }

    #[inline(always)]
    fn reserve_for_struct(&mut self, field_count: usize, top_level: bool) {
        let per_field = if top_level { 24 } else { 12 };
        self.buf.reserve(field_count.saturating_mul(per_field) + 8);
    }
}

impl<'a> ser::Serializer for &'a mut Encoder {
    type Ok = ();
    type Error = Error;

    type SerializeSeq = SeqEncoder<'a>;
    type SerializeTuple = TupleEncoder<'a>;
    type SerializeTupleStruct = TupleEncoder<'a>;
    type SerializeTupleVariant = TupleEncoder<'a>;
    type SerializeMap = ser::Impossible<(), Error>;
    type SerializeStruct = StructEncoder<'a>;
    type SerializeStructVariant = StructEncoder<'a>;

    #[inline]
    fn serialize_bool(self, v: bool) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("bool");
        }
        self.buf
            .extend_from_slice(if v { b"true" } else { b"false" });
        Ok(())
    }

    #[inline]
    fn serialize_i8(self, v: i8) -> Result<()> {
        self.serialize_i64(v as i64)
    }
    #[inline]
    fn serialize_i16(self, v: i16) -> Result<()> {
        self.serialize_i64(v as i64)
    }
    #[inline]
    fn serialize_i32(self, v: i32) -> Result<()> {
        self.serialize_i64(v as i64)
    }

    #[inline]
    fn serialize_i64(self, v: i64) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("int");
        }
        write_i64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    fn serialize_u8(self, v: u8) -> Result<()> {
        self.serialize_u64(v as u64)
    }
    #[inline]
    fn serialize_u16(self, v: u16) -> Result<()> {
        self.serialize_u64(v as u64)
    }
    #[inline]
    fn serialize_u32(self, v: u32) -> Result<()> {
        self.serialize_u64(v as u64)
    }

    #[inline]
    fn serialize_u64(self, v: u64) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("int");
        }
        write_u64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    fn serialize_f32(self, v: f32) -> Result<()> {
        self.serialize_f64(v as f64)
    }

    #[inline]
    fn serialize_f64(self, v: f64) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("float");
        }
        write_f64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("str");
        }
        let mut tmp = [0u8; 4];
        let s = v.encode_utf8(&mut tmp);
        self.buf.extend_from_slice(s.as_bytes());
        Ok(())
    }

    #[inline]
    fn serialize_str(self, v: &str) -> Result<()> {
        self.push_separator();
        if self.current_type_hint.is_none() && self.typed {
            self.current_type_hint = Some("str");
        }
        if needs_quoting(v) {
            write_escaped(&mut self.buf, v);
        } else {
            self.buf.extend_from_slice(v.as_bytes());
        }
        Ok(())
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<()> {
        self.push_separator();
        self.buf.push(b'[');
        for (i, &b) in v.iter().enumerate() {
            if i > 0 {
                self.buf.push(b',');
            }
            write_u64(&mut self.buf, b as u64);
        }
        self.buf.push(b']');
        Ok(())
    }

    #[inline]
    fn serialize_none(self) -> Result<()> {
        self.push_separator();
        // For typed mode: None doesn't set a type hint (the Some branch will)
        Ok(())
    }

    #[inline]
    fn serialize_some<T: ?Sized + Serialize>(self, value: &T) -> Result<()> {
        value.serialize(self)
    }

    #[inline]
    fn serialize_unit(self) -> Result<()> {
        self.push_separator();
        self.buf.extend_from_slice(b"()");
        Ok(())
    }

    #[inline]
    fn serialize_unit_struct(self, _name: &'static str) -> Result<()> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
    ) -> Result<()> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<()> {
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: ?Sized + Serialize>(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<()> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        self.buf.push(b',');
        self.first = true;
        value.serialize(&mut *self)?;
        self.buf.push(b')');
        Ok(())
    }

    fn serialize_seq(self, len: Option<usize>) -> Result<SeqEncoder<'a>> {
        if !self.in_tuple {
            // Top-level seq: Vec<T> — defer format until we know element types
            if let Some(len) = len {
                self.reserve_for_seq(len, true);
            }
            self.in_top_seq = true;
            self.in_tuple = true;
            self.top_seq_data_start = self.buf.len();
            self.top_seq_fields = None;
            self.top_seq_field_types = None;
            Ok(SeqEncoder {
                ser: self,
                first: true,
                is_top_seq: true,
            })
        } else {
            if let Some(len) = len {
                self.reserve_for_seq(len, false);
            }
            self.push_separator();
            self.buf.push(b'[');
            Ok(SeqEncoder {
                ser: self,
                first: true,
                is_top_seq: false,
            })
        }
    }

    fn serialize_tuple(self, _len: usize) -> Result<TupleEncoder<'a>> {
        self.push_separator();
        self.buf.push(b'(');
        Ok(TupleEncoder {
            ser: self,
            first: true,
        })
    }

    fn serialize_tuple_struct(self, _name: &'static str, _len: usize) -> Result<TupleEncoder<'a>> {
        self.push_separator();
        self.buf.push(b'(');
        Ok(TupleEncoder {
            ser: self,
            first: true,
        })
    }

    fn serialize_tuple_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<TupleEncoder<'a>> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        Ok(TupleEncoder {
            ser: self,
            first: false,
        })
    }

    fn serialize_map(self, _len: Option<usize>) -> Result<ser::Impossible<(), Error>> {
        Err(Error::Message("map fields are not supported".into()))
    }

    fn serialize_struct(self, _name: &'static str, len: usize) -> Result<StructEncoder<'a>> {
        let is_top = !self.in_tuple;
        let capture_for_seq = !is_top && self.in_top_seq && self.top_seq_fields.is_none();
        self.reserve_for_struct(len, is_top);
        if is_top {
            self.buf.push(b'(');
            self.in_tuple = true;
            Ok(StructEncoder {
                ser: self,
                fields: Vec::with_capacity(len),
                field_types: Vec::with_capacity(len),
                field_schemas: Vec::with_capacity(len),
                is_top: true,
                capture_for_seq: false,
                first: true,
            })
        } else {
            self.push_separator();
            self.buf.push(b'(');
            Ok(StructEncoder {
                ser: self,
                fields: Vec::with_capacity(len),
                field_types: Vec::with_capacity(len),
                field_schemas: Vec::with_capacity(len),
                is_top: false,
                capture_for_seq,
                first: true,
            })
        }
    }

    fn serialize_struct_variant(
        self,
        _name: &'static str,
        _variant_index: u32,
        variant: &'static str,
        _len: usize,
    ) -> Result<StructEncoder<'a>> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        self.buf.push(b',');
        Ok(StructEncoder {
            ser: self,
            fields: Vec::new(),
            field_types: Vec::new(),
            field_schemas: Vec::new(),
            is_top: false,
            capture_for_seq: false,
            first: true,
        })
    }
}

// ---------------------------------------------------------------------------
// SeqSerializer
// ---------------------------------------------------------------------------

pub struct SeqEncoder<'a> {
    ser: &'a mut Encoder,
    first: bool,
    is_top_seq: bool,
}

impl<'a> ser::SerializeSeq for SeqEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        if !self.first {
            self.ser.buf.push(b',');
        }
        self.first = false;
        self.ser.first = true;
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(self) -> Result<()> {
        if self.is_top_seq {
            if let Some(ref fields) = self.ser.top_seq_fields {
                // Struct elements: build header once, then append the already
                // serialized data buffer in a single pass.
                let mut data = core::mem::take(&mut self.ser.buf);
                let mut out = Vec::with_capacity(data.len() + fields.len() * 16 + 8);
                out.extend_from_slice(b"[{");
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(f.as_bytes());
                    // Nested schema takes priority over type hint
                    let has_nested = self
                        .ser
                        .top_seq_field_schemas
                        .as_ref()
                        .and_then(|schemas| schemas.get(i))
                        .and_then(|s| s.as_ref());
                    if let Some(schema) = has_nested {
                        out.push(b'@');
                        out.extend_from_slice(schema);
                    } else if self.ser.typed {
                        if let Some(ref field_types) = self.ser.top_seq_field_types {
                            if let Some(Some(type_hint)) = field_types.get(i) {
                                out.push(b'@');
                                out.extend_from_slice(type_hint.as_bytes());
                            }
                        }
                    }
                }
                out.extend_from_slice(b"}]:");
                out.append(&mut data);
                self.ser.buf = out;
            } else {
                // Non-struct elements (primitive Vec): wrap in [...]
                let mut data = core::mem::take(&mut self.ser.buf);
                let mut out = Vec::with_capacity(data.len() + 2);
                out.push(b'[');
                out.append(&mut data);
                out.push(b']');
                self.ser.buf = out;
            }
            self.ser.in_top_seq = false;
        } else {
            self.ser.buf.push(b']');
            // If elements were structs, wrap their schema in [...] and bubble up
            if let Some(schema) = self.ser.nested_schema.take() {
                let mut wrapped = Vec::with_capacity(schema.len() + 2);
                wrapped.push(b'[');
                wrapped.extend_from_slice(&schema);
                wrapped.push(b']');
                self.ser.nested_schema = Some(wrapped);
            } else if let Some(hint) = self.ser.current_type_hint.take() {
                // Primitive vec fields keep a structural scaffold even when
                // scalar element types are optional.
                let mut wrapped = Vec::with_capacity(hint.len() + 2);
                wrapped.push(b'[');
                wrapped.extend_from_slice(hint.as_bytes());
                wrapped.push(b']');
                self.ser.nested_schema = Some(wrapped);
            } else {
                self.ser.nested_schema = Some(b"[]".to_vec());
            }
        }
        self.ser.first = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TupleSerializer
// ---------------------------------------------------------------------------

pub struct TupleEncoder<'a> {
    ser: &'a mut Encoder,
    first: bool,
}

impl<'a> ser::SerializeTuple for TupleEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        if !self.first {
            self.ser.buf.push(b',');
        }
        self.first = false;
        self.ser.first = true;
        value.serialize(&mut *self.ser)
    }

    #[inline]
    fn end(self) -> Result<()> {
        self.ser.buf.push(b')');
        self.ser.first = false;
        Ok(())
    }
}

impl<'a> ser::SerializeTupleStruct for TupleEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        ser::SerializeTuple::serialize_element(self, value)
    }

    #[inline]
    fn end(self) -> Result<()> {
        ser::SerializeTuple::end(self)
    }
}

impl<'a> ser::SerializeTupleVariant for TupleEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        ser::SerializeTuple::serialize_element(self, value)
    }

    #[inline]
    fn end(self) -> Result<()> {
        ser::SerializeTuple::end(self)
    }
}

// ---------------------------------------------------------------------------
// StructSerializer
// ---------------------------------------------------------------------------

pub struct StructEncoder<'a> {
    ser: &'a mut Encoder,
    fields: Vec<&'static str>,
    /// Type hints collected for each field (only when typed mode is on)
    field_types: Vec<Option<&'static str>>,
    /// Nested schema fragments for struct/vec-of-struct fields
    field_schemas: Vec<Option<Vec<u8>>>,
    is_top: bool,
    capture_for_seq: bool,
    first: bool,
}

impl<'a> ser::SerializeStruct for StructEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        // Always capture field names for recursive schema generation
        self.fields.push(key);
        if self.ser.typed {
            self.ser.current_type_hint = None;
        }
        // Clear nested schema before serializing value
        self.ser.nested_schema = None;

        if !self.first {
            self.ser.buf.push(b',');
        }
        self.first = false;
        self.ser.first = true;
        self.ser.in_tuple = true;
        value.serialize(&mut *self.ser)?;

        // Capture nested schema (set by nested StructEncoder or SeqEncoder)
        self.field_schemas.push(self.ser.nested_schema.take());
        if self.ser.typed {
            self.field_types.push(self.ser.current_type_hint.take());
        }
        Ok(())
    }

    fn end(self) -> Result<()> {
        if self.is_top {
            self.ser.buf.push(b')');
            // Build top-level header once, then append the tuple payload.
            let mut data = core::mem::take(&mut self.ser.buf);
            let mut out = Vec::with_capacity(data.len() + self.fields.len() * 16 + 4);
            out.push(b'{');
            for (i, f) in self.fields.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                push_schema_field_name(&mut out, f);
                // Nested schema takes priority over type hint
                if let Some(Some(schema)) = self.field_schemas.get(i) {
                    out.push(b'@');
                    out.extend_from_slice(schema);
                } else if self.ser.typed {
                    if let Some(type_hint) = self.field_types.get(i).and_then(|t| *t) {
                        out.push(b'@');
                        out.extend_from_slice(type_hint.as_bytes());
                    }
                }
            }
            out.extend_from_slice(b"}:");
            out.append(&mut data);
            self.ser.buf = out;
        } else {
            self.ser.buf.push(b')');
            self.ser.first = false;
            if self.capture_for_seq {
                self.ser.top_seq_fields = Some(self.fields);
                self.ser.top_seq_field_schemas = Some(self.field_schemas);
                if self.ser.typed {
                    self.ser.top_seq_field_types = Some(self.field_types);
                }
            } else {
                // Build schema fragment for parent to consume
                let mut schema = Vec::with_capacity(64);
                schema.push(b'{');
                for (i, f) in self.fields.iter().enumerate() {
                    if i > 0 {
                        schema.push(b',');
                    }
                    push_schema_field_name(&mut schema, f);
                    if let Some(Some(nested)) = self.field_schemas.get(i) {
                        schema.push(b'@');
                        schema.extend_from_slice(nested);
                    } else if self.ser.typed {
                        if let Some(type_hint) = self.field_types.get(i).and_then(|t| *t) {
                            schema.push(b'@');
                            schema.extend_from_slice(type_hint.as_bytes());
                        }
                    }
                }
                schema.push(b'}');
                self.ser.nested_schema = Some(schema);
            }
            if self.ser.typed {
                self.ser.current_type_hint = None;
            }
        }
        Ok(())
    }
}

impl<'a> ser::SerializeStructVariant for StructEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_field<T: ?Sized + Serialize>(
        &mut self,
        _key: &'static str,
        value: &T,
    ) -> Result<()> {
        if !self.first {
            self.ser.buf.push(b',');
        }
        self.first = false;
        self.ser.first = true;
        value.serialize(&mut *self.ser)
    }

    fn end(self) -> Result<()> {
        self.ser.buf.push(b')');
        self.ser.first = false;
        Ok(())
    }
}
