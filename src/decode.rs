//! ASUN text decoding.
//!
//! The entry point is the free function [`decode`]; most users only need that
//! plus `#[derive(AsunDecode)]`. Decoding supports zero-copy borrowing — any
//! `&'de str` field in the target type borrows directly from the input.
//!
//! [`Decoder`] and its `struct_field_*` / `decode_*` / `begin_*` methods are the
//! low-level machinery the derive macro drives. They are `pub` so
//! derive-generated code in downstream crates can reach them via
//! `::asun::decode::...`; you rarely need to call them by hand.
//!
//! [`StructDecodeMode`] captures how a struct row is matched against its schema
//! (positional vs. by-name); it is exposed for the generated code and for
//! diagnostics.

use crate::error::{Error, Result};
use crate::simd;
use crate::traits::AsunDecode;
use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::sync::Arc;

type CachedSchemaNames = Arc<[Box<str>]>;

/// Maximum structural nesting the decoder will follow before bailing out.
///
/// Schema annotations (`@[[[…]]]`), nested schemas (`@{…}`) and nested
/// sequences all recurse, so without a cap a small hand-crafted payload can
/// exhaust the stack and abort the process — a crash that `catch_unwind`
/// cannot contain.
pub const MAX_DEPTH: u32 = 128;

/// Upper bound on the per-thread schema cache. Untrusted input can contain an
/// unbounded number of distinct schemas; without a cap the cache is an
/// unbounded memory leak.
const SCHEMA_CACHE_CAP: usize = 512;

/// FxHash — the rustc/Firefox multiply-xor-rotate hash. Schema keys are short
/// byte strings compared millions of times per second; SipHash (the std
/// default) shows up in profiles well above the cost of the lookup itself.
#[derive(Default)]
struct FxHasher {
    hash: u64,
}

const FX_SEED: u64 = 0x51_7c_c1_b7_27_22_0a_95;

impl FxHasher {
    #[inline(always)]
    fn add(&mut self, word: u64) {
        self.hash = (self.hash.rotate_left(5) ^ word).wrapping_mul(FX_SEED);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut b = bytes;
        while b.len() >= 8 {
            self.add(u64::from_le_bytes(b[..8].try_into().unwrap()));
            b = &b[8..];
        }
        if b.len() >= 4 {
            self.add(u32::from_le_bytes(b[..4].try_into().unwrap()) as u64);
            b = &b[4..];
        }
        for &x in b {
            self.add(x as u64);
        }
        self.add(bytes.len() as u64);
    }

    #[inline]
    fn write_usize(&mut self, n: usize) {
        self.add(n as u64);
    }

    #[inline]
    fn finish(&self) -> u64 {
        self.hash
    }
}

type FxBuild = BuildHasherDefault<FxHasher>;

thread_local! {
    /// Per-thread schema cache.
    ///
    /// Thread-local rather than a global `Mutex<HashMap>` so concurrent decodes
    /// never contend on a process-wide lock (and so a panic while holding it
    /// cannot poison every future decode), and bounded so hostile input cannot
    /// grow it without limit. Evicting is safe because every `Decoder` that
    /// takes an entry also stores a strong reference in its own arena — see
    /// [`Decoder::intern_schema`].
    static SCHEMA_CACHE: RefCell<HashMap<Box<[u8]>, CachedSchemaNames, FxBuild>> =
        RefCell::new(HashMap::default());
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct StructModeCacheKey {
    source_ptr: usize,
    source_len: usize,
    target_ptr: usize,
    target_len: usize,
}

/// The decode plan a derived struct impl must follow, chosen by
/// [`Decoder::begin_struct_decode`].
///
/// - `Exact`: the source tuple's fields line up 1:1 (same order, same names)
///   with the target struct. The derive reads fields positionally.
/// - `ByName`: the source schema differs (reordered / missing / extra fields).
///   The derive iterates source keys, matching each to a target field by name,
///   and fills any unmatched target field with its type default.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StructDecodeMode {
    Exact,
    ByName,
}

/// The ASUN text decode source that derive-generated [`AsunDecode`] impls pull
/// from. Prefer the [`decode`] free function; this type is exposed for the
/// generated code. The `'de` lifetime is the borrow of the input buffer, which
/// enables zero-copy `&'de str` fields.
///
/// [`AsunDecode`]: crate::AsunDecode
pub struct Decoder<'de> {
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
    /// Structural nesting depth, checked against [`MAX_DEPTH`].
    depth: u32,
    /// Tiny MRU cache of resolved field alignments. For deep nested structs
    /// (e.g. `Company > Division > Team > Project > Task`) several struct
    /// types interleave during a single decode; a 1-slot cache thrashes and
    /// every miss falls back to the `HashMap`. `MRU_SLOTS` is sized to
    /// comfortably cover realistic nesting depths.
    ///
    /// Entries are `Copy`, so an MRU hit costs a few integer compares — no
    /// refcount traffic, which used to dominate this path.
    last_struct_mode: [Option<CachedStructMode>; MRU_SLOTS],
    /// Index of the most recently filled MRU slot — checked first.
    last_struct_mode_head: usize,
    /// Per-decode cache for repeated nested struct/source-schema alignments.
    ///
    /// Deliberately *not* process-global: the key contains raw pointers into
    /// schema and target-field slices, which are only guaranteed stable for the
    /// lifetime of one decode (the arenas below pin them). A global cache would
    /// hand a recycled address the previous occupant's plan.
    struct_mode_cache_local: HashMap<StructModeCacheKey, StructPlan, FxBuild>,
    /// Strong references to every schema this decode touched. Keeps the names
    /// alive for the whole decode so [`SchemaFields`] can be a plain `Copy`
    /// borrow instead of a refcounted handle.
    schema_arena: Vec<CachedSchemaNames>,
    /// Backing store for `StructPlan::ByName` missing-field lists, referenced
    /// by index so plans stay `Copy`.
    missing_arena: Vec<Box<[&'static str]>>,
    /// When > 0, every scalar `decode_*` returns a type default instead of
    /// reading the input. This is the direct analog of the previous
    /// `DefaultValueDeserializer`: it lets a derived struct impl produce a
    /// default value for a missing field by simply calling `T::decode`.
    default_depth: u32,
    /// True while decoding an enum whose value was wrapped in `(...)`; tells
    /// `end_enum` to consume the trailing `)`.
    enum_opened_paren: bool,

    // --- Per-struct decode state (used by the begin_struct_decode seam) ---
    /// Stack of in-progress struct frames. Each `begin_struct_decode` pushes
    /// one; `end_struct_decode` pops it. Nesting depth mirrors data nesting.
    struct_frames: Vec<StructFrame<'de>>,
}

