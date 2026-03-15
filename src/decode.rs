use crate::error::{Error, Result};
use crate::simd;
use serde::Deserialize;
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

type CachedSchemaNames = Arc<[Box<str>]>;
type MissingFields = Arc<[&'static str]>;

fn schema_cache() -> &'static Mutex<HashMap<Vec<u8>, CachedSchemaNames>> {
    static CACHE: OnceLock<Mutex<HashMap<Vec<u8>, CachedSchemaNames>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct StructModeCacheKey {
    source_ptr: usize,
    source_len: usize,
    target_ptr: usize,
    target_len: usize,
}

#[derive(Clone)]
enum CachedStructPlan {
    Exact,
    WithDefaults(MissingFields),
}

fn struct_mode_cache() -> &'static Mutex<HashMap<StructModeCacheKey, CachedStructPlan>> {
    static CACHE: OnceLock<Mutex<HashMap<StructModeCacheKey, CachedStructPlan>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

pub struct Deserializer<'de> {
    input: &'de [u8],
    pos: usize,
    /// Schema field names for current object context (positional mapping)
    schema_fields: Option<SchemaFields<'de>>,
    /// Current field index within a tuple
    field_index: usize,
    /// True when schema_fields holds the shared vec-header schema,
    /// meaning the next struct should use those field names directly
    /// (source schema) rather than replacing with target struct fields.
    vec_schema_active: bool,
    /// Cache the last resolved field alignment for repeated rows using the
    /// same source schema and target field list.
    last_struct_mode: Option<CachedStructMode>,
    /// Per-decode cache for repeated nested struct/source-schema alignments.
    struct_mode_cache_local: HashMap<StructModeCacheKey, CachedStructPlan>,
}

pub fn decode<'a, T: Deserialize<'a>>(s: &'a str) -> Result<T> {
    let mut de = Deserializer {
        input: s.as_bytes(),
        pos: 0,
        schema_fields: None,
        field_index: 0,
        vec_schema_active: false,
        last_struct_mode: None,
        struct_mode_cache_local: HashMap::new(),
    };
    de.skip_whitespace_and_comments();
    let value = T::deserialize(&mut de)?;
    de.skip_whitespace_and_comments();
    if de.pos < de.input.len() {
        if de.input[de.pos..].iter().all(|&b| b.is_ascii_whitespace()) {
            Ok(value)
        } else {
            Err(Error::TrailingCharacters)
        }
    } else {
        Ok(value)
    }
}

impl<'de> Deserializer<'de> {
    #[inline(always)]
    fn is_layout_byte(b: u8) -> bool {
        matches!(b, b' ' | b'\t' | b'\n' | b'\r')
    }

    #[inline(always)]
    fn is_value_delim(b: u8) -> bool {
        matches!(b, b',' | b')' | b']' | b':')
    }

    #[inline(always)]
    fn is_token_end_at(&self, pos: usize) -> bool {
        pos >= self.input.len()
            || Self::is_value_delim(self.input[pos])
            || Self::is_layout_byte(self.input[pos])
    }

    #[inline(always)]
    fn parse_bool_literal(&mut self) -> Option<bool> {
        if self.pos + 4 <= self.input.len()
            && &self.input[self.pos..self.pos + 4] == b"true"
            && self.is_token_end_at(self.pos + 4)
        {
            self.pos += 4;
            return Some(true);
        }
        if self.pos + 5 <= self.input.len()
            && &self.input[self.pos..self.pos + 5] == b"false"
            && self.is_token_end_at(self.pos + 5)
        {
            self.pos += 5;
            return Some(false);
        }
        None
    }

