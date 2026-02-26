#[cfg(test)]
mod format_validation {
    use ason::{decode, encode_pretty, encode_pretty_typed};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct FmtUser {
        id: i64,
        name: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Score {
        id: i64,
        value: f64,
        label: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Inner {
        x: i64,
        label: String,
    }

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Outer {
        id: i64,
        inner: Inner,
    }

    const BAD_FMT: &str = "{id:int, name:str}:\n  (1, Alice),\n  (2, Bob),\n  (3, Carol)";
    const GOOD_FMT: &str = "[{id:int, name:str}]:\n  (1, Alice),\n  (2, Bob),\n  (3, Carol)";

    // -------------------------------------------------------------------------
    // Invalid format tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_bad_format_as_vec() {
        let result = decode::<Vec<FmtUser>>(BAD_FMT);
        assert!(result.is_err(), "should reject {{schema}}: format for Vec");
    }

    #[test]
    fn test_bad_format_as_single_trailing_rows() {
        let result = decode::<FmtUser>(BAD_FMT);
        assert!(result.is_err(), "should reject trailing rows");
    }

    #[test]
    fn test_bad_format_extra_tuple() {
        let bad = "{id:int,name:str}:(10,Dave),(11,Eve)";
        let result = decode::<FmtUser>(bad);
        assert!(result.is_err(), "should reject trailing tuple after single struct");
    }

    #[test]
    fn test_bad_format_many_rows_no_bracket() {
        let bad = "{id,name}:(1,A),(2,B),(3,C),(4,D),(5,E)";
        let result = decode::<Vec<FmtUser>>(bad);
        assert!(result.is_err(), "should reject {{schema}}: without [] for Vec");
    }

    #[test]
    fn test_good_format_as_vec() {
        let result = decode::<Vec<FmtUser>>(GOOD_FMT);
        assert!(result.is_ok(), "should accept [{{}}]: format for Vec");
        let users = result.unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(users[0].name, "Alice");
        assert_eq!(users[2].name, "Carol");
    }

    #[test]
    fn test_good_format_untyped() {
        let good = "[{id,name}]:\n  (1,Alice),\n  (2,Bob),\n  (3,Carol)";
        let result = decode::<Vec<FmtUser>>(good);
        assert!(result.is_ok(), "should accept untyped [{{}}]: format for Vec");
        let users = result.unwrap();
        assert_eq!(users.len(), 3);
        assert_eq!(users[1].name, "Bob");
    }

    // -------------------------------------------------------------------------
    // encodePretty -> decode roundtrip tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_pretty_simple_roundtrip() {
        let u = FmtUser { id: 42, name: "Alice".into() };
        let pretty = encode_pretty(&u).unwrap();
        let u2: FmtUser = decode(&pretty).unwrap();
        assert_eq!(u, u2);
    }

    #[test]
    fn test_pretty_typed_roundtrip() {
        let u = FmtUser { id: 7, name: "Bob".into() };
        let pretty = encode_pretty_typed(&u).unwrap();
        assert!(pretty.contains(":int") || pretty.contains(":str"));
        let u2: FmtUser = decode(&pretty).unwrap();
        assert_eq!(u, u2);
    }

    #[test]
    fn test_pretty_vec_roundtrip() {
        let users = vec![
            FmtUser { id: 1, name: "Alice".into() },
            FmtUser { id: 2, name: "Bob".into() },
            FmtUser { id: 3, name: "Carol".into() },
        ];
        let pretty = encode_pretty(&users).unwrap();
        assert!(pretty.contains('\n'), "expected multi-line output");
        let users2: Vec<FmtUser> = decode(&pretty).unwrap();
        assert_eq!(users, users2);
    }

    #[test]
    fn test_pretty_score_slice_roundtrip() {
        let scores = vec![
            Score { id: 1, value: 95.5, label: "excellent".into() },
            Score { id: 2, value: 72.3, label: "good".into() },
            Score { id: 3, value: 40.0, label: "fail".into() },
        ];
        let pretty = encode_pretty(&scores).unwrap();
        let scores2: Vec<Score> = decode(&pretty).unwrap();
        assert_eq!(scores, scores2);
        assert_eq!(scores2[0].label, "excellent");
        assert!((scores2[0].value - 95.5).abs() < 1e-9);
    }

    #[test]
    fn test_pretty_nested_roundtrip() {
        let o = Outer { id: 5, inner: Inner { x: 10, label: "test".into() } };
        let pretty = encode_pretty(&o).unwrap();
        let o2: Outer = decode(&pretty).unwrap();
        assert_eq!(o, o2);
    }

    #[test]
    fn test_pretty_large_vec_roundtrip() {
        let users: Vec<FmtUser> = (1..=50)
            .map(|i| FmtUser { id: i, name: format!("user{}", i) })
            .collect();
        let pretty = encode_pretty(&users).unwrap();
        let users2: Vec<FmtUser> = decode(&pretty).unwrap();
        assert_eq!(users, users2);
        assert_eq!(users2.last().unwrap().id, 50);
    }

    #[test]
    fn test_pretty_typed_vec_roundtrip() {
        let users = vec![
            FmtUser { id: 1, name: "Alice".into() },
            FmtUser { id: 2, name: "Bob".into() },
        ];
        let pretty = encode_pretty_typed(&users).unwrap();
        assert!(pretty.contains(":int") || pretty.contains(":str"));
        let users2: Vec<FmtUser> = decode(&pretty).unwrap();
        assert_eq!(users, users2);
    }
}