/// State for one struct currently being decoded via the derive seam.
struct StructFrame<'de> {
    /// The schema in effect for the parent context, saved so we can restore it
    /// on `end_struct_decode`.
    parent_schema: Option<SchemaFields<'de>>,
    parent_field_index: usize,
    /// True if the parent schema must be restored on end (i.e. we did not
    /// consume a vec-header schema in place).
    restore_parent: bool,
    /// True if this frame opened a `(` that `end_struct_decode` must close
    /// (after skipping any trailing source fields).
    close_paren: bool,

    // ByName-mode cursors.
    /// Number of source fields already consumed via `next_struct_key`.
    byname_source_index: usize,
    /// Whether we are still in the "read source fields" phase (vs. the
    /// "emit missing defaults" phase).
    byname_in_defaults: bool,
    byname_default_index: usize,
    /// Index into [`Decoder::missing_arena`], or `NO_MISSING` for Exact mode.
    byname_missing: u32,
}

/// Sentinel for "this frame has no missing-field list".
const NO_MISSING: u32 = u32::MAX;

const MRU_SLOTS: usize = 8;

pub fn decode<'a, T: AsunDecode<'a>>(s: &'a str) -> Result<T> {
    let mut de = Decoder::new(s.as_bytes());
    de.skip_whitespace_and_comments();
    let value = T::decode(&mut de)?;
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

impl<'de> Decoder<'de> {
    fn new(input: &'de [u8]) -> Self {
        Decoder {
            input,
            pos: 0,
            schema_fields: None,
            field_index: 0,
            vec_schema_active: false,
            depth: 0,
            last_struct_mode: [const { None }; MRU_SLOTS],
            last_struct_mode_head: 0,
            struct_mode_cache_local: HashMap::default(),
            schema_arena: Vec::new(),
            missing_arena: Vec::new(),
            default_depth: 0,
            enum_opened_paren: false,
            struct_frames: Vec::new(),
        }
    }

    #[inline(always)]
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(Error::DepthLimitExceeded);
        }
        Ok(())
    }

    #[inline(always)]
    fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Pin `names` for the rest of this decode and hand back a `'de` borrow.
    ///
    /// The arena holds a strong reference until the `Decoder` is dropped, and
    /// the `[Box<str>]` behind an `Arc` never moves, so the borrow outlives
    /// every use the decoder makes of it.
    #[inline]
    fn intern_schema(&mut self, names: CachedSchemaNames) -> &'de [Box<str>] {
        let slice: *const [Box<str>] = Arc::as_ptr(&names);
        self.schema_arena.push(names);
        unsafe { &*slice }
    }

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

    /// Find the `}` closing the schema opened at `open_pos`.
    ///
    /// Must skip over quoted field names and block comments: a `}` inside
    /// either is not structural. Getting this wrong truncates the cache key, so
    /// two different schemas can collide — and the same input can decode
    /// differently on the second call once the bad key is cached.
    #[inline]
    fn find_schema_end(&self, open_pos: usize) -> Result<usize> {
        let input = self.input;
        let len = input.len();
        let mut brace_depth = 1u32;
        let mut pos = open_pos + 1;
        while pos < len {
            match input[pos] {
                b'"' => {
                    pos += 1;
                    loop {
                        if pos >= len {
                            return Err(Error::UnclosedString);
                        }
                        match input[pos] {
                            b'\\' => pos += 2,
                            b'"' => {
                                pos += 1;
                                break;
                            }
                            _ => pos += 1,
                        }
                    }
                    continue;
                }
                b'/' if pos + 1 < len && input[pos + 1] == b'*' => {
                    pos += 2;
                    loop {
                        if pos + 1 >= len {
                            return Err(Error::UnclosedComment);
                        }
                        if input[pos] == b'*' && input[pos + 1] == b'/' {
                            pos += 2;
                            break;
                        }
                        pos += 1;
                    }
                    continue;
                }
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

    /// Inline scalar whitespace skipping — fastest for ASUN's compact format
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
        self.enter()?;
        let r = self.parse_schema_inner();
        self.leave();
        r
    }

    fn parse_schema_inner(&mut self) -> Result<SchemaFields<'de>> {
        let open_pos = self.pos;
        if self.next_byte()? != b'{' {
            return Err(Error::ExpectedOpenBrace);
        }
        let schema_end = self.find_schema_end(open_pos)?;
        // Copy the slice reference out of `self` so the key does not keep an
        // immutable borrow of the decoder alive across the parsing below.
        let schema_key: &'de [u8] = &self.input[open_pos..=schema_end];

        if let Some(names) = SCHEMA_CACHE.with(|c| c.borrow().get(schema_key).cloned()) {
            self.pos = schema_end + 1;
            return Ok(SchemaFields::Cached(self.intern_schema(names)));
        }

        let mut names = Vec::new();
        loop {
            self.skip_layout();
            if self.peek_byte()? == b'}' {
                self.pos += 1;
                break;
            }
            if !names.is_empty() {
                if self.next_byte()? != b',' {
                    return Err(Error::ExpectedComma);
                }
                self.skip_layout();
                if self.peek_byte()? == b'}' {
                    self.pos += 1;
                    break;
                }
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
                        // `/` terminates too: it cannot occur in a bare field
                        // name, and starts a comment (`ows` is legal here).
                        b',' | b'}' | b'@' | b':' | b'/' | b' ' | b'\t' | b'\n' | b'\r' => break,
                        _ => self.pos += 1,
                    }
                }
                let name = unsafe { core::str::from_utf8_unchecked(&self.input[start..self.pos]) };
                names.push(name.to_owned().into_boxed_str());
            }
            self.skip_layout();

            // Validate and skip optional @type hint or nested structural scaffold.
            if self.pos < self.input.len() && self.input[self.pos] == b'@' {
                self.pos += 1;
                self.skip_layout();
                self.parse_schema_annotation()?;
            }
        }

        let names: CachedSchemaNames = names.into_boxed_slice().into();
        SCHEMA_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            // Cheapest bounded policy that keeps the common case (a handful of
            // schemas reused forever) allocation-free: drop everything once the
            // cap is hit rather than tracking recency.
            if c.len() >= SCHEMA_CACHE_CAP {
                c.clear();
            }
            c.insert(schema_key.into(), names.clone());
        });
        Ok(SchemaFields::Cached(self.intern_schema(names)))
    }

    fn parse_schema_annotation(&mut self) -> Result<()> {
        self.enter()?;
        let r = self.parse_schema_annotation_inner();
        self.leave();
        r
    }

    fn parse_schema_annotation_inner(&mut self) -> Result<()> {
        if self.pos >= self.input.len() {
            return Err(Error::msg("expected schema type after '@'"));
        }
        match self.input[self.pos] {
            b'{' => {
                let _ = self.parse_schema()?;
                Ok(())
            }
            b'[' => {
                self.pos += 1;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b']' {
                    self.pos += 1;
                    return Ok(());
                }
                // The element type may itself be a struct `{...}`, a nested
                // array `[...]` (array-of-array, e.g. `@[[]]`), or a scalar.
                match self.input.get(self.pos) {
                    Some(b'{') => {
                        let _ = self.parse_schema()?;
                    }
                    Some(b'[') => {
                        self.parse_schema_annotation()?;
                    }
                    _ => {
                        self.parse_allowed_schema_scalar_type()?;
                    }
                }
                self.skip_layout();
                if self.pos >= self.input.len() || self.input[self.pos] != b']' {
                    return Err(Error::msg("expected ']' in array type annotation"));
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
                b',' | b'}' | b']' | b'/' | b' ' | b'\t' | b'\n' | b'\r' => break,
                _ => self.pos += 1,
            }
        }
        if start == self.pos {
            return Err(Error::msg("expected schema type after '@'"));
        }
        let mut token = unsafe { core::str::from_utf8_unchecked(&self.input[start..self.pos]) };
        if let Some(stripped) = token.strip_suffix('?') {
            token = stripped;
        }
        match token {
            "int" | "str" | "float" | "bool" => Ok(()),
            _ => Err(Error::msg(format!(
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
                    return Err(Error::msg("unbalanced brackets"));
                }
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
    }

    /// Skip a single ASUN value (string, number, bool, tuple, array, etc.)
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
                // A trailing backslash with no following byte is malformed; a
                // bare `self.pos += 2` would push past the end and panic on the
                // trim/slice below. Reject it instead.
                if self.pos + 2 > self.input.len() {
                    return Err(Error::InvalidEscape('\\'));
                }
                self.pos += 2;
            } else {
                break;
            }
        }
        let mut end = self.pos;
        while end > start && Self::is_layout_byte(self.input[end - 1]) {
            end -= 1;
        }
        // When escapes are present the slice may split a multi-byte UTF-8
        // sequence (pos advanced by 2 past a `\`), which would make the
        // `from_utf8_unchecked` reference invalid. Validate in that case; the
        // no-escape fast path is guaranteed valid because the input is `&str`.
        let bytes = &self.input[start..end];
        let raw = if has_escape {
            core::str::from_utf8(bytes).map_err(|_| Error::InvalidEscape('\\'))?
        } else {
            unsafe { core::str::from_utf8_unchecked(bytes) }
        };
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
                    b'r' => result.push('\r'),
                    b'b' => result.push('\u{0008}'),
                    b'f' => result.push('\u{000C}'),
                    b',' => result.push(','),
                    b'(' => result.push('('),
                    b')' => result.push(')'),
                    b'[' => result.push('['),
                    b']' => result.push(']'),
                    b'{' => result.push('{'),
                    b'}' => result.push('}'),
                    b':' => result.push(':'),
                    b'@' => result.push('@'),
                    b'u' => {
                        let ch = read_unicode_escape(self.input, &mut self.pos)?;
                        result.push(ch);
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
            // Detect overflow instead of silently wrapping; an out-of-range
            // integer is a decode error, not corrupt data.
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add(d as u64))
                .ok_or(Error::InvalidNumber)?;
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(Error::InvalidNumber);
        }
        if negative {
            // Magnitude fits in i64 (>= -2^63) exactly when val <= 2^63.
            if val > (i64::MAX as u64) + 1 {
                return Err(Error::InvalidNumber);
            }
            Ok((val as i64).wrapping_neg())
        } else {
            if val > i64::MAX as u64 {
                return Err(Error::InvalidNumber);
            }
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
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add(d as u64))
                .ok_or(Error::InvalidNumber)?;
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 {
            return Err(Error::InvalidNumber);
        }
        Ok(val)
    }

    /// Parse f64 directly using fast-float for speed.
    ///
    /// Enforces ABNF `float = ["-"] 1*DIGIT ( "." 1*DIGIT [exponent] / exponent )`:
    /// the integer part must have ≥1 digit, and if a decimal point is present
    /// the fractional part must also have ≥1 digit. Tokens like `"5."`, `".5"`,
    /// or `"+5"` are rejected so the caller can fall back to plain-string.
    #[inline]
    fn parse_f64_direct(&mut self) -> Result<f64> {
        let start = self.pos;
        if self.pos < self.input.len() && self.input[self.pos] == b'-' {
            self.pos += 1;
        }
        let int_start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        let int_digits = self.pos - int_start;
        let mut had_dot_or_exp = false;
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            had_dot_or_exp = true;
            self.pos += 1;
            let frac_start = self.pos;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            // ABNF requires at least one digit after the decimal point.
            if self.pos == frac_start {
                return Err(Error::InvalidNumber);
            }
        }
        // Handle scientific notation (e.g. 1.5e10)
        if self.pos < self.input.len()
            && (self.input[self.pos] == b'e' || self.input[self.pos] == b'E')
        {
            had_dot_or_exp = true;
            self.pos += 1;
            if self.pos < self.input.len()
                && (self.input[self.pos] == b'+' || self.input[self.pos] == b'-')
            {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            // ABNF requires at least one digit in the exponent.
            if self.pos == exp_start {
                return Err(Error::InvalidNumber);
            }
        }
        // Must have at least one integer digit, and must actually be a float
        // (contain "." or "e"/"E"); otherwise it's an integer / not a number.
        if int_digits == 0 || !had_dot_or_exp {
            return Err(Error::InvalidNumber);
        }
        let s = &self.input[start..self.pos];
        fast_float2::parse(s).map_err(|_| Error::InvalidNumber)
    }

    #[inline(always)]
    fn at_value_end(&self) -> bool {
        if self.pos >= self.input.len() {
            return true;
        }
        matches!(self.input[self.pos], b',' | b')' | b']')
    }

    #[inline(always)]
    fn struct_plan_uncached(&mut self, target_fields: &'static [&'static str]) -> StructPlan {
        let Some(source_fields) = self.schema_fields else {
            return StructPlan::Exact;
        };
        if source_fields.matches_exact(target_fields) {
            StructPlan::Exact
        } else {
            let missing = source_fields.missing_target_fields(target_fields);
            let idx = self.missing_arena.len() as u32;
            self.missing_arena.push(missing);
            StructPlan::ByName(idx)
        }
    }

    #[inline]
    fn struct_plan(&mut self, target_fields: &'static [&'static str]) -> StructPlan {
        let Some(source_fields) = self.schema_fields else {
            return StructPlan::Exact;
        };
        let source_key = source_fields.cache_key();
        let cache_key = StructModeCacheKey {
            source_ptr: source_key.ptr,
            source_len: source_key.len,
            target_ptr: target_fields.as_ptr() as usize,
            target_len: target_fields.len(),
        };

        // MRU fast path: linear-search a small fixed-size array. Skips the
        // `HashMap::get` when the same handful of struct shapes alternate
        // (typical in deeply nested data, e.g.
        // Company > Division > Team > Project > Task on every row).
        for slot in self.last_struct_mode.iter().flatten() {
            if slot.cache_key == cache_key {
                return slot.plan;
            }
        }

        let plan = match self.struct_mode_cache_local.get(&cache_key) {
            Some(&plan) => plan,
            None => {
                let plan = self.struct_plan_uncached(target_fields);
                self.struct_mode_cache_local.insert(cache_key, plan);
                plan
            }
        };
        self.mru_put(cache_key, plan);
        plan
    }

    #[inline]
    fn mru_put(&mut self, cache_key: StructModeCacheKey, plan: StructPlan) {
        let slot = self.last_struct_mode_head;
        self.last_struct_mode[slot] = Some(CachedStructMode { cache_key, plan });
        self.last_struct_mode_head = (slot + 1) % MRU_SLOTS;
    }

    // =======================================================================
    // Scalar decode primitives (called by the derive + built-in trait impls)
    //
    // Each honours `default_depth`: when in default mode (a missing struct
    // field being materialised), it returns the type default instead of
    // reading the input.
    // =======================================================================

    #[inline]
    pub fn decode_bool(&mut self) -> Result<bool> {
        if self.default_depth > 0 {
            return Ok(false);
        }
        self.skip_layout();
        if let Some(value) = self.parse_bool_literal() {
            return Ok(value);
        }
        Err(Error::InvalidBool)
    }

    #[inline]
    pub fn decode_i8(&mut self) -> Result<i8> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        i8::try_from(self.parse_i64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i16(&mut self) -> Result<i16> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        i16::try_from(self.parse_i64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i32(&mut self) -> Result<i32> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        i32::try_from(self.parse_i64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i64(&mut self) -> Result<i64> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        self.parse_i64()
    }

    #[inline]
    pub fn decode_u8(&mut self) -> Result<u8> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        u8::try_from(self.parse_u64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u16(&mut self) -> Result<u16> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        u16::try_from(self.parse_u64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u32(&mut self) -> Result<u32> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        u32::try_from(self.parse_u64()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u64(&mut self) -> Result<u64> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.skip_layout();
        self.parse_u64()
    }

    #[inline]
    pub fn decode_f32(&mut self) -> Result<f32> {
        if self.default_depth > 0 {
            return Ok(0.0);
        }
        self.skip_layout();
        Ok(self.parse_f64_direct()? as f32)
    }

    #[inline]
    pub fn decode_f64(&mut self) -> Result<f64> {
        if self.default_depth > 0 {
            return Ok(0.0);
        }
        self.skip_layout();
        self.parse_f64_direct()
    }

    #[inline]
    pub fn decode_char(&mut self) -> Result<char> {
        if self.default_depth > 0 {
            return Ok('\0');
        }
        self.skip_layout();
        let cow = self.parse_any_value_str()?;
        let s = cow.as_str();
        let mut chars = s.chars();
        chars.next().ok_or(Error::ExpectedValue)
    }

    #[inline]
    pub fn decode_string(&mut self) -> Result<String> {
        if self.default_depth > 0 {
            return Ok(String::new());
        }
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b'"' {
            let cow = self.parse_quoted_string_cow()?;
            Ok(match cow {
                CowStr::Borrowed(s) => s.to_owned(),
                CowStr::Owned(s) => s,
            })
        } else {
            let (v, has_escape) = self.parse_plain_value_meta()?;
            if has_escape {
                unescape_plain(v)
            } else {
                Ok(v.to_owned())
            }
        }
    }

    /// Zero-copy borrowed str decode.
    #[inline]
    pub fn decode_borrowed_str(&mut self) -> Result<&'de str> {
        if self.default_depth > 0 {
            return Ok("");
        }
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b'"' {
            let cow = self.parse_quoted_string_cow()?;
            match cow {
                CowStr::Borrowed(s) => Ok(s),
                // An escaped string cannot be borrowed; the previous serde impl
                // handed serde an owned String which it copied. Here the target
                // is `&'de str`, which fundamentally cannot hold an unescaped
                // owned buffer — reject, matching the borrow contract.
                CowStr::Owned(_) => Err(Error::msg("cannot borrow &str from an escaped string")),
            }
        } else {
            let (v, _has_escape) = self.parse_plain_value_meta()?;
            Ok(v)
        }
    }

    #[inline]
    pub fn decode_option<T: AsunDecode<'de>>(&mut self) -> Result<Option<T>> {
        if self.default_depth > 0 {
            return Ok(None);
        }
        self.skip_layout();
        if self.at_value_end() {
            Ok(None)
        } else {
            Ok(Some(T::decode(self)?))
        }
    }

    #[inline]
    pub fn decode_unit(&mut self) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        self.skip_layout();
        if self.pos + 1 < self.input.len()
            && self.input[self.pos] == b'('
            && self.input[self.pos + 1] == b')'
        {
            self.pos += 2;
            Ok(())
        } else if self.at_value_end() {
            Ok(())
        } else {
            Err(Error::ExpectedValue)
        }
    }

    /// Decode a homogeneous sequence `Vec<T>`.
    ///
    /// Handles both `[v1,v2,...]` (plain array) and `[{schema}]:(row),(row)`
    /// (struct array with a shared schema).
    pub fn decode_vec<T: AsunDecode<'de>>(&mut self) -> Result<Vec<T>> {
        if self.default_depth > 0 {
            return Ok(Vec::new());
        }
        self.enter()?;
        let r = self.decode_vec_inner();
        self.leave();
        r
    }

    fn decode_vec_inner<T: AsunDecode<'de>>(&mut self) -> Result<Vec<T>> {
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

            let mut out = Vec::new();
            let mut first = true;
            loop {
                self.skip_layout();
                if self.pos >= self.input.len() {
                    break;
                }
                if !first {
                    if self.input[self.pos] == b',' {
                        self.pos += 1;
                        self.skip_layout();
                    } else {
                        break;
                    }
                }
                first = false;
                if self.pos >= self.input.len() || self.input[self.pos] != b'(' {
                    break;
                }
                self.field_index = 0;
                self.vec_schema_active = true;
                out.push(T::decode(self)?);
            }

            self.vec_schema_active = false;
            self.schema_fields = None;
            Ok(out)
        } else {
            if self.next_byte()? != b'[' {
                return Err(Error::ExpectedOpenBracket);
            }
            let mut out = Vec::new();
            let mut first = true;
            loop {
                self.skip_layout();
                if self.pos >= self.input.len() {
                    break;
                }
                if self.input[self.pos] == b']' {
                    break;
                }
                if !first {
                    if self.input[self.pos] == b',' {
                        self.pos += 1;
                        self.skip_layout();
                        if self.pos < self.input.len() && self.input[self.pos] == b']' {
                            break;
                        }
                    } else {
                        break;
                    }
                }
                first = false;
                out.push(T::decode(self)?);
            }
            self.skip_layout();
            if self.pos < self.input.len() && self.input[self.pos] == b']' {
                self.pos += 1;
            }
            Ok(out)
        }
    }

    // -----------------------------------------------------------------------
    // Tuple seam (plain tuples, tuple structs, tuple enum variants)
    // -----------------------------------------------------------------------

    /// Begin decoding a tuple `(`. In default mode this is a no-op.
    #[inline]
    pub fn begin_tuple(&mut self) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        self.skip_layout();
        if self.next_byte()? != b'(' {
            return Err(Error::ExpectedOpenParen);
        }
        self.field_index = 0;
        Ok(())
    }

    /// Decode one tuple element in positional order.
    ///
    /// Mirrors `AsunTupleAccess`: elements are comma separated and terminated
    /// by `)`. A missing element (premature `)`) yields `T`'s default.
    #[inline]
    pub fn tuple_element<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        if self.default_depth > 0 {
            return T::decode(self);
        }
        self.skip_layout();
        let first = self.field_index == 0;
        // End-of-tuple or EOF: emit default.
        if self.pos >= self.input.len() || self.input[self.pos] == b')' {
            self.field_index += 1;
            return self.decode_default::<T>();
        }
        if !first {
            if self.input[self.pos] == b',' {
                self.pos += 1;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    self.field_index += 1;
                    return self.decode_default::<T>();
                }
            } else {
                // No further comma-separated element — treat remaining as default.
                self.field_index += 1;
                return self.decode_default::<T>();
            }
        }
        self.field_index += 1;
        T::decode(self)
    }

    /// Finish decoding a tuple of `_count` declared elements: skip any extra
    /// source values and consume the closing `)`.
    #[inline]
    pub fn end_tuple(&mut self, _count: usize) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b')' {
            self.pos += 1;
        }
        Ok(())
    }

    /// Produce a type default for `T` by running its decode logic in default
    /// mode. This is the direct analog of the previous `DefaultValueDeserializer`.
    #[inline]
    fn decode_default<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        self.default_depth += 1;
        let r = T::decode(self);
        self.default_depth -= 1;
        r
    }

    // -----------------------------------------------------------------------
    // Struct seam (used by derived AsunDecode impls)
    // -----------------------------------------------------------------------

    /// Begin decoding a struct with the given target field list.
    ///
    /// Parses/consumes any schema header and the opening `(`, sets up schema
    /// alignment, and returns the [`StructDecodeMode`] the derive should follow.
    /// Must be paired with [`Decoder::end_struct_decode`].
    pub fn begin_struct_decode(
        &mut self,
        target_fields: &'static [&'static str],
    ) -> Result<StructDecodeMode> {
        if self.default_depth > 0 {
            // Missing struct field: every leaf recurses in default mode. We
            // still push a frame so end_struct_decode stays balanced, but it
            // performs no input reads.
            self.struct_frames.push(StructFrame {
                parent_schema: None,
                parent_field_index: self.field_index,
                restore_parent: false,
                close_paren: false,
                byname_source_index: 0,
                byname_in_defaults: false,
                byname_default_index: 0,
                byname_missing: NO_MISSING,
            });
            return Ok(StructDecodeMode::Exact);
        }

        self.enter()?;
        self.skip_layout();

        // Resolve schema + opening paren + parent bookkeeping, mirroring the
        // previous `deserialize_struct` state machine exactly.
        let (parent_schema, restore_parent, close_paren);

        if self.schema_fields.is_some() {
            if self.peek_byte()? == b'(' {
                self.pos += 1;
                self.field_index = 0;
                let ps = self.schema_fields.take();
                let from_vec_header = self.vec_schema_active;
                if from_vec_header {
                    // Vec row: schema_fields holds the source field names from
                    // the vec header — keep them active for this row so we can
                    // match by name. The vec loop owns this schema across all
                    // rows, so we stash a copy in the frame and restore it on
                    // `end_struct_decode`; otherwise nested structs (which
                    // replace `schema_fields`) or the end-clear path would drop
                    // it, and every row after the first would decode with the
                    // wrong schema.
                    self.schema_fields = ps;
                    self.vec_schema_active = false;
                    parent_schema = ps;
                    restore_parent = true;
                } else {
                    self.schema_fields = Some(SchemaFields::Static(target_fields));
                    parent_schema = ps;
                    restore_parent = true;
                }
                close_paren = true;
            } else {
                parent_schema = self.schema_fields.take();
                self.schema_fields = Some(SchemaFields::Static(target_fields));
                self.field_index = 0;
                restore_parent = true;
                close_paren = false;
            }
        } else if self.peek_byte()? == b'{' {
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
            parent_schema = None;
            restore_parent = false; // schema_fields cleared to None on end
            close_paren = true;
        } else if self.peek_byte()? == b'(' {
            self.pos += 1;
            self.schema_fields = Some(SchemaFields::Static(target_fields));
            self.field_index = 0;
            parent_schema = None;
            restore_parent = false; // schema_fields cleared to None on end
            close_paren = true;
        } else {
            return Err(Error::ExpectedOpenBrace);
        }

        let (decode_mode, byname_missing) = match self.struct_plan(target_fields) {
            StructPlan::Exact => (StructDecodeMode::Exact, NO_MISSING),
            StructPlan::ByName(idx) => (StructDecodeMode::ByName, idx),
        };

        self.struct_frames.push(StructFrame {
            parent_schema,
            parent_field_index: 0,
            restore_parent,
            close_paren,
            byname_source_index: 0,
            byname_in_defaults: false,
            byname_default_index: 0,
            byname_missing,
        });
        Ok(decode_mode)
    }

    /// Exact mode: read the field at positional index `_index`.
    /// Mirrors `AsunStructSeqAccess`.
    #[inline]
    pub fn struct_field_positional<T: AsunDecode<'de>>(&mut self, index: usize) -> Result<T> {
        if self.default_depth > 0 {
            return T::decode(self);
        }
        // Hot path: the derive passes the positional index directly and calls
        // this once per field with `index` = 0,1,2,…, exactly tracking the
        // frame's `exact_index`. We therefore drive comma/paren logic off
        // `index` alone and never touch `self.struct_frames` here — the frame
        // is only read/restored in `end_struct_decode`. `self.field_index` is
        // kept in sync so a nested `T::decode` saves the right parent index.
        self.skip_layout();
        self.field_index = index + 1;

        if self.pos >= self.input.len() {
            // Ran out of input; remaining fields are defaults.
            return self.decode_default::<T>();
        }

        if self.input[self.pos] == b')' {
            return self.decode_default::<T>();
        }

        if index > 0 {
            if self.input[self.pos] == b',' {
                self.pos += 1;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    return self.decode_default::<T>();
                }
            } else {
                // No further field: emit default without advancing input.
                return self.decode_default::<T>();
            }
        }

        T::decode(self)
    }

    /// ByName mode: return the next source field's name, or `None` when the
    /// source tuple is exhausted. After the source is drained, this emits the
    /// names of missing target fields (whose values decode as defaults).
    /// Mirrors `AsunStructAccessWithDefaults::next_key_seed`.
    pub fn next_struct_key(&mut self) -> Result<Option<&'de str>> {
        let frame_idx = self.struct_frames.len() - 1;
        self.skip_layout();

        loop {
            if self.pos >= self.input.len() {
                // Source exhausted at EOF: fall into defaults phase.
                return self.next_missing_default_key(frame_idx);
            }

            // At ')' — source tuple done; emit missing defaults.
            if self.input[self.pos] == b')' {
                return self.next_missing_default_key(frame_idx);
            }

            let field_count = match &self.schema_fields {
                Some(f) => f.len(),
                None => return Ok(None),
            };

            let source_index = self.struct_frames[frame_idx].byname_source_index;
            if source_index >= field_count {
                return Ok(None);
            }

            if source_index > 0 {
                if self.pos < self.input.len() && self.input[self.pos] == b',' {
                    self.pos += 1;
                    self.skip_layout();
                    if self.pos < self.input.len() && self.input[self.pos] == b')' {
                        // Trailing comma then ')': retry loop → defaults phase.
                        continue;
                    }
                } else {
                    // No comma: end of source tuple.
                    return Ok(None);
                }
            }

            let field_name = self.schema_fields.unwrap().name_at(source_index);
            let frame = &mut self.struct_frames[frame_idx];
            frame.byname_source_index += 1;
            self.field_index = frame.byname_source_index;
            return Ok(Some(field_name));
        }
    }

    #[inline]
    fn next_missing_default_key(&mut self, frame_idx: usize) -> Result<Option<&'de str>> {
        let frame = &mut self.struct_frames[frame_idx];
        frame.byname_in_defaults = true;
        if frame.byname_missing == NO_MISSING {
            return Ok(None);
        }
        let idx = frame.byname_missing as usize;
        let k = frame.byname_default_index;
        let missing = &self.missing_arena[idx];
        if k >= missing.len() {
            return Ok(None);
        }
        // Elements are `&'static str`, so this is a copy, not a lifetime cast.
        let name: &'static str = missing[k];
        self.struct_frames[frame_idx].byname_default_index += 1;
        Ok(Some(name))
    }

    /// ByName mode: decode the value corresponding to the key just returned by
    /// `next_struct_key`. Mirrors `AsunStructAccessWithDefaults::next_value_seed`.
    #[inline]
    pub fn struct_field_value<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        let frame_idx = self.struct_frames.len() - 1;
        if self.struct_frames[frame_idx].byname_in_defaults {
            return self.decode_default::<T>();
        }
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b')' {
            self.decode_default::<T>()
        } else {
            T::decode(self)
        }
    }

    /// ByName mode: skip the value for an unmatched source key.
    #[inline]
    pub fn skip_struct_value(&mut self) -> Result<()> {
        let frame_idx = self.struct_frames.len() - 1;
        if self.struct_frames[frame_idx].byname_in_defaults {
            // A missing-target default key: nothing in the input to skip.
            return Ok(());
        }
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b')' {
            return Ok(());
        }
        self.skip_value()
    }

    /// ByName mode: produce a type default for an unmatched target field.
    #[inline]
    pub fn struct_field_default<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        self.decode_default::<T>()
    }

    /// Finish decoding a struct: skip any trailing source fields, close the
    /// tuple, and restore parent schema state.
    pub fn end_struct_decode(&mut self) -> Result<()> {
        let frame = self
            .struct_frames
            .pop()
            .expect("end_struct_decode without begin_struct_decode");

        if self.default_depth > 0 {
            self.field_index = frame.parent_field_index;
            return Ok(());
        }

        self.leave();

        if frame.close_paren {
            self.skip_layout();
            // Exact mode with a full field match leaves the cursor right on the
            // closing paren, which is the overwhelmingly common case.
            if self.pos < self.input.len() && self.input[self.pos] == b')' {
                self.pos += 1;
            } else {
                self.skip_remaining_tuple_values()?;
                self.skip_layout();
                if self.pos < self.input.len() && self.input[self.pos] == b')' {
                    self.pos += 1;
                }
            }
        }

        if frame.restore_parent {
            self.schema_fields = frame.parent_schema;
        } else if frame.close_paren && frame.parent_schema.is_none() {
            // Top-level `{schema}:(...)` and bare `(...)` paths clear to None.
            self.schema_fields = None;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Enum seam (used by derived AsunDecode impls)
    // -----------------------------------------------------------------------

    /// Begin decoding an enum: consume an optional opening `(` and read the
    /// variant name. Mirrors `deserialize_enum` + `variant_seed`.
    pub fn begin_enum(&mut self) -> Result<String> {
        if self.default_depth > 0 {
            return Err(Error::ExpectedValue);
        }
        self.skip_layout();
        let opened = if self.peek_byte()? == b'(' {
            self.pos += 1;
            true
        } else {
            false
        };
        self.enum_opened_paren = opened;
        self.skip_layout();
        let cow = self.parse_any_value_str()?;
        Ok(match cow {
            CowStr::Borrowed(s) => s.to_owned(),
            CowStr::Owned(s) => s,
        })
    }

    /// Unit variant: nothing to read (mirrors `VariantAccess::unit_variant`).
    #[inline]
    pub fn finish_unit_variant(&mut self) -> Result<()> {
        Ok(())
    }

    /// Newtype variant: skip the comma after the variant name, then decode the
    /// inner value. Mirrors `VariantAccess::newtype_variant_seed`.
    #[inline]
    pub fn newtype_variant_value<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b',' {
            self.pos += 1;
        }
        T::decode(self)
    }

    /// Tuple / struct variant body: skip the comma after the variant name, then
    /// read elements positionally via `tuple_element`. Mirrors
    /// `VariantAccess::tuple_variant` / `struct_variant` element reads.
    #[inline]
    pub fn begin_tuple_variant_body(&mut self) -> Result<()> {
        self.skip_layout();
        if self.pos < self.input.len() && self.input[self.pos] == b',' {
            self.pos += 1;
        }
        self.field_index = 0;
        Ok(())
    }

    /// Finish decoding an enum: consume the closing `)` if we opened one.
    #[inline]
    pub fn end_enum(&mut self) -> Result<()> {
        if self.enum_opened_paren {
            self.skip_layout();
            if self.pos < self.input.len() && self.input[self.pos] == b')' {
                self.pos += 1;
            }
            self.enum_opened_paren = false;
        }
        Ok(())
    }
}

