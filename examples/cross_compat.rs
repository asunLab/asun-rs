use asun::{AsunDecode, AsunEncode};

#[derive(AsunEncode, AsunDecode, Debug, PartialEq)]
struct Detail {
    #[asun(rename = "ID")]
    id: i64,
    #[asun(rename = "Name")]
    name: String,
    #[asun(rename = "Age")]
    age: i32,
    #[asun(rename = "Gender")]
    gender: bool,
}

#[derive(AsunEncode, AsunDecode, Debug, PartialEq)]
struct User {
    details: Vec<Detail>,
}

#[derive(AsunEncode, AsunDecode, Debug, PartialEq)]
struct Person {
    #[asun(rename = "ID")]
    id: i64,
    #[asun(rename = "Name")]
    name: String,
    #[asun(rename = "Age")]
    age: i32,
}

#[derive(AsunEncode, AsunDecode, Debug, PartialEq)]
struct Human {
    details: Vec<Person>,
}

fn main() {
    let users = vec![User {
        details: vec![
            Detail {
                id: 1,
                name: "Alice".to_string(),
                age: 30,
                gender: true,
            },
            Detail {
                id: 2,
                name: "Bob".to_string(),
                age: 25,
                gender: false,
            },
        ],
    }];

    // Encode
    let asun_str = asun::encode(&users).unwrap();
    println!("Encoded ASUN:\n{}", asun_str);

    // Decode into Human
    let decoded: Vec<Human> = asun::decode(&asun_str).unwrap();
    println!("\nDecoded into Human list:\n{:?}", decoded);
}
