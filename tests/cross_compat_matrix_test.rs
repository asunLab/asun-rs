use asun::decode;
use serde::Deserialize;

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixPerson {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixPersonWithActive {
    id: i64,
    name: String,
    active: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixInnerThin {
    x: i64,
    y: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixOuterThin {
    name: String,
    inner: MatrixInnerThin,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixTaskThin {
    title: String,
    done: bool,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixProjectThin {
    name: String,
    tasks: Vec<MatrixTaskThin>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixDstFewerOptionals {
    id: i64,
    label: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixL3Thin {
    a: i64,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixL2Thin {
    name: String,
    sub: MatrixL3Thin,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixL1Thin {
    id: i64,
    child: MatrixL2Thin,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixPersonScore {
    id: i64,
    score: f64,
}

#[derive(Debug, Deserialize, PartialEq, Default)]
#[serde(default)]
struct MatrixNoOverlap {
    foo: i64,
    bar: String,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixNestedOptionalThin {
    name: String,
    nick: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct MatrixUserWithNestedOptional {
    id: i64,
    profile: MatrixNestedOptionalThin,
}

#[test]
fn matrix_a2_typed_single_extra_field_dropped() {
    let input = "{id@int,name@str,active@bool}:(42,Alice,true)";
    let dst: MatrixPerson = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPerson {
            id: 42,
            name: "Alice".into(),
        }
    );
}

#[test]
fn matrix_a1_typed_single_exact_match() {
    let input = "{id@int,name@str}:(42,Alice)";
    let dst: MatrixPerson = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPerson {
            id: 42,
            name: "Alice".into(),
        }
    );
}

#[test]
fn matrix_a1_untyped_single_exact_match() {
    let input = "{id,name}:(42,Alice)";
    let dst: MatrixPerson = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPerson {
            id: 42,
            name: "Alice".into(),
        }
    );
}

#[test]
fn matrix_a2_untyped_single_extra_field_dropped() {
    let input = "{id,name,active}:(42,Alice,true)";
    let dst: MatrixPerson = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPerson {
            id: 42,
            name: "Alice".into(),
        }
    );
}

#[test]
fn matrix_a3_typed_single_target_extra_field_defaulted() {
    let input = "{id@int,name@str}:(42,Alice)";
    let dst: MatrixPersonWithActive = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPersonWithActive {
            id: 42,
            name: "Alice".into(),
            active: false,
        }
    );
}

#[test]
fn matrix_a3_untyped_single_target_extra_field_defaulted() {
    let input = "{id,name}:(42,Alice)";
    let dst: MatrixPersonWithActive = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPersonWithActive {
            id: 42,
            name: "Alice".into(),
            active: false,
        }
    );
}

#[test]
fn matrix_a4_typed_single_field_reorder() {
    let input = "{active@bool,id@int,name@str}:(true,42,Alice)";
    let dst: MatrixPersonWithActive = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPersonWithActive {
            id: 42,
            name: "Alice".into(),
            active: true,
        }
    );
}

#[test]
fn matrix_a4_untyped_single_field_reorder() {
    let input = "{active,id,name}:(true,42,Alice)";
    let dst: MatrixPersonWithActive = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixPersonWithActive {
            id: 42,
            name: "Alice".into(),
            active: true,
        }
    );
}

#[test]
fn matrix_a5_typed_vec_extra_field_dropped() {
    let input = "[{id@int,name@str,active@bool}]:(42,Alice,true),(7,Bob,false)";
    let dst: Vec<MatrixPerson> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixPerson {
                id: 42,
                name: "Alice".into(),
            },
            MatrixPerson {
                id: 7,
                name: "Bob".into(),
            },
        ]
    );
}

#[test]
fn matrix_a5_untyped_vec_extra_field_dropped() {
    let input = "[{id,name,active}]:(42,Alice,true),(7,Bob,false)";
    let dst: Vec<MatrixPerson> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixPerson {
                id: 42,
                name: "Alice".into(),
            },
            MatrixPerson {
                id: 7,
                name: "Bob".into(),
            },
        ]
    );
}

#[test]
fn matrix_n1_typed_nested_extra_fields_dropped() {
    let input =
        "{name@str,inner@{x@int,y@int,z@float,w@bool},flag@bool}:(test,(10,20,3.14,true),true)";
    let dst: MatrixOuterThin = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixOuterThin {
            name: "test".into(),
            inner: MatrixInnerThin { x: 10, y: 20 },
        }
    );
}

#[test]
fn matrix_n1_untyped_nested_extra_fields_dropped() {
    let input = "{name,inner@{x,y,z,w},flag}:(test,(10,20,3.14,true),true)";
    let dst: MatrixOuterThin = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixOuterThin {
            name: "test".into(),
            inner: MatrixInnerThin { x: 10, y: 20 },
        }
    );
}

#[test]
fn matrix_n2_typed_nested_vec_extra_fields_dropped() {
    let input = "[{name@str,tasks@[{title@str,done@bool,priority@int,weight@float}]}]:(Alpha,[(Design,true,1,0.5),(Code,false,2,0.8)]),(Beta,[(Test,false,3,1.0)])";
    let dst: Vec<MatrixProjectThin> = decode(input).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0].name, "Alpha");
    assert_eq!(dst[0].tasks.len(), 2);
    assert_eq!(
        dst[0].tasks[0],
        MatrixTaskThin {
            title: "Design".into(),
            done: true,
        }
    );
    assert_eq!(dst[1].name, "Beta");
    assert_eq!(dst[1].tasks.len(), 1);
}