#[derive(Clone, Copy)]
enum SchemaFields<'de> {
    /// Borrowed from a schema pinned in [`Decoder::schema_arena`].
    Cached(&'de [Box<str>]),
    Static(&'static [&'static str]),
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct SchemaFieldsKey {
    ptr: usize,
    len: usize,
}

#[derive(Clone, Copy)]
struct CachedStructMode {
    cache_key: StructModeCacheKey,
    plan: StructPlan,
}

impl<'de> SchemaFields<'de> {
    #[inline(always)]
    fn len(&self) -> usize {
        match self {
            Self::Cached(names) => names.len(),
            Self::Static(fields) => fields.len(),
        }
    }

    #[inline(always)]
    fn name_at(&self, index: usize) -> &'de str {
        match self {
            Self::Cached(names) => &names[index],
            Self::Static(fields) => fields[index],
        }
    }

    #[inline(always)]
    fn cache_key(&self) -> SchemaFieldsKey {
        match self {
            Self::Cached(names) => SchemaFieldsKey {
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
            Self::Cached(names) => names.iter().any(|n| &**n == target),
            Self::Static(fields) => fields.contains(&target),
        }
    }

    #[inline]
    fn missing_target_fields(&self, target_fields: &'static [&'static str]) -> Box<[&'static str]> {
        target_fields
            .iter()
            .copied()
            .filter(|target| !self.contains_name(target))
            .collect()
    }
}

