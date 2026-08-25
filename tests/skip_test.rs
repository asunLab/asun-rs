//! Tests for the serde-aligned `#[asun(skip / skip_serializing /
//! skip_deserializing / skip_serializing_if)]` field attributes and the
//! `#[asun(default = "path")]` value source.
//!
//! Two wire formats with different skip semantics are exercised:
//!   - text   : self-describing (schema carries field names) — a skipped field
//!              can simply be omitted on encode; decode fills the default.
//!   - binary : no schema, fixed field order — any skip MUST be symmetric
//!              (encode omits ⇒ decode omits), or field alignment breaks.
//!   - skip_serializing_if is text-only: binary always writes the field.

use asun::{AsunDecode, AsunEncode};

// --- skip: dropped on both encode and decode, in both formats ---------------

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct WithSkip {
    a: u32,
    #[asun(skip)]
    secret: String,
    b: u32,
}

#[test]
fn skip_text_omits_and_defaults() {
    let v = WithSkip {
        a: 1,
        secret: "hidden".into(),
        b: 2,
    };
    let txt = asun::encode(&v).unwrap();
    // The skipped field name must not appear in the schema/payload.
    assert!(
        !txt.contains("secret"),
        "text should not contain skipped field: {txt}"
    );
    assert!(
        !txt.contains("hidden"),
        "text should not contain skipped value: {txt}"
    );

    let back: WithSkip = asun::decode(&txt).unwrap();
    assert_eq!(
        back,
        WithSkip {
            a: 1,
            secret: String::new(),
            b: 2
        }
    );
}

#[test]
fn skip_binary_symmetric() {
    let v = WithSkip {
        a: 1,
        secret: "hidden".into(),
        b: 2,
    };
    let bin = asun::encode_binary(&v).unwrap();
    let back: WithSkip = asun::decode_binary(&bin).unwrap();
    // secret is neither written nor read → default; a and b survive.
    assert_eq!(
        back,
        WithSkip {
            a: 1,
            secret: String::new(),
            b: 2
        }
    );
}

// --- skip_serializing: not written, but read from wire when present ---------
//
// Note on defaults: `skip_serializing` only suppresses the *write*. It is still
// a normal read field. When it happens to be absent from the wire (e.g. when
// decoding our own skip_serializing output), it falls back to
// `Default::default()` — a custom `#[asun(default = "path")]` only takes effect
// for fields that are skipped on *deserialize* (skip / skip_deserializing),
// where the derive controls the value directly. See WithNonDefault below.

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct WithSkipSer {
    a: u32,
    #[asun(skip_serializing)]
    tag: String,
    b: u32,
}

#[test]
fn skip_serializing_text_reads_when_present() {
    let v = WithSkipSer {
        a: 1,
        tag: "written".into(),
        b: 2,
    };
    let txt = asun::encode(&v).unwrap();
    assert!(
        !txt.contains("written"),
        "value must not be serialized: {txt}"
    );

    // Decoding our own (tag-less) output → tag absent → Default::default().
    let back: WithSkipSer = asun::decode(&txt).unwrap();
    assert_eq!(
        back,
        WithSkipSer {
            a: 1,
            tag: String::new(),
            b: 2
        }
    );

    // If a tag field IS present in the text, decode still reads it
    // (skip_serializing only affects the write side).
    let with_tag = "{a,tag,b}:(1,\"present\",2)";
    let back2: WithSkipSer = asun::decode(with_tag).unwrap();
    assert_eq!(
        back2,
        WithSkipSer {
            a: 1,
            tag: "present".into(),
            b: 2
        }
    );
}

#[test]
fn skip_serializing_binary_symmetric() {
    let v = WithSkipSer {
        a: 1,
        tag: "written".into(),
        b: 2,
    };
    let bin = asun::encode_binary(&v).unwrap();
    let back: WithSkipSer = asun::decode_binary(&bin).unwrap();
    // Binary is forced symmetric: tag not written ⇒ not read ⇒ Default::default().
    assert_eq!(
        back,
        WithSkipSer {
            a: 1,
            tag: String::new(),
            b: 2
        }
    );
}