    #[inline]
    fn find_schema_end(&self, open_pos: usize) -> Result<usize> {
        let mut brace_depth = 1u32;
        let mut pos = open_pos + 1;
        while pos < self.input.len() {
            match self.input[pos] {
                b'{' => brace_depth += 1,
                b'}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        return Ok(pos);
                    }
                }
                _ => {}
            }
            pos += 1;
        }
        Err(Error::Eof)
    }

    #[inline(always)]
    fn peek_byte(&self) -> Result<u8> {
        if self.pos < self.input.len() {
            Ok(self.input[self.pos])
        } else {
            Err(Error::Eof)
        }
    }

    #[inline(always)]
    fn next_byte(&mut self) -> Result<u8> {
        if self.pos < self.input.len() {
            let b = self.input[self.pos];
            self.pos += 1;
            Ok(b)
        } else {
            Err(Error::Eof)
        }
    }

    /// Inline scalar whitespace skipping — fastest for ASON's compact format
    /// where values are separated by commas with no whitespace.
    /// SIMD overhead (splat/compare/movemask) is too costly when the
    /// common case is 0 whitespace bytes.
    #[inline(always)]
    fn skip_whitespace(&mut self) {
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    #[inline]
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.skip_whitespace();
            if self.pos + 1 < self.input.len()
                && self.input[self.pos] == b'/'
                && self.input[self.pos + 1] == b'*'
            {
                self.pos += 2;
                while self.pos + 1 < self.input.len() {
                    if self.input[self.pos] == b'*' && self.input[self.pos + 1] == b'/' {
                        self.pos += 2;
                        break;
                    }
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    #[inline(always)]
    fn skip_layout(&mut self) {
        self.skip_whitespace();
        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'/'
            && self.input[self.pos + 1] == b'*'
        {
            self.skip_whitespace_and_comments();
        }
    }

    fn parse_schema(&mut self) -> Result<SchemaFields<'de>> {
        let open_pos = self.pos;
        if self.next_byte()? != b'{' {
            return Err(Error::ExpectedOpenBrace);
        }
        let schema_end = self.find_schema_end(open_pos)?;
        let schema_key = &self.input[open_pos..=schema_end];

        if let Some(names) = schema_cache().lock().unwrap().get(schema_key).cloned() {
            self.pos = schema_end + 1;
            return Ok(SchemaFields::Cached {
                names,
                _marker: core::marker::PhantomData,
            });
        }

        let mut names = Vec::new();
        loop {
            self.skip_whitespace();
            if self.peek_byte()? == b'}' {
                self.pos += 1;
                break;
            }
            if !names.is_empty() {
                if self.next_byte()? != b',' {
                    return Err(Error::ExpectedComma);
                }
                self.skip_whitespace();
            }
            if self.peek_byte()? == b'"' {
                let cow = self.parse_quoted_string_cow()?;
                let name = match cow {
                    CowStr::Borrowed(s) => s.to_owned().into_boxed_str(),
                    CowStr::Owned(s) => s.into_boxed_str(),
                };
                names.push(name);
            } else {
                let start = self.pos;
                while self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b',' | b'}' | b'@' | b':' | b' ' | b'\t' => break,
                        _ => self.pos += 1,
                    }
                }
                let name = unsafe { core::str::from_utf8_unchecked(&self.input[start..self.pos]) };
                names.push(name.to_owned().into_boxed_str());
            }
            self.skip_whitespace();

            // Validate and skip optional @type hint or nested structural scaffold.
            if self.pos < self.input.len() && self.input[self.pos] == b'@' {
                self.pos += 1;
                self.skip_whitespace();
                self.parse_schema_annotation()?;
            } else if self.pos < self.input.len() && self.input[self.pos] == b':' {
                return Err(Error::Message(
                    "legacy ':' field annotations are not supported; use '@'".into(),
                ));
            }
        }

        let names: CachedSchemaNames = names.into_boxed_slice().into();
        schema_cache()
            .lock()
            .unwrap()
            .insert(schema_key.to_vec(), names.clone());
        Ok(SchemaFields::Cached {
            names,
            _marker: core::marker::PhantomData,
        })
    }

    fn parse_schema_annotation(&mut self) -> Result<()> {
        if self.pos >= self.input.len() {
            return Err(Error::Message("expected schema type after '@'".into()));
        }
        match self.input[self.pos] {
            b'{' => {
                let _ = self.parse_schema()?;
                Ok(())
            }
            b'[' => {
                self.pos += 1;
                self.skip_whitespace();
                if self.pos < self.input.len() && self.input[self.pos] == b']' {
                    self.pos += 1;
                    return Ok(());
                }
                if self.pos < self.input.len() && self.input[self.pos] == b'{' {
                    let _ = self.parse_schema()?;
                } else {
                    self.parse_allowed_schema_scalar_type()?;
                }
                self.skip_whitespace();
                if self.pos >= self.input.len() || self.input[self.pos] != b']' {
                    return Err(Error::Message("expected ']' in array type annotation".into()));
                }
                self.pos += 1;
                Ok(())
            }
            _ => self.parse_allowed_schema_scalar_type(),
        }
    }

    fn parse_allowed_schema_scalar_type(&mut self) -> Result<()> {
        let start = self.pos;
        while self.pos < self.input.len() {
            match self.input[self.pos] {
                b',' | b'}' | b']' | b' ' | b'\t' => break,
                _ => self.pos += 1,
            }
        }
        if start == self.pos {
            return Err(Error::Message("expected schema type after '@'".into()));
        }
        let mut token = unsafe { core::str::from_utf8_unchecked(&self.input[start..self.pos]) };
        if let Some(stripped) = token.strip_suffix('?') {
            token = stripped;
        }
        match token {
            "int" | "str" | "float" | "bool" => Ok(()),
            _ => Err(Error::Message(format!(
                "unsupported schema type '{token}'; use int, str, float, or bool"
            ))),
        }
    }

    #[inline]
    fn skip_balanced(&mut self, open: u8, close: u8) -> Result<()> {
        let mut depth = 0u32;
        loop {
            if self.pos >= self.input.len() {
                return Err(Error::Eof);
            }
            let b = self.input[self.pos];
            self.pos += 1;
            if b == open {
                depth += 1;
            } else if b == close {
                if depth == 0 {
                    return Err(Error::Message("unbalanced brackets".into()));
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
    }

    /// Skip a single ASON value (string, number, bool, tuple, array, etc.)
    fn skip_value(&mut self) -> Result<()> {
        self.skip_layout();
        if self.pos >= self.input.len() {
            return Ok(());
        }
        match self.input[self.pos] {
            b'(' => self.skip_balanced(b'(', b')'),
            b'[' => self.skip_balanced(b'[', b']'),
            b'"' => self.skip_quoted_string(),
            _ => {
                while self.pos < self.input.len() {
                    match self.input[self.pos] {
                        b',' | b')' | b']' | b'}' | b':' => break,
                        _ => self.pos += 1,
                    }
                }
                Ok(())
            }
        }
    }

    /// Skip remaining comma-separated values until ')'.
    /// Used when the source tuple has more fields than the target struct.
    fn skip_remaining_tuple_values(&mut self) -> Result<()> {
        self.skip_layout();
        while self.pos < self.input.len() && self.input[self.pos] != b')' {
            if self.input[self.pos] == b',' {
                self.pos += 1;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    break;
                }
            }
            if self.pos < self.input.len() && self.input[self.pos] != b')' {
                self.skip_value()?;
                self.skip_layout();
            }
        }
        Ok(())
    }

    /// Parse a plain (unquoted) string value, stopping at delimiters.
    /// Returns zerocopy borrowed str.
    #[inline]
    fn parse_plain_value_meta(&mut self) -> Result<(&'de str, bool)> {
        let start = self.pos;
        let mut has_escape = false;
        while self.pos < self.input.len() {
            let hit = simd::simd_find_plain_delimiter(self.input, self.pos);
            self.pos = hit;
            if self.pos >= self.input.len() {
                break;
            }
            if self.input[self.pos] == b'\\' {
                has_escape = true;
                self.pos += 2;
            } else {
                break;
            }
        }
        let mut end = self.pos;
        while end > start && Self::is_layout_byte(self.input[end - 1]) {
            end -= 1;
        }
        let raw = unsafe { core::str::from_utf8_unchecked(&self.input[start..end]) };
        Ok((raw, has_escape))
    }

    /// Parse a quoted string. Zerocopy when no escapes; allocates only when escapes present.
    /// Uses SIMD to scan for `"` or `\` in 16-byte chunks.
    #[inline]
    fn parse_quoted_string_cow(&mut self) -> Result<CowStr<'de>> {
        // Skip opening quote
        self.pos += 1;
        let start = self.pos;

        // SIMD fast scan: look for the closing quote or escape
        let hit = simd::simd_find_quote_or_backslash(self.input, self.pos);
        if hit < self.input.len() && self.input[hit] == b'"' {
            // No escapes found — zerocopy path
            let s = unsafe { core::str::from_utf8_unchecked(&self.input[start..hit]) };
            self.pos = hit + 1;
            return Ok(CowStr::Borrowed(s));
        }

        // Slow path: build owned string with escapes
        let scan = hit;
        let mut result = String::with_capacity(scan - start + 16);
        if scan > start {
            let prefix = unsafe { core::str::from_utf8_unchecked(&self.input[start..scan]) };
            result.push_str(prefix);
        }
        self.pos = scan;

        loop {
            if self.pos >= self.input.len() {
                return Err(Error::UnclosedString);
            }
            let b = self.input[self.pos];
            if b == b'"' {
                self.pos += 1;
                return Ok(CowStr::Owned(result));
            }
            if b == b'\\' {
                self.pos += 1;
                if self.pos >= self.input.len() {
                    return Err(Error::UnclosedString);
                }
                let esc = self.input[self.pos];
                self.pos += 1;
                match esc {
                    b'"' => result.push('"'),
                    b'\\' => result.push('\\'),
                    b'n' => result.push('\n'),
                    b't' => result.push('\t'),
                    b',' => result.push(','),
                    b'(' => result.push('('),
                    b')' => result.push(')'),
                    b'[' => result.push('['),
                    b']' => result.push(']'),
                    b':' => result.push(':'),
                    b'u' => {
                        if self.pos + 4 > self.input.len() {
                            return Err(Error::InvalidUnicodeEscape);
                        }
                        let hex = unsafe {
                            core::str::from_utf8_unchecked(&self.input[self.pos..self.pos + 4])
                        };
                        let cp = u32::from_str_radix(hex, 16)
                            .map_err(|_| Error::InvalidUnicodeEscape)?;
                        let ch = char::from_u32(cp).ok_or(Error::InvalidUnicodeEscape)?;
                        result.push(ch);
                        self.pos += 4;
                    }
                    _ => return Err(Error::InvalidEscape(esc as char)),
                }
            } else {
                // After an escape sequence, SIMD scan for next quote/backslash
                let next_hit = simd::simd_find_quote_or_backslash(self.input, self.pos);
                // Bulk copy the safe run
                if next_hit > self.pos {
                    let chunk =
                        unsafe { core::str::from_utf8_unchecked(&self.input[self.pos..next_hit]) };
                    result.push_str(chunk);
                    self.pos = next_hit;
                } else {
                    result.push(b as char);
                    self.pos += 1;
                }
            }
        }
    }

    #[inline]
    fn skip_quoted_string(&mut self) -> Result<()> {
        self.pos += 1;
        loop {
            if self.pos >= self.input.len() {
                return Err(Error::Eof);
            }
            let hit = simd::simd_find_quote_or_backslash(self.input, self.pos);
            if hit >= self.input.len() {
                return Err(Error::Eof);
            }
            self.pos = hit;
            match self.input[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return Ok(());
                }
                b'\\' => self.pos += 2,
                _ => unreachable!(),
            }
        }
    }

    /// Parse any value as a string.
    #[inline]
    fn parse_any_value_str(&mut self) -> Result<CowStr<'de>> {
        self.skip_layout();
        if self.pos >= self.input.len() {
            return Ok(CowStr::Borrowed(""));
        }
        if self.input[self.pos] == b'"' {
            self.parse_quoted_string_cow()
        } else {
            let (v, has_escape) = self.parse_plain_value_meta()?;
            if has_escape {
                Ok(CowStr::Owned(unescape_plain(v)?))
            } else {
                Ok(CowStr::Borrowed(v))
            }
        }
    }

    /// Parse number directly without intermediate string::parse for integers.
    /// Optimized loop with minimal branching.
    #[inline]
    fn parse_i64(&mut self) -> Result<i64> {
        let negative = self.pos < self.input.len() && self.input[self.pos] == b'-';
        if negative {
            self.pos += 1;
        }
        let mut val: u64 = 0;
        let mut digits = 0u32;
        while self.pos < self.input.len() {
            let d = self.input[self.pos].wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            val = val.wrapping_mul(10).wrapping_add(d as u64);
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(Error::InvalidNumber);
        }
        if negative {
            Ok(-(val as i64))
        } else {
            Ok(val as i64)
        }
    }

    /// Parse u64 directly. Optimized loop with wrapping_sub for digit check.
    #[inline]
    fn parse_u64(&mut self) -> Result<u64> {
        let mut val: u64 = 0;
        let mut digits = 0u32;
        while self.pos < self.input.len() {
            let d = self.input[self.pos].wrapping_sub(b'0');
            if d > 9 {
                break;
            }
            val = val.wrapping_mul(10).wrapping_add(d as u64);
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(Error::InvalidNumber);
        }
        Ok(val)
    }

    /// Parse f64 directly using fast-float for speed.
    #[inline]
    fn parse_f64_direct(&mut self) -> Result<f64> {
        let start = self.pos;
        if self.pos < self.input.len() && self.input[self.pos] == b'-' {
            self.pos += 1;
        }
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            self.pos += 1;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        // Handle scientific notation (e.g. 1.5e10)
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'e' || self.input[self.pos] == b'E')
        {
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'+' || self.input[self.pos] == b'-')
            {
                self.pos += 1;
            }
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if self.pos == start || (self.pos == start + 1 && self.input[start] == b'-') {
            return Err(Error::InvalidNumber);
        }
        let s = &self.input[start..self.pos];
        fast_float2::parse(s).map_err(|_| Error::InvalidNumber)
    }

    /// Peek ahead to determine value type without consuming
    #[inline]
    fn peek_value_type(&self) -> ValueType {
        if self.pos >= self.input.len() {
            return ValueType::Null;
        }
        match self.input[self.pos] {
            b'"' => ValueType::String,
            b'(' => ValueType::Tuple,
            b'[' => ValueType::Array,
            b't' | b'f' => ValueType::Bool,
            b'-' => {
                if self.pos + 1 < self.input.len() && self.input[self.pos + 1].is_ascii_digit() {
                    ValueType::Number
                } else {
                    ValueType::String
                }
            }
            b'0'..=b'9' => ValueType::Number,
            b',' | b')' | b']' | b':' => ValueType::Null,
            _ => ValueType::String,
        }
    }

    #[inline(always)]
    fn at_value_end(&self) -> bool {
        if self.pos >= self.input.len() {
            return true;
        }
        matches!(self.input[self.pos], b',' | b')' | b']')
    }

    #[inline(always)]
    fn struct_mode(&self, target_fields: &'static [&'static str]) -> StructMode {
        let Some(source_fields) = self.schema_fields.as_ref() else {
            return StructMode::Exact;
        };
        if source_fields.matches_exact(target_fields) {
            StructMode::Exact
        } else {
            StructMode::WithDefaults {
                missing_fields: source_fields.missing_target_fields(target_fields),
            }
        }
    }

    #[inline]
    fn struct_mode_cached(&mut self, target_fields: &'static [&'static str]) -> StructMode {
        let Some(source_fields) = self.schema_fields.as_ref() else {
            return StructMode::Exact;
        };
        let source_key = source_fields.cache_key();
        let target_ptr = target_fields.as_ptr() as usize;
        let target_len = target_fields.len();
        let cache_key = StructModeCacheKey {
            source_ptr: source_key.ptr,
            source_len: source_key.len,
            target_ptr,
            target_len,
        };

        if let Some(cached) = &self.last_struct_mode {
            if cached.cache_key == cache_key
                && cached.target_ptr == target_ptr
                && cached.target_len == target_len
            {
                return cached.mode.clone();
            }
        }

        if let Some(cached) = self.struct_mode_cache_local.get(&cache_key).cloned() {
            let mode = match cached {
                CachedStructPlan::Exact => StructMode::Exact,
                CachedStructPlan::WithDefaults(missing_fields) => {
                    StructMode::WithDefaults { missing_fields }
                }
            };
            self.last_struct_mode = Some(CachedStructMode {
                cache_key,
                target_ptr,
                target_len,
                mode: mode.clone(),
            });
            return mode;
        }

        if let Some(cached) = struct_mode_cache().lock().unwrap().get(&cache_key).cloned() {
            let mode = match &cached {
                CachedStructPlan::Exact => StructMode::Exact,
                CachedStructPlan::WithDefaults(missing_fields) => StructMode::WithDefaults {
                    missing_fields: missing_fields.clone(),
                },
            };
            self.struct_mode_cache_local.insert(cache_key, cached);
            self.last_struct_mode = Some(CachedStructMode {
                cache_key,
                target_ptr,
                target_len,
                mode: mode.clone(),
            });
            return mode;
        }

        let mode = self.struct_mode(target_fields);
        let cached = match &mode {
            StructMode::Exact => CachedStructPlan::Exact,
            StructMode::WithDefaults { missing_fields } => {
                CachedStructPlan::WithDefaults(missing_fields.clone())
            }
        };
        self.struct_mode_cache_local
            .insert(cache_key, cached.clone());
        struct_mode_cache()
            .lock()
            .unwrap()
            .insert(cache_key, cached);
        self.last_struct_mode = Some(CachedStructMode {
            cache_key,
            target_ptr,
            target_len,
            mode: mode.clone(),
        });
        mode
    }
}

