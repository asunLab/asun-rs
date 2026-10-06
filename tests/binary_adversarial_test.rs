//! Malformed and hostile ASUN-BIN input: every case must return an error (or
//! the exact value) without panicking, hanging or exhausting memory or stack.

use asun::binary::{BinaryDecoder, DEFAULT_MAX_SEQUENCE_LEN};
use asun::{
    AsunDecode, AsunDecodeBinary, AsunEncode, Error, decode_binary, decode_binary_exact,
    encode_binary,
};
use std::time::{Duration, Instant};

#[derive(Debug, AsunEncode, AsunDecode, PartialEq, Clone)]
enum Shape {
    Unit,
    New(i32),
    Tup(u8, String),
    Rec { w: f64, tag: Option<String> },
}

#[derive(Debug, AsunEncode, AsunDecode, PartialEq, Clone)]
struct Rich {
    a: bool,
    b: i8,
    c: i16,
    d: i32,
    e: i64,
    f: u8,
    g: u16,
    h: u32,
    i: u64,
    j: f32,
    k: f64,
    l: char,
    m: String,
    n: Option<String>,
    o: Vec<i64>,
    p: Vec<Option<u16>>,
    q: (u8, String, bool),
    r: Vec<Shape>,
    s: Option<Option<i32>>,
}

fn rich() -> Rich {
    Rich {
        a: true,
        b: -128,
        c: i16::MIN,
        d: i32::MAX,
        e: i64::MIN,
        f: 255,
        g: u16::MAX,
        h: u32::MAX,
        i: u64::MAX,
        j: f32::from_bits(0x7fc0_0001),
        k: -0.0,
        l: '\u{10FFFF}',
        m: "héllo wörld, 中文 🎉".repeat(5),
        n: Some(String::new()),
        o: vec![0, -1, 1, i64::MAX, i64::MIN, 63, 64, -64, -65],
        p: vec![None, Some(0), Some(u16::MAX)],
        q: (7, "x".repeat(200), false),
        r: vec![
            Shape::Unit,
            Shape::New(-5),
            Shape::Tup(1, "t".into()),
            Shape::Rec {
                w: f64::MIN_POSITIVE / 2.0,
                tag: None,
            },
        ],
        s: Some(None),
    }
}

#[derive(Debug, AsunEncode, AsunDecode, PartialEq, Default)]
struct Node {
    kids: Vec<Node>,
}

#[derive(Debug, AsunEncode, AsunDecode, PartialEq, Default)]
struct Empty {}

fn varint(v: u64) -> Vec<u8> {
    encode_binary(&v).unwrap()
}

#[test]
fn extreme_values_roundtrip_bit_exact() {
    let v = rich();
    let b = encode_binary(&v).unwrap();
    let back: Rich = decode_binary_exact(&b).unwrap();
    assert_eq!(back.j.to_bits(), v.j.to_bits(), "NaN payload");
    assert!(back.k.is_sign_negative(), "-0.0 sign");
    // NaN != NaN, so compare everything else through Debug.
    assert_eq!(format!("{back:?}"), format!("{v:?}"));
}

#[test]
fn every_truncation_is_an_error() {
    let b = encode_binary(&rich()).unwrap();
    for n in 0..b.len() {
        assert!(
            decode_binary::<Rich>(&b[..n]).is_err(),
            "prefix of {n} bytes decoded"
        );
    }
}

#[test]
fn byte_mutations_never_panic() {
    let b = encode_binary(&rich()).unwrap();
    for i in 0..b.len() {
        for x in [0x00u8, 0x01, 0x02, 0x7f, 0x80, 0xc0, 0xff] {
            let mut m = b.clone();
            m[i] = x;
            let r = std::panic::catch_unwind(|| {
                let _ = decode_binary::<Rich>(&m);
            });
            assert!(r.is_ok(), "panic with byte {i} = {x:#x}");
        }
    }
}

