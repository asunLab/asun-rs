//! ASUN text encoding.
//!
//! The entry points are the free functions [`encode`] (plain schema) and
//! [`encode_typed`] (schema with scalar type hints); most users only need those
//! plus `#[derive(AsunEncode)]`.
//!
//! [`Encoder`] and its helper sinks ([`SeqEncoder`], [`TupleEncoder`],
//! [`StructEncoder`]) are the low-level machinery the derive macro drives. They
//! are `pub` so derive-generated code in downstream crates can reach them via
//! `::asun::encode::...`; you rarely need to construct or call them by hand.

use crate::error::{Error, Result};
use crate::simd;
use crate::traits::AsunEncode;

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

/// Above this magnitude consecutive integers are no longer exactly
/// representable, so a plain decimal is not necessarily the shortest
/// round-tripping form.
const EXACT_INT_LIMIT: f64 = 9_007_199_254_740_992.0; // 2^53

#[inline(always)]
fn split_sign(k: i64) -> (bool, u64) {
    if k < 0 {
        (true, k.unsigned_abs())
    } else {
        (false, k as u64)
    }
}

/// Write `k / 10` as a plain decimal.
#[inline]
fn write_one_decimal(buf: &mut Vec<u8>, k: i64) {
    let (neg, mag) = split_sign(k);
    if neg {
        buf.push(b'-');
    }
    write_u64(buf, mag / 10);
    buf.push(b'.');
    buf.push(b'0' + (mag % 10) as u8);
}

/// Write `k / 100` as a plain decimal, dropping a trailing zero.
#[inline]
fn write_two_decimals(buf: &mut Vec<u8>, k: i64) {
    let (neg, mag) = split_sign(k);
    if neg {
        buf.push(b'-');
    }
    write_u64(buf, mag / 100);
    buf.push(b'.');
    let f = (mag % 100) as u8;
    buf.push(b'0' + f / 10);
    let last = f % 10;
    if last != 0 {
        buf.push(b'0' + last);
    }
}