enum SchemaFields<'de> {
    Cached {
        names: CachedSchemaNames,
        _marker: core::marker::PhantomData<&'de ()>,
    },
    Static(&'static [&'static str]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SchemaFieldsKey {
    ptr: usize,
    len: usize,
}

struct CachedStructMode {
    cache_key: StructModeCacheKey,
    target_ptr: usize,
    target_len: usize,
    mode: StructMode,
}

impl<'de> SchemaFields<'de> {
    #[inline(always)]
    fn len(&self) -> usize {
        match self {
            Self::Cached { names, .. } => names.len(),
            Self::Static(fields) => fields.len(),
        }
    }

    #[inline(always)]
    fn name_at(&self, index: usize) -> &'de str {
        match self {
            Self::Cached { names, .. } => unsafe {
                core::mem::transmute::<&str, &'de str>(&names[index])
            },
            Self::Static(fields) => unsafe {
                core::mem::transmute::<&str, &'de str>(fields[index])
            },
        }
    }

    #[inline(always)]
    fn cache_key(&self) -> SchemaFieldsKey {
        match self {
            Self::Cached { names, .. } => SchemaFieldsKey {
                ptr: names.as_ptr() as usize,
                len: names.len(),
            },
            Self::Static(fields) => SchemaFieldsKey {
                ptr: fields.as_ptr() as usize,
                len: fields.len(),
            },
        }
    }

    #[inline]
    fn matches_exact(&self, target_fields: &'static [&'static str]) -> bool {
        if self.len() != target_fields.len() {
            return false;
        }
        target_fields
            .iter()
            .enumerate()
            .all(|(idx, target)| self.name_at(idx) == *target)
    }

    #[inline]
    fn contains_name(&self, target: &str) -> bool {
        match self {
            Self::Cached { .. } => (0..self.len()).any(|index| self.name_at(index) == target),
            Self::Static(fields) => fields.iter().any(|field| *field == target),
        }
    }

    #[inline]
    fn missing_target_fields(&self, target_fields: &'static [&'static str]) -> MissingFields {
        target_fields
            .iter()
            .copied()
            .filter(|target| !self.contains_name(target))
            .collect::<Vec<_>>()
            .into()
    }
}

