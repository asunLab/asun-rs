use ason::{decode, encode, encode_typed};
use serde::{Deserialize, Serialize};

// ============================================================================
// Dimension 1: Extra trailing fields — source has more fields than target
// ============================================================================

#[derive(Debug, Serialize)]
struct FullUser {
    id: i64,
    name: String,
    age: i32,
    active: bool,
    score: f64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MiniUser {
    id: i64,
    name: String,
}

#[test]
fn cross_trailing_fields_dropped_vec() {
    let src = vec![
        FullUser {
            id: 1,
            name: "Alice".into(),
            age: 30,
            active: true,
            score: 95.5,
        },
        FullUser {
            id: 2,
            name: "Bob".into(),
            age: 25,
            active: false,
            score: 87.0,
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<MiniUser> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(
        dst[0],
        MiniUser {
            id: 1,
            name: "Alice".into()
        }
    );
    assert_eq!(
        dst[1],
        MiniUser {
            id: 2,
            name: "Bob".into()
        }
    );
}

#[test]
fn cross_trailing_fields_dropped_single() {
    let src = FullUser {
        id: 99,
        name: "Zara".into(),
        age: 40,
        active: true,
        score: 100.0,
    };
    let data = encode(&src).unwrap();
    let dst: MiniUser = decode(&data).unwrap();
    assert_eq!(
        dst,
        MiniUser {
            id: 99,
            name: "Zara".into()
        }
    );
}

// ============================================================================
// Dimension 2: Trailing field is complex (array, map)
// ============================================================================

#[derive(Debug, Serialize)]
struct RichProfile {
    id: i64,
    name: String,
    tags: Vec<String>,
    scores: Vec<i64>,
    meta: std::collections::HashMap<String, i64>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct ThinProfile {
    id: i64,
    name: String,
}

#[test]
fn cross_skip_trailing_array_and_map() {
    let mut meta = std::collections::HashMap::new();
    meta.insert("level".into(), 5);
    meta.insert("xp".into(), 1200);
    let src = RichProfile {
        id: 1,
        name: "Alice".into(),
        tags: vec!["go".into(), "rust".into()],
        scores: vec![90, 85, 92],
        meta,
    };
    let data = encode(&src).unwrap();
    let dst: ThinProfile = decode(&data).unwrap();
    assert_eq!(
        dst,
        ThinProfile {
            id: 1,
            name: "Alice".into()
        }
    );
}

// ============================================================================
// Dimension 3: Nested struct — inner has fewer fields
// ============================================================================

#[derive(Debug, Serialize)]
struct InnerFull {
    x: i64,
    y: i64,
    z: f64,
    w: bool,
}

#[derive(Debug, Serialize)]
struct OuterFull {
    name: String,
    inner: InnerFull,
    flag: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct InnerThin {
    x: i64,
    y: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct OuterThin {
    name: String,
    inner: InnerThin,
}

#[test]
fn cross_nested_struct_fewer_fields() {
    let src = OuterFull {
        name: "test".into(),
        inner: InnerFull {
            x: 10,
            y: 20,
            z: 3.14,
            w: true,
        },
        flag: true,
    };
    let data = encode(&src).unwrap();
    let dst: OuterThin = decode(&data).unwrap();
    assert_eq!(
        dst,
        OuterThin {
            name: "test".into(),
            inner: InnerThin { x: 10, y: 20 }
        }
    );
}

// ============================================================================
// Dimension 4: Vec of nested structs — inner has extra fields
// ============================================================================

#[derive(Debug, Serialize)]
struct TaskFull {
    title: String,
    done: bool,
    priority: i64,
    weight: f64,
}

#[derive(Debug, Serialize)]
struct ProjectFull {
    name: String,
    tasks: Vec<TaskFull>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct TaskThin {
    title: String,
    done: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct ProjectThin {
    name: String,
    tasks: Vec<TaskThin>,
}

#[test]
fn cross_vec_nested_struct_skip_extra() {
    let src = vec![
        ProjectFull {
            name: "Alpha".into(),
            tasks: vec![
                TaskFull {
                    title: "Design".into(),
                    done: true,
                    priority: 1,
                    weight: 0.5,
                },
                TaskFull {
                    title: "Code".into(),
                    done: false,
                    priority: 2,
                    weight: 0.8,
                },
            ],
        },
        ProjectFull {
            name: "Beta".into(),
            tasks: vec![TaskFull {
                title: "Test".into(),
                done: false,
                priority: 3,
                weight: 1.0,
            }],
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<ProjectThin> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0].name, "Alpha");
    assert_eq!(dst[0].tasks.len(), 2);
    assert_eq!(
        dst[0].tasks[0],
        TaskThin {
            title: "Design".into(),
            done: true
        }
    );
    assert_eq!(
        dst[0].tasks[1],
        TaskThin {
            title: "Code".into(),
            done: false
        }
    );
    assert_eq!(dst[1].name, "Beta");
    assert_eq!(dst[1].tasks.len(), 1);
}

// ============================================================================
// Dimension 5: Deep 3-level nesting, each level drops fields
// ============================================================================

#[derive(Debug, Serialize)]
struct L3Full {
    a: i64,
    b: String,
    c: bool,
}

#[derive(Debug, Serialize)]
struct L2Full {
    name: String,
    sub: L3Full,
    code: i64,
    tags: Vec<String>,
}

#[derive(Debug, Serialize)]
struct L1Full {
    id: i64,
    child: L2Full,
    extra: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct L3Thin {
    a: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct L2Thin {
    name: String,
    sub: L3Thin,
}

#[derive(Debug, Deserialize, PartialEq)]
struct L1Thin {
    id: i64,
    child: L2Thin,
}

#[test]
fn cross_deep_nesting_3_levels() {
    let src = L1Full {
        id: 1,
        child: L2Full {
            name: "mid".into(),
            sub: L3Full {
                a: 42,
                b: "hello".into(),
                c: true,
            },
            code: 7,
            tags: vec!["x".into(), "y".into()],
        },
        extra: "dropped".into(),
    };
    let data = encode(&src).unwrap();
    let dst: L1Thin = decode(&data).unwrap();
    assert_eq!(
        dst,
        L1Thin {
            id: 1,
            child: L2Thin {
                name: "mid".into(),
                sub: L3Thin { a: 42 }
            }
        }
    );
}

// ============================================================================
// Dimension 6: Field reorder
// ============================================================================

#[derive(Debug, Serialize)]
struct OrderABC {
    a: i64,
    b: String,
    c: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct OrderCAB {
    c: bool,
    a: i64,
    b: String,
}

#[test]
fn cross_field_reorder() {
    let src = OrderABC {
        a: 1,
        b: "hi".into(),
        c: true,
    };
    let data = encode(&src).unwrap();
    let dst: OrderCAB = decode(&data).unwrap();
    assert_eq!(
        dst,
        OrderCAB {
            c: true,
            a: 1,
            b: "hi".into()
        }
    );
}

// ============================================================================
// Dimension 7: Reorder + drop trailing
// ============================================================================

#[derive(Debug, Serialize)]
struct BigRecord {
    id: i64,
    name: String,
    score: f64,
    active: bool,
    level: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct SmallReordered {
    score: f64,
    id: i64,
}

#[test]
fn cross_reorder_plus_drop_trailing() {
    let src = vec![
        BigRecord {
            id: 1,
            name: "A".into(),
            score: 9.5,
            active: true,
            level: 3,
        },
        BigRecord {
            id: 2,
            name: "B".into(),
            score: 8.0,
            active: false,
            level: 1,
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<SmallReordered> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0], SmallReordered { score: 9.5, id: 1 });
    assert_eq!(dst[1], SmallReordered { score: 8.0, id: 2 });
}

// ============================================================================
// Dimension 8: Target has extra fields (zero-value)
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcSmall {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(default)]
struct DstBig {
    id: i64,
    name: String,
    missing: bool,
    extra: f64,
}

#[test]
fn cross_target_has_extra_fields() {
    let src = SrcSmall {
        id: 42,
        name: "Alice".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstBig = decode(&data).unwrap();
    assert_eq!(dst.id, 42);
    assert_eq!(dst.name, "Alice");
    assert_eq!(dst.missing, false);
    assert_eq!(dst.extra, 0.0);
}

// ============================================================================
// Dimension 9: Optional fields across compat
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcWithOptionals {
    id: i64,
    label: Option<String>,
    score: Option<f64>,
    flag: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstFewerOptionals {
    id: i64,
    label: Option<String>,
}

#[test]
fn cross_optional_fields_skip_trailing() {
    let src = SrcWithOptionals {
        id: 1,
        label: Some("hello".into()),
        score: Some(95.5),
        flag: true,
    };
    let data = encode(&src).unwrap();
    let dst: DstFewerOptionals = decode(&data).unwrap();
    assert_eq!(
        dst,
        DstFewerOptionals {
            id: 1,
            label: Some("hello".into())
        }
    );
}

#[test]
fn cross_optional_nil_skip_trailing() {
    let src = SrcWithOptionals {
        id: 2,
        label: None,
        score: None,
        flag: false,
    };
    let data = encode(&src).unwrap();
    let dst: DstFewerOptionals = decode(&data).unwrap();
    assert_eq!(dst, DstFewerOptionals { id: 2, label: None });
}

// ============================================================================
// Dimension 10: Quoted strings with special chars in trailing
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcSpecialStr {
    id: i64,
    name: String,
    bio: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstNoStr {
    id: i64,
}

#[test]
fn cross_skip_quoted_string_special_chars() {
    let src = SrcSpecialStr {
        id: 1,
        name: "comma,here".into(),
        bio: "paren(test) and \"quotes\"".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstNoStr = decode(&data).unwrap();
    assert_eq!(dst.id, 1);
}

// ============================================================================
// Dimension 11: Skip trailing array fields in vec
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcNestedArray {
    id: i64,
    matrix: Vec<i64>,
    tags: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstNestedArrayThin {
    id: i64,
}

#[test]
fn cross_skip_trailing_array_fields() {
    let src = vec![
        SrcNestedArray {
            id: 1,
            matrix: vec![1, 2, 3],
            tags: vec!["a".into(), "b".into()],
        },
        SrcNestedArray {
            id: 2,
            matrix: vec![4, 5],
            tags: vec!["c".into()],
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<DstNestedArrayThin> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0].id, 1);
    assert_eq!(dst[1].id, 2);
}

// ============================================================================
// Dimension 12: Int widening (text is the same for int32/int64)
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcNarrow {
    id: i32,
    score: i32,
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstWide {
    id: i64,
    score: i64,
    name: String,
}

#[test]
fn cross_int_widening() {
    let src = SrcNarrow {
        id: 100,
        score: 999,
        name: "wide".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstWide = decode(&data).unwrap();
    assert_eq!(
        dst,
        DstWide {
            id: 100,
            score: 999,
            name: "wide".into()
        }
    );
}

// ============================================================================
// Dimension 13: Float roundtrip precision
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct SrcFloats {
    id: i64,
    value: f64,
}

#[test]
fn cross_float_roundtrip() {
    let src = SrcFloats {
        id: 1,
        value: 3.14159,
    };
    let data = encode(&src).unwrap();
    let dst: SrcFloats = decode(&data).unwrap();
    assert!((dst.value - 3.14159).abs() < 1e-10);
}

// ============================================================================
// Dimension 14: Negative numbers, skip trailing
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcNegative {
    a: i64,
    b: i64,
    c: f64,
    d: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstNegativeThin {
    a: i64,
    b: i64,
}

#[test]
fn cross_negative_numbers_skip_trailing() {
    let src = SrcNegative {
        a: -1,
        b: -999999,
        c: -3.14,
        d: "neg".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstNegativeThin = decode(&data).unwrap();
    assert_eq!(dst, DstNegativeThin { a: -1, b: -999999 });
}

// ============================================================================
// Dimension 15: Empty string fields
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcEmpty {
    id: i64,
    name: String,
    bio: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstEmptyThin {
    id: i64,
}

#[test]
fn cross_empty_string_fields() {
    let src = SrcEmpty {
        id: 1,
        name: "".into(),
        bio: "".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstEmptyThin = decode(&data).unwrap();
    assert_eq!(dst.id, 1);
}

// ============================================================================
// Dimension 16: Skip trailing map field
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcWithMap {
    id: i64,
    name: String,
    meta: std::collections::HashMap<String, i64>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstNoMap {
    id: i64,
    name: String,
}

#[test]
fn cross_skip_trailing_map_field() {
    let mut meta = std::collections::HashMap::new();
    meta.insert("age".into(), 30);
    meta.insert("score".into(), 95);
    let src = SrcWithMap {
        id: 1,
        name: "Alice".into(),
        meta,
    };
    let data = encode(&src).unwrap();
    let dst: DstNoMap = decode(&data).unwrap();
    assert_eq!(
        dst,
        DstNoMap {
            id: 1,
            name: "Alice".into()
        }
    );
}

// ============================================================================
// Dimension 17: Vec decode with typed schema
// ============================================================================

#[test]
fn cross_typed_schema_vec_decode() {
    let src = vec![FullUser {
        id: 1,
        name: "Alice".into(),
        age: 30,
        active: true,
        score: 95.5,
    }];
    let data = encode_typed(&src).unwrap();
    let dst: Vec<MiniUser> = decode(&data).unwrap();
    assert_eq!(dst.len(), 1);
    assert_eq!(
        dst[0],
        MiniUser {
            id: 1,
            name: "Alice".into()
        }
    );
}

// ============================================================================
// Dimension 18: Single struct typed schema
// ============================================================================

#[test]
fn cross_typed_schema_single_decode() {
    let src = FullUser {
        id: 42,
        name: "Bob".into(),
        age: 25,
        active: false,
        score: 88.0,
    };
    let data = encode_typed(&src).unwrap();
    let dst: MiniUser = decode(&data).unwrap();
    assert_eq!(
        dst,
        MiniUser {
            id: 42,
            name: "Bob".into()
        }
    );
}

// ============================================================================
// Dimension 19: Nested vec-of-struct + trailing outer fields
// ============================================================================

#[derive(Debug, Serialize)]
struct DetailFull {
    #[serde(rename = "ID")]
    id: i64,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Age")]
    age: i32,
    #[serde(rename = "Gender")]
    gender: bool,
}

#[derive(Debug, Serialize)]
struct UserFull {
    details: Vec<DetailFull>,
    code: i64,
    label: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct PersonThin {
    #[serde(rename = "ID")]
    id: i64,
    #[serde(rename = "Name")]
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct HumanThin {
    details: Vec<PersonThin>,
}

#[test]
fn cross_nested_vec_plus_trailing_outer() {
    let src = vec![UserFull {
        details: vec![
            DetailFull {
                id: 1,
                name: "Alice".into(),
                age: 30,
                gender: true,
            },
            DetailFull {
                id: 2,
                name: "Bob".into(),
                age: 25,
                gender: false,
            },
        ],
        code: 42,
        label: "test".into(),
    }];
    let data = encode(&src).unwrap();
    let dst: Vec<HumanThin> = decode(&data).unwrap();
    assert_eq!(dst.len(), 1);
    assert_eq!(dst[0].details.len(), 2);
    assert_eq!(
        dst[0].details[0],
        PersonThin {
            id: 1,
            name: "Alice".into()
        }
    );
    assert_eq!(
        dst[0].details[1],
        PersonThin {
            id: 2,
            name: "Bob".into()
        }
    );
}

// ============================================================================
// Dimension 20: Skip trailing bools
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcBools {
    id: i64,
    a: bool,
    b: bool,
    c: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstBoolsThin {
    id: i64,
}

#[test]
fn cross_skip_trailing_bools() {
    let src = vec![
        SrcBools {
            id: 1,
            a: true,
            b: false,
            c: true,
        },
        SrcBools {
            id: 2,
            a: false,
            b: true,
            c: false,
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<DstBoolsThin> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0].id, 1);
    assert_eq!(dst[1].id, 2);
}

// ============================================================================
// Dimension 21: Pick middle field only
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcFiveFields {
    a: i64,
    b: String,
    c: f64,
    d: bool,
    e: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstMiddleOnly {
    c: f64,
}

#[test]
fn cross_pick_middle_field_only() {
    let src = SrcFiveFields {
        a: 1,
        b: "hi".into(),
        c: 3.14,
        d: true,
        e: 99,
    };
    let data = encode(&src).unwrap();
    let dst: DstMiddleOnly = decode(&data).unwrap();
    assert_eq!(dst.c, 3.14);
}

// ============================================================================
// Dimension 22: Pick last field only
// ============================================================================

#[derive(Debug, Deserialize, PartialEq)]
struct DstLastOnly {
    e: i64,
}

#[test]
fn cross_pick_last_field_only() {
    let src = SrcFiveFields {
        a: 1,
        b: "hi".into(),
        c: 3.14,
        d: true,
        e: 42,
    };
    let data = encode(&src).unwrap();
    let dst: DstLastOnly = decode(&data).unwrap();
    assert_eq!(dst.e, 42);
}

// ============================================================================
// Dimension 23: No overlapping fields
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcAlpha {
    x: i64,
    y: String,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(default)]
struct DstBeta {
    p: i64,
    q: String,
}

#[test]
fn cross_no_overlapping_fields() {
    let src = SrcAlpha {
        x: 1,
        y: "hello".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstBeta = decode(&data).unwrap();
    assert_eq!(dst.p, 0);
    assert_eq!(dst.q, "");
}

// ============================================================================
// Dimension 24: Nested array of structs with extra fields
// ============================================================================

#[derive(Debug, Serialize)]
struct WorkerFull {
    name: String,
    skills: Vec<String>,
    years_xp: i64,
    rating: f64,
}

#[derive(Debug, Serialize)]
struct TeamFull2 {
    lead: String,
    workers: Vec<WorkerFull>,
    budget: f64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct WorkerThin {
    name: String,
    skills: Vec<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct TeamThin2 {
    lead: String,
    workers: Vec<WorkerThin>,
}

#[test]
fn cross_nested_array_of_structs_extra_fields() {
    let src = TeamFull2 {
        lead: "Alice".into(),
        workers: vec![
            WorkerFull {
                name: "Bob".into(),
                skills: vec!["go".into(), "rust".into()],
                years_xp: 5,
                rating: 4.5,
            },
            WorkerFull {
                name: "Carol".into(),
                skills: vec!["python".into()],
                years_xp: 3,
                rating: 3.8,
            },
        ],
        budget: 100000.0,
    };
    let data = encode(&src).unwrap();
    let dst: TeamThin2 = decode(&data).unwrap();
    assert_eq!(dst.lead, "Alice");
    assert_eq!(dst.workers.len(), 2);
    assert_eq!(dst.workers[0].name, "Bob");
    assert_eq!(dst.workers[0].skills, vec!["go", "rust"]);
    assert_eq!(dst.workers[1].name, "Carol");
    assert_eq!(dst.workers[1].skills, vec!["python"]);
}

// ============================================================================
// Dimension 25: Typed schema with mixed match/missing fields
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcTyped {
    a: i64,
    b: String,
    c: f64,
    d: bool,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(default)]
struct DstMixed {
    b: String,
    d: bool,
    extra: i64,
    more: f64,
}

#[test]
fn cross_typed_schema_mixed_fields() {
    let src = SrcTyped {
        a: 1,
        b: "test".into(),
        c: 2.5,
        d: true,
    };
    let data = encode_typed(&src).unwrap();
    let dst: DstMixed = decode(&data).unwrap();
    assert_eq!(dst.b, "test");
    assert_eq!(dst.d, true);
    assert_eq!(dst.extra, 0);
    assert_eq!(dst.more, 0.0);
}

// ============================================================================
// Dimension 26: Many trailing fields (10→1)
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcWide {
    f1: i64,
    f2: String,
    f3: bool,
    f4: i64,
    f5: String,
    f6: bool,
    f7: i64,
    f8: String,
    f9: bool,
    f10: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstNarrow {
    f1: i64,
}

#[test]
fn cross_many_trailing_fields() {
    let src = SrcWide {
        f1: 42,
        f2: "a".into(),
        f3: true,
        f4: 4,
        f5: "b".into(),
        f6: false,
        f7: 7,
        f8: "c".into(),
        f9: true,
        f10: 10,
    };
    let data = encode(&src).unwrap();
    let dst: DstNarrow = decode(&data).unwrap();
    assert_eq!(dst.f1, 42);
}

// ============================================================================
// Dimension 27: Vec single row
// ============================================================================

#[test]
fn cross_vec_single_row() {
    let src = vec![FullUser {
        id: 1,
        name: "Alice".into(),
        age: 30,
        active: true,
        score: 95.5,
    }];
    let data = encode(&src).unwrap();
    let dst: Vec<MiniUser> = decode(&data).unwrap();
    assert_eq!(dst.len(), 1);
    assert_eq!(
        dst[0],
        MiniUser {
            id: 1,
            name: "Alice".into()
        }
    );
}

// ============================================================================
// Dimension 28: ASON-like syntax in strings
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcAsonLike {
    id: i64,
    data: String,
    code: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstAsonLikeThin {
    id: i64,
}

#[test]
fn cross_skip_string_containing_ason_syntax() {
    let src = SrcAsonLike {
        id: 1,
        data: "{a,b}:(1,2)".into(),
        code: "[(x,y),(z,w)]".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstAsonLikeThin = decode(&data).unwrap();
    assert_eq!(dst.id, 1);
}

// ============================================================================
// Dimension 29: Unicode in trailing
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcUnicode {
    id: i64,
    name: String,
    bio: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstUnicodeThin {
    id: i64,
}

#[test]
fn cross_skip_unicode_in_trailing() {
    let src = SrcUnicode {
        id: 1,
        name: "日本語テスト".into(),
        bio: "中文描述，包含逗号".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstUnicodeThin = decode(&data).unwrap();
    assert_eq!(dst.id, 1);
}

// ============================================================================
// Dimension 30: Roundtrip A→B→A
// ============================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
struct VersionA {
    id: i64,
    name: String,
    active: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct VersionB {
    id: i64,
    name: String,
}

#[test]
fn cross_roundtrip_abba() {
    // A→B
    let src_a = VersionA {
        id: 1,
        name: "test".into(),
        active: true,
    };
    let data_a = encode(&src_a).unwrap();
    let dst_b: VersionB = decode(&data_a).unwrap();
    assert_eq!(
        dst_b,
        VersionB {
            id: 1,
            name: "test".into()
        }
    );

    // B→A (missing active = false)
    let data_b = encode(&dst_b).unwrap();
    let dst_a: VersionA = decode(&data_b).unwrap();
    assert_eq!(
        dst_a,
        VersionA {
            id: 1,
            name: "test".into(),
            active: false
        }
    );
}

// ============================================================================
// Dimension 31: Empty arrays in middle field
// ============================================================================

#[derive(Debug, Serialize)]
struct SrcWithArr {
    id: i64,
    items: Vec<String>,
    score: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstWithArrThin {
    id: i64,
    items: Vec<String>,
}

#[test]
fn cross_empty_array_in_middle_field() {
    let src = vec![
        SrcWithArr {
            id: 1,
            items: vec![],
            score: 10,
        },
        SrcWithArr {
            id: 2,
            items: vec!["a".into(), "b".into()],
            score: 20,
        },
    ];
    let data = encode(&src).unwrap();
    let dst: Vec<DstWithArrThin> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(
        dst[0],
        DstWithArrThin {
            id: 1,
            items: vec![]
        }
    );
    assert_eq!(
        dst[1],
        DstWithArrThin {
            id: 2,
            items: vec!["a".into(), "b".into()]
        }
    );
}

// ============================================================================
// Dimension 32: Skip nested struct as tuple
// ============================================================================

#[derive(Debug, Serialize)]
struct InnerForSkip {
    a: i64,
    b: String,
}

#[derive(Debug, Serialize)]
struct SrcWithNested {
    id: i64,
    inner: InnerForSkip,
    tail: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct DstFlat {
    id: i64,
}

#[test]
fn cross_skip_nested_struct_as_tuple() {
    let src = SrcWithNested {
        id: 1,
        inner: InnerForSkip {
            a: 10,
            b: "nested".into(),
        },
        tail: "end".into(),
    };
    let data = encode(&src).unwrap();
    let dst: DstFlat = decode(&data).unwrap();
    assert_eq!(dst.id, 1);
}

// ============================================================================
// Dimension 33: Stress test — 100 rows
// ============================================================================

#[test]
fn cross_many_rows_stress() {
    let src: Vec<FullUser> = (0..100)
        .map(|i| FullUser {
            id: i,
            name: "user".into(),
            age: i as i32,
            active: i % 2 == 0,
            score: i as f64 * 0.1,
        })
        .collect();
    let data = encode(&src).unwrap();
    let dst: Vec<MiniUser> = decode(&data).unwrap();
    assert_eq!(dst.len(), 100);
    for (i, d) in dst.iter().enumerate() {
        assert_eq!(d.id, i as i64);
        assert_eq!(d.name, "user");
    }
}

// ============================================================================
// Dimension 34: Typed encode, target subset + reorder
// ============================================================================

#[test]
fn cross_typed_encode_subset_reorder() {
    let src = vec![
        BigRecord {
            id: 1,
            name: "A".into(),
            score: 9.5,
            active: true,
            level: 3,
        },
        BigRecord {
            id: 2,
            name: "B".into(),
            score: 8.0,
            active: false,
            level: 1,
        },
    ];
    let data = encode_typed(&src).unwrap();
    let dst: Vec<SmallReordered> = decode(&data).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0], SmallReordered { score: 9.5, id: 1 });
    assert_eq!(dst[1], SmallReordered { score: 8.0, id: 2 });
}

// ============================================================================
// Dimension 35: Zero-value source fields
// ============================================================================

#[test]
fn cross_zero_value_source_fields() {
    let src = FullUser {
        id: 0,
        name: "".into(),
        age: 0,
        active: false,
        score: 0.0,
    };
    let data = encode(&src).unwrap();
    let dst: MiniUser = decode(&data).unwrap();
    assert_eq!(
        dst,
        MiniUser {
            id: 0,
            name: "".into()
        }
    );
}
