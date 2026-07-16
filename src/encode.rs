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

/// Write u64 to buffer. Uses `itoap` (community-maintained itoa optimised
/// with 8-digit-at-a-time SWAR techniques); benchmarks ~10-15% faster than
/// the in-tree two-digit-at-a-time formatter on long integers and matches
/// it on short ones.
#[inline(always)]
fn write_u64(buf: &mut Vec<u8>, v: u64) {
    itoap::write_to_vec(buf, v);
}

/// Write i64 to buffer. See `write_u64` for the rationale on `itoap`.
#[inline(always)]
fn write_i64(buf: &mut Vec<u8>, v: i64) {
    itoap::write_to_vec(buf, v);
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
    // Any leading/trailing ASCII whitespace must force quoting (SPEC §S2 trim).
    let first = bytes[0];
    let last = bytes[bytes.len() - 1];
    if matches!(first, b' ' | b'\t' | b'\n' | b'\r') || matches!(last, b' ' | b'\t' | b'\n' | b'\r')
    {
        return true;
    }
    // Bool / null lookalikes.
    if matches!(
        bytes,
        b"true" | b"false" | b"True" | b"False" | b"TRUE" | b"FALSE"
    ) {
        return true;
    }

    // SIMD fast-path: check for ASUN special chars in bulk
    // (covers space, control, structural, comment-introducing, etc.)
    if simd::simd_has_special_chars(bytes) {
        return true;
    }

    // Number-pattern check: only relevant when the first byte could plausibly
    // begin a number literal. For strings starting with a letter or any other
    // non-numeric byte, the whole-string pattern match cannot succeed, so we
    // skip the inner loop entirely. This is the common case for ASCII names,
    // emails (already caught by '@' above), tags, etc.
    if !matches!(first, b'-' | b'+' | b'0'..=b'9' | b'.') {
        return false;
    }

    // Number-pattern check: anything decoder might re-read as a number.
    // Accepts optional sign, digits, decimal point, scientific exponent.
    let mut i = 0;
    if first == b'-' || first == b'+' {
        i = 1;
    }
    let mut saw_digit = false;
    let mut saw_dot = false;
    let mut saw_exp = false;
    let mut number_like = true;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_digit() {
            saw_digit = true;
        } else if b == b'.' && !saw_dot && !saw_exp {
            saw_dot = true;
        } else if (b == b'e' || b == b'E') && saw_digit && !saw_exp {
            saw_exp = true;
            if i + 1 < bytes.len() && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-') {
                i += 1;
            }
            saw_digit = false;
        } else {
            number_like = false;
            break;
        }
        i += 1;
    }
    if number_like && saw_digit {
        return true;
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
    /// Set by `SeqEncoder` after the first struct element has been seen, so
    /// that the remaining rows of a homogeneous `Vec<Struct>` can skip the
    /// per-row schema bookkeeping (allocations + bubble-up build) entirely.
    /// The schema captured from the first row is reused for every subsequent
    /// row in the same sequence.
    skip_schema_capture: bool,
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
        skip_schema_capture: false,
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
        skip_schema_capture: false,
    };
    value.serialize(&mut serializer)?;
    Ok(unsafe { String::from_utf8_unchecked(serializer.buf) })
}