#[derive(Clone)]
enum StructMode {
    Exact,
    WithDefaults { missing_fields: MissingFields },
}

/// Lightweight Cow-like enum to avoid std::borrow::Cow overhead
enum CowStr<'a> {
    Borrowed(&'a str),
    Owned(String),
}

impl<'a> CowStr<'a> {
    #[inline]
    fn as_str(&self) -> &str {
        match self {
            CowStr::Borrowed(s) => s,
            CowStr::Owned(s) => s,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum ValueType {
    Null,
    Bool,
    Number,
    String,
    Array,
    Tuple,
}

fn unescape_plain(s: &str) -> Result<String> {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 1;
            if i >= bytes.len() {
                return Err(Error::Eof);
            }
            match bytes[i] {
                b',' => result.push(','),
                b'(' => result.push('('),
                b')' => result.push(')'),
                b'[' => result.push('['),
                b']' => result.push(']'),
                b':' => result.push(':'),
                b'"' => result.push('"'),
                b'\\' => result.push('\\'),
                b'n' => result.push('\n'),
                b't' => result.push('\t'),
                b'u' => {
                    if i + 4 >= bytes.len() {
                        return Err(Error::InvalidUnicodeEscape);
                    }
                    let hex = unsafe { core::str::from_utf8_unchecked(&bytes[i + 1..i + 5]) };
                    let cp =
                        u32::from_str_radix(hex, 16).map_err(|_| Error::InvalidUnicodeEscape)?;
                    let ch = char::from_u32(cp).ok_or(Error::InvalidUnicodeEscape)?;
                    result.push(ch);
                    i += 4;
                }
                other => return Err(Error::InvalidEscape(other as char)),
            }
        } else {
            result.push(bytes[i] as char);
        }
        i += 1;
    }
    Ok(result)
}

impl<'de, 'a> de::Deserializer<'de> for &'a mut Deserializer<'de> {
    type Error = Error;