#[test]
fn varint_edges() {
    let max = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01];
    assert_eq!(decode_binary_exact::<u64>(&max).unwrap(), u64::MAX);
    // Only the minimal encoding is accepted, so each value has one byte form.
    let overlong: [&[u8]; 4] = [
        &[0x80, 0x00],
        &[0x81, 0x00],
        &[0xff, 0x80, 0x00],
        &[0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00],
    ];
    for b in overlong {
        assert!(
            matches!(decode_binary::<u64>(b), Err(Error::VarintOverflow)),
            "{b:02x?}"
        );
    }
    assert!(matches!(
        decode_binary::<String>(&[0x80, 0x00]),
        Err(Error::VarintOverflow)
    ));
    for shift in 0..64 {
        for v in [1u64 << shift, (1u64 << shift) - 1, u64::MAX >> shift] {
            let b = varint(v);
            assert_eq!(decode_binary_exact::<u64>(&b).unwrap(), v);
        }
    }
    let tenth_too_big = [0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x02];
    assert!(matches!(
        decode_binary::<u64>(&tenth_too_big),
        Err(Error::VarintOverflow)
    ));
    let eleven = [
        0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x80, 0x00,
    ];
    assert!(matches!(
        decode_binary::<u64>(&eleven),
        Err(Error::VarintOverflow)
    ));
    assert!(matches!(
        decode_binary::<u64>(&[0x80, 0x80, 0x80]),
        Err(Error::Eof)
    ));

    for v in [i64::MIN, i64::MAX, -1, 0, 63, -64, 64, -65] {
        let b = encode_binary(&v).unwrap();
        assert_eq!(decode_binary_exact::<i64>(&b).unwrap(), v);
    }
    // Narrow targets reject values that only fit a wider type.
    assert!(matches!(
        decode_binary::<u16>(&varint(65536)),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(
        decode_binary::<i16>(&encode_binary(&40000i32).unwrap()),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(
        decode_binary::<i32>(&encode_binary(&i64::MIN).unwrap()),
        Err(Error::IntegerOutOfRange)
    ));
    // u8 is a raw byte, not a varint: a two-byte varint leaves one byte over.
    assert!(matches!(
        decode_binary_exact::<u8>(&encode_binary(&200u16).unwrap()),
        Err(Error::TrailingBytes)
    ));
}

#[test]
fn tags_and_scalars_are_strict() {
    assert_eq!(decode_binary_exact::<Option<u8>>(&[0]).unwrap(), None);
    assert_eq!(decode_binary_exact::<Option<u8>>(&[1, 9]).unwrap(), Some(9));
    for tag in [2u8, 0x80, 0xff] {
        assert!(
            matches!(
                decode_binary::<Option<u8>>(&[tag, 9]),
                Err(Error::InvalidTag)
            ),
            "option tag {tag:#x}"
        );
    }
    assert!(matches!(
        decode_binary::<bool>(&[2]),
        Err(Error::InvalidBool)
    ));
    for cp in [0xD800u64, 0xDFFF, 0x11_0000, u32::MAX as u64] {
        assert!(decode_binary::<char>(&varint(cp)).is_err(), "char {cp:#x}");
    }
    assert!(
        decode_binary::<Shape>(&[4]).is_err(),
        "variant index past the end"
    );
    assert!(matches!(
        decode_binary::<Shape>(&varint(u32::MAX as u64 + 1)),
        Err(Error::IntegerOutOfRange)
    ));
    assert!(matches!(decode_binary::<f32>(&[0, 0, 0]), Err(Error::Eof)));
    assert!(matches!(decode_binary::<f64>(&[0; 7]), Err(Error::Eof)));
}

#[test]
fn lengths_past_the_input_fail_fast() {
    assert!(matches!(
        decode_binary::<String>(&[5, b'a', b'b']),
        Err(Error::Eof)
    ));
    assert!(matches!(
        decode_binary::<String>(&varint(1 << 63)),
        Err(Error::Eof)
    ));
    assert!(matches!(
        decode_binary::<&str>(&varint(u64::MAX)),
        Err(Error::Eof)
    ));
    assert!(matches!(
        decode_binary::<Vec<u8>>(&varint(1 << 40)),
        Err(Error::SequenceTooLong)
    ));
    let limit = DEFAULT_MAX_SEQUENCE_LEN as u64;
    assert!(matches!(
        decode_binary::<Vec<u8>>(&varint(limit)),
        Err(Error::Eof)
    ));
    assert!(matches!(
        decode_binary::<Vec<u8>>(&varint(limit + 1)),
        Err(Error::SequenceTooLong)
    ));
}

#[test]
fn length_prefix_width_at_boundaries() {
    for (n, prefix) in [
        (0usize, 1usize),
        (63, 1),
        (64, 1),
        (127, 1),
        (128, 2),
        (16383, 2),
        (16384, 3),
    ] {
        let s = "a".repeat(n);
        let b = encode_binary(&s).unwrap();
        assert_eq!(b.len() - n, prefix, "n={n}");
        assert_eq!(decode_binary_exact::<String>(&b).unwrap(), s);
        let v = vec![7u8; n];
        let b = encode_binary(&v).unwrap();
        assert_eq!(b.len() - n, prefix, "vec n={n}");
        assert_eq!(decode_binary_exact::<Vec<u8>>(&b).unwrap(), v);
    }
}

#[test]
fn zero_sized_elements_are_budgeted() {
    let limit = DEFAULT_MAX_SEQUENCE_LEN as u64;
    // One sequence up to the limit is fine.
    assert_eq!(
        decode_binary_exact::<Vec<()>>(&varint(limit))
            .unwrap()
            .len(),
        DEFAULT_MAX_SEQUENCE_LEN
    );
    assert_eq!(
        decode_binary_exact::<Vec<Empty>>(&varint(limit))
            .unwrap()
            .len(),
        DEFAULT_MAX_SEQUENCE_LEN
    );
    // Nested ones would turn a few hundred bytes into ~10^9 iterations.
    let mut buf = varint(64);
    for _ in 0..64 {
        buf.extend(varint(limit));
    }
    let t = Instant::now();
    assert!(matches!(
        decode_binary::<Vec<Vec<()>>>(&buf),
        Err(Error::SequenceTooLong)
    ));
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn preallocation_is_capped() {
    #[derive(Debug, AsunEncode, AsunDecode, PartialEq, Default)]
    struct Big {
        a: (u64, u64, u64, u64, u64, u64, u64, u64),
        b: (u64, u64, u64, u64, u64, u64, u64, u64),
        c: (u64, u64, u64, u64, u64, u64, u64, u64),
        d: (u64, u64, u64, u64, u64, u64, u64, u64),
    }
    // 1M claimed 256-byte elements backed by 1 MiB of zeros: each element
    // takes 32 bytes, so this ends in Eof without reserving 256 MiB upfront.
    let mut buf = varint(1 << 20);
    buf.resize(buf.len() + (1 << 20), 0);
    assert!(matches!(decode_binary::<Vec<Big>>(&buf), Err(Error::Eof)));
    // A legitimate large vector still decodes.
    let v: Vec<Big> = (0..40_000).map(|_| Big::default()).collect();
    let b = encode_binary(&v).unwrap();
    assert_eq!(decode_binary_exact::<Vec<Big>>(&b).unwrap().len(), 40_000);
}

#[test]
fn deep_recursion_is_rejected_not_a_stack_overflow() {
    // Node { kids: [Node { kids: [ ... ] }] }: 0x01 per level, 0x00 innermost.
    let nested = |levels: usize| {
        let mut buf = vec![1u8; levels];
        buf.push(0);
        buf
    };
    let mut ok = nested(127);
    let n: Node = decode_binary_exact(&ok).unwrap();
    assert_eq!(encode_binary(&n).unwrap(), ok);
    ok = nested(128);
    assert!(matches!(
        decode_binary::<Node>(&ok),
        Err(Error::DepthLimitExceeded)
    ));
    // A megabyte of nesting would otherwise overflow even an 8 MiB stack.
    let deep = nested(1 << 20);
    let r = std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || matches!(decode_binary::<Node>(&deep), Err(Error::DepthLimitExceeded)))
        .unwrap()
        .join()
        .unwrap();
    assert!(r);

    // Depth counts nesting, not total sequences: many deep siblings are fine,
    // both inside one value and across values read from one decoder.
    let mut wide = vec![200u8, 1]; // 200 kids (two-byte varint)
    for _ in 0..200 {
        wide.extend(nested(120));
    }
    let n: Node = decode_binary_exact(&wide).unwrap();
    assert_eq!(n.kids.len(), 200);
    let twice = [nested(127), nested(127)].concat();
    let mut d = BinaryDecoder::new(&twice);
    Node::decode_binary(&mut d).unwrap();
    Node::decode_binary(&mut d).unwrap();
    d.finish().unwrap();
}

#[test]
fn borrowed_strs_point_into_the_input() {
    #[derive(Debug, AsunEncode, AsunDecode, PartialEq)]
    struct B<'a> {
        s: &'a str,
        t: Vec<&'a str>,
    }
    let v = B {
        s: "zero-copy",
        t: vec!["a", "", "é"],
    };
    let b = encode_binary(&v).unwrap();
    let back: B = decode_binary_exact(&b).unwrap();
    assert_eq!(back, v);
    assert!(b.as_ptr_range().contains(&back.s.as_ptr()));
}