/// How a struct row lines up with the source schema. `Copy` on purpose: it is
/// looked up once per struct value, and the previous refcounted representation
/// cost an atomic increment/decrement pair on every single row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StructPlan {
    Exact,
    /// Index into [`Decoder::missing_arena`].
    ByName(u32),
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

/// Parse exactly four hex digits at `at`.
///
/// Deliberately byte-wise: the previous `from_utf8_unchecked` over four raw
/// input bytes could build an invalid `&str` when the escape was followed by a
/// multi-byte character, and `from_str_radix` additionally accepted junk like
/// `+123`.
#[inline]
fn hex4(bytes: &[u8], at: usize) -> Result<u32> {
    if at + 4 > bytes.len() {
        return Err(Error::InvalidUnicodeEscape);
    }
    let mut cp = 0u32;
    for k in 0..4 {
        let d = match bytes[at + k] {
            c @ b'0'..=b'9' => c - b'0',
            c @ b'a'..=b'f' => c - b'a' + 10,
            c @ b'A'..=b'F' => c - b'A' + 10,
            _ => return Err(Error::InvalidUnicodeEscape),
        };
        cp = (cp << 4) | d as u32;
    }
    Ok(cp)
}

/// Read a `\uXXXX` escape whose first hex digit is at `*pos`, joining a
/// UTF-16 surrogate pair when present. `*pos` ends just past the escape.
#[inline]
fn read_unicode_escape(input: &[u8], pos: &mut usize) -> Result<char> {
    let hi = hex4(input, *pos)?;
    *pos += 4;
    let cp = if (0xD800..0xDC00).contains(&hi) {
        if input.get(*pos) != Some(&b'\\') || input.get(*pos + 1) != Some(&b'u') {
            return Err(Error::InvalidUnicodeEscape);
        }
        let lo = hex4(input, *pos + 2)?;
        if !(0xDC00..0xE000).contains(&lo) {
            return Err(Error::InvalidUnicodeEscape);
        }
        *pos += 6;
        0x1_0000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
    } else if (0xDC00..0xE000).contains(&hi) {
        // Unpaired low surrogate.
        return Err(Error::InvalidUnicodeEscape);
    } else {
        hi
    };
    char::from_u32(cp).ok_or(Error::InvalidUnicodeEscape)
}