    #[inline]
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_whitespace_and_comments();
        match self.peek_value_type() {
            ValueType::Null => visitor.visit_none(),
            ValueType::Bool => {
                // peek_value_type heuristically classifies 't'/'f' prefixed
                // values as Bool, but unquoted strings like "test" or "foo"
                // also start with these chars. Verify it's actually a bool
                // keyword before committing; otherwise treat as string.
                let start = self.pos;
                if self.parse_bool_literal().is_some() {
                    self.pos = start;
                    self.deserialize_bool(visitor)
                } else {
                    // Not a real bool — fall back to string
                    let cow = self.parse_any_value_str()?;
                    match cow {
                        CowStr::Borrowed(s) => visitor.visit_borrowed_str(s),
                        CowStr::Owned(s) => visitor.visit_string(s),
                    }
                }
            }
            ValueType::Number => {
                // Parse integer directly; only fall back to float if we hit '.'
                let negative = self.pos < self.input.len() && self.input[self.pos] == b'-';
                let sign_pos = self.pos;
                if negative {
                    self.pos += 1;
                }
                let mut val: u64 = 0;
                while self.pos < self.input.len() {
                    let d = self.input[self.pos].wrapping_sub(b'0');
                    if d > 9 {
                        break;
                    }
                    val = val.wrapping_mul(10).wrapping_add(d as u64);
                    self.pos += 1;
                }
                // Check if there's a decimal point → parse as float
                if self.pos < self.input.len() && self.input[self.pos] == b'.' {
                    // Reset and parse entire number as float with fast-float
                    self.pos = sign_pos;
                    let f = self.parse_f64_direct()?;
                    visitor.visit_f64(f)
                } else {
                    let i = if negative { -(val as i64) } else { val as i64 };
                    visitor.visit_i64(i)
                }
            }
            ValueType::String => {
                let cow = self.parse_any_value_str()?;
                match cow {
                    CowStr::Borrowed(s) => visitor.visit_borrowed_str(s),
                    CowStr::Owned(s) => visitor.visit_string(s),
                }
            }
            ValueType::Array => self.deserialize_seq(visitor),
            ValueType::Tuple => self.deserialize_map(visitor),
        }
    }

    #[inline]
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        if let Some(value) = self.parse_bool_literal() {
            return visitor.visit_bool(value);
        }
        Err(Error::InvalidBool)
    }

    #[inline]
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_i64()?;
        visitor.visit_i8(v as i8)
    }

    #[inline]
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_i64()?;
        visitor.visit_i16(v as i16)
    }

    #[inline]
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_i64()?;
        visitor.visit_i32(v as i32)
    }

    #[inline]
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_i64()?;
        visitor.visit_i64(v)
    }

    #[inline]
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_u64()?;
        visitor.visit_u8(v as u8)
    }

    #[inline]
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_u64()?;
        visitor.visit_u16(v as u16)
    }

    #[inline]
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_u64()?;
        visitor.visit_u32(v as u32)
    }

    #[inline]
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_u64()?;
        visitor.visit_u64(v)
    }

    #[inline]
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_f64_direct()? as f32;
        visitor.visit_f32(v)
    }

    #[inline]
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let v = self.parse_f64_direct()?;
        visitor.visit_f64(v)
    }

    #[inline]
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        let cow = self.parse_any_value_str()?;
        let s = cow.as_str();
        let mut chars = s.chars();
        let c = chars.next().ok_or(Error::ExpectedValue)?;
        visitor.visit_char(c)
    }

    #[inline]
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b'"' {
            let cow = self.parse_quoted_string_cow()?;
            match cow {
                CowStr::Borrowed(s) => visitor.visit_borrowed_str(s),
                CowStr::Owned(s) => visitor.visit_string(s),
            }
        } else {
            let (v, has_escape) = self.parse_plain_value_meta()?;
            if has_escape {
                visitor.visit_string(unescape_plain(v)?)
            } else {
                visitor.visit_borrowed_str(v)
            }
        }
    }

    #[inline]
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    #[inline]
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }

    #[inline]
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_seq(visitor)
    }

    #[inline]
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        if self.at_value_end() {
            visitor.visit_none()
        } else {
            visitor.visit_some(self)
        }
    }

    #[inline]
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'('
            && self.input[self.pos + 1] == b')'
        {
            self.pos += 2;
            visitor.visit_unit()
        } else if self.at_value_end() {
            visitor.visit_unit()
        } else {
            Err(Error::ExpectedValue)
        }
    }

    #[inline]
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_unit(visitor)
    }

    #[inline]
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }

    #[inline]
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        // [{schema}]:(v1,...),(v2,...) — struct array with shared schema
        if self.peek_byte()? == b'['
            && self.pos + 1 < self.input.len()
            && self.input[self.pos + 1] == b'{'
        {
            self.pos += 1; // skip '['
            let fields = self.parse_schema()?;
            self.skip_layout();
            if self.next_byte()? != b']' {
                return Err(Error::ExpectedCloseBracket);
            }
            self.skip_layout();
            if self.next_byte()? != b':' {
                return Err(Error::ExpectedColon);
            }
            self.schema_fields = Some(fields);
            self.vec_schema_active = true;
            let value = visitor.visit_seq(AsonVecAccess {
                de: self,
                first: true,
            })?;
            self.vec_schema_active = false;
            self.schema_fields = None;
            Ok(value)
        } else {
            if self.next_byte()? != b'[' {
                return Err(Error::ExpectedOpenBracket);
            }
            let value = visitor.visit_seq(AsonSeqAccess {
                de: self,
                first: true,
            })?;
            self.skip_layout();
            if self.pos < self.input.len() && self.input[self.pos] == b']' {
                self.pos += 1;
            }
            Ok(value)
        }
    }

    #[inline]
    fn deserialize_tuple<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        if self.next_byte()? != b'(' {
            return Err(Error::ExpectedOpenParen);
        }
        let value = visitor.visit_seq(AsonTupleAccess {
            de: self,
            first: true,
        })?;
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b')' {
            self.pos += 1;
        }
        Ok(value)
    }

    #[inline]
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value> {
        self.deserialize_tuple(len, visitor)
    }

    #[inline]
    fn deserialize_map<V: Visitor<'de>>(self, _visitor: V) -> Result<V::Value> {
        Err(Error::Message(
            "map fields are no longer supported; model key-value data as a list of entry tuples"
                .into(),
        ))
    }

    #[inline]
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.skip_layout();

        if self.schema_fields.is_some() {
            if self.peek_byte()? == b'(' {
                self.pos += 1;
                self.field_index = 0;
                let mut parent_schema = self.schema_fields.take();
                let from_vec_header = self.vec_schema_active;
                if from_vec_header {
                    // Vec row: schema_fields holds the source field names from
                    // the vec header — keep them so serde can match by name.
                    self.schema_fields = parent_schema.take();
                    self.vec_schema_active = false;
                } else {
                    self.schema_fields = Some(SchemaFields::Static(fields));
                }

                let mode = self.struct_mode_cached(fields);
                let value = match mode {
                    StructMode::Exact => visitor.visit_seq(AsonStructSeqAccess {
                        de: self,
                        field_index: 0,
                        field_count: fields.len(),
                    })?,
                    StructMode::WithDefaults { missing_fields } => {
                        visitor.visit_map(AsonStructAccessWithDefaults {
                            de: self,
                            field_index: 0,
                            default_index: 0,
                            missing_fields,
                        })?
                    }
                };
                self.skip_remaining_tuple_values()?;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    self.pos += 1;
                }
                if !from_vec_header {
                    self.schema_fields = parent_schema;
                }
                return Ok(value);
            }
            let parent_schema = self.schema_fields.take();
            self.schema_fields = Some(SchemaFields::Static(fields));
            self.field_index = 0;
            let mode = self.struct_mode_cached(fields);
            let value = match mode {
                StructMode::Exact => visitor.visit_seq(AsonStructSeqAccess {
                    de: self,
                    field_index: 0,
                    field_count: fields.len(),
                })?,
                StructMode::WithDefaults { missing_fields } => {
                    visitor.visit_map(AsonStructAccessWithDefaults {
                        de: self,
                        field_index: 0,
                        default_index: 0,
                        missing_fields,
                    })?
                }
            };
            self.schema_fields = parent_schema;
            Ok(value)
        } else {
            if self.peek_byte()? == b'{' {
                let parsed_fields = self.parse_schema()?;
                self.skip_layout();
                if self.next_byte()? != b':' {
                    return Err(Error::ExpectedColon);
                }
                self.skip_layout();
                self.schema_fields = Some(parsed_fields);
                if self.next_byte()? != b'(' {
                    return Err(Error::ExpectedOpenParen);
                }
                self.field_index = 0;
                let mode = self.struct_mode_cached(fields);
                let value = match mode {
                    StructMode::Exact => visitor.visit_seq(AsonStructSeqAccess {
                        de: self,
                        field_index: 0,
                        field_count: fields.len(),
                    })?,
                    StructMode::WithDefaults { missing_fields } => {
                        visitor.visit_map(AsonStructAccessWithDefaults {
                            de: self,
                            field_index: 0,
                            default_index: 0,
                            missing_fields,
                        })?
                    }
                };
                self.skip_remaining_tuple_values()?;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    self.pos += 1;
                }
                self.schema_fields = None;
                Ok(value)
            } else if self.peek_byte()? == b'(' {
                self.pos += 1;
                self.schema_fields = Some(SchemaFields::Static(fields));
                self.field_index = 0;
                let mode = self.struct_mode_cached(fields);
                let value = match mode {
                    StructMode::Exact => visitor.visit_seq(AsonStructSeqAccess {
                        de: self,
                        field_index: 0,
                        field_count: fields.len(),
                    })?,
                    StructMode::WithDefaults { missing_fields } => {
                        visitor.visit_map(AsonStructAccessWithDefaults {
                            de: self,
                            field_index: 0,
                            default_index: 0,
                            missing_fields,
                        })?
                    }
                };
                self.skip_remaining_tuple_values()?;
                self.skip_whitespace_and_comments();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    self.pos += 1;
                }
                self.schema_fields = None;
                Ok(value)
            } else {
                Err(Error::ExpectedOpenBrace)
            }
        }
    }

    #[inline]
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.skip_layout();
        if self.peek_byte()? == b'(' {
            self.pos += 1;
            let value = visitor.visit_enum(AsonEnumAccess { de: self })?;
            self.skip_layout();
            if self.pos < self.input.len() && self.input[self.pos] == b')' {
                self.pos += 1;
            }
            Ok(value)
        } else {
            visitor.visit_enum(AsonEnumAccess { de: self })
        }
    }

    #[inline]
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.deserialize_str(visitor)
    }

    #[inline]
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        self.skip_layout();
        self.skip_value()?;
        visitor.visit_unit()
    }
}

