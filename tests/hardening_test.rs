//! Regression tests for correctness and hardening fixes.
//!
//! Every test here corresponds to a defect that was reproducible before the
//! fix: silent data corruption, a decode that depended on cache state, or an
//! input that aborted the process outright.

use asun::{AsunDecode, AsunEncode, Error, decode, encode};

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct S {
    a: String,
    b: i64,
}

// ---------------------------------------------------------------------------
// Schema scanning / cache
// ---------------------------------------------------------------------------

/// The schema-end scan used to count braces without tracking quotes, so a `}`
/// inside a quoted field name truncated the cache key. The first decode took
/// the slow path and succeeded; the second hit the poisoned entry and jumped to
/// the wrong offset.
#[test]
fn quoted_brace_in_field_name_is_not_structural() {
    let input = r#"{"a}b",b}:(hello,7)"#;
    let first: S = decode(input).expect("first decode");
    let second: S = decode(input).expect("second decode");
    assert_eq!(first, second);
    assert_eq!(first.b, 7);
}

#[test]
fn quoted_brace_schema_does_not_collide_with_other_schema() {
    // Both used to truncate to the same `{"a}` key.
    let a: S = decode(r#"{"a}x",b}:(one,1)"#).unwrap();
    let b: S = decode(r#"{"a}yyyy",b}:(two,2)"#).unwrap();
    assert_eq!(a.b, 1);
    assert_eq!(b.b, 2);
}

#[test]
fn comment_inside_schema_is_skipped() {
    let v: S = decode("{a/* } not a brace */,b}:(hi,3)").unwrap();
    assert_eq!(v.b, 3);
}

#[test]
fn unterminated_quote_in_schema_errors() {
    let r: Result<S, Error> = decode(r#"{"unterminated,b}:(x,1)"#);
    assert!(r.is_err());
}

/// The schema cache is bounded now; decoding far more distinct schemas than the
/// cap must stay correct (eviction must not hand back a stale entry).
#[test]
fn schema_cache_eviction_stays_correct() {
    for i in 0..3_000u32 {
        let input = format!("{{a,b,filler_{i}}}:(v{i},{i},0)");
        let v: S = decode(&input).unwrap();
        assert_eq!(v.b, i as i64);
        assert_eq!(v.a, format!("v{i}"));
    }
}

// ---------------------------------------------------------------------------
// Depth limiting
// ---------------------------------------------------------------------------

/// `@[[[…]]]` recursed once per bracket with no bound; ~200k brackets aborted
/// the process with a stack overflow (not a catchable panic).
#[test]
fn deeply_nested_array_annotation_is_rejected() {
    let depth = 200_000;
    let mut input = String::from("{a@");
    input.push_str(&"[".repeat(depth));
    input.push_str(&"]".repeat(depth));
    input.push_str(",b}:(x,1)");
    let r: Result<S, Error> = decode(&input);
    assert!(matches!(r, Err(Error::DepthLimitExceeded)));
}

/// Nested `@{…}` schemas recurse through `parse_schema` too.
#[test]
fn deeply_nested_schema_annotation_is_rejected() {
    let depth = 50_000;
    let mut input = String::from("{a@");
    input.push_str(&"{x@".repeat(depth));
    input.push_str("int");
    input.push_str(&"}".repeat(depth));
    input.push_str(",b}:(x,1)");
    let r: Result<S, Error> = decode(&input);
    assert!(matches!(r, Err(Error::DepthLimitExceeded)));
}

#[test]
fn realistic_nesting_still_works() {
    #[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
    struct L4 {
        v: i64,
    }
    #[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
    struct L3 {
        items: Vec<L4>,
    }
    #[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
    struct L2 {
        items: Vec<L3>,
    }
    #[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
    struct L1 {
        items: Vec<L2>,
    }

    let v = L1 {
        items: vec![L2 {
            items: vec![L3 {
                items: vec![L4 { v: 42 }],
            }],
        }],
    };
    let text = encode(&v).unwrap();
    assert_eq!(decode::<L1>(&text).unwrap(), v);
}

// ---------------------------------------------------------------------------
// Escapes and UTF-8
// ---------------------------------------------------------------------------

/// Plain-value unescaping pushed `byte as char`, reinterpreting each byte as a
/// code point, which mangled every multi-byte character in a value that also
/// contained an escape.
#[test]
fn plain_value_escape_preserves_utf8() {
    let v: S = decode("{a,b}:(你好\\,世界,1)").unwrap();
    assert_eq!(v.a, "你好,世界");
}

#[test]
fn plain_value_escape_preserves_utf8_mixed() {
    let v: S = decode("{a,b}:(日本語\\:テスト\\(x\\),9)").unwrap();
    assert_eq!(v.a, "日本語:テスト(x)");
    assert_eq!(v.b, 9);
}

#[test]
fn quoted_string_preserves_utf8_with_escapes() {
    let v: S = decode(r#"{a,b}:("héllo\n世界\t✓",1)"#).unwrap();
    assert_eq!(v.a, "héllo\n世界\t✓");
}

#[test]
fn unicode_escape_surrogate_pair() {
    // U+1F600 GRINNING FACE, written as a UTF-16 surrogate pair.
    let v: S = decode(r#"{a,b}:("\uD83D\uDE00",1)"#).unwrap();
    assert_eq!(v.a, "😀");
}

#[test]
fn unicode_escape_bmp() {
    let v: S = decode(r#"{a,b}:("\u4f60\u597d",1)"#).unwrap();
    assert_eq!(v.a, "你好");
}

#[test]
fn unpaired_surrogate_is_rejected() {
    let r: Result<S, Error> = decode(r#"{a,b}:("\uD83D",1)"#);
    assert!(matches!(r, Err(Error::InvalidUnicodeEscape)));
    let r: Result<S, Error> = decode(r#"{a,b}:("\uDE00",1)"#);
    assert!(matches!(r, Err(Error::InvalidUnicodeEscape)));
}

/// `from_str_radix` accepted things that are not four hex digits, and the old
/// code built the hex `&str` with `from_utf8_unchecked` over raw input bytes.
#[test]
fn malformed_unicode_escape_is_rejected() {
    for bad in [r#""\u+123""#, r#""\uZZZZ""#, r#""\u12""#, r#""\u 123""#] {
        let input = format!("{{a,b}}:({bad},1)");
        let r: Result<S, Error> = decode(&input);
        assert!(r.is_err(), "expected error for {bad}");
    }
}

#[test]
fn unicode_escape_next_to_multibyte_char() {
    // The 4 bytes after `\u` used to be reinterpreted as a `str` unchecked.
    let r: Result<S, Error> = decode("{a,b}:(\"\\u你好x\",1)");
    assert!(r.is_err());
}

#[test]
fn special_char_strings_roundtrip() {
    for s in [
        "plain",
        "",
        " leading",
        "trailing ",
        "with,comma",
        "with(paren)",
        "with{brace}",
        "with:colon",
        "with@at",
        "with\"quote",
        "with\\backslash",
        "with\nnewline",
        "with\ttab",
        "true",
        "false",
        "123",
        "-4.5e10",
        "with\u{7f}del",
        "unicode 世界 🌍",
        "a longer string that exceeds sixteen bytes, with a comma",
    ] {
        let v = S {
            a: s.to_string(),
            b: 1,
        };
        let text = encode(&v).unwrap();
        let back: S = decode(&text).unwrap_or_else(|e| panic!("{s:?} -> {text} -> {e}"));
        assert_eq!(back, v, "roundtrip failed for {s:?} (encoded {text})");
    }
}

// ---------------------------------------------------------------------------
// Numbers
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct Narrow {
    i: i8,
    u: u8,
    w: i32,
}

/// Narrowing used a plain `as` cast, so `999` silently decoded to `i8` as -25.
#[test]
fn out_of_range_integers_are_rejected() {
    assert!(matches!(
        decode::<Narrow>("{i,u,w}:(999,0,0)"),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(
        decode::<Narrow>("{i,u,w}:(0,999,0)"),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(
        decode::<Narrow>("{i,u,w}:(0,0,5000000000)"),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(
        decode::<Narrow>("{i,u,w}:(0,-1,0)"),
        Err(Error::InvalidNumber)
    ));
}

#[test]
fn in_range_integers_still_decode() {
    let v: Narrow = decode("{i,u,w}:(-128,255,-2147483648)").unwrap();
    assert_eq!(
        v,
        Narrow {
            i: -128,
            u: 255,
            w: -2147483648
        }
    );
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct F {
    x: f64,
}

/// The hand-rolled one/two-decimal formatting paths were not round-trip safe:
/// a sweep over random finite doubles found ~1 failure per 1600 values.
#[test]
fn float_roundtrip_is_exact() {
    let mut state = 0x243F_6A88_85A3_08D3u64;
    let mut checked = 0u32;
    for _ in 0..300_000 {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let v = f64::from_bits(state);
        if !v.is_finite() {
            continue;
        }
        let text = encode(&F { x: v }).unwrap();
        let back: F = decode(&text).unwrap();
        assert_eq!(
            back.x.to_bits(),
            v.to_bits(),
            "{v:?} encoded as {text} decoded as {:?}",
            back.x
        );
        checked += 1;
    }
    assert!(checked > 100_000);
}

#[test]
fn float_special_values_roundtrip() {
    for v in [
        0.0f64,
        -0.0,
        1.0,
        -1.0,
        0.5,
        -0.5,
        0.25,
        50.5,
        1e-300,
        1e300,
        f64::MIN,
        f64::MAX,
        f64::MIN_POSITIVE,
        9_007_199_254_740_992.0,
        9_007_199_254_740_993.0,
        1.7976931348623157e308,
    ] {
        let text = encode(&F { x: v }).unwrap();
        let back: F = decode(&text).unwrap();
        assert_eq!(back.x.to_bits(), v.to_bits(), "{v:?} encoded as {text}");
    }
}

#[test]
fn negative_zero_keeps_its_sign() {
    let text = encode(&F { x: -0.0 }).unwrap();
    assert_eq!(text, "{x}:(-0.0)");
    let back: F = decode(&text).unwrap();
    assert!(back.x.is_sign_negative());
}

#[test]
fn non_finite_floats_are_rejected() {
    assert!(encode(&F { x: f64::NAN }).is_err());
    assert!(encode(&F { x: f64::INFINITY }).is_err());
}

// ---------------------------------------------------------------------------
// Zero-copy decode through the derive
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, AsunDecode)]
struct Borrowed<'a> {
    id: i64,
    name: &'a str,
    city: &'a str,
}

/// The derive used to inject a fresh `'de` unrelated to the type's own
/// lifetime, so no borrowing struct could compile — the advertised zero-copy
/// path was unreachable.
#[test]
fn derive_supports_borrowed_str_fields() {
    let input = String::from("{id,name,city}:(7,Alice,Shanghai)");
    let v: Borrowed = decode(&input).unwrap();
    assert_eq!(v.id, 7);
    assert_eq!(v.name, "Alice");

    // Genuinely borrowed, not copied: the field points into the input buffer.
    let base = input.as_ptr() as usize;
    let field = v.name.as_ptr() as usize;
    assert!(
        field >= base && field < base + input.len(),
        "field was not borrowed from the input"
    );
}

#[test]
fn derive_supports_borrowed_seq() {
    let input = String::from("[{id,name,city}]:(1,Alice,SH),(2,Bob,BJ)");
    let v: Vec<Borrowed> = decode(&input).unwrap();
    assert_eq!(v.len(), 2);
    assert_eq!(v[1].name, "Bob");
    let base = input.as_ptr() as usize;
    assert!((v[1].name.as_ptr() as usize) >= base);
}

// ---------------------------------------------------------------------------
// Allocation behaviour
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct RowWithList {
    id: i64,
    tags: Vec<i64>,
}

/// A nested array must be sized from its own contents, never from "how much
/// input is left" — that figure is the whole remaining document, so using it
/// reserves a huge buffer for every element of the enclosing sequence and turns
/// decoding quadratic in time and memory.
///
/// Asserting on capacity keeps this deterministic; the same defect showed up in
/// benchmarks as a 14x slowdown that grew with document size.
#[test]
fn nested_arrays_are_not_over_reserved() {
    let rows: Vec<RowWithList> = (0..2_000)
        .map(|i| RowWithList {
            id: i,
            tags: vec![i, i + 1],
        })
        .collect();
    let text = encode(&rows).unwrap();
    assert!(text.len() > 20_000, "need a document big enough to matter");

    let back: Vec<RowWithList> = decode(&text).unwrap();
    assert_eq!(back.len(), rows.len());
    for row in &back {
        assert_eq!(row.tags.len(), 2);
        assert!(
            row.tags.capacity() <= 8,
            "nested Vec reserved {} slots for 2 elements",
            row.tags.capacity()
        );
    }
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// The schema and struct-plan caches used to be process-global behind a
/// `Mutex`; they are per-thread now, so this exercises both correctness and the
/// absence of cross-thread state.
#[test]
fn concurrent_decodes_are_independent() {
    let handles: Vec<_> = (0..8u32)
        .map(|t| {
            std::thread::spawn(move || {
                for i in 0..2_000u32 {
                    let input = format!("{{a,b,t{t}_{i}}}:(x{i},{i},0)");
                    let v: S = decode(&input).unwrap();
                    assert_eq!(v.b, i as i64);
                    assert_eq!(v.a, format!("x{i}"));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }
}