// --- skip_deserializing: written, but always defaulted on decode ------------

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct WithSkipDe {
    a: u32,
    #[asun(skip_deserializing)]
    computed: u32,
    b: u32,
}

#[test]
fn skip_deserializing_text() {
    // Hand-written input that DOES contain the field — decode must ignore it
    // and use the default.
    let txt = "{a,computed,b}:(1,999,2)";
    let back: WithSkipDe = asun::decode(txt).unwrap();
    assert_eq!(
        back,
        WithSkipDe {
            a: 1,
            computed: 0,
            b: 2
        }
    );
}

#[test]
fn skip_deserializing_binary_symmetric() {
    // skip_de is written on encode but must not be read on decode. For binary
    // that means encode must ALSO omit it, else alignment breaks. Round-trip
    // through our own encoder proves the symmetry holds.
    let v = WithSkipDe {
        a: 1,
        computed: 42,
        b: 2,
    };
    let bin = asun::encode_binary(&v).unwrap();
    let back: WithSkipDe = asun::decode_binary(&bin).unwrap();
    assert_eq!(
        back,
        WithSkipDe {
            a: 1,
            computed: 0,
            b: 2
        }
    );
}

// --- skip_serializing_if: conditional text skip, always written in binary ---

fn is_zero(n: &u32) -> bool {
    *n == 0
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct WithSkipIf {
    a: u32,
    #[asun(skip_serializing_if = "is_zero")]
    maybe: u32,
    b: u32,
}

#[test]
fn skip_serializing_if_text_conditional() {
    // Condition true → field omitted from text; decode fills default (0).
    let v = WithSkipIf {
        a: 1,
        maybe: 0,
        b: 2,
    };
    let txt = asun::encode(&v).unwrap();
    assert!(
        !txt.contains("maybe"),
        "field should be omitted when predicate holds: {txt}"
    );
    let back: WithSkipIf = asun::decode(&txt).unwrap();
    assert_eq!(back, v);

    // Condition false → field present and round-trips.
    let v2 = WithSkipIf {
        a: 1,
        maybe: 7,
        b: 2,
    };
    let txt2 = asun::encode(&v2).unwrap();
    assert!(
        txt2.contains("maybe"),
        "field should be present when predicate fails: {txt2}"
    );
    let back2: WithSkipIf = asun::decode(&txt2).unwrap();
    assert_eq!(back2, v2);
}

#[test]
fn skip_serializing_if_binary_always_written() {
    // Binary ignores the predicate — the field is always written and read, so
    // it round-trips faithfully even when the predicate holds.
    for maybe in [0u32, 7u32] {
        let v = WithSkipIf { a: 1, maybe, b: 2 };
        let bin = asun::encode_binary(&v).unwrap();
        let back: WithSkipIf = asun::decode_binary(&bin).unwrap();
        assert_eq!(back, v);
    }
}

// --- default = "path" on a type that does NOT implement Default -------------

#[derive(Debug, PartialEq)]
struct NoDefault(u32);

fn make_no_default() -> NoDefault {
    NoDefault(123)
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct WithNonDefault {
    a: u32,
    #[asun(skip, default = "make_no_default")]
    inner: NoDefault,
}

#[test]
fn default_path_for_non_default_type() {
    let v = WithNonDefault {
        a: 5,
        inner: NoDefault(999),
    };
    // text
    let txt = asun::encode(&v).unwrap();
    let back: WithNonDefault = asun::decode(&txt).unwrap();
    assert_eq!(
        back,
        WithNonDefault {
            a: 5,
            inner: NoDefault(123)
        }
    );
    // binary
    let bin = asun::encode_binary(&v).unwrap();
    let back_b: WithNonDefault = asun::decode_binary(&bin).unwrap();
    assert_eq!(
        back_b,
        WithNonDefault {
            a: 5,
            inner: NoDefault(123)
        }
    );
}
