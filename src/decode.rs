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

type CachedSchema = Arc<Schema>;

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
    static SCHEMA_CACHE: RefCell<HashMap<Box<[u8]>, CachedSchema, FxBuild>> =
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
    /// Source schema of the struct currently being decoded (positional or
    /// by-name mapping), or of the `[{schema}]:` rows being decoded.
    schema_fields: Option<SchemaFields<'de>>,
    /// True when schema_fields holds the shared vec-header schema,
    /// meaning the next struct should use those field names directly
    /// (source schema) rather than replacing with target struct fields.
    vec_schema_active: bool,
    /// Declared type (`@...` binding) of the slot about to be decoded, handed
    /// from the enclosing schema to the value's decoder. `None` = no binding.
    /// Consumers `take()` it, so it never leaks into a sibling slot.
    pending: Option<&'de Ty>,
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
    schema_arena: Vec<CachedSchema>,
    /// Backing store for `StructPlan::ByName` missing-field lists, referenced
    /// by index so plans stay `Copy`.
    missing_arena: Vec<Box<[&'static str]>>,
    /// When > 0, every scalar `decode_*` returns a type default instead of
    /// reading the input. This is the direct analog of the previous
    /// `DefaultValueDeserializer`: it lets a derived struct impl produce a
    /// default value for a missing field by simply calling `T::decode`.
    default_depth: u32,
    /// Open positional groups: Rust tuples / tuple structs (`[..]` or `(..)`)
    /// and enum payloads. Each frame knows its closer and how many slots it
    /// has read, so commas and the closer are checked exactly.
    seq_frames: Vec<SeqFrame>,

    // --- Per-struct decode state (used by the begin_struct_decode seam) ---
    /// Stack of in-progress struct frames. Each `begin_struct_decode` pushes
    /// one; `end_struct_decode` pops it. Nesting depth mirrors data nesting.
    struct_frames: Vec<StructFrame<'de>>,
}

/// One open positional group (see [`Decoder::seq_frames`]).
struct SeqFrame {
    /// `)` or `]`; `0` for a group that owns no brackets (an enum variant
    /// body, whose brackets belong to the enclosing enum frame, or a bare
    /// unit variant).
    closer: u8,
    /// Slots read so far.
    index: u32,
}

/// State for one struct currently being decoded via the derive seam.
struct StructFrame<'de> {
    /// The schema in effect for the parent context, restored on
    /// `end_struct_decode`.
    parent_schema: Option<SchemaFields<'de>>,

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

/// Decode one ASUN document into `T`.
///
/// The whole input must be exactly one top-level form (GRAMMAR.abnf
/// `asun-document`): a single leading BOM and surrounding whitespace /
/// comments are allowed; an empty document, a bare top-level tuple and
/// trailing content are errors.
pub fn decode<'a, T: AsunDecode<'a>>(s: &'a str) -> Result<T> {
    let mut de = Decoder::new(s.as_bytes());
    if de.input.starts_with("\u{FEFF}".as_bytes()) {
        de.pos = 3;
    }
    de.skip_layout();
    match de.input.get(de.pos) {
        None => return Err(de.unexpected_or(Error::EmptyDocument)),
        Some(b'(') => return Err(Error::BareTuple),
        Some(_) => {}
    }
    let value = T::decode(&mut de)?;
    de.skip_layout();
    if de.pos < de.input.len() {
        return Err(de.unexpected_or(Error::TrailingCharacters));
    }
    Ok(value)
}

impl<'de> Decoder<'de> {
    fn new(input: &'de [u8]) -> Self {
        Decoder {
            input,
            pos: 0,
            schema_fields: None,
            vec_schema_active: false,
            pending: None,
            depth: 0,
            last_struct_mode: [const { None }; MRU_SLOTS],
            last_struct_mode_head: 0,
            struct_mode_cache_local: HashMap::default(),
            schema_arena: Vec::new(),
            missing_arena: Vec::new(),
            default_depth: 0,
            seq_frames: Vec::new(),
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

    /// Pin `schema` for the rest of this decode and hand back a `'de` borrow.
    ///
    /// The arena holds a strong reference until the `Decoder` is dropped, and
    /// the `Schema` behind an `Arc` never moves, so the borrow (and borrows of
    /// any nested schema it owns) outlives every use the decoder makes of it.
    #[inline]
    fn intern_schema(&mut self, schema: CachedSchema) -> &'de Schema {
        let ptr: *const Schema = Arc::as_ptr(&schema);
        self.schema_arena.push(schema);
        unsafe { &*ptr }
    }

    // -----------------------------------------------------------------------
    // Lexical helpers
    // -----------------------------------------------------------------------

    #[inline(always)]
    fn at_comment(&self, pos: usize) -> bool {
        pos + 1 < self.input.len() && self.input[pos] == b'/' && self.input[pos + 1] == b'*'
    }

    /// True when a scalar token (number, keyword) may end at `pos`: end of
    /// input, a slot delimiter, layout, or a comment.
    #[inline(always)]
    fn is_token_end_at(&self, pos: usize) -> bool {
        match self.input.get(pos) {
            None => true,
            Some(b',' | b')' | b']' | b' ' | b'\t' | b'\n' | b'\r') => true,
            Some(b'/') => self.at_comment(pos),
            Some(_) => false,
        }
    }

    /// The cursor is at an empty slot: the next byte closes or separates it.
    #[inline(always)]
    fn at_value_end(&self) -> bool {
        matches!(self.input.get(self.pos), None | Some(b',' | b')' | b']'))
    }

    /// The cursor is at the keyword `null`.
    #[inline(always)]
    fn at_null_keyword(&self) -> bool {
        self.input.len() >= self.pos + 4
            && &self.input[self.pos..self.pos + 4] == b"null"
            && self.is_token_end_at(self.pos + 4)
    }

    /// The cursor is at a null slot (empty or `null`). Cold diagnostic helper.
    #[cold]
    fn at_null(&self) -> bool {
        self.at_value_end() || self.at_null_keyword()
    }

    /// The most precise error for "the byte at the cursor is not acceptable
    /// here": end of input, an unclosed comment, or the offending character.
    #[cold]
    #[inline(never)]
    fn unexpected(&self) -> Error {
        self.unexpected_or(Error::Eof)
    }

    #[cold]
    #[inline(never)]
    fn unexpected_or(&self, at_end: Error) -> Error {
        if self.pos >= self.input.len() {
            return at_end;
        }
        if self.at_comment(self.pos) {
            return Error::UnclosedComment;
        }
        let rest = unsafe { core::str::from_utf8_unchecked(&self.input[self.pos..]) };
        match rest.chars().next() {
            Some(c) => Error::UnexpectedChar(c),
            None => at_end,
        }
    }

    /// Diagnose a scalar that failed to parse: a null slot gets its own error.
    #[cold]
    #[inline(never)]
    fn scalar_error(&self, fallback: Error) -> Error {
        if self.at_null() {
            Error::NullNotAllowed
        } else {
            fallback
        }
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

    /// Skip whitespace and complete comments. An unterminated comment is left
    /// in place so the caller's next token check reports it (see
    /// [`Decoder::unexpected`]).
    #[inline]
    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.skip_whitespace();
            if !self.at_comment(self.pos) {
                break;
            }
            let mut p = self.pos + 2;
            loop {
                if p + 1 >= self.input.len() {
                    return;
                }
                if self.input[p] == b'*' && self.input[p + 1] == b'/' {
                    break;
                }
                p += 1;
            }
            self.pos = p + 2;
        }
    }

