//! Cross-platform SIMD utilities for accelerating ASON parsing and serialization.
//!
//! Provides 128-bit SIMD operations for:
//! - **aarch64**: ARM NEON intrinsics (always available on AArch64)
//! - **x86_64**: SSE2 intrinsics (always available on x86-64)
//! - **fallback**: Scalar emulation for other architectures
//!
//! Inspired by sonic-rs / simdjson techniques.

// ============================================================================
// Platform-specific SIMD implementations
// ============================================================================

/// Number of bytes processed per SIMD iteration.
pub const LANES: usize = 16;

/// A 16-byte bitmask result from SIMD comparisons.
/// Each bit corresponds to one byte lane.
pub type Mask16 = u16;

/// Find the index of the first set bit in a mask.
#[inline(always)]
pub fn first_set_bit(mask: Mask16) -> u32 {
    mask.trailing_zeros()
}

/// Clear the `n` highest bits (set bits above position `n` to zero).
/// Used when processing a tail chunk smaller than LANES.
#[inline(always)]
pub fn clear_high_bits(mask: Mask16, n: usize) -> Mask16 {
    if n >= 16 {
        mask
    } else {
        mask & ((1u16 << n) - 1)
    }
}

// ---------------------------------------------------------------------------
// aarch64 / ARM NEON
// ---------------------------------------------------------------------------
#[cfg(target_arch = "aarch64")]
mod imp {
    use super::*;
    use core::arch::aarch64::*;

    /// Load 16 bytes from `ptr` (unaligned).
    #[inline(always)]
    pub unsafe fn load(ptr: *const u8) -> uint8x16_t {
        unsafe { vld1q_u8(ptr) }
    }

    /// Store 16 bytes from SIMD register to `ptr` (unaligned).
    #[inline(always)]
    pub unsafe fn store(ptr: *mut u8, v: uint8x16_t) {
        unsafe { vst1q_u8(ptr, v) };
    }

    /// Broadcast a single byte to all 16 lanes.
    #[inline(always)]
    pub unsafe fn splat(b: u8) -> uint8x16_t {
        unsafe { vdupq_n_u8(b) }
    }

    /// Compare equal: result lane = 0xFF where a[i] == b[i], else 0x00.
    #[inline(always)]
    pub unsafe fn cmpeq(a: uint8x16_t, b: uint8x16_t) -> uint8x16_t {
        unsafe { vceqq_u8(a, b) }
    }

    /// Compare less-than-or-equal (unsigned): result lane = 0xFF where a[i] <= b[i].
    #[inline(always)]
    pub unsafe fn cmple(a: uint8x16_t, b: uint8x16_t) -> uint8x16_t {
        unsafe { vcleq_u8(a, b) }
    }

    /// Bitwise OR of two vectors.
    #[inline(always)]
    pub unsafe fn or(a: uint8x16_t, b: uint8x16_t) -> uint8x16_t {
        unsafe { vorrq_u8(a, b) }
    }

    /// Convert a 16-byte comparison result to a 16-bit bitmask.
    /// Bit i is set if lane i is 0xFF.
    ///
    /// Uses the simdjson/sonic-rs shift-right-and-accumulate technique
    /// instead of 16 individual bit extractions.
    #[inline(always)]
    pub unsafe fn movemask(v: uint8x16_t) -> Mask16 {
        unsafe {
            // Extract high bit of each byte: 0xFF → 0x01, 0x00 → 0x00
            let high_bits = vreinterpretq_u16_u8(vshrq_n_u8(v, 7));
            // Shift-right-accumulate packs pairs: bits 0,1 → byte
            let paired16 = vreinterpretq_u32_u16(vsraq_n_u16(high_bits, high_bits, 7));
            // Pack quads
            let paired32 = vreinterpretq_u64_u32(vsraq_n_u32(paired16, paired16, 14));
            // Pack octets
            let paired64 = vreinterpretq_u8_u64(vsraq_n_u64(paired32, paired32, 28));
            // Extract low byte (lanes 0-7) and byte 8 (lanes 8-15)
            let lo = vgetq_lane_u8(paired64, 0) as u16;
            let hi = vgetq_lane_u8(paired64, 8) as u16;
            lo | (hi << 8)
        }
    }
}

