//! Adversarial format tests. Expected results follow `conformance/GRAMMAR.abnf`
//! (declared authoritative), falling back to `docs/SPEC.md` where the grammar
//! is silent. Each category runs every case, prints a PASS/FAIL table, and
//! fails at the end if anything mismatched — so one run shows every finding.
//!
//! Run: cargo test --test adversarial_test -- --nocapture --test-threads=1

use asun::{AsunDecode, AsunEncode, decode, encode, encode_pretty, encode_typed};
use std::fmt::Debug;
use std::panic::{AssertUnwindSafe, catch_unwind};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

enum Outcome<T> {
    Done(asun::Result<T>),
    Panic,
    Hang,
}

/// Decode on a worker thread so an infinite loop is reported instead of
/// stalling the whole run. A hung worker is leaked; the process exit reaps it.
fn run_decode<T>(input: &str) -> Outcome<T>
where
    T: for<'a> AsunDecode<'a> + Send + 'static,
{
    trace(input);
    let owned = input.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(|| decode::<T>(&owned)));
            let _ = tx.send(r);
        })
        .unwrap();
    match rx.recv_timeout(std::time::Duration::from_secs(2)) {
        Ok(Ok(r)) => Outcome::Done(r),
        Ok(Err(_)) => Outcome::Panic,
        Err(_) => Outcome::Hang,
    }
}

#[derive(Default)]
struct Report {
    name: &'static str,
    rows: Vec<(bool, String, String, String)>, // pass, input, expected, actual
}

/// `ASUN_TRACE=1` prints each input before decoding, to locate hangs.
fn trace(input: &str) {
    if std::env::var_os("ASUN_TRACE").is_some() {
        eprintln!(
            "TRACE {}",
            show(&input.chars().take(120).collect::<String>())
        );
    }
}

fn show(s: &str) -> String {
    format!("{s:?}")
}

impl Report {
    fn new(name: &'static str) -> Self {
        Report {
            name,
            rows: Vec::new(),
        }
    }

    fn ok<T>(&mut self, input: &str, expected: T)
    where
        T: for<'a> AsunDecode<'a> + PartialEq + Debug + Send + 'static,
    {
        let (pass, actual) = match run_decode::<T>(input) {
            Outcome::Hang => (false, "HANG (>2s, infinite loop?)".to_string()),
            Outcome::Panic => (false, "PANIC".to_string()),
            Outcome::Done(Ok(v)) => (v == expected, format!("Ok({v:?})")),
            Outcome::Done(Err(e)) => (false, format!("Err({e})")),
        };
        self.rows
            .push((pass, show(input), format!("Ok({expected:?})"), actual));
    }

    fn err<T>(&mut self, input: &str)
    where
        T: for<'a> AsunDecode<'a> + Debug + Send + 'static,
    {
        let (pass, actual) = match run_decode::<T>(input) {
            Outcome::Hang => (false, "HANG (>2s, infinite loop?)".to_string()),
            Outcome::Panic => (false, "PANIC".to_string()),
            Outcome::Done(Ok(v)) => (false, format!("Ok({v:?})")),
            Outcome::Done(Err(e)) => (true, format!("Err({e})")),
        };
        self.rows
            .push((pass, show(input), "Err".to_string(), actual));
    }

    /// encode → decode must reproduce the value exactly.
    fn roundtrip<T>(&mut self, value: T)
    where
        T: AsunEncode + for<'a> AsunDecode<'a> + PartialEq + Debug,
    {
        for (mode, enc) in [
            ("plain", encode(&value)),
            ("typed", encode_typed(&value)),
            ("pretty", encode_pretty(&value)),
        ] {
            let res = catch_unwind(AssertUnwindSafe(|| match &enc {
                Err(e) => Err(format!("encode Err({e})")),
                Ok(text) => match decode::<T>(text) {
                    Ok(v) if v == value => Ok(()),
                    Ok(v) => Err(format!("decoded {v:?} from {text:?}")),
                    Err(e) => Err(format!("decode Err({e}) on {text:?}")),
                },
            }));
            let (pass, actual) = match res {
                Err(_) => (false, "PANIC".to_string()),
                Ok(Ok(())) => (true, "same".to_string()),
                Ok(Err(m)) => (false, m),
            };
            self.rows
                .push((pass, format!("[{mode}] {value:?}"), "same".into(), actual));
        }
    }