#[test]
fn matrix_n2_untyped_nested_vec_extra_fields_dropped() {
    let input = "[{name,tasks@[{title,done,priority,weight}]}]:(Alpha,[(Design,true,1,0.5),(Code,false,2,0.8)]),(Beta,[(Test,false,3,1.0)])";
    let dst: Vec<MatrixProjectThin> = decode(input).unwrap();
    assert_eq!(dst.len(), 2);
    assert_eq!(dst[0].tasks[1].title, "Code");
    assert!(!dst[0].tasks[1].done);
}

#[test]
fn matrix_o1_typed_optional_skip_trailing() {
    let input = "[{id@int,label@str?,score@float?,flag@bool}]:(1,hello,95.5,true),(2,,,false)";
    let dst: Vec<MatrixDstFewerOptionals> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixDstFewerOptionals {
                id: 1,
                label: Some("hello".into()),
            },
            MatrixDstFewerOptionals { id: 2, label: None },
        ]
    );
}

#[test]
fn matrix_a6_typed_vec_target_extra_field_defaulted() {
    let input = "[{id@int,name@str}]:(42,Alice),(7,Bob)";
    let dst: Vec<MatrixPersonWithActive> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixPersonWithActive {
                id: 42,
                name: "Alice".into(),
                active: false,
            },
            MatrixPersonWithActive {
                id: 7,
                name: "Bob".into(),
                active: false,
            },
        ]
    );
}

#[test]
fn matrix_a6_untyped_vec_target_extra_field_defaulted() {
    let input = "[{id,name}]:(42,Alice),(7,Bob)";
    let dst: Vec<MatrixPersonWithActive> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixPersonWithActive {
                id: 42,
                name: "Alice".into(),
                active: false,
            },
            MatrixPersonWithActive {
                id: 7,
                name: "Bob".into(),
                active: false,
            },
        ]
    );
}

#[test]
fn matrix_n3_typed_deep_nested_extra_fields_dropped() {
    let input = "{id@int,child@{name@str,sub@{a@int,b@str,c@bool},code@int,tags@[str]},extra@str}:(7,(leaf,(11,hello,true),99,[x,y]),tail)";
    let dst: MatrixL1Thin = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixL1Thin {
            id: 7,
            child: MatrixL2Thin {
                name: "leaf".into(),
                sub: MatrixL3Thin { a: 11 },
            },
        }
    );
}

#[test]
fn matrix_n3_untyped_deep_nested_extra_fields_dropped() {
    let input =
        "{id,child@{name,sub@{a,b,c},code,tags},extra}:(7,(leaf,(11,hello,true),99,[x,y]),tail)";
    let dst: MatrixL1Thin = decode(input).unwrap();
    assert_eq!(
        dst,
        MatrixL1Thin {
            id: 7,
            child: MatrixL2Thin {
                name: "leaf".into(),
                sub: MatrixL3Thin { a: 11 },
            },
        }
    );
}

#[test]
fn matrix_o1_untyped_optional_skip_trailing() {
    let input = "[{id,label,score,flag}]:(1,hello,95.5,true),(2,,,false)";
    let dst: Vec<MatrixDstFewerOptionals> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixDstFewerOptionals {
                id: 1,
                label: Some("hello".into()),
            },
            MatrixDstFewerOptionals { id: 2, label: None },
        ]
    );
}

#[test]
fn matrix_p1_typed_partial_overlap() {
    let input = "{id@int,name@str,score@float,active@bool}:(42,Alice,9.5,true)";
    let dst: MatrixPersonScore = decode(input).unwrap();
    assert_eq!(dst, MatrixPersonScore { id: 42, score: 9.5 });
}

#[test]
fn matrix_p1_untyped_partial_overlap() {
    let input = "{id,name,score,active}:(42,Alice,9.5,true)";
    let dst: MatrixPersonScore = decode(input).unwrap();
    assert_eq!(dst, MatrixPersonScore { id: 42, score: 9.5 });
}

#[test]
fn matrix_p2_typed_no_overlap_defaults() {
    let input = "{id@int,name@str}:(42,Alice)";
    let dst: MatrixNoOverlap = decode(input).unwrap();
    assert_eq!(dst, MatrixNoOverlap::default());
}

#[test]
fn matrix_p2_untyped_no_overlap_defaults() {
    let input = "{id,name}:(42,Alice)";
    let dst: MatrixNoOverlap = decode(input).unwrap();
    assert_eq!(dst, MatrixNoOverlap::default());
}

#[test]
fn matrix_n4_typed_nested_optional_subset() {
    let input = "[{id@int,profile@{name@str,nick@str?,score@float?},active@bool}]:(1,(Alice,ally,9.5),true),(2,(Bob,,),false)";
    let dst: Vec<MatrixUserWithNestedOptional> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixUserWithNestedOptional {
                id: 1,
                profile: MatrixNestedOptionalThin {
                    name: "Alice".into(),
                    nick: Some("ally".into()),
                },
            },
            MatrixUserWithNestedOptional {
                id: 2,
                profile: MatrixNestedOptionalThin {
                    name: "Bob".into(),
                    nick: None,
                },
            },
        ]
    );
}

#[test]
fn matrix_n4_untyped_nested_optional_subset() {
    let input =
        "[{id,profile@{name,nick,score},active}]:(1,(Alice,ally,9.5),true),(2,(Bob,,),false)";
    let dst: Vec<MatrixUserWithNestedOptional> = decode(input).unwrap();
    assert_eq!(
        dst,
        vec![
            MatrixUserWithNestedOptional {
                id: 1,
                profile: MatrixNestedOptionalThin {
                    name: "Alice".into(),
                    nick: Some("ally".into()),
                },
            },
            MatrixUserWithNestedOptional {
                id: 2,
                profile: MatrixNestedOptionalThin {
                    name: "Bob".into(),
                    nick: None,
                },
            },
        ]
    );
}