// ---------------------------------------------------------------------------
// x86_64 / SSE2
// ---------------------------------------------------------------------------
#[cfg(target_arch = "x86_64")]
mod imp {
    use super::*;
    #[cfg(target_arch = "x86_64")]
    use core::arch::x86_64::*;

    /// Load 16 bytes from `ptr` (unaligned).
    #[inline(always)]
    pub unsafe fn load(ptr: *const u8) -> __m128i {
        _mm_loadu_si128(ptr as *const __m128i)
    }

    /// Store 16 bytes from SIMD register to `ptr` (unaligned).
    #[inline(always)]
    pub unsafe fn store(ptr: *mut u8, v: __m128i) {
        _mm_storeu_si128(ptr as *mut __m128i, v);
    }

    /// Broadcast a single byte to all 16 lanes.
    #[inline(always)]
    pub unsafe fn splat(b: u8) -> __m128i {
        _mm_set1_epi8(b as i8)
    }

    /// Compare equal: result lane = 0xFF where a[i] == b[i], else 0x00.
    #[inline(always)]
    pub unsafe fn cmpeq(a: __m128i, b: __m128i) -> __m128i {
        _mm_cmpeq_epi8(a, b)
    }

    /// Compare less-than-or-equal (unsigned): a[i] <= b[i].
    /// SSE2 lacks unsigned LE compare, so we use: a <= b ↔ max(a,b) == b.
    #[inline(always)]
    pub unsafe fn cmple(a: __m128i, b: __m128i) -> __m128i {
        _mm_cmpeq_epi8(_mm_max_epu8(a, b), b)
    }

    /// Bitwise OR of two vectors.
    #[inline(always)]
    pub unsafe fn or(a: __m128i, b: __m128i) -> __m128i {
        _mm_or_si128(a, b)
    }

    /// Convert a 16-byte comparison result to a 16-bit bitmask.
    /// Bit i is set if the high bit of lane i is set (i.e. lane == 0xFF).
    #[inline(always)]
    pub unsafe fn movemask(v: __m128i) -> Mask16 {
        _mm_movemask_epi8(v) as u16
    }
}

// ---------------------------------------------------------------------------
// Fallback (scalar emulation)
// ---------------------------------------------------------------------------
#[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
mod imp {
    use super::*;

    #[derive(Clone, Copy)]
    pub struct SimdVec([u8; LANES]);

    #[inline(always)]
    pub unsafe fn load(ptr: *const u8) -> SimdVec {
        let mut v = SimdVec([0u8; LANES]);
        core::ptr::copy_nonoverlapping(ptr, v.0.as_mut_ptr(), LANES);
        v
    }

    #[inline(always)]
    pub unsafe fn store(ptr: *mut u8, v: SimdVec) {
        core::ptr::copy_nonoverlapping(v.0.as_ptr(), ptr, LANES);
    }

    #[inline(always)]
    pub unsafe fn splat(b: u8) -> SimdVec {
        SimdVec([b; LANES])
    }

    #[inline(always)]
    pub unsafe fn cmpeq(a: SimdVec, b: SimdVec) -> SimdVec {
        let mut r = SimdVec([0u8; LANES]);
        for i in 0..LANES {
            r.0[i] = if a.0[i] == b.0[i] { 0xFF } else { 0x00 };
        }
        r
    }

    #[inline(always)]
    pub unsafe fn cmple(a: SimdVec, b: SimdVec) -> SimdVec {
        let mut r = SimdVec([0u8; LANES]);
        for i in 0..LANES {
            r.0[i] = if a.0[i] <= b.0[i] { 0xFF } else { 0x00 };
        }
        r
    }

    #[inline(always)]
    pub unsafe fn or(a: SimdVec, b: SimdVec) -> SimdVec {
        let mut r = SimdVec([0u8; LANES]);
        for i in 0..LANES {
            r.0[i] = a.0[i] | b.0[i];
        }
        r
    }

    #[inline(always)]
    pub unsafe fn movemask(v: SimdVec) -> Mask16 {
        let mut result: u16 = 0;
        for i in 0..LANES {
            if v.0[i] & 0x80 != 0 {
                result |= 1 << i;
            }
        }
        result
    }
}

