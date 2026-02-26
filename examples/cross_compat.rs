use ason;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Detail {
    #[serde(rename = "ID")]
    id: i64,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Age")]
    age: i32,
    #[serde(rename = "Gender")]
    gender: bool,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct User {
    details: Vec<Detail>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct Person {
    #[serde(rename = "ID")]
    id: i64,
    #[serde(rename = "Name")]
    name: String,
    #[serde(rename = "Age")]
    age: i32,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
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
    let ason_str = ason::encode(&users).unwrap();
    println!("Encoded ASON:\n{}", ason_str);

    // Decode into Human
    let decoded: Vec<Human> = ason::decode(&ason_str).unwrap();
    println!("\nDecoded into Human list:\n{:?}", decoded);
}