// --- Seq Access ---
struct AsonSeqAccess<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    first: bool,
}

impl<'a, 'de> SeqAccess<'de> for AsonSeqAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.de.skip_layout();
        if self.de.pos >= self.de.input.len() {
            return Ok(None);
        }
        if self.de.input[self.de.pos] == b']' {
            return Ok(None);
        }
        if !self.first {
            if self.de.input[self.de.pos] == b',' {
                self.de.pos += 1;
                self.de.skip_layout();
                if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b']' {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }
        self.first = false;
        seed.deserialize(&mut *self.de).map(Some)
    }
}

// --- Vec<Struct> Access for [{schema}]: format ---
struct AsonVecAccess<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    first: bool,
}

impl<'a, 'de> SeqAccess<'de> for AsonVecAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.de.skip_layout();
        if self.de.pos >= self.de.input.len() {
            return Ok(None);
        }
        if !self.first {
            if self.de.input[self.de.pos] == b',' {
                self.de.pos += 1;
                self.de.skip_layout();
            } else {
                return Ok(None);
            }
        }
        self.first = false;
        if self.de.pos >= self.de.input.len() || self.de.input[self.de.pos] != b'(' {
            return Ok(None);
        }
        self.de.field_index = 0;
        self.de.vec_schema_active = true;
        seed.deserialize(&mut *self.de).map(Some)
    }
}