pub use imp::*;

// ============================================================================
// High-level SIMD string operations
// ============================================================================

/// SIMD-accelerated check: does `s` contain any byte that needs ASON quoting?
/// Checks for: control chars (<=0x1f), comma, parens, brackets, double-quote, backslash.
///
/// Returns `true` if any special byte is found.
#[inline]
pub fn simd_has_special_chars(bytes: &[u8]) -> bool {
    let len = bytes.len();
    let mut i = 0;

    unsafe {
        let v_1f = splat(0x1f);
        let v_comma = splat(b',');
        let v_lparen = splat(b'(');
        let v_rparen = splat(b')');
        let v_lbracket = splat(b'[');
        let v_rbracket = splat(b']');
        let v_quote = splat(b'"');
        let v_backslash = splat(b'\\');

        while i + LANES <= len {
            let chunk = load(bytes.as_ptr().add(i));
            let mask = movemask(or(
                or(
                    or(cmple(chunk, v_1f), cmpeq(chunk, v_comma)),
                    or(cmpeq(chunk, v_lparen), cmpeq(chunk, v_rparen)),
                ),
                or(
                    or(cmpeq(chunk, v_lbracket), cmpeq(chunk, v_rbracket)),
                    or(cmpeq(chunk, v_quote), cmpeq(chunk, v_backslash)),
                ),
            ));
            if mask != 0 {
                return true;
            }
            i += LANES;
        }
    }

    // Scalar tail
    static NEEDS_QUOTE: [bool; 256] = {
        let mut t = [false; 256];
        let mut j = 0usize;
        while j < 32 {
            t[j] = true;
            j += 1;
        }
        t[b',' as usize] = true;
        t[b'(' as usize] = true;
        t[b')' as usize] = true;
        t[b'[' as usize] = true;
        t[b']' as usize] = true;
        t[b'"' as usize] = true;
        t[b'\\' as usize] = true;
        t
    };

    while i < len {
        if NEEDS_QUOTE[bytes[i] as usize] {
            return true;
        }
        i += 1;
    }
    false
}

/// SIMD-accelerated: find first byte needing escape in a string being serialized.
/// Escapes needed: `"` → `\"`, `\` → `\\`, `\n` → `\\n`, `\t` → `\\t`
///
/// Returns the offset of the first escape-needing byte, or `len` if none found.
#[inline]
pub fn simd_find_escape(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;

    // Escape table: bytes that need escaping during serialization
    // " (0x22), \ (0x5C), and control chars (0x00-0x1F)
    unsafe {
        let v_1f = splat(0x1f);
        let v_quote = splat(b'"');
        let v_backslash = splat(b'\\');

        while i + LANES <= len {
            let chunk = load(bytes.as_ptr().add(i));
            let mask = movemask(or(
                cmple(chunk, v_1f),
                or(cmpeq(chunk, v_quote), cmpeq(chunk, v_backslash)),
            ));
            if mask != 0 {
                return i + first_set_bit(mask) as usize;
            }
            i += LANES;
        }
    }

    // Scalar tail
    while i < len {
        let b = bytes[i];
        if b <= 0x1f || b == b'"' || b == b'\\' {
            return i;
        }
        i += 1;
    }
    len
}

/// SIMD-accelerated: find first quote (`"`) or backslash (`\`) in bytes.
/// Used for fast-path quoted string scanning during deserialization.
///
/// Returns the offset from `start`, or `len` if neither found.
#[inline]
pub fn simd_find_quote_or_backslash(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;

    unsafe {
        let v_quote = splat(b'"');
        let v_backslash = splat(b'\\');

        while i + LANES <= len {
            let chunk = load(bytes.as_ptr().add(i));
            let mask = movemask(or(cmpeq(chunk, v_quote), cmpeq(chunk, v_backslash)));
            if mask != 0 {
                return i + first_set_bit(mask) as usize;
            }
            i += LANES;
        }
    }

    // Scalar tail
    while i < len {
        let b = bytes[i];
        if b == b'"' || b == b'\\' {
            return i;
        }
        i += 1;
    }
    len
}