/// Unescape a plain (unquoted) value.
///
/// Works on bytes and bulk-copies the runs between escapes. The previous
/// implementation pushed `bytes[i] as char`, which reinterprets each byte as a
/// code point and therefore corrupted every multi-byte character in any value
/// that also contained an escape.
fn unescape_plain(s: &str) -> Result<String> {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let run_start = i;
        while i < bytes.len() && bytes[i] != b'\\' {
            i += 1;
        }
        out.extend_from_slice(&bytes[run_start..i]);
        if i >= bytes.len() {
            break;
        }
        i += 1;
        if i >= bytes.len() {
            return Err(Error::Eof);
        }
        match bytes[i] {
            b @ (b',' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b':' | b'@' | b'"' | b'\\') => {
                out.push(b)
            }
            b'n' => out.push(b'\n'),
            b't' => out.push(b'\t'),
            b'r' => out.push(b'\r'),
            b'b' => out.push(0x08),
            b'f' => out.push(0x0c),
            b'u' => {
                let mut p = i + 1;
                let ch = read_unicode_escape(bytes, &mut p)?;
                let mut tmp = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                i = p - 1;
            }
            other => return Err(Error::InvalidEscape(other as char)),
        }
        i += 1;
    }
    // SAFETY: `s` is valid UTF-8; runs are cut at ASCII `\` so they stay on
    // character boundaries, and every pushed replacement is valid UTF-8.
    Ok(unsafe { String::from_utf8_unchecked(out) })
}
