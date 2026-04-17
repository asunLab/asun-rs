use asun::{decode, decode_binary, encode, encode_binary, encode_typed};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

fn main() {
    println!("=== ASUN Basic Examples ===\n");

    // 1. Encode a single struct
    let user = User {
        id: 1,
        name: "Alice".into(),
        active: true,
    };
    let asun_str = encode(&user).unwrap();
    println!("Encode single struct:");
    println!("  {}\n", asun_str);

    // 2. Encode with type annotations (encode_typed)
    let typed_str = encode_typed(&user).unwrap();
    println!("Encode with type annotations:");
    println!("  {}\n", typed_str);
    assert!(typed_str.starts_with("{id@int,name@str,active@bool}:"));

    // 3. Decode from ASUN (accepts both annotated and unannotated)
    let input = "{id@int,name@str,active@bool}:(1,Alice,true)";
    let user: User = decode(input).unwrap();
    println!("Decode single struct:");
    println!("  {:?}\n", user);

    // 4. Encode a vec of structs (schema-driven)
    let users = vec![
        User {
            id: 1,
            name: "Alice".into(),
            active: true,
        },
        User {
            id: 2,
            name: "Bob".into(),
            active: false,
        },
        User {
            id: 3,
            name: "Carol Smith".into(),
            active: true,
        },
    ];
    let asun_vec = encode(&users).unwrap();
    println!("Encode vec (schema-driven):");
    println!("  {}\n", asun_vec);

    // 5. Encode vec with type annotations (encode_typed)
    let typed_vec = encode_typed(&users).unwrap();
    println!("Encode vec with type annotations:");
    println!("  {}\n", typed_vec);
    assert!(typed_vec.starts_with("[{id@int,name@str,active@bool}]:"));

    // 6. Decode vec
    let input =
        "[{id@int,name@str,active@bool}]:(1,Alice,true),(2,Bob,false),(3,\"Carol Smith\",true)";
    let users: Vec<User> = decode(input).unwrap();
    println!("Decode vec:");
    for u in &users {
        println!("  {:?}", u);
    }

    // 7. Multiline format
    println!("\nMultiline format:");
    let multiline = "[{id@int, name@str, active@bool}]:
  (1, Alice, true),
  (2, Bob, false),
  (3, \"Carol Smith\", true)";
    let users: Vec<User> = decode(multiline).unwrap();
    for u in &users {
        println!("  {:?}", u);
    }

    // 8. Roundtrip (ASUN-text + ASUN-bin + JSON)
    println!("\n8. Roundtrip (ASUN-text vs ASUN-bin vs JSON):");
    let original = User {
        id: 42,
        name: "Test User".into(),
        active: true,
    };
    // ASUN text
    let asun_str = encode(&original).unwrap();
    let from_asun: User = decode(&asun_str).unwrap();
    assert_eq!(original, from_asun);
    // ASUN binary
    let asun_bin = encode_binary(&original).unwrap();
    let decode_binary_val: User = decode_binary(&asun_bin).unwrap();
    assert_eq!(original, decode_binary_val);
    // JSON
    let json_str = serde_json::to_string(&original).unwrap();
    let from_json: User = serde_json::from_str(&json_str).unwrap();
    assert_eq!(original, from_json);
    println!("  original:     {:?}", original);
    println!("  ASUN text:    {} ({} B)", asun_str, asun_str.len());
    println!("  ASUN binary:  {} B", asun_bin.len());
    println!("  JSON:         {} ({} B)", json_str, json_str.len());
    println!("  ✓ all 3 formats roundtrip OK");

    // 9. Vec roundtrip (ASUN-text + ASUN-bin + JSON)
    println!("\n9. Vec roundtrip (ASUN-text vs ASUN-bin vs JSON):");
    let vec_asun = encode(&users).unwrap();
    let vec_bin = encode_binary(&users).unwrap();
    let vec_json = serde_json::to_string(&users).unwrap();
    let v1: Vec<User> = decode(&vec_asun).unwrap();
    let v2: Vec<User> = decode_binary(&vec_bin).unwrap();
    let v3: Vec<User> = serde_json::from_str(&vec_json).unwrap();
    assert_eq!(users, v1);
    assert_eq!(users, v2);
    assert_eq!(users, v3);
    println!("  ASUN text:   {} B", vec_asun.len());
    println!("  ASUN binary: {} B", vec_bin.len());
    println!("  JSON:        {} B", vec_json.len());
    println!(
        "  BIN vs JSON: {:.0}% smaller",
        (1.0 - vec_bin.len() as f64 / vec_json.len() as f64) * 100.0
    );
    println!("  ✓ vec roundtrip OK (all 3 formats)");

    // 10. Optional fields
    println!("\n10. Optional fields:");
    #[derive(Debug, Deserialize)]
    struct Item {
        id: i64,
        label: Option<String>,
    }
    let input = "{id,label}:(1,hello)";
    let item: Item = decode(input).unwrap();
    println!("  with value: {:?}", item);

    let input = "{id,label}:(2,)";
    let item: Item = decode(input).unwrap();
    println!("  with null:  {:?}", item);

    // 11. Array fields
    println!("\n11. Array fields:");
    #[derive(Debug, Deserialize)]
    struct Tagged {
        name: String,
        tags: Vec<String>,
    }
    let input = "{name,tags@[]}:(Alice,[rust,go,python])";
    let t: Tagged = decode(input).unwrap();
    println!("  {:?}", t);

    // 12. Comments
    println!("\n12. With comments:");
    let input = "/* user list */ {id,name,active}:(1,Alice,true)";
    let user: User = decode(input).unwrap();
    println!("  {:?}", user);

    println!("\n=== All examples passed! ===");
}