/// SIMD-accelerated: find first delimiter for plain (unquoted) value parsing.
/// Delimiters: `,` `)` `]` `\`
///
/// Returns the offset from `start`, or `len` if none found.
#[inline]
pub fn simd_find_plain_delimiter(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;

    unsafe {
        let v_comma = splat(b',');
        let v_rparen = splat(b')');
        let v_rbracket = splat(b']');
        let v_backslash = splat(b'\\');

        while i + LANES <= len {
            let chunk = load(bytes.as_ptr().add(i));
            let mask = movemask(or(
                or(cmpeq(chunk, v_comma), cmpeq(chunk, v_rparen)),
                or(cmpeq(chunk, v_rbracket), cmpeq(chunk, v_backslash)),
            ));
            if mask != 0 {
                return i + first_set_bit(mask) as usize;
            }
            i += LANES;
        }
    }

    // Scalar tail
    while i < len {
        match bytes[i] {
            b',' | b')' | b']' | b'\\' => return i,
            _ => i += 1,
        }
    }
    len
}

/// SIMD-accelerated whitespace skipping.
/// Returns the offset of the first non-whitespace byte at or after `start`.
#[inline]
pub fn simd_skip_whitespace(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start;

    unsafe {
        let v_space = splat(b' ');
        let v_tab = splat(b'\t');
        let v_nl = splat(b'\n');
        let v_cr = splat(b'\r');

        while i + LANES <= len {
            let chunk = load(bytes.as_ptr().add(i));
            // Build mask of whitespace bytes
            let ws_mask = movemask(or(
                or(cmpeq(chunk, v_space), cmpeq(chunk, v_tab)),
                or(cmpeq(chunk, v_nl), cmpeq(chunk, v_cr)),
            ));
            // If all 16 bytes are whitespace, skip the whole chunk
            if ws_mask == 0xFFFF {
                i += LANES;
                continue;
            }
            // If no bytes are whitespace, we're done
            if ws_mask == 0 {
                return i;
            }
            // Find first non-whitespace: invert the mask and find first set bit
            let non_ws = !ws_mask & 0xFFFF;
            return i + first_set_bit(non_ws) as usize;
        }
    }

    // Scalar tail
    while i < len {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => return i,
        }
    }
    i
}

/// SIMD-accelerated: write string with escaping, used during serialization.
/// Processes 16 bytes at a time, bulk-copying non-escaped runs.
///
/// Returns the number of bytes written.
#[inline]
pub fn simd_write_escaped(buf: &mut Vec<u8>, s: &[u8]) {
    /// Escape lookup: maps byte → (replacement_char, 0 if no escape needed)
    static ESCAPE: [u8; 256] = {
        let mut t = [0u8; 256];
        t[b'"' as usize] = b'"';
        t[b'\\' as usize] = b'\\';
        t[b'\n' as usize] = b'n';
        t[b'\t' as usize] = b't';
        t
    };

    buf.push(b'"');

    let len = s.len();
    let mut start = 0;

    // Use SIMD to scan for bytes that need escaping
    while start < len {
        let next_esc = simd_find_escape(s, start);
        // Bulk copy the non-escaped run
        if next_esc > start {
            buf.extend_from_slice(&s[start..next_esc]);
        }
        if next_esc >= len {
            break;
        }
        // Escape the byte
        let b = s[next_esc];
        let esc = ESCAPE[b as usize];
        if esc != 0 {
            buf.push(b'\\');
            buf.push(esc);
        } else {
            // Control char: \u00XX
            buf.extend_from_slice(b"\\u00");
            static HEX: &[u8; 16] = b"0123456789abcdef";
            buf.push(HEX[(b >> 4) as usize]);
            buf.push(HEX[(b & 0xf) as usize]);
        }
        start = next_esc + 1;
    }

    buf.push(b'"');
}

// ============================================================================
// SIMD-accelerated bulk memory copy (used by binary serializer)
// ============================================================================