    /// Skip layout (GRAMMAR.abnf `ows`): whitespace and comments.
    #[inline(always)]
    fn skip_layout(&mut self) {
        self.skip_whitespace();
        if self.at_comment(self.pos) {
            self.skip_whitespace_and_comments();
        }
    }

    /// Consume the `,` before slot `index` of a group whose closer is
    /// `closer`, reporting a too-short group precisely.
    #[inline(always)]
    fn expect_slot_comma(&mut self, closer: u8, expected: usize, index: usize) -> Result<()> {
        self.skip_layout();
        match self.input.get(self.pos) {
            Some(b',') => {
                self.pos += 1;
                Ok(())
            }
            Some(&b) if b == closer => Err(Error::FieldCountMismatch {
                expected: expected as u32,
                got: index as u32,
            }),
            _ => Err(self.unexpected()),
        }
    }

    /// Consume `closer` at the end of a group of `expected` slots.
    #[inline(always)]
    fn expect_group_close(&mut self, closer: u8, expected: usize) -> Result<()> {
        self.skip_layout();
        match self.input.get(self.pos) {
            Some(&b) if b == closer => {
                self.pos += 1;
                Ok(())
            }
            Some(b',') => Err(Error::FieldCountMismatch {
                expected: expected as u32,
                got: expected as u32 + 1,
            }),
            None => Err(if closer == b')' {
                Error::UnclosedParen
            } else {
                Error::UnclosedBracket
            }),
            _ => Err(self.unexpected()),
        }
    }

    // -----------------------------------------------------------------------
    // Schema
    // -----------------------------------------------------------------------

    /// Parse the `{...}` schema at the cursor, through the per-thread cache.
    fn parse_schema(&mut self) -> Result<SchemaFields<'de>> {
        let open_pos = self.pos;
        if self.peek_byte()? != b'{' {
            return Err(Error::ExpectedOpenBrace);
        }
        let schema_end = self.find_schema_end(open_pos)?;
        // Copy the slice reference out of `self` so the key does not keep an
        // immutable borrow of the decoder alive across the parsing below.
        let schema_key: &'de [u8] = &self.input[open_pos..=schema_end];

        if let Some(schema) = SCHEMA_CACHE.with(|c| c.borrow().get(schema_key).cloned()) {
            self.pos = schema_end + 1;
            return Ok(SchemaFields::Cached(self.intern_schema(schema)));
        }