/// Write f64 to buffer.
///
/// The integer and short-decimal paths avoid `ryu` for the shapes that dominate
/// real payloads (counts, prices, scores). They used to be applied on the
/// strength of `(v * 10.0).fract() == 0.0` alone, which is *not* sufficient:
/// a fuzz sweep found roughly one finite double in 1600 decoding back to a
/// different value. Dividing the scaled integer back is correctly rounded, so
/// `k as f64 / scale == v` proves the digits we are about to print parse back
/// to exactly `v`.
#[inline]
fn write_f64(buf: &mut Vec<u8>, v: f64) {
    if v.abs() < EXACT_INT_LIMIT {
        if v.fract() == 0.0 {
            if v == 0.0 && v.is_sign_negative() {
                buf.extend_from_slice(b"-0.0");
                return;
            }
            write_i64(buf, v as i64);
            buf.extend_from_slice(b".0");
            return;
        }
        let s10 = v * 10.0;
        if s10.fract() == 0.0 && s10.abs() < EXACT_INT_LIMIT {
            let k = s10 as i64;
            if k as f64 / 10.0 == v {
                write_one_decimal(buf, k);
                return;
            }
        }
        let s100 = v * 100.0;
        if s100.fract() == 0.0 && s100.abs() < EXACT_INT_LIMIT {
            let k = s100 as i64;
            if k as f64 / 100.0 == v {
                write_two_decimals(buf, k);
                return;
            }
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

/// Insert `header` at the front of `buf` in place.
///
/// The obvious `mem::take` + fresh `Vec` + `append` costs a second allocation
/// the size of the whole payload plus a full copy of it; this is a single
/// `memmove` inside the buffer we already own.
#[inline]
fn prepend(buf: &mut Vec<u8>, header: &[u8]) {
    let h = header.len();
    if h == 0 {
        return;
    }
    let n = buf.len();
    buf.reserve(h);
    unsafe {
        let p = buf.as_mut_ptr();
        core::ptr::copy(p, p.add(h), n);
        core::ptr::copy_nonoverlapping(header.as_ptr(), p, h);
        buf.set_len(n + h);
    }
}

// ---------------------------------------------------------------------------
// String quoting / escaping
// ---------------------------------------------------------------------------

/// Decide whether `s` must be quoted, and if so where escape scanning can
/// start.
///
/// `None` means the string can go on the wire bare. `Some(i)` means it must be
/// quoted and `s[..i]` is already known to be escape-free, so the writer does
/// not have to re-scan it.
#[inline]
fn quote_scan(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.is_empty() {
        return Some(0);
    }

    // One pass covers control chars, space (so leading/trailing whitespace is
    // caught per SPEC §S2), and every structural / comment-introducing byte.
    let special = simd::simd_find_special(bytes);
    if special < bytes.len() {
        return Some(special);
    }

    // Bool / null lookalikes.
    if matches!(
        bytes,
        b"true" | b"false" | b"True" | b"False" | b"TRUE" | b"FALSE"
    ) {
        return Some(bytes.len());
    }

    // Number-pattern check: only relevant when the first byte could plausibly
    // begin a number literal. For strings starting with a letter or any other
    // non-numeric byte, the whole-string pattern match cannot succeed, so we
    // skip the inner loop entirely. This is the common case for ASCII names,
    // emails (already caught by '@' above), tags, etc.
    let first = bytes[0];
    if !matches!(first, b'-' | b'+' | b'0'..=b'9' | b'.') {
        return None;
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
        return Some(bytes.len());
    }
    None
}

// ---------------------------------------------------------------------------
// Encoder
// ---------------------------------------------------------------------------

/// The ASUN text encode sink that derive-generated [`AsunEncode`] impls write
/// into. Prefer the [`encode`] / [`encode_typed`] free functions; this type is
/// exposed for the generated code.
pub struct Encoder {
    pub(crate) buf: Vec<u8>,
    in_tuple: bool,
    first: bool,
    /// When true, record type hints for top-level struct fields.
    typed: bool,
    /// Accumulates type hint for the current field being serialized.
    current_type_hint: Option<&'static str>,
    /// Top-level seq (`Vec<Struct>`) support
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

/// Serialize a value to an ASUN text string with a plain schema.
///
/// Output example: `{id,name,active}:(1,Alice,true)`. Use [`encode_typed`] for a
/// schema that carries scalar type hints.
pub fn encode<T: AsunEncode + ?Sized>(value: &T) -> Result<String> {
    let mut encoder = Encoder::new(false);
    value.encode(&mut encoder)?;
    Ok(unsafe { String::from_utf8_unchecked(encoder.buf) })
}

/// Serialize a single struct to ASUN string with type-annotated schema.
///
/// Output example: `{id@int,name@str,active@bool}:(1,Alice,true)`
pub fn encode_typed<T: AsunEncode + ?Sized>(value: &T) -> Result<String> {
    let mut encoder = Encoder::new(true);
    value.encode(&mut encoder)?;
    Ok(unsafe { String::from_utf8_unchecked(encoder.buf) })
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
    #[inline]
    fn new(typed: bool) -> Self {
        Encoder {
            buf: Vec::with_capacity(256),
            in_tuple: false,
            first: true,
            typed,
            current_type_hint: None,
            in_top_seq: false,
            top_seq_data_start: 0,
            top_seq_fields: None,
            top_seq_field_types: None,
            top_seq_field_schemas: None,
            nested_schema: None,
            skip_schema_capture: false,
        }
    }

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

    // -----------------------------------------------------------------------
    // Scalar encode primitives (called by the derive + built-in trait impls)
    // -----------------------------------------------------------------------

    #[inline]
    pub fn encode_bool(&mut self, v: bool) -> Result<()> {
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
            self.current_type_hint = Some("bool");
        }
        self.buf
            .extend_from_slice(if v { b"true" } else { b"false" });
        Ok(())
    }

    #[inline]
    pub fn encode_i8(&mut self, v: i8) -> Result<()> {
        self.encode_i64(v as i64)
    }
    #[inline]
    pub fn encode_i16(&mut self, v: i16) -> Result<()> {
        self.encode_i64(v as i64)
    }
    #[inline]
    pub fn encode_i32(&mut self, v: i32) -> Result<()> {
        self.encode_i64(v as i64)
    }

    #[inline]
    pub fn encode_i64(&mut self, v: i64) -> Result<()> {
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
            self.current_type_hint = Some("int");
        }
        write_i64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    pub fn encode_u8(&mut self, v: u8) -> Result<()> {
        self.encode_u64(v as u64)
    }
    #[inline]
    pub fn encode_u16(&mut self, v: u16) -> Result<()> {
        self.encode_u64(v as u64)
    }
    #[inline]
    pub fn encode_u32(&mut self, v: u32) -> Result<()> {
        self.encode_u64(v as u64)
    }

    #[inline]
    pub fn encode_u64(&mut self, v: u64) -> Result<()> {
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
            self.current_type_hint = Some("int");
        }
        write_u64(&mut self.buf, v);
        Ok(())
    }

    #[inline]
    pub fn encode_f32(&mut self, v: f32) -> Result<()> {
        self.encode_f64(v as f64)
    }

    #[inline]
    pub fn encode_f64(&mut self, v: f64) -> Result<()> {
        // ASUN text has no representation for NaN/±Infinity, and the decoder
        // rejects them, so encoding one would produce output that cannot
        // round-trip. Reject at encode time (matching serde_json's default).
        if !v.is_finite() {
            return Err(Error::msg(
                "cannot serialize non-finite float (NaN/Infinity)",
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
    pub fn encode_char(&mut self, v: char) -> Result<()> {
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
    pub fn encode_str(&mut self, v: &str) -> Result<()> {
        self.push_separator();
        if self.typed && self.current_type_hint.is_none() {
            self.current_type_hint = Some("str");
        }
        match quote_scan(v) {
            Some(first_escape) => {
                simd::simd_write_escaped_from(&mut self.buf, v.as_bytes(), first_escape)
            }
            None => self.buf.extend_from_slice(v.as_bytes()),
        }
        Ok(())
    }

    /// Encode a `&[u8]` byte slice as a `[0,1,2,...]` array (matches the
    /// previous `serialize_bytes` behavior).
    pub fn encode_bytes(&mut self, v: &[u8]) -> Result<()> {
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
    pub fn encode_none(&mut self) -> Result<()> {
        self.push_separator();
        // For typed mode: None doesn't set a type hint (the Some branch will)
        Ok(())
    }

    #[inline]
    pub fn encode_some<T: AsunEncode + ?Sized>(&mut self, value: &T) -> Result<()> {
        value.encode(self)
    }

    #[inline]
    pub fn encode_unit(&mut self) -> Result<()> {
        self.push_separator();
        self.buf.extend_from_slice(b"()");
        Ok(())
    }

    /// Encode a unit enum variant (text: bare variant name).
    #[inline]
    pub fn encode_unit_variant(&mut self, variant: &str) -> Result<()> {
        self.encode_str(variant)
    }

    /// Encode a newtype enum variant `(variant,value)`.
    pub fn encode_newtype_variant<T: AsunEncode + ?Sized>(
        &mut self,
        variant: &str,
        value: &T,
    ) -> Result<()> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        self.buf.push(b',');
        self.first = true;
        value.encode(&mut *self)?;
        self.buf.push(b')');
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Sequence / tuple / struct entry points
    // -----------------------------------------------------------------------

    /// Encode a homogeneous sequence `&[T]` in one shot.
    #[inline]
    pub fn encode_seq<T: AsunEncode>(&mut self, items: &[T]) -> Result<()> {
        let mut seq = self.begin_seq(Some(items.len()))?;
        for item in items {
            seq.element(self, item)?;
        }
        seq.end(self)
    }

    /// Begin a sequence. Mirrors the previous `serialize_seq`.
    fn begin_seq(&mut self, len: Option<usize>) -> Result<SeqEncoder> {
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
                first: true,
                is_top_seq: false,
                cached_nested_schema: None,
                skip_was_set: false,
            })
        }
    }

    /// Begin a tuple `(`. Used by tuple / tuple-struct built-in impls.
    #[inline]
    pub fn begin_tuple(&mut self) -> Result<()> {
        self.push_separator();
        self.buf.push(b'(');
        self.in_tuple = true;
        self.first = true;
        Ok(())
    }

    /// Encode one tuple element.
    #[inline]
    pub fn tuple_element<T: AsunEncode + ?Sized>(&mut self, value: &T) -> Result<()> {
        if !self.first {
            self.buf.push(b',');
        }
        self.first = true;
        self.in_tuple = true;
        value.encode(&mut *self)
    }

    /// Close a tuple `)`.
    #[inline]
    pub fn end_tuple(&mut self) -> Result<()> {
        self.buf.push(b')');
        self.first = false;
        Ok(())
    }

    /// Begin a tuple enum variant `(variant`. Elements follow via `element`.
    pub fn begin_tuple_variant(&mut self, variant: &str) -> Result<TupleEncoder> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        Ok(TupleEncoder { first: false })
    }

    /// Begin a struct. Mirrors the previous `serialize_struct`.
    pub fn begin_struct(&mut self, len: usize) -> Result<StructEncoder> {
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
                fields: Vec::with_capacity(len),
                // Type hints are only ever recorded (and read back) in typed
                // mode; allocating them otherwise is a wasted malloc per struct.
                field_types: if self.typed {
                    Vec::with_capacity(len)
                } else {
                    Vec::new()
                },
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
                    if self.typed {
                        Vec::with_capacity(len)
                    } else {
                        Vec::new()
                    },
                    Vec::with_capacity(len),
                )
            };
            Ok(StructEncoder {
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

    /// Begin a struct enum variant `(variant,`. Fields follow via `element`.
    pub fn begin_struct_variant(&mut self, variant: &str) -> Result<StructEncoder> {
        self.push_separator();
        self.buf.push(b'(');
        self.buf.extend_from_slice(variant.as_bytes());
        self.buf.push(b',');
        Ok(StructEncoder {
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
// SeqEncoder
// ---------------------------------------------------------------------------

/// Sink for encoding a sequence (`Vec<_>` / slice), created by
/// [`Encoder::encode_seq`]. Part of the derive's plumbing.
pub struct SeqEncoder {
    first: bool,
    is_top_seq: bool,
    /// For nested `Vec<Struct>`: schema fragment captured from row 1 so we can
    /// restore it after later rows in skip-mode wipe `nested_schema`.
    cached_nested_schema: Option<Vec<u8>>,
    /// Tracks whether *this* seq is the one that asserted the encoder's
    /// `skip_schema_capture` flag. Without this we'd reset on `end()` even
    /// when an outer seq owns the flag (e.g. inner primitive `Vec<i64>`
    /// running while the outer `Vec<Struct>` is still iterating).
    skip_was_set: bool,
}

impl SeqEncoder {
    #[inline]
    pub fn element<T: AsunEncode + ?Sized>(&mut self, enc: &mut Encoder, value: &T) -> Result<()> {
        if !self.first {
            enc.buf.push(b',');
        }
        let was_first = self.first;
        self.first = false;
        enc.first = true;
        let result = value.encode(&mut *enc);
        // After the first homogeneous struct row of a top-level seq has been
        // serialized, fields/types/schemas are cached on the encoder. Tell
        // subsequent rows to skip per-row schema bookkeeping.
        if was_first && self.is_top_seq && enc.top_seq_fields.is_some() {
            enc.skip_schema_capture = true;
            self.skip_was_set = true;
        }
        // For nested Vec<Struct>: row 1's StructEncoder::end() bubbled up a
        // schema fragment via `nested_schema`. Stash it on the seq so we can
        // restore it after this seq ends, and ask later rows to skip rebuild.
        if was_first && !self.is_top_seq && enc.nested_schema.is_some() {
            self.cached_nested_schema = enc.nested_schema.clone();
            enc.skip_schema_capture = true;
            self.skip_was_set = true;
        }
        result
    }

    #[inline]
    pub fn end(mut self, enc: &mut Encoder) -> Result<()> {
        // Only reset the encoder's `skip_schema_capture` if WE were the ones
        // who set it. Without this guard, a nested primitive seq (e.g. a
        // `Vec<i64>` field on row 2 of a top-level `Vec<Struct>`) would
        // clobber the outer seq's skip flag and force every later row of the
        // outer seq to redo schema bookkeeping. That bug wasted >10 % of
        // total encode time for 16-field structs.
        if self.skip_was_set {
            enc.skip_schema_capture = false;
        }
        // Restore the nested schema captured from row 1 (skip-mode wiped it).
        if let Some(cached) = self.cached_nested_schema.take() {
            enc.nested_schema = Some(cached);
        }
        if self.is_top_seq {
            if let Some(ref fields) = enc.top_seq_fields {
                // Struct elements: build the header once, then slide the
                // already-serialized payload aside to make room for it.
                let mut out = Vec::with_capacity(fields.len() * 16 + 8);
                out.extend_from_slice(b"[{");
                for (i, f) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(b',');
                    }
                    out.extend_from_slice(f.as_bytes());
                    // Nested schema takes priority over type hint
                    let has_nested = enc
                        .top_seq_field_schemas
                        .as_ref()
                        .and_then(|schemas| schemas.get(i))
                        .and_then(|s| s.as_ref());
                    if let Some(schema) = has_nested {
                        out.push(b'@');
                        out.extend_from_slice(schema);
                    } else if enc.typed
                        && let Some(ref field_types) = enc.top_seq_field_types
                        && let Some(Some(type_hint)) = field_types.get(i)
                    {
                        out.push(b'@');
                        out.extend_from_slice(type_hint.as_bytes());
                    }
                }
                out.extend_from_slice(b"}]:");
                prepend(&mut enc.buf, &out);
            } else {
                // Non-struct elements (primitive Vec): wrap in [...]
                prepend(&mut enc.buf, b"[");
                enc.buf.push(b']');
            }
            enc.in_top_seq = false;
        } else {
            enc.buf.push(b']');
            // The schema-fragment bubble-up below feeds the parent struct's
            // schema header. When the encoder is in skip-schema mode (rows
            // 2+ of a homogeneous Vec<Struct>) the parent will discard
            // anything we put here, so there's no need to allocate the
            // `[...]` wrapper Vec at all.
            if enc.skip_schema_capture {
                enc.nested_schema = None;
                if enc.typed {
                    enc.current_type_hint = None;
                }
            } else if let Some(schema) = enc.nested_schema.take() {
                let mut wrapped = Vec::with_capacity(schema.len() + 2);
                wrapped.push(b'[');
                wrapped.extend_from_slice(&schema);
                wrapped.push(b']');
                enc.nested_schema = Some(wrapped);
            } else if let Some(hint) = enc.current_type_hint.take() {
                // Primitive vec fields keep a structural scaffold even when
                // scalar element types are optional.
                let mut wrapped = Vec::with_capacity(hint.len() + 2);
                wrapped.push(b'[');
                wrapped.extend_from_slice(hint.as_bytes());
                wrapped.push(b']');
                enc.nested_schema = Some(wrapped);
            } else {
                enc.nested_schema = Some(b"[]".to_vec());
            }
        }
        enc.first = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TupleEncoder (used only for tuple enum variants; plain tuples use the
// inherent begin_tuple/tuple_element/end_tuple methods on Encoder).
// ---------------------------------------------------------------------------

/// Sink for encoding a tuple enum variant's body, created by
/// [`Encoder::begin_tuple_variant`]. Part of the derive's plumbing.
pub struct TupleEncoder {
    first: bool,
}

impl TupleEncoder {
    #[inline]
    pub fn element<T: AsunEncode + ?Sized>(&mut self, enc: &mut Encoder, value: &T) -> Result<()> {
        if !self.first {
            enc.buf.push(b',');
        }
        self.first = false;
        enc.first = true;
        value.encode(&mut *enc)
    }

    #[inline]
    pub fn end(self, enc: &mut Encoder) -> Result<()> {
        enc.buf.push(b')');
        enc.first = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// StructEncoder
// ---------------------------------------------------------------------------

/// Sink for encoding a struct's fields (name + value), created by
/// [`Encoder::begin_struct`]. Skipped fields are handled by simply not calling
/// [`StructEncoder::field`]. Part of the derive's plumbing.
pub struct StructEncoder {
    fields: Vec<&'static str>,
    /// Type hints collected for each field (only when typed mode is on)
    field_types: Vec<Option<&'static str>>,
    /// Nested schema fragments for struct/vec-of-struct fields
    field_schemas: Vec<Option<Vec<u8>>>,
    is_top: bool,
    capture_for_seq: bool,
    /// True for the 2nd+ row of a homogeneous `Vec<Struct>`: skip recording field
    /// names / types / nested schemas, since the seq's first row already did.
    skip_schema: bool,
    first: bool,
}

impl StructEncoder {
    /// Encode a named struct field. `key` must be `'static` so it can be
    /// stored for the schema header.
    #[inline]
    pub fn field<T: AsunEncode + ?Sized>(
        &mut self,
        enc: &mut Encoder,
        key: &'static str,
        value: &T,
    ) -> Result<()> {
        if !self.skip_schema {
            // Capture field names + per-field hint state only when this struct
            // will actually emit a schema header / fragment.
            self.fields.push(key);
            if enc.typed {
                enc.current_type_hint = None;
            }
            enc.nested_schema = None;
        }

        if !self.first {
            enc.buf.push(b',');
        }
        self.first = false;
        enc.first = true;
        enc.in_tuple = true;
        value.encode(&mut *enc)?;

        if !self.skip_schema {
            self.field_schemas.push(enc.nested_schema.take());
            if enc.typed {
                self.field_types.push(enc.current_type_hint.take());
            }
        } else {
            // Discard transient state nested serializers may have set; we are
            // not using it.
            enc.nested_schema = None;
            if enc.typed {
                enc.current_type_hint = None;
            }
        }
        Ok(())
    }

    /// Encode a struct enum variant field (no schema header — positional).
    #[inline]
    pub fn element<T: AsunEncode + ?Sized>(&mut self, enc: &mut Encoder, value: &T) -> Result<()> {
        if !self.first {
            enc.buf.push(b',');
        }
        self.first = false;
        enc.first = true;
        value.encode(&mut *enc)
    }

    pub fn end(self, enc: &mut Encoder) -> Result<()> {
        if self.is_top {
            enc.buf.push(b')');
            // Build the top-level header once, then slide the tuple payload
            // aside to make room for it.
            let mut out = Vec::with_capacity(self.fields.len() * 16 + 4);
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
                } else if enc.typed
                    && let Some(type_hint) = self.field_types.get(i).and_then(|t| *t)
                {
                    out.push(b'@');
                    out.extend_from_slice(type_hint.as_bytes());
                }
            }
            out.extend_from_slice(b"}:");
            prepend(&mut enc.buf, &out);
        } else if self.skip_schema {
            // Homogeneous Vec<Struct> non-first row: only the data tuple was
            // emitted. No header bubble-up to do.
            enc.buf.push(b')');
            enc.first = false;
            if enc.typed {
                enc.current_type_hint = None;
            }
        } else {
            enc.buf.push(b')');
            enc.first = false;
            if self.capture_for_seq {
                enc.top_seq_fields = Some(self.fields);
                enc.top_seq_field_schemas = Some(self.field_schemas);
                if enc.typed {
                    enc.top_seq_field_types = Some(self.field_types);
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
                    } else if enc.typed
                        && let Some(type_hint) = self.field_types.get(i).and_then(|t| *t)
                    {
                        schema.push(b'@');
                        schema.extend_from_slice(type_hint.as_bytes());
                    }
                }
                schema.push(b'}');
                enc.nested_schema = Some(schema);
            }
            if enc.typed {
                enc.current_type_hint = None;
            }
        }
        Ok(())
    }
}