    /// Record a case where the encoder itself should refuse the value.
    fn encode_err<T: AsunEncode + Debug>(&mut self, value: T) {
        let got = catch_unwind(AssertUnwindSafe(|| encode(&value)));
        let (pass, actual) = match got {
            Err(_) => (false, "PANIC".to_string()),
            Ok(Ok(s)) => (false, format!("Ok({s:?})")),
            Ok(Err(e)) => (true, format!("Err({e})")),
        };
        self.rows
            .push((pass, format!("encode {value:?}"), "Err".into(), actual));
    }

    fn finish(self) {
        let fails = self.rows.iter().filter(|r| !r.0).count();
        println!(
            "\n=== {} — {} cases, {} failed ===",
            self.name,
            self.rows.len(),
            fails
        );
        for (pass, input, expected, actual) in &self.rows {
            if !pass {
                println!("FAIL  {input}\n      expected {expected}\n      actual   {actual}");
            }
        }
        assert_eq!(
            fails, 0,
            "{} adversarial cases failed in {}",
            fails, self.name
        );
    }
}

// ---------------------------------------------------------------------------
// Target types
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct Uuid {
    #[asun(rename = "id uuid")]
    id: String,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct A {
    a: String,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct OA {
    a: Option<String>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct S {
    a: String,
    b: i64,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct I {
    i: i64,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct OI {
    i: Option<i64>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct U8 {
    u: u8,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct U64 {
    u: u64,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct F {
    f: f64,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct B {
    b: bool,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct C {
    c: char,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct V {
    v: Vec<i64>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct VO {
    v: Vec<Option<i64>>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct VS {
    v: Vec<String>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct VV {
    v: Vec<Vec<i64>>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct P {
    x: i64,
    y: i64,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct N {
    n: P,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct ON {
    n: Option<P>,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct Weird {
    #[asun(rename = "a,b")]
    comma: String,
    #[asun(rename = "q\"q")]
    quote: String,
    #[asun(rename = "中文")]
    cjk: String,
    #[asun(rename = "{}[]@:")]
    punct: String,
    #[asun(rename = "")]
    empty: String,
    #[asun(rename = " ")]
    space: String,
    #[asun(rename = "1st")]
    digit: String,
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct Deep {
    name: String,
    tags: Vec<String>,
    pts: Vec<P>,
    opt: Option<Vec<Option<P>>>,
    grid: Vec<Vec<String>>,
}

fn a(s: &str) -> A {
    A { a: s.into() }
}
fn sb(s: &str, b: i64) -> S {
    S { a: s.into(), b }
}

// ---------------------------------------------------------------------------
// 1. Unquoted / quoted strings — the user's original probe lives here
// ---------------------------------------------------------------------------

#[test]
fn strings() {
    let mut r = Report::new("strings");

    // User's probe: raw `"` and `@` are forbidden in plain-string (GRAMMAR pchar).
    r.err::<Uuid>(r#"{"id uuid"}:(赛d " sdf@ )"#);
    r.ok(
        r#"{"id uuid"}:("赛d \" sdf@ ")"#,
        Uuid {
            id: "赛d \" sdf@ ".into(),
        },
    );
    r.ok(r#"{"id uuid"@str}:(赛d)"#, Uuid { id: "赛d".into() });

    // Raw reserved characters in an unquoted value must be rejected.
    r.ok("{a}:(hello@world)", a("hello@world"));
    r.ok("{a}:(http://x.com)", a("http://x.com"));
    r.ok("{a}:(a:b)", a("a:b"));
    r.err::<A>("{a}:(a{b)");
    r.err::<A>("{a}:(a}b)");
    r.err::<A>(r#"{a}:(a"b")"#);
    r.err::<A>(r#"{a}:("a"b)"#);
    r.err::<A>(r#"{a}:("a" "b")"#);
    r.err::<A>("{a}:(a\nb)"); // raw LF inside a plain string (control char)
    r.err::<A>("{a}:(a\u{1}b)");
    r.err::<A>("{a}:(\"a\nb\")"); // raw LF inside a quoted string
    r.err::<A>("{a}:(\"a\u{0}b\")");

    // Escapes.
    r.ok(r#"{a}:(\,\(\)\[\]\{\}\:\@\\\")"#, a(r#",()[]{}:@\""#));
    r.ok(r#"{a}:("\n\t\r\b\f")"#, a("\n\t\r\u{8}\u{c}"));
    r.ok(r#"{a}:(\u4e2d\u6587)"#, a("中文"));
    r.ok(r#"{a}:("\u4E2D")"#, a("中"));
    r.ok(r#"{a}:("\uD83D\uDE00")"#, a("😀")); // surrogate pair
    r.err::<A>(r#"{a}:("\uD800")"#); // lone surrogate is not a scalar value
    r.err::<A>(r#"{a}:("\uDE00x")"#);
    r.err::<A>(r#"{a}:(\u12)"#);
    r.err::<A>(r#"{a}:("\u12G4")"#);
    r.err::<A>(r#"{a}:(\x)"#);
    r.err::<A>(r#"{a}:("\q")"#);
    r.err::<A>(r#"{a}:(abc\)"#); // `\)` eats the closer
    r.err::<A>(r#"{a}:("abc\")"#);
    r.err::<A>(r#"{a}:("abc)"#);

    // Whitespace and trimming.
    r.ok("{a}:(  spaced   inside  )", a("spaced   inside"));
    r.ok("{a}:(\tx\t)", a("x"));
    r.ok("{a}:(a\tb)", a("a\tb"));
    r.ok(r#"{a}:("  x  ")"#, a("  x  "));
    r.ok(r#"{a}:(   "x"   )"#, a("x"));
    r.ok("{a}:(\u{3000}x\u{3000})", a("\u{3000}x\u{3000}")); // only ASCII ws trimmed
    r.ok("{a}:(中文 字符 🎉)", a("中文 字符 🎉"));
    r.err::<A>("{a}:(\\ x\\ )"); // `\ ` is not in the escape list
    r.ok(r#"{a}:("")"#, a(""));

    // Things that look like other types but land in a String.
    r.ok("{a}:(- 5)", a("- 5"));
    r.ok("{a}:(--1)", a("--1"));
    r.ok("{a}:(.5)", a(".5"));
    r.ok("{a}:(1e)", a("1e"));
    r.ok("{a}:(123abc)", a("123abc"));
    r.ok("{a}:(a/b*c)", a("a/b*c"));
    r.err::<A>("{a}:(null)"); // keyword null into a String
    r.ok(r#"{a}:("null")"#, a("null"));
    r.ok("{a}:(True)", a("True"));

    // Null vs empty string.
    r.ok("{a}:()", OA { a: None });
    r.ok("{a}:(   )", OA { a: None });
    r.ok(r#"{a}:("")"#, OA { a: Some("".into()) });
    r.err::<A>("{a}:()"); // non-Option String cannot be null

    r.finish();
}

// ---------------------------------------------------------------------------
// 2. Numbers and booleans
// ---------------------------------------------------------------------------

#[test]
fn numbers() {
    let mut r = Report::new("numbers");

    r.ok("{i}:(-0)", I { i: 0 });
    r.ok("{i}:(007)", I { i: 7 });
    r.ok("{i}:(  42  )", I { i: 42 });
    r.ok("{i}:(9223372036854775807)", I { i: i64::MAX });
    r.ok("{i}:(-9223372036854775808)", I { i: i64::MIN });
    r.err::<I>("{i}:(9223372036854775808)");
    r.err::<I>("{i}:(-9223372036854775809)");
    r.err::<I>("{i}:(99999999999999999999999999)");
    r.ok("{u}:(18446744073709551615)", U64 { u: u64::MAX });
    r.err::<U64>("{u}:(18446744073709551616)");
    r.err::<U64>("{u}:(-1)");
    r.ok("{u}:(-0)", U64 { u: 0 });
    r.ok("{u}:(255)", U8 { u: 255 });
    r.err::<U8>("{u}:(256)");
    r.err::<U8>("{u}:(-1)");
    r.err::<I>("{i}:(+5)");
    r.err::<I>("{i}:(- 5)");
    r.err::<I>("{i}:(4 2)");
    r.err::<I>("{i}:(1.0)");
    r.err::<I>("{i}:(1e3)");
    r.err::<I>("{i}:(0x10)");
    r.err::<I>("{i}:(1_000)");
    r.err::<I>(r#"{i}:("42")"#);
    r.err::<I>("{i}:(42abc)");
    r.err::<I>("{i}:(-)");
    r.err::<I>("{i}:()");
    r.ok("{i}:()", OI { i: None });
    r.err::<OI>("{i}:(abc)");

    r.ok("{f}:(3)", F { f: 3.0 });
    r.ok("{f}:(-0.0)", F { f: -0.0 });
    r.ok("{f}:(1.5e-3)", F { f: 1.5e-3 });
    r.ok("{f}:(-1.0E+100)", F { f: -1.0e100 });
    r.ok("{f}:(1e10)", F { f: 1e10 });
    r.ok("{f}:(0.30000000000000004)", F { f: 0.1 + 0.2 });
    r.ok("{f}:(1.7976931348623157e308)", F { f: f64::MAX });
    r.ok("{f}:(5e-324)", F { f: 5e-324 });
    r.err::<F>("{f}:(1e309)"); // overflows to inf: no inf literal in grammar
    r.err::<F>("{f}:(.5)");
    r.err::<F>("{f}:(5.)");
    r.err::<F>("{f}:(+1.5)");
    r.err::<F>("{f}:(1e)");
    r.err::<F>("{f}:(1e+)");
    r.err::<F>("{f}:(NaN)");
    r.err::<F>("{f}:(inf)");
    r.err::<F>("{f}:(infinity)");
    r.err::<F>("{f}:(1.2.3)");
    r.err::<F>("{f}:(1,5)"); // European decimal → 2 elements
    r.err::<F>(r#"{f}:("1.5")"#);

    r.ok("{b}:(true)", B { b: true });
    r.ok("{b}:( false )", B { b: false });
    r.err::<B>("{b}:(True)");
    r.err::<B>("{b}:(TRUE)");
    r.err::<B>("{b}:(1)");
    r.err::<B>("{b}:(yes)");
    r.err::<B>(r#"{b}:("true")"#);
    r.err::<B>("{b}:(truex)");
    r.err::<B>("{b}:(t)");

    r.ok("{c}:(x)", C { c: 'x' });
    r.ok("{c}:(赛)", C { c: '赛' });
    r.ok(r#"{c}:(\,)"#, C { c: ',' });
    r.ok(r#"{c}:(" ")"#, C { c: ' ' });
    r.err::<C>("{c}:(xy)");
    r.err::<C>(r#"{c}:("")"#);

    r.finish();
}

// ---------------------------------------------------------------------------
// 3. Schema header
// ---------------------------------------------------------------------------

#[test]
fn schema() {
    let mut r = Report::new("schema");

    r.ok(r#"{"a",b}:(x,1)"#, sb("x", 1));
    r.ok(r#"{"a"@str,"b"@int}:(x,1)"#, sb("x", 1));
    r.ok("{a@ str , b @int}:(x,1)", sb("x", 1));
    r.ok("{ a , b }:(x,1)", sb("x", 1));
    r.err::<S>("{a,b,}:(x,1)");
    r.ok("{a,b} : (x,1)", sb("x", 1));
    r.ok("{a,b}\r\n:\r\n(x,1)", sb("x", 1));
    r.ok("{a /* c */ , b /* } ) , */}:(x,1)", sb("x", 1));
    r.ok("{b,a}:(1,x)", sb("x", 1)); // reordered → by name
    r.ok("{a,b,c}:(x,1,zzz)", sb("x", 1)); // unknown extra field skipped
    r.ok("{a,z,b}:(x,\\),1)", sb("x", 1));
    r.ok(r#"{a,z,b}:(x,")(,\"",1)"#, sb("x", 1));
    r.ok("{a,z@{q@[int]},b}:(x,([1,(2)]),1)", sb("x", 1));
    r.ok("{a,z@[{p,q}],b}:(x,[(1,[a,b]),(2,())],1)", sb("x", 1));

    r.err::<S>("{a b}:(x,1)"); // bare name with space
    r.err::<S>("{中文,b}:(x,1)"); // non-ASCII bare name
    r.err::<S>("{a-b,b}:(x,1)");
    r.err::<S>("{a.b,b}:(x,1)");
    r.err::<S>("{a@,b}:(x,1)");
    r.err::<S>("{a@int@str,b}:(x,1)");
    r.err::<S>("{a@integer,b}:(x,1)");
    r.err::<S>("{a@Str,b}:(x,1)");
    r.err::<S>("{a@[,b}:(x,1)");
    r.err::<S>("{a@{,b}:(x,1)");
    r.err::<S>("{a@[]}:([])"); // empty array-type-hint not in grammar
    r.err::<S>("{,a,b}:(x,1)");
    r.err::<S>("{a,,b}:(x,1)");
    r.ok("{}:()", sb("", 0)); // missing target fields default
    r.err::<S>("{a,b");
    r.err::<S>("{a,b}");
    r.err::<S>("{a,b}:");
    r.err::<S>("{a,b}(x,1)");
    r.err::<S>("{a,b}::(x,1)");
    r.err::<S>("{a,b}=(x,1)");
    r.err::<S>("{a,b}:[x,1]");
    r.err::<S>("{a,b}:{x,1}");
    r.err::<S>("{a,b}:(x,1)(y,2)");
    r.err::<S>("{a,b}:(x,1),(y,2)");
    r.err::<S>("{a,b}:(x,1) junk");
    r.err::<S>("{a,b}:(x,1));");
    r.err::<S>("(x,1)"); // bare tuple without schema
    r.err::<S>("");
    r.err::<S>("   ");
    r.err::<S>("{a,a}:(x,y)"); // duplicate field name
    r.err::<S>("{a,b,b}:(x,1,2)");
    r.err::<S>("{a@int,b}:(hello,1)"); // S3: hint is authoritative
    r.err::<S>("{a,b@str}:(x,1)"); // hint says str, target i64
    r.err::<S>(r#"{"a,b}:(x,1)"#);
    r.err::<S>(r#"{"a\",b}:(x,1)"#);

    // Weird quoted field names round-trip through rename.
    r.ok(
        r#"{"a,b","q\"q","中文","{}[]@:",""," ","1st"}:(1,2,3,4,5,6,7)"#,
        Weird {
            comma: "1".into(),
            quote: "2".into(),
            cjk: "3".into(),
            punct: "4".into(),
            empty: "5".into(),
            space: "6".into(),
            digit: "7".into(),
        },
    );
    r.ok("{1st}:(x)", A2 { first: "x".into() });

    r.finish();
}

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct A2 {
    #[asun(rename = "1st")]
    first: String,
}

// ---------------------------------------------------------------------------
// 4. Tuple alignment, commas, comments
// ---------------------------------------------------------------------------

#[test]
fn tuples_and_comments() {
    let mut r = Report::new("tuples_and_comments");

    r.err::<S>("{a,b}:(x,1,)"); // 3 slots
    r.err::<S>("{a,b}:( x , 1 , )");
    r.err::<S>("{a,b}:(x,1,,)"); // S5: 3 elements
    r.err::<S>("{a,b}:(x)");
    r.err::<S>("{a,b}:(x,)"); // b = null into i64
    r.err::<S>("{a,b}:(x,1,2)");
    r.err::<S>("{a,b}:(,x,1)");
    r.err::<S>("{a,b}:(x,1");
    r.err::<S>("{a,b}:x,1)");
    r.err::<S>("{a,b}:((x,1))");
    r.err::<S>("{a,b}:(x,(1))");
    r.err::<S>("{a,b}:(x,[1])");

    r.ok("/* lead */ {a,b}:(x,1)", sb("x", 1));
    r.ok("{a,b}:(x,1) /* trail */", sb("x", 1));
    r.ok("{a,b}:/* mid */(x,1)", sb("x", 1));
    r.ok("/**/{a,b}:(x,1)/**/", sb("x", 1));
    r.ok("/* a */ /* b */ {a,b}:(x,1)", sb("x", 1));
    r.ok("/* ** / * */{a,b}:(x,1)", sb("x", 1));
    r.err::<S>("{a,b}:(x,1) /* unclosed");
    r.err::<S>("/* unclosed {a,b}:(x,1)");
    r.err::<S>("/* /* nested */ */{a,b}:(x,1)");
    r.err::<S>("// line comment\n{a,b}:(x,1)");
    r.err::<S>("# hash\n{a,b}:(x,1)");
    // S4: comments inside tuples are ERROR in conformance (samples/01 uses them!).
    r.ok("{a,b}:(x, /* c */ 1)", sb("x", 1));
    r.ok("{a,b}:(/* c */x,1)", sb("x", 1));
    r.ok("{a,b}:(x,1 /* c */)", sb("x", 1));
    // A comment-looking run in the middle of a plain string.
    r.err::<S>("{a,b}:(x/*y*/z,1)"); // a comment cannot split a value
    r.ok(
        r#"{a,b}:("/* not a comment */",1)"#,
        sb("/* not a comment */", 1),
    );

    // BOM and odd whitespace at document level.
    r.ok("\u{FEFF}{a,b}:(x,1)", sb("x", 1));
    r.ok("\n\n\t {a,b}:(x,1) \n\n", sb("x", 1));
    r.ok("{a,b}:(x,1)\r\n", sb("x", 1));
    r.err::<S>("{a,b}:(x,1)\u{0}");
    r.err::<S>("{a,b}:(x,1)\u{3000}");

    r.finish();
}

// ---------------------------------------------------------------------------
// 5. Arrays, nesting, vec-of-struct
// ---------------------------------------------------------------------------

#[test]
fn arrays_and_nesting() {
    let mut r = Report::new("arrays_and_nesting");

    r.ok("{v@[int]}:([1,2,3])", V { v: vec![1, 2, 3] });
    r.err::<V>("{v@[int]}:([1,2,3,])"); // trailing null into i64
    r.ok("{v@[int]}:([ 1 , 2 ])", V { v: vec![1, 2] });
    r.ok("{v@[int]}:([])", V { v: vec![] });
    r.ok("{v@[int]}:([   ])", V { v: vec![] });
    r.ok("{v}:([1,2])", V { v: vec![1, 2] }); // binding omitted
    r.err::<V>("{v@[int]}:([1,,3])");
    r.ok(
        "{v@[int]}:([1,,3])",
        VO {
            v: vec![Some(1), None, Some(3)],
        },
    );
    r.err::<V>("{v@[int]}:([1,2,,])"); // S5 also applies to arrays → [1,2,null]
    r.err::<V>("{v@[int]}:([1,2)");
    r.err::<V>("{v@[int]}:([1,2]]");
    r.err::<V>("{v@[int]}:((1,2))");
    r.err::<V>("{v@[int]}:(1,2)");
    r.err::<V>("{v@[int]}:()");
    r.err::<V>("{v@[int]}:([1,[2]])");
    r.err::<V>("{v@[int]}:([1 2])");
    r.ok("{v@[int]}:([/* c */1])", V { v: vec![1] });
    r.ok(
        r#"{v@[str]}:([a\,b,"c,d",\],""])"#,
        VS {
            v: vec!["a,b".into(), "c,d".into(), "]".into(), "".into()],
        },
    );
    r.ok(
        "{v@[str]}:([a:b])",
        VS {
            v: vec!["a:b".into()],
        },
    );
    r.ok(
        "{v@[[int]]}:([[1],[2,3],[]])",
        VV {
            v: vec![vec![1], vec![2, 3], vec![]],
        },
    );
    r.err::<VV>("{v@[[int]]}:([1,[2]])");

    r.ok(
        "{n@{x,y}}:((1,2))",
        N {
            n: P { x: 1, y: 2 },
        },
    );
    r.ok(
        "{n@{y,x}}:((2,1))",
        N {
            n: P { x: 1, y: 2 },
        },
    ); // nested reorder
    r.err::<ON>("{n@{x,y}}:(())"); // empty tuple for a 2-field schema
    r.ok("{n@{x,y}}:()", ON { n: None });
    r.err::<N>("{n@{x,y}}:((1,2,3))");
    r.err::<N>("{n@{x,y}}:((1))");
    r.err::<N>("{n@{x,y}}:([1,2])");
    r.err::<N>("{n@{x,y}}:(1,2)");
    r.err::<N>("{n@{x,y}}:({1,2})");
    r.err::<N>("{n@{x,y}}:(((1,2)))");

    let v2 = vec![sb("x", 1), sb("y", 2)];
    r.ok("[{a,b}]:(x,1),(y,2)", sb_vec(&v2));
    r.err::<Vec<S>>("[{a,b}]:(x,1),(y,2),");
    r.ok("[{a,b}]:\n  (x,1),\n  (y,2)\n", sb_vec(&v2));
    r.ok("[ {a,b} ] : (x,1) , (y,2)", sb_vec(&v2));
    r.ok("[{b,a}]:(1,x),(2,y)", sb_vec(&v2));
    r.ok("[{a,b}]:(x,1),/* c */(y,2)", sb_vec(&v2));
    r.ok("[]", Vec::<S>::new());
    r.ok("[{a,b}]:", Vec::<S>::new());
    r.err::<Vec<S>>("[{a,b}]:(x,1)(y,2)");
    r.err::<Vec<S>>("[{a,b}]:(x,1),,(y,2)");
    r.err::<Vec<S>>("[{a,b}]:,(x,1)");
    r.err::<Vec<S>>("[{a,b}]:(x,1),(y)");
    r.err::<Vec<S>>("[{a,b}]:(x,1),(y,2,3)");
    r.err::<Vec<S>>("[{a,b}]:(x,1),(y,two)");
    r.err::<Vec<S>>("[{a,b}:(x,1)");
    r.err::<Vec<S>>("[{a,b}]]:(x,1)");
    r.err::<Vec<S>>("[{a,b}]:[(x,1)]");
    r.err::<Vec<S>>("{a,b}:(x,1)"); // single-object form into Vec
    r.err::<S>("[{a,b}]:(x,1)"); // array form into single struct

    r.finish();
}

fn sb_vec(v: &[S]) -> Vec<S> {
    v.iter().map(|s| sb(&s.a, s.b)).collect()
}

// ---------------------------------------------------------------------------
// 6. Hostile sizes / depth — must error cleanly, never crash
// ---------------------------------------------------------------------------

#[test]
fn hostile_inputs() {
    let mut r = Report::new("hostile_inputs");

    let deep_arr = format!(
        "{{v@[[int]]}}:({}{})",
        "[".repeat(200_000),
        "]".repeat(200_000)
    );
    r.err::<VV>(&deep_arr);
    let deep_hint = format!(
        "{{v@{}int{}}}:([])",
        "[".repeat(200_000),
        "]".repeat(200_000)
    );
    r.err::<V>(&deep_hint);
    let deep_schema = format!(
        "{{a,z@{}q{}}}:(x)",
        "{z@".repeat(100_000),
        "}".repeat(100_000)
    );
    r.err::<A>(&deep_schema);
    let deep_tuple = format!("{{a,z}}:(x,{}{})", "(".repeat(200_000), ")".repeat(200_000));
    r.err::<A>(&deep_tuple); // nesting cap applies to skipped values too (SPEC 9.1)
    let unclosed = "{a}:(\"".to_string() + &"x".repeat(1_000_000);
    r.err::<A>(&unclosed);
    let many_fields = format!(
        "{{a,{}b}}:(x,{}1)",
        (0..50_000).map(|i| format!("f{i},")).collect::<String>(),
        ",".repeat(50_000)
    );
    r.ok(&many_fields, sb("x", 1));
    let big_str = "y".repeat(5_000_000);
    r.ok(&format!("{{a}}:({big_str})"), a(&big_str));
    let backslashes = "\\\\".repeat(100_000);
    r.ok(&format!("{{a}}:({backslashes})"), a(&"\\".repeat(100_000)));
    r.err::<A>(&format!("{{a}}:({}\\)", "\\\\".repeat(1000)));
    // Truncate a valid document at every byte: never panic.
    let doc = r#"[{a,z@{p@[{q,r}]},b}]:(x,([(1,"a\"b"),(2,[c])]),1),(y,(),2)"#;
    for cut in 0..doc.len() {
        let p = &doc[..cut];
        // Cutting right after a complete row (`...,1)` / `...,1),`) is valid.
        if doc.is_char_boundary(cut)
            && !p.ends_with(",1)")
            && !p.ends_with(",1),")
            && !p.ends_with(":")
        {
            r.err::<Vec<S>>(p);
        }
    }

    r.finish();
}

// ---------------------------------------------------------------------------
// 7. Encoder must produce text its own decoder accepts unchanged
// ---------------------------------------------------------------------------

#[test]
fn encode_roundtrip() {
    let mut r = Report::new("encode_roundtrip");

    let tricky = [
        "",
        " ",
        "  x  ",
        "\t",
        "true",
        "false",
        "123",
        "-5",
        "007",
        "1.5",
        "1e5",
        "-0",
        "- 5",
        ".5",
        "null",
        "a,b",
        "(",
        ")",
        "[",
        "]",
        "{",
        "}",
        ":",
        "@",
        "\"",
        "\\",
        "\\u0041",
        "\n",
        "\r\n",
        "\t",
        "\u{0}",
        "\u{1f}",
        "\u{7f}",
        "/*",
        "*/",
        "/* x */",
        "a/*b*/c",
        "中文",
        "😀",
        "a\u{2028}b",
        "\u{FEFF}",
        "\u{3000}x",
        "x\u{3000}",
        "{a}:(b)",
        "[{a}]:(b),(c)",
        "\"quoted\"",
        "trailing\\",
        "http://x.com/?a=1&b=(2)",
        "e@mail.com",
        "#tag",
        "'single'",
        "`tick`",
    ];
    for s in tricky {
        r.roundtrip(A { a: s.into() });
        r.roundtrip(OA { a: Some(s.into()) });
    }
    r.roundtrip(OA { a: None });
    r.roundtrip(VS {
        v: tricky.iter().map(|s| s.to_string()).collect(),
    });
    r.roundtrip(VS { v: vec![] });
    r.roundtrip(VS { v: vec!["".into()] });
    r.roundtrip(VO { v: vec![None] });
    r.roundtrip(VO {
        v: vec![None, None],
    });
    r.roundtrip(VO {
        v: vec![Some(1), None],
    });
    r.roundtrip(VV { v: vec![vec![]] });
    r.roundtrip(VV {
        v: vec![vec![], vec![]],
    });
    r.roundtrip(ON { n: None });
    r.roundtrip(ON {
        n: Some(P { x: -1, y: 0 }),
    });
    r.roundtrip(Vec::<S>::new());
    r.roundtrip(vec![sb("", 0)]);
    r.roundtrip(vec![sb(",", i64::MIN), sb(")", i64::MAX)]);
    r.roundtrip(I { i: i64::MIN });
    r.roundtrip(U64 { u: u64::MAX });
    for f in [
        0.0,
        -0.0,
        0.1 + 0.2,
        1e-300,
        5e-324,
        f64::MAX,
        f64::MIN,
        f64::MIN_POSITIVE,
        1e21,
        123456789.0,
    ] {
        r.roundtrip(F { f });
    }
    for c in [',', '(', '"', '\\', ' ', '\n', '\0', '赛', '😀', '@', ':'] {
        r.roundtrip(C { c });
    }
    r.roundtrip(Weird {
        comma: ",".into(),
        quote: "\"".into(),
        cjk: "中".into(),
        punct: "@".into(),
        empty: "".into(),
        space: " ".into(),
        digit: "1".into(),
    });
    r.roundtrip(Uuid {
        id: "赛d \" sdf@ ".into(),
    });
    r.roundtrip(Deep {
        name: "  (deep)  ".into(),
        tags: vec!["".into(), ",".into(), "]".into()],
        pts: vec![P { x: 1, y: 2 }],
        opt: Some(vec![None, Some(P { x: 3, y: 4 }), None]),
        grid: vec![vec![], vec!["".into(), " ".into()], vec!["[".into()]],
    });
    r.roundtrip(Deep {
        name: "".into(),
        tags: vec![],
        pts: vec![],
        opt: Some(vec![]),
        grid: vec![],
    });
    r.roundtrip(Deep {
        name: "x".into(),
        tags: vec![],
        pts: vec![],
        opt: None,
        grid: vec![],
    });

    // Non-finite floats have no textual form in the grammar.
    r.encode_err(F { f: f64::NAN });
    r.encode_err(F { f: f64::INFINITY });
    r.encode_err(F {
        f: f64::NEG_INFINITY,
    });

    // Pretty expansion (> 100 columns) must keep a leading null slot.
    let long = "z".repeat(120);
    r.roundtrip(Lead {
        a: None,
        b: long.clone(),
        c: None,
    });
    r.roundtrip(vec![Lead {
        a: None,
        b: long.clone(),
        c: Some((None, long.clone())),
    }]);
    // Field names outside bare-name must be quoted in every header form.
    let odd = Odd {
        a_b: 1,
        sp: "x".into(),
    };
    r.roundtrip(odd.clone());
    r.roundtrip(vec![odd.clone(), odd.clone()]);
    r.roundtrip(Lead2 { inner: vec![odd] });
    // Tuple / enum fields: heterogeneous, so no scalar binding may leak.
    r.roundtrip(vec![
        Ev::Unit,
        Ev::New(3),
        Ev::Tup(1, "x".into()),
        Ev::Rec { k: 2 },
        Ev::Empty {},
    ]);
    r.roundtrip(Lead {
        a: Some(1),
        b: "b".into(),
        c: Some((Some(Ev::Rec { k: 1 }), "t".into())),
    });
    r.finish();
}

#[derive(Debug, PartialEq, Clone, AsunEncode, AsunDecode)]
struct Lead {
    a: Option<i64>,
    b: String,
    c: Option<(Option<Ev>, String)>,
}

#[derive(Debug, PartialEq, Clone, AsunEncode, AsunDecode)]
enum Ev {
    Unit,
    New(i64),
    Tup(i64, String),
    Rec { k: i64 },
    Empty {},
}

#[derive(Debug, PartialEq, Clone, AsunEncode, AsunDecode)]
struct Odd {
    #[asun(rename = "a-b")]
    a_b: i64,
    #[asun(rename = "s p")]
    sp: String,
}

#[derive(Debug, PartialEq, Clone, AsunEncode, AsunDecode)]
struct Lead2 {
    inner: Vec<Odd>,
}