        let schema: CachedSchema = Arc::new(self.parse_schema_body()?);
        debug_assert_eq!(self.pos, schema_end + 1);
        SCHEMA_CACHE.with(|c| {
            let mut c = c.borrow_mut();
            // Cheapest bounded policy that keeps the common case (a handful of
            // schemas reused forever) allocation-free: drop everything once the
            // cap is hit rather than tracking recency.
            if c.len() >= SCHEMA_CACHE_CAP {
                c.clear();
            }
            c.insert(schema_key.into(), schema.clone());
        });
        Ok(SchemaFields::Cached(self.intern_schema(schema)))
    }

    /// Parse `{ field, field, ... }` (GRAMMAR.abnf `schema`) into a tree.
    fn parse_schema_body(&mut self) -> Result<Schema> {
        self.enter()?;
        let r = self.parse_schema_body_inner();
        self.leave();
        r
    }

    fn parse_schema_body_inner(&mut self) -> Result<Schema> {
        if self.peek_byte()? != b'{' {
            return Err(Error::ExpectedOpenBrace);
        }
        self.pos += 1;
        let mut names: Vec<Box<str>> = Vec::new();
        let mut types: Vec<Ty> = Vec::new();
        self.skip_layout();
        if self.peek_byte()? == b'}' {
            self.pos += 1;
            return Ok(Schema::new(names, types));
        }
        loop {
            self.skip_layout();
            let name: Box<str> = match self.peek_byte()? {
                b'"' => match self.parse_quoted_string_cow()? {
                    CowStr::Borrowed(s) => s.into(),
                    CowStr::Owned(s) => s.into_boxed_str(),
                },
                _ => {
                    let start = self.pos;
                    while self.pos < self.input.len()
                        && matches!(self.input[self.pos], b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_')
                    {
                        self.pos += 1;
                    }
                    let ok_end = match self.input.get(self.pos) {
                        None => true,
                        Some(b',' | b'}' | b'@' | b' ' | b'\t' | b'\n' | b'\r') => true,
                        Some(b'/') => self.at_comment(self.pos),
                        Some(_) => false,
                    };
                    if start == self.pos {
                        // `{a,}` / `{,a}`: no empty field slots in a schema.
                        return Err(self.unexpected());
                    }
                    if !ok_end {
                        return Err(Error::InvalidFieldName);
                    }
                    // Bare names are ASCII by construction.
                    unsafe { core::str::from_utf8_unchecked(&self.input[start..self.pos]) }.into()
                }
            };
            self.skip_layout();
            let ty = if self.input.get(self.pos) == Some(&b'@') {
                self.pos += 1;
                self.skip_layout();
                self.parse_binding()?
            } else {
                Ty::Any
            };
            names.push(name);
            types.push(ty);
            self.skip_layout();
            match self.input.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(self.unexpected()),
            }
        }
        // S6: duplicate names would make by-name mapping ambiguous. Schemas
        // are parsed once and then cached, so this cost is off the hot path.
        if names.len() <= 16 {
            for i in 1..names.len() {
                if names[..i].contains(&names[i]) {
                    return Err(Error::DuplicateField);
                }
            }
        } else {
            let mut seen = std::collections::HashSet::with_capacity(names.len());
            if !names.iter().all(|n| seen.insert(&**n)) {
                return Err(Error::DuplicateField);
            }
        }
        Ok(Schema::new(names, types))
    }

    /// Parse the binding after `@` (GRAMMAR.abnf `binding`).
    fn parse_binding(&mut self) -> Result<Ty> {
        self.enter()?;
        let r = self.parse_binding_inner();
        self.leave();
        r
    }

    fn parse_binding_inner(&mut self) -> Result<Ty> {
        match self.input.get(self.pos) {
            Some(b'{') => Ok(Ty::Obj(self.parse_schema_body()?)),
            Some(b'[') => {
                self.pos += 1;
                self.skip_layout();
                if self.input.get(self.pos) == Some(&b']') {
                    self.pos += 1;
                    return Ok(Ty::Arr(Box::new(Ty::Any)));
                }
                let inner = self.parse_binding()?;
                self.skip_layout();
                if self.input.get(self.pos) != Some(&b']') {
                    return Err(Error::msg("expected ']' in array type annotation"));
                }
                self.pos += 1;
                Ok(Ty::Arr(Box::new(inner)))
            }
            _ => {
                let start = self.pos;
                while self.pos < self.input.len() && self.input[self.pos].is_ascii_alphanumeric() {
                    self.pos += 1;
                }
                match &self.input[start..self.pos] {
                    b"int" => Ok(Ty::Int),
                    b"float" => Ok(Ty::Float),
                    b"str" => Ok(Ty::Str),
                    b"bool" => Ok(Ty::Bool),
                    b"" => Err(Error::msg("expected schema type after '@'")),
                    other => Err(Error::msg(format!(
                        "unsupported schema type '{}'; use int, str, float, or bool",
                        String::from_utf8_lossy(other)
                    ))),
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // Skipping (unknown source fields)
    // -----------------------------------------------------------------------

    /// Skip the value in one slot, leaving the cursor on the `,` / `)` / `]`
    /// that ends it. An empty slot is a no-op.
    ///
    /// Tracks quotes, escapes, comments and bracket nesting, and every
    /// iteration consumes at least one byte or returns, so no input can make it
    /// loop forever.
    fn skip_value(&mut self) -> Result<()> {
        self.skip_layout();
        // One bit per open group: 1 = `(`, 0 = `[`.
        let mut stack: u128 = 0;
        let mut depth: u32 = 0;
        loop {
            let Some(&b) = self.input.get(self.pos) else {
                return if depth == 0 { Ok(()) } else { Err(Error::Eof) };
            };
            match b {
                b'"' => self.skip_quoted_string()?,
                b'\\' => {
                    if self.pos + 1 >= self.input.len() {
                        return Err(Error::InvalidEscape('\\'));
                    }
                    self.pos += 2;
                }
                b'/' if self.at_comment(self.pos) => {
                    let before = self.pos;
                    self.skip_whitespace_and_comments();
                    if self.pos == before {
                        return Err(Error::UnclosedComment);
                    }
                }
                b'(' | b'[' => {
                    if depth >= MAX_DEPTH {
                        return Err(Error::DepthLimitExceeded);
                    }
                    stack = (stack << 1) | (b == b'(') as u128;
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' => {
                    if depth == 0 {
                        return Ok(());
                    }
                    if (stack & 1 == 1) != (b == b')') {
                        return Err(Error::UnexpectedChar(b as char));
                    }
                    stack >>= 1;
                    depth -= 1;
                    self.pos += 1;
                }
                b',' if depth == 0 => return Ok(()),
                b'{' | b'}' => return Err(Error::UnexpectedChar(b as char)),
                _ => self.pos += 1,
            }
        }
    }

    // -----------------------------------------------------------------------
    // Strings
    // -----------------------------------------------------------------------

    /// Parse a plain (unquoted) string value (GRAMMAR.abnf `plain-string`),
    /// stopping at a delimiter, a control character or a comment, and trimming
    /// trailing whitespace. Returns a zero-copy borrow plus whether it
    /// contains escapes.
    #[inline]
    fn parse_plain_value_meta(&mut self) -> Result<(&'de str, bool)> {
        let start = self.pos;
        let mut has_escape = false;
        loop {
            self.pos = simd::simd_find_plain_delimiter(self.input, self.pos);
            match self.input.get(self.pos) {
                Some(b'\\') => {
                    has_escape = true;
                    // A trailing backslash with no following byte is malformed;
                    // a bare `self.pos += 2` would push past the end.
                    if self.pos + 1 >= self.input.len() {
                        return Err(Error::InvalidEscape('\\'));
                    }
                    self.pos += 2;
                }
                // Tab is whitespace, legal inside a plain string.
                Some(b'\t') => self.pos += 1,
                // A lone `/` is content; `/*` opens a comment.
                Some(b'/') if !self.at_comment(self.pos) => self.pos += 1,
                _ => break,
            }
        }
        let mut end = self.pos;
        while end > start && matches!(self.input[end - 1], b' ' | b'\t') {
            end -= 1;
        }
        // When escapes are present the slice may split a multi-byte UTF-8
        // sequence (pos advanced by 2 past a `\`), which would make the
        // `from_utf8_unchecked` reference invalid. Validate in that case; the
        // no-escape fast path is guaranteed valid because the input is `&str`
        // and every stop byte is ASCII.
        let bytes = &self.input[start..end];
        let raw = if has_escape {
            core::str::from_utf8(bytes).map_err(|_| Error::InvalidEscape('\\'))?
        } else {
            unsafe { core::str::from_utf8_unchecked(bytes) }
        };
        Ok((raw, has_escape))
    }

    /// Parse a quoted string. Zerocopy when no escapes; allocates only when escapes present.
    /// Uses SIMD to scan for `"`, `\` or a (forbidden) raw control character.
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
                    b'/' => result.push('/'),
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
            } else if b < 0x20 {
                return Err(Error::ControlCharInString);
            } else {
                // After an escape sequence, SIMD scan for next quote/backslash
                let next_hit = simd::simd_find_quote_or_backslash(self.input, self.pos);
                let chunk =
                    unsafe { core::str::from_utf8_unchecked(&self.input[self.pos..next_hit]) };
                result.push_str(chunk);
                self.pos = next_hit;
            }
        }
    }

    #[inline]
    fn skip_quoted_string(&mut self) -> Result<()> {
        self.pos += 1;
        loop {
            let hit = simd::simd_find_quote_or_backslash(self.input, self.pos);
            if hit >= self.input.len() {
                return Err(Error::UnclosedString);
            }
            self.pos = hit;
            match self.input[self.pos] {
                b'"' => {
                    self.pos += 1;
                    return Ok(());
                }
                b'\\' => self.pos += 2,
                _ => return Err(Error::ControlCharInString),
            }
        }
    }

    /// Parse a string-typed slot (quoted or plain). A null slot is an error.
    #[inline]
    fn parse_string_slot(&mut self) -> Result<CowStr<'de>> {
        if let Some(hint) = self.pending.take()
            && !matches!(hint, Ty::Str)
        {
            return Err(Error::HintMismatch);
        }
        self.skip_layout();
        if self.input.get(self.pos) == Some(&b'"') {
            return self.parse_quoted_string_cow();
        }
        let (v, has_escape) = self.parse_plain_value_meta()?;
        if v.is_empty() {
            return Err(self.scalar_error(self.unexpected()));
        }
        if has_escape {
            return Ok(CowStr::Owned(unescape_plain(v)?));
        }
        if v == "null" {
            return Err(Error::NullNotAllowed);
        }
        Ok(CowStr::Borrowed(v))
    }

    // -----------------------------------------------------------------------
    // Numbers
    // -----------------------------------------------------------------------

    /// Check a numeric hint against an integer target.
    #[inline(always)]
    fn take_int_hint(&mut self) -> Result<()> {
        match self.pending.take() {
            None | Some(Ty::Int) => Ok(()),
            Some(_) => Err(Error::HintMismatch),
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
                .ok_or(Error::IntegerOutOfRange)?;
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 || !self.is_token_end_at(self.pos) {
            return Err(Error::InvalidNumber);
        }
        if negative {
            // Magnitude fits in i64 (>= -2^63) exactly when val <= 2^63.
            if val > (i64::MAX as u64) + 1 {
                return Err(Error::IntegerOutOfRange);
            }
            Ok((val as i64).wrapping_neg())
        } else {
            if val > i64::MAX as u64 {
                return Err(Error::IntegerOutOfRange);
            }
            Ok(val as i64)
        }
    }

    /// Parse u64 directly. Optimized loop with wrapping_sub for digit check.
    #[inline]
    fn parse_u64(&mut self) -> Result<u64> {
        let negative = self.input.get(self.pos) == Some(&b'-');
        let start = self.pos;
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
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add(d as u64))
                .ok_or(Error::IntegerOutOfRange)?;
            self.pos += 1;
            digits += 1;
        }
        if digits == 0 || !self.is_token_end_at(self.pos) {
            self.pos = start;
            return Err(Error::InvalidNumber);
        }
        // `-0` is the integer 0; any other negative value does not fit.
        if negative && val != 0 {
            return Err(Error::IntegerOutOfRange);
        }
        Ok(val)
    }

    /// Parse f64 directly using fast-float for speed.
    ///
    /// Enforces GRAMMAR.abnf `number = ["-"] 1*DIGIT ["." 1*DIGIT] [exponent]`:
    /// integers are accepted (a float target takes `3` as `3.0`), but `"5."`,
    /// `".5"` or `"+5"` are not numbers. With `int_only` (an `@int` hint) a
    /// fraction or exponent is a hint mismatch. Overflow to infinity is an
    /// error (SPEC S4).
    #[inline]
    fn parse_f64_direct(&mut self, int_only: bool) -> Result<f64> {
        let start = self.pos;
        if self.pos < self.input.len() && self.input[self.pos] == b'-' {
            self.pos += 1;
        }
        let int_start = self.pos;
        while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
            self.pos += 1;
        }
        if self.pos == int_start {
            return Err(Error::InvalidNumber);
        }
        let mut had_dot_or_exp = false;
        if self.pos < self.input.len() && self.input[self.pos] == b'.' {
            had_dot_or_exp = true;
            self.pos += 1;
            let frac_start = self.pos;
            while self.pos < self.input.len() && self.input[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
            if self.pos == frac_start {
                return Err(Error::InvalidNumber);
            }
        }
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
            if self.pos == exp_start {
                return Err(Error::InvalidNumber);
            }
        }
        if !self.is_token_end_at(self.pos) {
            return Err(Error::InvalidNumber);
        }
        if int_only && had_dot_or_exp {
            return Err(Error::HintMismatch);
        }
        let v: f64 =
            fast_float2::parse(&self.input[start..self.pos]).map_err(|_| Error::InvalidNumber)?;
        if v.is_infinite() {
            return Err(Error::FloatOverflow);
        }
        Ok(v)
    }

    /// Check a numeric hint against a float target; `true` = integer literal
    /// required.
    #[inline(always)]
    fn take_float_hint(&mut self) -> Result<bool> {
        match self.pending.take() {
            None | Some(Ty::Float) => Ok(false),
            Some(Ty::Int) => Ok(true),
            Some(_) => Err(Error::HintMismatch),
        }
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
    // reading the input. Each consumes the slot's pending type hint.
    // =======================================================================

    #[inline]
    pub fn decode_bool(&mut self) -> Result<bool> {
        if self.default_depth > 0 {
            return Ok(false);
        }
        if let Some(hint) = self.pending.take()
            && !matches!(hint, Ty::Bool)
        {
            return Err(Error::HintMismatch);
        }
        self.skip_layout();
        if let Some(value) = self.parse_bool_literal() {
            return Ok(value);
        }
        Err(self.scalar_error(Error::InvalidBool))
    }

    #[inline]
    fn decode_signed(&mut self) -> Result<i64> {
        self.take_int_hint()?;
        self.skip_layout();
        let start = self.pos;
        self.parse_i64().map_err(|e| {
            self.pos = start;
            self.scalar_error(e)
        })
    }

    #[inline]
    fn decode_unsigned(&mut self) -> Result<u64> {
        self.take_int_hint()?;
        self.skip_layout();
        let start = self.pos;
        self.parse_u64().map_err(|e| {
            self.pos = start;
            self.scalar_error(e)
        })
    }

    #[inline]
    pub fn decode_i8(&mut self) -> Result<i8> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        i8::try_from(self.decode_signed()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i16(&mut self) -> Result<i16> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        i16::try_from(self.decode_signed()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i32(&mut self) -> Result<i32> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        i32::try_from(self.decode_signed()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_i64(&mut self) -> Result<i64> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.decode_signed()
    }

    #[inline]
    pub fn decode_u8(&mut self) -> Result<u8> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        u8::try_from(self.decode_unsigned()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u16(&mut self) -> Result<u16> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        u16::try_from(self.decode_unsigned()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u32(&mut self) -> Result<u32> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        u32::try_from(self.decode_unsigned()?).map_err(|_| Error::IntegerOutOfRange)
    }

    #[inline]
    pub fn decode_u64(&mut self) -> Result<u64> {
        if self.default_depth > 0 {
            return Ok(0);
        }
        self.decode_unsigned()
    }

    #[inline]
    pub fn decode_f32(&mut self) -> Result<f32> {
        if self.default_depth > 0 {
            return Ok(0.0);
        }
        let v = self.decode_f64_inner()?;
        let narrow = v as f32;
        if narrow.is_infinite() {
            return Err(Error::FloatOverflow);
        }
        Ok(narrow)
    }

    #[inline]
    pub fn decode_f64(&mut self) -> Result<f64> {
        if self.default_depth > 0 {
            return Ok(0.0);
        }
        self.decode_f64_inner()
    }

    #[inline]
    fn decode_f64_inner(&mut self) -> Result<f64> {
        let int_only = self.take_float_hint()?;
        self.skip_layout();
        let start = self.pos;
        self.parse_f64_direct(int_only).map_err(|e| {
            self.pos = start;
            self.scalar_error(e)
        })
    }

    #[inline]
    pub fn decode_char(&mut self) -> Result<char> {
        if self.default_depth > 0 {
            return Ok('\0');
        }
        let cow = self.parse_string_slot()?;
        let mut chars = cow.as_str().chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => Ok(c),
            _ => Err(Error::msg("expected a single character")),
        }
    }

    #[inline]
    pub fn decode_string(&mut self) -> Result<String> {
        if self.default_depth > 0 {
            return Ok(String::new());
        }
        Ok(match self.parse_string_slot()? {
            CowStr::Borrowed(s) => s.to_owned(),
            CowStr::Owned(s) => s,
        })
    }

    /// Zero-copy borrowed str decode.
    #[inline]
    pub fn decode_borrowed_str(&mut self) -> Result<&'de str> {
        if self.default_depth > 0 {
            return Ok("");
        }
        match self.parse_string_slot()? {
            CowStr::Borrowed(s) => Ok(s),
            // An escaped string cannot be borrowed: the target is `&'de str`,
            // which fundamentally cannot hold an unescaped owned buffer.
            CowStr::Owned(_) => Err(Error::msg("cannot borrow &str from an escaped string")),
        }
    }

    #[inline]
    pub fn decode_option<T: AsunDecode<'de>>(&mut self) -> Result<Option<T>> {
        if self.default_depth > 0 {
            return Ok(None);
        }
        self.skip_layout();
        if self.at_value_end() {
            self.pending = None;
            Ok(None)
        } else if self.at_null_keyword() {
            self.pending = None;
            self.pos += 4;
            Ok(None)
        } else {
            Ok(Some(T::decode(self)?))
        }
    }

    /// Decode `()` / a unit struct. Unit carries no data, so it is a null slot
    /// (empty or `null`); the legacy `()` spelling is accepted too.
    #[inline]
    pub fn decode_unit(&mut self) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        self.pending = None;
        self.skip_layout();
        if self.at_value_end() {
            return Ok(());
        }
        if self.at_null_keyword() {
            self.pos += 4;
            return Ok(());
        }
        if self.input.get(self.pos) == Some(&b'(') {
            self.pos += 1;
            self.skip_layout();
            if self.input.get(self.pos) == Some(&b')') {
                self.pos += 1;
                return Ok(());
            }
        }
        Err(self.unexpected())
    }

    /// Decode a homogeneous sequence `Vec<T>`.
    ///
    /// Handles both `[v1,v2,...]` (plain array) and, at the top level,
    /// `[{schema}]:(row),(row)` (struct array with a shared schema).
    pub fn decode_vec<T: AsunDecode<'de>>(&mut self) -> Result<Vec<T>> {
        if self.default_depth > 0 {
            return Ok(Vec::new());
        }
        self.enter()?;
        let r = self.decode_vec_inner();
        self.leave();
        r
    }

    /// After `[` at `pos`, does `{` follow (a `[{schema}]:` header)?
    #[inline]
    fn header_follows(&mut self) -> bool {
        let save = self.pos;
        self.pos += 1;
        self.skip_layout();
        let r = self.input.get(self.pos) == Some(&b'{');
        self.pos = save;
        r
    }

    fn decode_vec_inner<T: AsunDecode<'de>>(&mut self) -> Result<Vec<T>> {
        let pending = self.pending.take();
        self.skip_layout();
        if self.input.get(self.pos) != Some(&b'[') {
            return Err(self.scalar_error(Error::ExpectedOpenBracket));
        }
        if self.depth == 1 && pending.is_none() && self.header_follows() {
            return self.decode_vec_rows();
        }
        let elem: Option<&'de Ty> = match pending {
            None => None,
            Some(Ty::Arr(e)) => e.as_hint(),
            Some(_) => return Err(Error::HintMismatch),
        };
        self.pos += 1;
        let mut out = Vec::new();
        self.skip_layout();
        if self.input.get(self.pos) == Some(&b']') {
            self.pos += 1;
            return Ok(out);
        }
        loop {
            self.pending = elem;
            out.push(T::decode(self)?);
            self.skip_layout();
            match self.input.get(self.pos) {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    return Ok(out);
                }
                None => return Err(Error::UnclosedBracket),
                _ => return Err(self.unexpected()),
            }
        }
    }

    /// `[{schema}]:(row),(row),...` — zero or more rows, no trailing comma,
    /// never a null row (SPEC S9).
    fn decode_vec_rows<T: AsunDecode<'de>>(&mut self) -> Result<Vec<T>> {
        self.pos += 1; // '['
        self.skip_layout();
        let fields = self.parse_schema()?;
        self.skip_layout();
        if self.input.get(self.pos) != Some(&b']') {
            return Err(Error::ExpectedCloseBracket);
        }
        self.pos += 1;
        self.skip_layout();
        if self.input.get(self.pos) != Some(&b':') {
            return Err(Error::ExpectedColon);
        }
        self.pos += 1;

        let mut out = Vec::new();
        self.skip_layout();
        if self.input.get(self.pos) == Some(&b'(') {
            loop {
                self.schema_fields = Some(fields);
                self.vec_schema_active = true;
                out.push(T::decode(self)?);
                self.vec_schema_active = false;
                self.skip_layout();
                if self.input.get(self.pos) != Some(&b',') {
                    break;
                }
                self.pos += 1;
                self.skip_layout();
                if self.input.get(self.pos) != Some(&b'(') {
                    return Err(self.unexpected());
                }
            }
        }
        self.schema_fields = None;
        Ok(out)
    }

    // -----------------------------------------------------------------------
    // Tuple seam (plain tuples, tuple structs, tuple enum variants)
    // -----------------------------------------------------------------------

    /// Begin decoding a positional tuple: `[a,b]` (current encoding) or
    /// `(a,b)` (legacy, inside data only). In default mode this is a no-op.
    #[inline]
    pub fn begin_tuple(&mut self) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        match self.pending.take() {
            None | Some(Ty::Arr(_)) => {}
            Some(_) => return Err(Error::HintMismatch),
        }
        self.skip_layout();
        let closer = match self.input.get(self.pos) {
            Some(b'[') => b']',
            Some(b'(') => b')',
            _ => return Err(self.scalar_error(Error::ExpectedOpenBracket)),
        };
        self.enter()?;
        self.pos += 1;
        self.seq_frames.push(SeqFrame { closer, index: 0 });
        Ok(())
    }

    /// Decode the next tuple element in positional order.
    #[inline]
    pub fn tuple_element<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        if self.default_depth > 0 {
            return T::decode(self);
        }
        let frame = self
            .seq_frames
            .last_mut()
            .expect("tuple_element outside a tuple");
        let (closer, index) = (frame.closer, frame.index as usize);
        frame.index += 1;
        if index > 0 {
            self.expect_slot_comma(closer, index + 1, index)?;
        } else if closer == b']' {
            // `[]` is an array with zero slots, not one null slot.
            self.skip_layout();
            if self.input.get(self.pos) == Some(&b']') {
                return Err(Error::FieldCountMismatch {
                    expected: 1,
                    got: 0,
                });
            }
        }
        T::decode(self)
    }

    /// Finish decoding a tuple of `count` declared elements and consume the
    /// closer; extra source elements are an error.
    #[inline]
    pub fn end_tuple(&mut self, count: usize) -> Result<()> {
        if self.default_depth > 0 {
            return Ok(());
        }
        let frame = self.seq_frames.pop().expect("end_tuple outside a tuple");
        if frame.closer == 0 {
            // Enum variant body: the enum frame owns the closer.
            return Ok(());
        }
        self.leave();
        self.expect_group_close(frame.closer, count)
    }

    /// Produce a type default for `T` by running its decode logic in default
    /// mode. This is the direct analog of the previous `DefaultValueDeserializer`.
    #[inline]
    fn decode_default<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        self.pending = None;
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
                byname_source_index: 0,
                byname_in_defaults: false,
                byname_default_index: 0,
                byname_missing: NO_MISSING,
            });
            return Ok(StructDecodeMode::Exact);
        }

        let pending = self.pending.take();
        self.enter()?;
        self.skip_layout();

        let parent_schema = self.schema_fields;
        match self.input.get(self.pos) {
            Some(b'(') if self.vec_schema_active => {
                // A row of `[{schema}]:` — the header is the source schema.
                self.pos += 1;
                self.vec_schema_active = false;
            }
            Some(b'(') => {
                // Nested object data: names come from the parent's `@{...}`
                // binding when there is one, else from the target, by position.
                self.pos += 1;
                self.schema_fields = Some(match pending {
                    None => SchemaFields::Static(target_fields),
                    Some(Ty::Obj(schema)) => SchemaFields::Cached(schema),
                    Some(_) => return Err(Error::HintMismatch),
                });
            }
            Some(b'{') if self.depth == 1 => {
                // Top-level `{schema}:(...)`.
                let parsed = self.parse_schema()?;
                self.skip_layout();
                if self.input.get(self.pos) != Some(&b':') {
                    return Err(Error::ExpectedColon);
                }
                self.pos += 1;
                self.skip_layout();
                if self.input.get(self.pos) != Some(&b'(') {
                    return Err(Error::ExpectedOpenParen);
                }
                self.pos += 1;
                self.schema_fields = Some(parsed);
            }
            _ => return Err(self.scalar_error(Error::ExpectedOpenParen)),
        }

        let (decode_mode, byname_missing) = match self.struct_plan(target_fields) {
            StructPlan::Exact => (StructDecodeMode::Exact, NO_MISSING),
            StructPlan::ByName(idx) => (StructDecodeMode::ByName, idx),
        };

        self.struct_frames.push(StructFrame {
            parent_schema,
            byname_source_index: 0,
            byname_in_defaults: false,
            byname_default_index: 0,
            byname_missing,
        });
        Ok(decode_mode)
    }

    /// Number of slots the current struct's source tuple must have.
    #[inline(always)]
    fn source_len(&self) -> usize {
        self.schema_fields.map_or(0, |f| f.len())
    }

    /// Exact mode: read the field at positional index `index`.
    #[inline]
    pub fn struct_field_positional<T: AsunDecode<'de>>(&mut self, index: usize) -> Result<T> {
        if self.default_depth > 0 {
            return T::decode(self);
        }
        // Hot path: the derive calls this once per field with `index` =
        // 0,1,2,… so commas are driven off `index` alone and the frame stack
        // is never touched here.
        if index > 0 {
            self.expect_slot_comma(b')', self.source_len(), index)?;
        }
        self.pending = match self.schema_fields {
            Some(fields) => fields.ty_at(index),
            None => None,
        };
        T::decode(self)
    }

    /// ByName mode: return the next source field's name, or `None` when the
    /// source tuple is exhausted. After the source is drained, this emits the
    /// names of missing target fields (whose values decode as defaults).
    pub fn next_struct_key(&mut self) -> Result<Option<&'de str>> {
        let frame_idx = self.struct_frames.len() - 1;
        if self.struct_frames[frame_idx].byname_in_defaults {
            return self.next_missing_default_key(frame_idx);
        }
        let Some(fields) = self.schema_fields else {
            return Ok(None);
        };
        let source_index = self.struct_frames[frame_idx].byname_source_index;
        if source_index >= fields.len() {
            return self.next_missing_default_key(frame_idx);
        }
        if source_index > 0 {
            self.expect_slot_comma(b')', fields.len(), source_index)?;
        }
        self.struct_frames[frame_idx].byname_source_index += 1;
        self.pending = fields.ty_at(source_index);
        Ok(Some(fields.name_at(source_index)))
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
    /// `next_struct_key`.
    #[inline]
    pub fn struct_field_value<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        let frame_idx = self.struct_frames.len() - 1;
        if self.struct_frames[frame_idx].byname_in_defaults {
            return self.decode_default::<T>();
        }
        T::decode(self)
    }

    /// ByName mode: skip the value for an unmatched source key.
    #[inline]
    pub fn skip_struct_value(&mut self) -> Result<()> {
        let frame_idx = self.struct_frames.len() - 1;
        if self.struct_frames[frame_idx].byname_in_defaults {
            // A missing-target default key: nothing in the input to skip.
            return Ok(());
        }
        self.pending = None;
        self.skip_value()
    }

    /// ByName mode: produce a type default for an unmatched target field.
    #[inline]
    pub fn struct_field_default<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        self.decode_default::<T>()
    }

    /// Finish decoding a struct: the tuple must close right after its last
    /// slot (SPEC S1), then the parent schema state is restored.
    pub fn end_struct_decode(&mut self) -> Result<()> {
        let frame = self
            .struct_frames
            .pop()
            .expect("end_struct_decode without begin_struct_decode");

        if self.default_depth > 0 {
            return Ok(());
        }

        self.leave();
        // Exact mode with a full field match leaves the cursor right on the
        // closing paren, which is the overwhelmingly common case.
        if self.input.get(self.pos) == Some(&b')') {
            self.pos += 1;
        } else {
            let n = self.source_len();
            self.expect_group_close(b')', n)?;
        }
        self.schema_fields = frame.parent_schema;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Enum seam (used by derived AsunDecode impls)
    // -----------------------------------------------------------------------

    /// Begin decoding an enum: a bare variant name (unit variant) or
    /// `[variant, payload...]` (`(...)` accepted as the legacy spelling), and
    /// read the variant name.
    pub fn begin_enum(&mut self) -> Result<String> {
        if self.default_depth > 0 {
            return Err(Error::ExpectedValue);
        }
        self.pending = None;
        self.skip_layout();
        let closer = match self.input.get(self.pos) {
            Some(b'[') => b']',
            Some(b'(') => b')',
            _ => 0,
        };
        if closer != 0 {
            self.enter()?;
            self.pos += 1;
        }
        self.seq_frames.push(SeqFrame { closer, index: 1 });
        Ok(match self.parse_string_slot()? {
            CowStr::Borrowed(s) => s.to_owned(),
            CowStr::Owned(s) => s,
        })
    }

    /// Unit variant: nothing to read.
    #[inline]
    pub fn finish_unit_variant(&mut self) -> Result<()> {
        Ok(())
    }

    /// Newtype variant: skip the comma after the variant name, then decode the
    /// inner value.
    #[inline]
    pub fn newtype_variant_value<T: AsunDecode<'de>>(&mut self) -> Result<T> {
        let closer = self.seq_frames.last().map_or(0, |f| f.closer);
        if closer == 0 {
            return Err(Error::ExpectedOpenBracket);
        }
        self.expect_slot_comma(closer, 2, 1)?;
        T::decode(self)
    }

    /// Tuple / struct variant body: the elements that follow the variant name
    /// are read positionally via `tuple_element` and closed by `end_tuple`.
    #[inline]
    pub fn begin_tuple_variant_body(&mut self) -> Result<()> {
        let closer = self.seq_frames.last().map_or(0, |f| f.closer);
        if closer == 0 {
            return Err(Error::ExpectedOpenBracket);
        }
        // A bracket-less frame: its commas are checked against the enum's
        // closer, which `end_enum` consumes.
        self.seq_frames.push(SeqFrame {
            closer: 0,
            index: 1,
        });
        Ok(())
    }

    /// Finish decoding an enum: consume the closer if it was bracketed.
    #[inline]
    pub fn end_enum(&mut self) -> Result<()> {
        let frame = self.seq_frames.pop().expect("end_enum without begin_enum");
        if frame.closer != 0 {
            self.leave();
            self.skip_layout();
            if self.input.get(self.pos) != Some(&frame.closer) {
                return Err(self.unexpected());
            }
            self.pos += 1;
        }
        Ok(())
    }
}