/// SIMD-accelerated bulk copy: append `src` bytes into `dst` Vec<u8>.
///
/// For payloads ≥ 32 bytes, copies 16 bytes per iteration using platform SIMD
/// `load` + `store`, reducing loop overhead and enabling hardware memory
/// bandwidth utilization. Falls back to `extend_from_slice` for small payloads
/// where SIMD setup cost would outweigh benefits.
///
/// This is the primary SIMD contribution of the binary format:
/// writing long string payloads (names, emails, descriptions) benefits from
/// vectorized copies on both NEON (aarch64) and SSE2 (x86_64).
#[inline]
pub fn simd_bulk_extend(dst: &mut Vec<u8>, src: &[u8]) {
    let n = src.len();
    if n == 0 {
        return;
    }
    // Small payload: standard path (LLVM auto-vectorizes short copies already)
    if n < 32 {
        dst.extend_from_slice(src);
        return;
    }
    // Large payload: explicit SIMD loop — 16 bytes per iteration
    dst.reserve(n);
    let dst_start = dst.len();
    unsafe {
        let src_ptr = src.as_ptr();
        let dst_ptr = dst.as_mut_ptr().add(dst_start);
        let mut i = 0usize;
        // Process 16 bytes at a time with SIMD load+store
        while i + LANES <= n {
            let chunk = load(src_ptr.add(i));
            store(dst_ptr.add(i), chunk);
            i += LANES;
        }
        // Handle remainder with scalar copy
        if i < n {
            core::ptr::copy_nonoverlapping(src_ptr.add(i), dst_ptr.add(i), n - i);
        }
        dst.set_len(dst_start + n);
    }
}

// ============================================================================
// SIMD-accelerated integer formatting
// ============================================================================

/// Two-digit lookup table for fast integer formatting (itoa-style).
static DEC_DIGITS: &[u8; 200] = b"0001020304050607080910111213141516171819\
2021222324252627282930313233343536373839\
4041424344454647484950515253545556575859\
6061626364656667686970717273747576777879\
8081828384858687888990919293949596979899";

/// Write u64 to buffer using 2-digits-at-a-time itoa technique.
/// Writes directly into spare capacity to avoid bounds checks.
#[inline(always)]
pub fn fast_write_u64(buf: &mut Vec<u8>, mut v: u64) {
    if v < 10 {
        buf.push(b'0' + v as u8);
        return;
    }
    if v < 100 {
        let idx = v as usize * 2;
        buf.push(DEC_DIGITS[idx]);
        buf.push(DEC_DIGITS[idx + 1]);
        return;
    }
    // Write digits in reverse using 2-at-a-time technique
    let mut tmp = [0u8; 20];
    let mut i = 20usize;
    while v >= 100 {
        let rem = (v % 100) as usize;
        v /= 100;
        i -= 2;
        unsafe {
            *tmp.get_unchecked_mut(i) = DEC_DIGITS[rem * 2];
            *tmp.get_unchecked_mut(i + 1) = DEC_DIGITS[rem * 2 + 1];
        }
    }
    if v >= 10 {
        let idx = v as usize * 2;
        i -= 2;
        unsafe {
            *tmp.get_unchecked_mut(i) = DEC_DIGITS[idx];
            *tmp.get_unchecked_mut(i + 1) = DEC_DIGITS[idx + 1];
        }
    } else {
        i -= 1;
        tmp[i] = b'0' + v as u8;
    }
    buf.extend_from_slice(&tmp[i..]);
}