// --- Tuple Access ---
struct AsonTupleAccess<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    first: bool,
}

impl<'a, 'de> SeqAccess<'de> for AsonTupleAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.de.skip_layout();
        if self.de.pos >= self.de.input.len() {
            return Ok(None);
        }
        if self.de.input[self.de.pos] == b')' {
            return Ok(None);
        }
        if !self.first {
            if self.de.input[self.de.pos] == b',' {
                self.de.pos += 1;
                self.de.skip_layout();
                if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b')' {
                    return Ok(None);
                }
            } else {
                return Ok(None);
            }
        }
        self.first = false;
        seed.deserialize(&mut *self.de).map(Some)
    }
}

// --- Struct (positional) Seq Access (Exact field order) ---
struct AsonStructSeqAccess<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    field_index: usize,
    field_count: usize,
}

impl<'a, 'de> SeqAccess<'de> for AsonStructSeqAccess<'a, 'de> {
    type Error = Error;

    #[inline(always)]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>> {
        self.de.skip_layout();
        if self.de.pos >= self.de.input.len() {
            return Ok(None);
        }
        if self.field_index >= self.field_count {
            return Ok(None);
        }
        if self.de.input[self.de.pos] == b')' {
            self.field_index += 1;
            self.de.field_index = self.field_index;
            return seed.deserialize(DefaultValueDeserializer).map(Some);
        }

        if self.field_index > 0 {
            if self.de.input[self.de.pos] == b',' {
                self.de.pos += 1;
                self.de.skip_layout();
                if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b')' {
                    self.field_index += 1;
                    self.de.field_index = self.field_index;
                    return seed.deserialize(DefaultValueDeserializer).map(Some);
                }
            } else {
                return Ok(None);
            }
        }

        self.field_index += 1;
        self.de.field_index = self.field_index;
        seed.deserialize(&mut *self.de).map(Some)
    }
}

// --- Struct (positional) Access (With Defaults) ---
struct AsonStructAccessWithDefaults<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
    field_index: usize,
    default_index: usize,
    missing_fields: MissingFields,
}

impl<'a, 'de> MapAccess<'de> for AsonStructAccessWithDefaults<'a, 'de> {
    type Error = Error;

    #[inline(always)]
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>> {
        self.de.skip_layout();
        if self.de.pos >= self.de.input.len() {
            return Ok(None);
        }

        // Phase 2: if we are at ')', we have exhausted the source tuple
        if self.de.input[self.de.pos] == b')' {
            if self.missing_fields.is_empty() {
                return Ok(None);
            }
            if self.default_index >= self.missing_fields.len() {
                return Ok(None);
            }
            let name_str: &'de str = unsafe {
                core::mem::transmute::<&str, &'de str>(self.missing_fields[self.default_index])
            };
            self.default_index += 1;
            return seed
                .deserialize(FieldNameDeserializer { name: name_str })
                .map(Some);
        }

        // Phase 1: parse fields from source
        let field_count = match &self.de.schema_fields {
            Some(f) => f.len(),
            None => return Ok(None),
        };

        if self.field_index >= field_count {
            return Ok(None);
        }

        if self.field_index > 0 {
            if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b',' {
                self.de.pos += 1;
                self.de.skip_layout();
                if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b')' {
                    return self.next_key_seed(seed);
                }
            } else if self.de.input[self.de.pos] != b')' {
                return Ok(None);
            } else {
                return Ok(None);
            }
        }

        let field_name = self
            .de
            .schema_fields
            .as_ref()
            .unwrap()
            .name_at(self.field_index);
        self.field_index += 1;
        self.de.field_index = self.field_index;

        seed.deserialize(FieldNameDeserializer { name: field_name })
            .map(Some)
    }

    #[inline(always)]
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value> {
        self.de.skip_layout();
        if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b')' {
            seed.deserialize(DefaultValueDeserializer)
        } else {
            seed.deserialize(&mut *self.de)
        }
    }
}

