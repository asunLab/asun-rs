// Tight-loop profiling target. Designed for `perf record`:
//
//   cargo build --profile=profiling --example profile_target
//   perf record -F 1999 -g --call-graph=dwarf -- \
//       /home/X/.cache/rust/profiling/examples/profile_target encode_alltypes
//   perf report --stdio --sort=overhead,sym --no-children | head -50
//
// Subcommands:
//   encode_user    — flat 8-field struct vec (Section 1 in bench.rs)
//   encode_alltypes — 16-field primitive struct vec (Section 2 in bench.rs)
//   encode_deep    — 5-level deep nested struct (Section 3 in bench.rs)
//   decode_user    — same payload, decode loop
//   decode_alltypes — same payload, decode loop
//
// Each subcommand runs a single tight loop with no allocation outside the
// hot path so perf samples are concentrated on encode/decode internals.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct User {
    id: i64,
    name: String,
    email: String,
    age: i64,
    score: f64,
    active: bool,
    role: String,
    city: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AllTypes {
    b: bool,
    i8v: i8,
    i16v: i16,
    i32v: i32,
    i64v: i64,
    u8v: u8,
    u16v: u16,
    u32v: u32,
    u64v: u64,
    f32v: f32,
    f64v: f64,
    s: String,
    opt_some: Option<i64>,
    opt_none: Option<i64>,
    vec_int: Vec<i64>,
    vec_str: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Task {
    id: i64,
    title: String,
    done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Project {
    id: i64,
    name: String,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Team {
    id: i64,
    name: String,
    projects: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Division {
    id: i64,
    name: String,
    teams: Vec<Team>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Company {
    id: i64,
    name: String,
    divisions: Vec<Division>,
}

fn generate_users(n: usize) -> Vec<User> {
    let names = ["Alice", "Bob", "Charlie", "Diana", "Eve"];
    let roles = ["engineer", "designer", "manager", "analyst"];
    let cities = ["NYC", "LA", "Chicago", "Houston", "Phoenix"];
    (0..n)
        .map(|i| User {
            id: i as i64,
            name: names[i % names.len()].into(),
            email: format!("{}@example.com", names[i % names.len()].to_lowercase()),
            age: 25 + (i % 40) as i64,
            score: 50.0 + (i % 50) as f64 + 0.5,
            active: i % 3 != 0,
            role: roles[i % roles.len()].into(),
            city: cities[i % cities.len()].into(),
        })
        .collect()
}

fn generate_all_types(n: usize) -> Vec<AllTypes> {
    (0..n)
        .map(|i| AllTypes {
            b: i % 2 == 0,
            i8v: (i % 256) as i8,
            i16v: -(i as i16),
            i32v: i as i32 * 1000,
            i64v: i as i64 * 100_000,
            u8v: (i % 256) as u8,
            u16v: (i % 65536) as u16,
            u32v: i as u32 * 7919,
            u64v: i as u64 * 1_000_000_007,
            f32v: (i as f32) * 1.5,
            f64v: (i as f64) * 0.25 + 0.5,
            s: format!("item_{}", i),
            opt_some: if i % 2 == 0 { Some(i as i64) } else { None },
            opt_none: None,
            vec_int: vec![i as i64, (i + 1) as i64, (i + 2) as i64],
            vec_str: vec![format!("tag{}", i % 5), format!("cat{}", i % 3)],
        })
        .collect()
}

fn generate_companies(n: usize) -> Vec<Company> {
    (0..n)
        .map(|i| Company {
            id: i as i64,
            name: format!("Company_{}", i),
            divisions: (0..3)
                .map(|d| Division {
                    id: d,
                    name: format!("Div_{}", d),
                    teams: (0..3)
                        .map(|t| Team {
                            id: t,
                            name: format!("Team_{}", t),
                            projects: (0..3)
                                .map(|p| Project {
                                    id: p,
                                    name: format!("Proj_{}", p),
                                    tasks: (0..3)
                                        .map(|tk| Task {
                                            id: tk,
                                            title: format!("Task_{}", tk),
                                            done: tk % 2 == 0,
                                        })
                                        .collect(),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

fn main() {
    let arg = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "encode_alltypes".into());
    let iters: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);

    match arg.as_str() {
        "encode_user" => {
            let data = generate_users(1000);
            for _ in 0..iters {
                let s = asun::encode(&data).unwrap();
                std::hint::black_box(s);
            }
        }
        "encode_alltypes" => {
            let data = generate_all_types(500);
            for _ in 0..iters {
                let s = asun::encode(&data).unwrap();
                std::hint::black_box(s);
            }
        }
        "encode_deep" => {
            let data = generate_companies(50);
            for _ in 0..iters {
                let s = asun::encode(&data).unwrap();
                std::hint::black_box(s);
            }
        }
        "decode_user" => {
            let data = generate_users(1000);
            let encoded = asun::encode(&data).unwrap();
            for _ in 0..iters {
                let v: Vec<User> = asun::decode(&encoded).unwrap();
                std::hint::black_box(v);
            }
        }
        "decode_alltypes" => {
            let data = generate_all_types(500);
            let encoded = asun::encode(&data).unwrap();
            for _ in 0..iters {
                let v: Vec<AllTypes> = asun::decode(&encoded).unwrap();
                std::hint::black_box(v);
            }
        }
        "decode_deep" => {
            let data = generate_companies(50);
            let encoded = asun::encode(&data).unwrap();
            for _ in 0..iters {
                let v: Vec<Company> = asun::decode(&encoded).unwrap();
                std::hint::black_box(v);
            }
        }
        other => {
            eprintln!("unknown subcommand: {other}");
            eprintln!(
                "valid: encode_user, encode_alltypes, encode_deep, decode_user, decode_alltypes, decode_deep"
            );
            std::process::exit(2);
        }
    }
}