/// Per GRAMMAR.abnf `bare-field-name = 1*( ALPHA / DIGIT / "_" )`. Anything
/// outside that set forces a quoted-string field name, plus:
///   - bare reserved words `true` / `false` (would re-decode as booleans);
///   - all-digit names (cross-implementation compatibility — some decoders
///     are stricter than the grammar and reject digit-only bare names).
///
/// Hot path: gets called once per field per `encode()`. For 16-field
/// structs it accounts for >10 % of total encode time, so it's hand-tuned to
/// a single byte-scan with no per-byte trait dispatch.
#[inline]
fn schema_field_name_needs_quotes(name: &str) -> bool {
    let bytes = name.as_bytes();
    let n = bytes.len();
    if n == 0 {
        return true;
    }

    // Single byte-scan: every byte must be ALPHA / DIGIT / `_`. Anything else
    // (including space, control chars, structural punctuation) needs quoting.
    // While scanning, also track whether the name is purely digit-only so we
    // can quote those for cross-implementation safety.
    let mut all_digits = true;
    let mut i = 0;
    while i < n {
        let b = bytes[i];
        let is_digit = b.is_ascii_digit();
        let is_alpha = b.is_ascii_uppercase() || b.is_ascii_lowercase();
        if !(is_alpha || is_digit || b == b'_') {
            return true;
        }
        if !is_digit {
            all_digits = false;
        }
        i += 1;
    }
    if all_digits {
        return true;
    }

    // Reserved keywords (would be re-parsed as booleans).
    matches!(bytes, b"true" | b"false")
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
        if self.typed && self.current_type_hint.is_none() {
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
        if self.typed && self.current_type_hint.is_none() {
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
        if self.typed && self.current_type_hint.is_none() {
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
        // ASUN text has no representation for NaN/±Infinity, and the decoder
        // rejects them, so encoding one would produce output that cannot
        // round-trip. Reject at encode time (matching serde_json's default).
        if !v.is_finite() {
            return Err(Error::Message(
                "cannot serialize non-finite float (NaN/Infinity)".into(),
            ));
        }
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
            self.current_type_hint = Some("float");
        }
        write_f64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    fn serialize_char(self, v: char) -> Result<()> {
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
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
        if self.typed && self.current_type_hint.is_none() {
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
                cached_nested_schema: None,
                skip_was_set: false,
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
                cached_nested_schema: None,
                skip_was_set: false,
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
        // Skip per-row schema bookkeeping for the 2nd+ rows of a homogeneous
        // Vec<Struct>. The first row populated `top_seq_fields/_types/_schemas`,
        // and subsequent rows produce the same schema fragment by construction.
        let skip = self.skip_schema_capture;
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
                skip_schema: false,
                first: true,
            })
        } else {
            self.push_separator();
            self.buf.push(b'(');
            // When skipping, allocate empty Vecs (no capacity) — they won't be
            // pushed into. This keeps the struct field types stable while
            // avoiding per-row 3 × len allocations.
            let (fields, field_types, field_schemas) = if skip {
                (Vec::new(), Vec::new(), Vec::new())
            } else {
                (
                    Vec::with_capacity(len),
                    Vec::with_capacity(len),
                    Vec::with_capacity(len),
                )
            };
            Ok(StructEncoder {
                ser: self,
                fields,
                field_types,
                field_schemas,
                is_top: false,
                capture_for_seq,
                skip_schema: skip,
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
            skip_schema: false,
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
    /// For nested Vec<Struct>: schema fragment captured from row 1 so we can
    /// restore it after later rows in skip-mode wipe `nested_schema`.
    cached_nested_schema: Option<Vec<u8>>,
    /// Tracks whether *this* seq is the one that asserted the encoder's
    /// `skip_schema_capture` flag. Without this we'd reset on `end()` even
    /// when an outer seq owns the flag (e.g. inner primitive `Vec<i64>`
    /// running while the outer `Vec<Struct>` is still iterating).
    skip_was_set: bool,
}

impl<'a> ser::SerializeSeq for SeqEncoder<'a> {
    type Ok = ();
    type Error = Error;

    #[inline]
    fn serialize_element<T: ?Sized + Serialize>(&mut self, value: &T) -> Result<()> {
        if !self.first {
            self.ser.buf.push(b',');
        }
        let was_first = self.first;
        self.first = false;
        self.ser.first = true;
        let result = value.serialize(&mut *self.ser);
        // After the first homogeneous struct row of a top-level seq has been
        // serialized, fields/types/schemas are cached on the encoder. Tell
        // subsequent rows to skip per-row schema bookkeeping.
        if was_first && self.is_top_seq && self.ser.top_seq_fields.is_some() {
            self.ser.skip_schema_capture = true;
            self.skip_was_set = true;
        }
        // For nested Vec<Struct>: row 1's StructEncoder::end() bubbled up a
        // schema fragment via `nested_schema`. Stash it on the seq so we can
        // restore it after this seq ends, and ask later rows to skip rebuild.
        if was_first && !self.is_top_seq && self.ser.nested_schema.is_some() {
            self.cached_nested_schema = self.ser.nested_schema.clone();
            self.ser.skip_schema_capture = true;
            self.skip_was_set = true;
        }
        result
    }

    #[inline]
    fn end(mut self) -> Result<()> {
        // Only reset the encoder's `skip_schema_capture` if WE were the ones
        // who set it. Without this guard, a nested primitive seq (e.g. a
        // `Vec<i64>` field on row 2 of a top-level `Vec<Struct>`) would
        // clobber the outer seq's skip flag and force every later row of the
        // outer seq to redo schema bookkeeping. That bug wasted >10 % of
        // total encode time for 16-field structs.
        if self.skip_was_set {
            self.ser.skip_schema_capture = false;
        }
        // Restore the nested schema captured from row 1 (skip-mode wiped it).
        if let Some(cached) = self.cached_nested_schema.take() {
            self.ser.nested_schema = Some(cached);
        }
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
                    } else if self.ser.typed
                        && let Some(ref field_types) = self.ser.top_seq_field_types
                        && let Some(Some(type_hint)) = field_types.get(i)
                    {
                        out.push(b'@');
                        out.extend_from_slice(type_hint.as_bytes());
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
            // The schema-fragment bubble-up below feeds the parent struct's
            // schema header. When the encoder is in skip-schema mode (rows
            // 2+ of a homogeneous Vec<Struct>) the parent will discard
            // anything we put here, so there's no need to allocate the
            // `[...]` wrapper Vec at all.
            if self.ser.skip_schema_capture {
                self.ser.nested_schema = None;
                if self.ser.typed {
                    self.ser.current_type_hint = None;
                }
            } else if let Some(schema) = self.ser.nested_schema.take() {
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
    /// True for the 2nd+ row of a homogeneous Vec<Struct>: skip recording field
    /// names / types / nested schemas, since the seq's first row already did.
    skip_schema: bool,
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
        if !self.skip_schema {
            // Capture field names + per-field hint state only when this struct
            // will actually emit a schema header / fragment.
            self.fields.push(key);
            if self.ser.typed {
                self.ser.current_type_hint = None;
            }
            self.ser.nested_schema = None;
        }

        if !self.first {
            self.ser.buf.push(b',');
        }
        self.first = false;
        self.ser.first = true;
        self.ser.in_tuple = true;
        value.serialize(&mut *self.ser)?;

        if !self.skip_schema {
            self.field_schemas.push(self.ser.nested_schema.take());
            if self.ser.typed {
                self.field_types.push(self.ser.current_type_hint.take());
            }
        } else {
            // Discard transient state nested serializers may have set; we are
            // not using it.
            self.ser.nested_schema = None;
            if self.ser.typed {
                self.ser.current_type_hint = None;
            }
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
                } else if self.ser.typed
                    && let Some(type_hint) = self.field_types.get(i).and_then(|t| *t)
                {
                    out.push(b'@');
                    out.extend_from_slice(type_hint.as_bytes());
                }
            }
            out.extend_from_slice(b"}:");
            out.append(&mut data);
            self.ser.buf = out;
        } else if self.skip_schema {
            // Homogeneous Vec<Struct> non-first row: only the data tuple was
            // emitted. No header bubble-up to do.
            self.ser.buf.push(b')');
            self.ser.first = false;
            if self.ser.typed {
                self.ser.current_type_hint = None;
            }
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
                    } else if self.ser.typed
                        && let Some(type_hint) = self.field_types.get(i).and_then(|t| *t)
                    {
                        schema.push(b'@');
                        schema.extend_from_slice(type_hint.as_bytes());
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