/// Write i64 to buffer.
#[inline(always)]
pub fn fast_write_i64(buf: &mut Vec<u8>, v: i64) {
    if v < 0 {
        buf.push(b'-');
        fast_write_u64(buf, (-(v as i128)) as u64);
    } else {
        fast_write_u64(buf, v as u64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simd_has_special_chars() {
        assert!(!simd_has_special_chars(b"hello world"));
        assert!(simd_has_special_chars(b"hello,world"));
        assert!(simd_has_special_chars(b"hello(world"));
        assert!(simd_has_special_chars(b"hello)world"));
        assert!(simd_has_special_chars(b"hello[world"));
        assert!(simd_has_special_chars(b"hello]world"));
        assert!(simd_has_special_chars(b"hello\"world"));
        assert!(simd_has_special_chars(b"hello\\world"));
        assert!(simd_has_special_chars(b"hello\nworld"));
        assert!(simd_has_special_chars(b"hello\tworld"));
        assert!(!simd_has_special_chars(b"abcdefghijklmnop")); // exactly 16 bytes
        assert!(!simd_has_special_chars(b"abcdefghijklmnopqrstuvwx")); // > 16 bytes
        assert!(simd_has_special_chars(b"abcdefghijklmno,")); // special at pos 15
        assert!(simd_has_special_chars(b"abcdefghijklmnop,")); // special at pos 16 (tail)
    }

    #[test]
    fn test_simd_find_escape() {
        assert_eq!(simd_find_escape(b"hello world", 0), 11);
        assert_eq!(simd_find_escape(b"hello\"world", 0), 5);
        assert_eq!(simd_find_escape(b"hello\\world", 0), 5);
        assert_eq!(simd_find_escape(b"hello\nworld", 0), 5);
        assert_eq!(simd_find_escape(b"abcdefghijklmnop\"", 0), 16); // in tail
    }

    #[test]
    fn test_simd_find_quote_or_backslash() {
        assert_eq!(simd_find_quote_or_backslash(b"hello world", 0), 11);
        assert_eq!(simd_find_quote_or_backslash(b"hello\"world", 0), 5);
        assert_eq!(simd_find_quote_or_backslash(b"hello\\world", 0), 5);
        assert_eq!(simd_find_quote_or_backslash(b"abcdefghijklmnop\"", 0), 16);
    }

    #[test]
    fn test_simd_find_plain_delimiter() {
        assert_eq!(simd_find_plain_delimiter(b"hello world", 0), 11);
        assert_eq!(simd_find_plain_delimiter(b"hello,world", 0), 5);
        assert_eq!(simd_find_plain_delimiter(b"hello)world", 0), 5);
        assert_eq!(simd_find_plain_delimiter(b"hello]world", 0), 5);
    }

    #[test]
    fn test_simd_skip_whitespace() {
        assert_eq!(simd_skip_whitespace(b"   hello", 0), 3);
        assert_eq!(simd_skip_whitespace(b"\t\n\r hello", 0), 4);
        assert_eq!(simd_skip_whitespace(b"hello", 0), 0);
        assert_eq!(simd_skip_whitespace(b"   ", 0), 3);
        // > 16 bytes of whitespace
        assert_eq!(simd_skip_whitespace(b"                  hello", 0), 18);
    }

    #[test]
    fn test_simd_write_escaped() {
        let mut buf = Vec::new();
        simd_write_escaped(&mut buf, b"hello");
        assert_eq!(buf, b"\"hello\"");

        buf.clear();
        simd_write_escaped(&mut buf, b"hello\"world");
        assert_eq!(buf, b"\"hello\\\"world\"");

        buf.clear();
        simd_write_escaped(&mut buf, b"line1\nline2");
        assert_eq!(buf, b"\"line1\\nline2\"");

        buf.clear();
        simd_write_escaped(&mut buf, b"tab\there");
        assert_eq!(buf, b"\"tab\\there\"");

        buf.clear();
        simd_write_escaped(&mut buf, b"back\\slash");
        assert_eq!(buf, b"\"back\\\\slash\"");

        // Control char
        buf.clear();
        simd_write_escaped(&mut buf, &[0x01]);
        assert_eq!(buf, b"\"\\u0001\"");
    }

    #[test]
    fn test_fast_write_u64() {
        let cases = [0u64, 1, 9, 10, 99, 100, 999, 1234, 99999, 1000000, u64::MAX];
        for &v in &cases {
            let mut buf1 = Vec::new();
            fast_write_u64(&mut buf1, v);
            let expected = v.to_string();
            assert_eq!(
                String::from_utf8(buf1).unwrap(),
                expected,
                "fast_write_u64({}) failed",
                v
            );
        }
    }

    #[test]
    fn test_fast_write_i64() {
        let cases = [0i64, 1, -1, 42, -42, i64::MAX, i64::MIN];
        for &v in &cases {
            let mut buf1 = Vec::new();
            fast_write_i64(&mut buf1, v);
            let expected = v.to_string();
            assert_eq!(
                String::from_utf8(buf1).unwrap(),
                expected,
                "fast_write_i64({}) failed",
                v
            );
        }
    }
}