/// Declared type of one schema slot: its `@...` binding (GRAMMAR.abnf
/// `binding`), or [`Ty::Any`] when the field has none.
pub(crate) enum Ty {
    Any,
    Int,
    Float,
    Str,
    Bool,
    /// `@{...}`: the nested object's own field names and types.
    Obj(Schema),
    /// `@[...]`; `Arr(Any)` for the untyped `@[]`.
    Arr(Box<Ty>),
}

impl Ty {
    /// The hint to hand to a value decoder: `None` for an unbound slot.
    #[inline(always)]
    fn as_hint(&self) -> Option<&Ty> {
        match self {
            Ty::Any => None,
            t => Some(t),
        }
    }
}

/// A parsed `{...}` schema: field names and their declared types.
pub(crate) struct Schema {
    names: Box<[Box<str>]>,
    types: Box<[Ty]>,
}

impl Schema {
    fn new(names: Vec<Box<str>>, types: Vec<Ty>) -> Self {
        Schema {
            names: names.into_boxed_slice(),
            types: types.into_boxed_slice(),
        }
    }
}

#[derive(Clone, Copy)]
enum SchemaFields<'de> {
    /// Borrowed from a schema pinned in [`Decoder::schema_arena`].
    Cached(&'de Schema),
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
            Self::Cached(schema) => schema.names.len(),
            Self::Static(fields) => fields.len(),
        }
    }

    #[inline(always)]
    fn name_at(&self, index: usize) -> &'de str {
        match self {
            Self::Cached(schema) => &schema.names[index],
            Self::Static(fields) => fields[index],
        }
    }

    /// Declared type of slot `index`, or `None` when it has no binding.
    #[inline(always)]
    fn ty_at(&self, index: usize) -> Option<&'de Ty> {
        match self {
            Self::Cached(schema) => schema.types.get(index).and_then(Ty::as_hint),
            Self::Static(_) => None,
        }
    }

    #[inline(always)]
    fn cache_key(&self) -> SchemaFieldsKey {
        match self {
            Self::Cached(schema) => SchemaFieldsKey {
                ptr: schema.names.as_ptr() as usize,
                len: schema.names.len(),
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
            Self::Cached(schema) => schema.names.iter().any(|n| &**n == target),
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
            b @ (b',' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b':' | b'@' | b'"' | b'\\'
            | b'/') => out.push(b),
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