// --- Enum Access ---
struct AsonEnumAccess<'a, 'de: 'a> {
    de: &'a mut Deserializer<'de>,
}

impl<'a, 'de> de::EnumAccess<'de> for AsonEnumAccess<'a, 'de> {
    type Error = Error;
    type Variant = Self;

    #[inline]
    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self::Variant)> {
        self.de.skip_layout();
        let val = seed.deserialize(&mut *self.de)?;
        Ok((val, self))
    }
}

impl<'a, 'de> de::VariantAccess<'de> for AsonEnumAccess<'a, 'de> {
    type Error = Error;

    #[inline]
    fn unit_variant(self) -> Result<()> {
        Ok(())
    }

    #[inline]
    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value> {
        self.de.skip_layout();
        if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b',' {
            self.de.pos += 1;
        }
        seed.deserialize(&mut *self.de)
    }

    #[inline]
    fn tuple_variant<V: Visitor<'de>>(self, _len: usize, visitor: V) -> Result<V::Value> {
        self.de.skip_layout();
        if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b',' {
            self.de.pos += 1;
        }
        let value = visitor.visit_seq(AsonTupleAccess {
            de: self.de,
            first: true,
        })?;
        Ok(value)
    }

    #[inline]
    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        self.de.skip_layout();
        if self.de.pos < self.de.input.len() && self.de.input[self.de.pos] == b',' {
            self.de.pos += 1;
        }
        let parent_schema = self.de.schema_fields.take();
        let parent_field_index = self.de.field_index;
        self.de.schema_fields = Some(SchemaFields::Static(fields));
        self.de.field_index = 0;
        let mode = self.de.struct_mode_cached(fields);
        let value = match mode {
            StructMode::Exact => visitor.visit_seq(AsonStructSeqAccess {
                de: self.de,
                field_index: 0,
                field_count: fields.len(),
            })?,
            StructMode::WithDefaults { missing_fields } => {
                visitor.visit_map(AsonStructAccessWithDefaults {
                    de: self.de,
                    field_index: 0,
                    default_index: 0,
                    missing_fields,
                })?
            }
        };
        self.de.schema_fields = parent_schema;
        self.de.field_index = parent_field_index;
        Ok(value)
    }
}

// --- FieldName Deserializer (zerocopy: just returns &str) ---
struct FieldNameDeserializer<'de> {
    name: &'de str,
}

impl<'de> de::Deserializer<'de> for FieldNameDeserializer<'de> {
    type Error = Error;

    #[inline]
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str(self.name)
    }

    #[inline]
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str(self.name)
    }

    #[inline]
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str(self.name)
    }

    #[inline]
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str(self.name)
    }

    serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 u8 u16 u32 u64 f32 f64 char bytes byte_buf
        option unit unit_struct newtype_struct seq tuple tuple_struct
        map struct enum ignored_any
    }
}

// --- Default Value Deserializer ---
// Produces a "zero" value for any type, enabling missing schema fields to use
// type-specific defaults: false, 0, 0.0, "", [], None, etc.
// This matches Go/Java behaviour: missing source fields → target field default.
struct DefaultValueDeserializer;

impl<'de> de::Deserializer<'de> for DefaultValueDeserializer {
    type Error = Error;

    #[inline]
    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_none()
    }
    #[inline]
    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_bool(false)
    }
    #[inline]
    fn deserialize_i8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i8(0)
    }
    #[inline]
    fn deserialize_i16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i16(0)
    }
    #[inline]
    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i32(0)
    }
    #[inline]
    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_i64(0)
    }
    #[inline]
    fn deserialize_u8<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u8(0)
    }
    #[inline]
    fn deserialize_u16<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u16(0)
    }
    #[inline]
    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u32(0)
    }
    #[inline]
    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_u64(0)
    }
    #[inline]
    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f32(0.0)
    }
    #[inline]
    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_f64(0.0)
    }
    #[inline]
    fn deserialize_char<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_char('\0')
    }
    #[inline]
    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str("")
    }
    #[inline]
    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_string(String::new())
    }
    #[inline]
    fn deserialize_bytes<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_bytes(&[])
    }
    #[inline]
    fn deserialize_byte_buf<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_byte_buf(Vec::new())
    }
    #[inline]
    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_none()
    }
    #[inline]
    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }
    #[inline]
    fn deserialize_identifier<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_borrowed_str("")
    }
    #[inline]
    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_unit()
    }

    #[inline]
    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_unit()
    }
    #[inline]
    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_newtype_struct(self)
    }
    #[inline]
    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_seq(EmptySeqAccess)
    }
    #[inline]
    fn deserialize_tuple<V: Visitor<'de>>(self, _l: usize, visitor: V) -> Result<V::Value> {
        visitor.visit_seq(EmptySeqAccess)
    }
    #[inline]
    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        _l: usize,
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_seq(EmptySeqAccess)
    }
    #[inline]
    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value> {
        visitor.visit_map(EmptyMapAccess)
    }
    #[inline]
    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _n: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value> {
        visitor.visit_map(EmptyMapAccess)
    }
    #[inline]
    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _n: &'static str,
        _v: &'static [&'static str],
        _visitor: V,
    ) -> Result<V::Value> {
        Err(Error::ExpectedValue)
    }
}

struct EmptySeqAccess;
struct EmptyMapAccess;

impl<'de> SeqAccess<'de> for EmptySeqAccess {
    type Error = Error;
    #[inline]
    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, _seed: T) -> Result<Option<T::Value>> {
        Ok(None)
    }
}
impl<'de> MapAccess<'de> for EmptyMapAccess {
    type Error = Error;
    #[inline]
    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, _seed: K) -> Result<Option<K::Value>> {
        Ok(None)
    }
    #[inline]
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, _seed: V) -> Result<V::Value> {
        Err(Error::ExpectedValue)
    }
}
