use asun::{decode, decode_binary, encode, encode_binary, encode_typed};
use serde::{Deserialize, Serialize};
use std::time::Instant;

// ===========================================================================
// 1. Flat struct (8 fields) — original benchmark
// ===========================================================================

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

// ===========================================================================
// 2. All-types struct — covers every ASUN primitive/compound
// ===========================================================================

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

// ===========================================================================
// 3. 5-level deep struct: Company > Division > Team > Project > Task
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Task {
    id: i64,
    title: String,
    priority: i64,
    done: bool,
    hours: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Project {
    name: String,
    budget: f64,
    active: bool,
    tasks: Vec<Task>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Team {
    name: String,
    lead: String,
    size: i64,
    projects: Vec<Project>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Division {
    name: String,
    location: String,
    headcount: i64,
    teams: Vec<Team>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Company {
    name: String,
    founded: i64,
    revenue_m: f64,
    public: bool,
    divisions: Vec<Division>,
    tags: Vec<String>,
}

// ===========================================================================
// Data generators
// ===========================================================================

fn generate_users(n: usize) -> Vec<User> {
    let names = [
        "Alice", "Bob", "Carol", "David", "Eve", "Frank", "Grace", "Hank",
    ];
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
    let divisions_per = 2;
    let teams_per = 2;
    let projects_per = 3;
    let tasks_per = 4;

    (0..n)
        .map(|i| Company {
            name: format!("Corp_{}", i),
            founded: 1990 + (i % 35) as i64,
            revenue_m: 10.0 + (i as f64) * 5.5,
            public: i % 2 == 0,
            divisions: (0..divisions_per)
                .map(|d| Division {
                    name: format!("Div_{}_{}", i, d),
                    location: ["NYC", "London", "Tokyo", "Berlin"][d % 4].into(),
                    headcount: 50 + (d * 20) as i64,
                    teams: (0..teams_per)
                        .map(|t| Team {
                            name: format!("Team_{}_{}_{}", i, d, t),
                            lead: ["Alice", "Bob", "Carol", "David"][t % 4].into(),
                            size: 5 + (t * 2) as i64,
                            projects: (0..projects_per)
                                .map(|p| Project {
                                    name: format!("Proj_{}_{}", t, p),
                                    budget: 100.0 + (p as f64) * 50.5,
                                    active: p % 2 == 0,
                                    tasks: (0..tasks_per)
                                        .map(|tk| Task {
                                            id: (i * 100 + d * 10 + t * 5 + tk) as i64,
                                            title: format!("Task_{}", tk),
                                            priority: (tk % 3 + 1) as i64,
                                            done: tk % 2 == 0,
                                            hours: 2.0 + (tk as f64) * 1.5,
                                        })
                                        .collect(),
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
            tags: vec![
                "enterprise".into(),
                "tech".into(),
                format!("sector_{}", i % 5),
            ],
        })
        .collect()
}

// ===========================================================================
// Memory measurement helpers
// ===========================================================================

#[cfg(target_os = "macos")]
fn get_rss_bytes() -> usize {
    use std::mem::MaybeUninit;
    unsafe {
        let mut info: MaybeUninit<libc::rusage> = MaybeUninit::uninit();
        libc::getrusage(libc::RUSAGE_SELF, info.as_mut_ptr());
        info.assume_init().ru_maxrss as usize // macOS returns bytes
    }
}

#[cfg(not(target_os = "macos"))]
fn get_rss_bytes() -> usize {
    // On Linux, ru_maxrss is in KB
    use std::mem::MaybeUninit;
    unsafe {
        let mut info: MaybeUninit<libc::rusage> = MaybeUninit::uninit();
        libc::getrusage(libc::RUSAGE_SELF, info.as_mut_ptr());
        info.assume_init().ru_maxrss as usize * 1024
    }
}

fn format_bytes(b: usize) -> String {
    if b >= 1_048_576 {
        format!("{:.1} MB", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.1} KB", b as f64 / 1024.0)
    } else {
        format!("{} B", b)
    }
}

fn format_ratio(base: f64, target: f64) -> String {
    if target <= 0.0 {
        return "infx".into();
    }
    let s = format!("{:.1}", base / target);
    s.trim_end_matches('0').trim_end_matches('.').to_string() + "x"
}

fn format_percent(part: usize, whole: usize) -> String {
    if whole == 0 {
        return "0%".into();
    }
    let s = format!("{:.1}", part as f64 * 100.0 / whole as f64);
    s.trim_end_matches('0').trim_end_matches('.').to_string() + "%"
}

// ===========================================================================
// Benchmark runner
// ===========================================================================

struct BenchResult {
    name: String,
    json_ser_ms: f64,
    asun_ser_ms: f64,
    bin_ser_ms: f64,
    json_de_ms: f64,
    asun_de_ms: f64,
    bin_de_ms: f64,
    json_bytes: usize,
    asun_bytes: usize,
    bin_bytes: usize,
}

impl BenchResult {
    fn print(&self) {
        println!("  {}", self.name);
        println!(
            "    Encode:      JSON {:.2}ms/{}B | ASUN {:.2}ms({})/{}B({}) | BIN {:.2}ms({})/{}B({})",
            self.json_ser_ms,
            self.json_bytes,
            self.asun_ser_ms,
            format_ratio(self.json_ser_ms, self.asun_ser_ms),
            self.asun_bytes,
            format_percent(self.asun_bytes, self.json_bytes),
            self.bin_ser_ms,
            format_ratio(self.json_ser_ms, self.bin_ser_ms),
            self.bin_bytes,
            format_percent(self.bin_bytes, self.json_bytes),
        );
        println!(
            "    Decode:      JSON {:>8.2}ms | ASUN {:>8.2}ms ({}) | BIN {:>8.2}ms ({})",
            self.json_de_ms,
            self.asun_de_ms,
            format_ratio(self.json_de_ms, self.asun_de_ms),
            self.bin_de_ms,
            format_ratio(self.json_de_ms, self.bin_de_ms),
        );
    }
}

// ---------------------------------------------------------------------------
// Flat struct benchmarks
// ---------------------------------------------------------------------------

fn bench_flat(count: usize, iterations: u32) -> BenchResult {
    let users = generate_users(count);

    // JSON encode
    let mut json_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        json_str = serde_json::to_string(&users).unwrap();
    }
    let json_ser = start.elapsed();

    // ASUN encode
    let mut asun_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        asun_str = encode(&users).unwrap();
    }
    let asun_ser = start.elapsed();

    let mut bin_buf = Vec::new();
    let start = Instant::now();
    for _ in 0..iterations {
        bin_buf = encode_binary(&users).unwrap();
    }
    let bin_ser = start.elapsed();

    // JSON decode
    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = serde_json::from_str(&json_str).unwrap();
    }
    let json_de = start.elapsed();

    // ASUN decode
    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = decode(&asun_str).unwrap();
    }
    let asun_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = decode_binary(&bin_buf).unwrap();
    }
    let bin_de = start.elapsed();

    // Verify
    let decoded: Vec<User> = decode(&asun_str).unwrap();
    assert_eq!(users, decoded, "flat {} roundtrip failed", count);
    let decoded_bin: Vec<User> = decode_binary(&bin_buf).unwrap();
    assert_eq!(users, decoded_bin, "flat {} binary roundtrip failed", count);

    BenchResult {
        name: format!("Flat struct × {} (8 fields, vec)", count),
        json_ser_ms: json_ser.as_secs_f64() * 1000.0,
        asun_ser_ms: asun_ser.as_secs_f64() * 1000.0,
        bin_ser_ms: bin_ser.as_secs_f64() * 1000.0,
        json_de_ms: json_de.as_secs_f64() * 1000.0,
        asun_de_ms: asun_de.as_secs_f64() * 1000.0,
        bin_de_ms: bin_de.as_secs_f64() * 1000.0,
        json_bytes: json_str.len(),
        asun_bytes: asun_str.len(),
        bin_bytes: bin_buf.len(),
    }
}

// ---------------------------------------------------------------------------
// All-types struct benchmark
// ---------------------------------------------------------------------------

fn bench_all_types(count: usize, iterations: u32) -> BenchResult {
    let items = generate_all_types(count);

    let mut json_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        json_str = serde_json::to_string(&items).unwrap();
    }
    let json_ser = start.elapsed();

    // ASUN: encode vec directly
    let mut asun_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        asun_str = encode(&items).unwrap();
    }
    let asun_ser = start.elapsed();

    let mut bin_buf = Vec::new();
    let start = Instant::now();
    for _ in 0..iterations {
        bin_buf = encode_binary(&items).unwrap();
    }
    let bin_ser = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<AllTypes> = serde_json::from_str(&json_str).unwrap();
    }
    let json_de = start.elapsed();

    // ASUN: decode vec directly
    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<AllTypes> = decode(&asun_str).unwrap();
    }
    let asun_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<AllTypes> = decode_binary(&bin_buf).unwrap();
    }
    let bin_de = start.elapsed();

    // Verify
    let decoded: Vec<AllTypes> = decode(&asun_str).unwrap();
    assert_eq!(items, decoded, "all-types {} roundtrip failed", count);
    let decoded_bin: Vec<AllTypes> = decode_binary(&bin_buf).unwrap();
    assert_eq!(
        items, decoded_bin,
        "all-types {} binary roundtrip failed",
        count
    );

    BenchResult {
        name: format!("All-types struct × {} (16 fields, vec)", count),
        json_ser_ms: json_ser.as_secs_f64() * 1000.0,
        asun_ser_ms: asun_ser.as_secs_f64() * 1000.0,
        bin_ser_ms: bin_ser.as_secs_f64() * 1000.0,
        json_de_ms: json_de.as_secs_f64() * 1000.0,
        asun_de_ms: asun_de.as_secs_f64() * 1000.0,
        bin_de_ms: bin_de.as_secs_f64() * 1000.0,
        json_bytes: json_str.len(),
        asun_bytes: asun_str.len(),
        bin_bytes: bin_buf.len(),
    }
}

// ---------------------------------------------------------------------------
// 5-level deep struct benchmark
// ---------------------------------------------------------------------------

fn bench_deep(count: usize, iterations: u32) -> BenchResult {
    let companies = generate_companies(count);

    let mut json_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        json_str = serde_json::to_string(&companies).unwrap();
    }
    let json_ser = start.elapsed();

    // ASUN: encode vec directly
    let mut asun_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        asun_str = encode(&companies).unwrap();
    }
    let asun_ser = start.elapsed();

    let mut bin_buf = Vec::new();
    let start = Instant::now();
    for _ in 0..iterations {
        bin_buf = encode_binary(&companies).unwrap();
    }
    let bin_ser = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = serde_json::from_str(&json_str).unwrap();
    }
    let json_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = decode(&asun_str).unwrap();
    }
    let asun_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = decode_binary(&bin_buf).unwrap();
    }
    let bin_de = start.elapsed();

    // Verify
    let decoded: Vec<Company> = decode(&asun_str).unwrap();
    assert_eq!(companies, decoded, "deep {} roundtrip failed", count);
    let decoded_bin: Vec<Company> = decode_binary(&bin_buf).unwrap();
    assert_eq!(
        companies, decoded_bin,
        "deep {} binary roundtrip failed",
        count
    );

    BenchResult {
        name: format!(
            "5-level deep × {} (Company>Division>Team>Project>Task)",
            count
        ),
        json_ser_ms: json_ser.as_secs_f64() * 1000.0,
        asun_ser_ms: asun_ser.as_secs_f64() * 1000.0,
        bin_ser_ms: bin_ser.as_secs_f64() * 1000.0,
        json_de_ms: json_de.as_secs_f64() * 1000.0,
        asun_de_ms: asun_de.as_secs_f64() * 1000.0,
        bin_de_ms: bin_de.as_secs_f64() * 1000.0,
        json_bytes: json_str.len(),
        asun_bytes: asun_str.len(),
        bin_bytes: bin_buf.len(),
    }
}

// ---------------------------------------------------------------------------
// Single struct roundtrip benchmark
// ---------------------------------------------------------------------------

fn bench_single_roundtrip(iterations: u32) -> (f64, f64) {
    let user = User {
        id: 1,
        name: "Alice".into(),
        email: "alice@example.com".into(),
        age: 30,
        score: 95.5,
        active: true,
        role: "engineer".into(),
        city: "NYC".into(),
    };

    let start = Instant::now();
    for _ in 0..iterations {
        let s = encode(&user).unwrap();
        let _: User = decode(&s).unwrap();
    }
    let asun_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    for _ in 0..iterations {
        let s = serde_json::to_string(&user).unwrap();
        let _: User = serde_json::from_str(&s).unwrap();
    }
    let json_ms = start.elapsed().as_secs_f64() * 1000.0;

    (asun_ms, json_ms)
}

// ---------------------------------------------------------------------------
// Deep single struct roundtrip
// ---------------------------------------------------------------------------

fn bench_deep_single_roundtrip(iterations: u32) -> (f64, f64) {
    let company = Company {
        name: "MegaCorp".into(),
        founded: 2000,
        revenue_m: 500.5,
        public: true,
        divisions: vec![Division {
            name: "Engineering".into(),
            location: "SF".into(),
            headcount: 200,
            teams: vec![Team {
                name: "Backend".into(),
                lead: "Alice".into(),
                size: 12,
                projects: vec![Project {
                    name: "API v3".into(),
                    budget: 250.0,
                    active: true,
                    tasks: vec![
                        Task {
                            id: 1,
                            title: "Design".into(),
                            priority: 1,
                            done: true,
                            hours: 40.0,
                        },
                        Task {
                            id: 2,
                            title: "Implement".into(),
                            priority: 1,
                            done: false,
                            hours: 120.0,
                        },
                        Task {
                            id: 3,
                            title: "Test".into(),
                            priority: 2,
                            done: false,
                            hours: 30.0,
                        },
                    ],
                }],
            }],
        }],
        tags: vec!["tech".into(), "public".into()],
    };

    let start = Instant::now();
    for _ in 0..iterations {
        let s = encode(&company).unwrap();
        let _: Company = decode(&s).unwrap();
    }
    let asun_ms = start.elapsed().as_secs_f64() * 1000.0;

    let start = Instant::now();
    for _ in 0..iterations {
        let s = serde_json::to_string(&company).unwrap();
        let _: Company = serde_json::from_str(&s).unwrap();
    }
    let json_ms = start.elapsed().as_secs_f64() * 1000.0;

    (asun_ms, json_ms)
}

// ===========================================================================
// Section 9: Binary Format (ASUN-BIN) helpers
// ===========================================================================

struct BinBenchResult {
    name: String,
    json_ser_ms: f64,
    asun_ser_ms: f64,
    bin_ser_ms: f64,
    json_de_ms: f64,
    asun_de_ms: f64,
    bin_de_ms: f64,
    json_bytes: usize,
    asun_bytes: usize,
    bin_bytes: usize,
}

impl BinBenchResult {
    fn print(&self) {
        println!("  {}", self.name);
        println!(
            "    Encode:      JSON {:.2}ms/{}B | ASUN {:.2}ms({})/{}B({}) | BIN {:.2}ms({})/{}B({})",
            self.json_ser_ms,
            self.json_bytes,
            self.asun_ser_ms,
            format_ratio(self.json_ser_ms, self.asun_ser_ms),
            self.asun_bytes,
            format_percent(self.asun_bytes, self.json_bytes),
            self.bin_ser_ms,
            format_ratio(self.json_ser_ms, self.bin_ser_ms),
            self.bin_bytes,
            format_percent(self.bin_bytes, self.json_bytes),
        );
        println!(
            "    Decode:      JSON {:>8.2}ms | ASUN {:>8.2}ms ({}) | BIN {:>8.2}ms ({})",
            self.json_de_ms,
            self.asun_de_ms,
            format_ratio(self.json_de_ms, self.asun_de_ms),
            self.bin_de_ms,
            format_ratio(self.json_de_ms, self.bin_de_ms),
        );
    }
}

fn bench_flat_bin(count: usize, iterations: u32) -> BinBenchResult {
    let users = generate_users(count);

    let mut json_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        json_str = serde_json::to_string(&users).unwrap();
    }
    let json_ser = start.elapsed();

    let mut asun_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        asun_str = encode(&users).unwrap();
    }
    let asun_ser = start.elapsed();

    let mut bin_buf: Vec<u8> = Vec::new();
    let start = Instant::now();
    for _ in 0..iterations {
        bin_buf = encode_binary(&users).unwrap();
    }
    let bin_ser = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = serde_json::from_str(&json_str).unwrap();
    }
    let json_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = decode(&asun_str).unwrap();
    }
    let asun_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<User> = decode_binary(&bin_buf).unwrap();
    }
    let bin_de = start.elapsed();

    let decoded: Vec<User> = decode_binary(&bin_buf).unwrap();
    assert_eq!(users, decoded, "bin flat {} roundtrip failed", count);

    BinBenchResult {
        name: format!("Flat struct × {} (8 fields)", count),
        json_ser_ms: json_ser.as_secs_f64() * 1000.0,
        asun_ser_ms: asun_ser.as_secs_f64() * 1000.0,
        bin_ser_ms: bin_ser.as_secs_f64() * 1000.0,
        json_de_ms: json_de.as_secs_f64() * 1000.0,
        asun_de_ms: asun_de.as_secs_f64() * 1000.0,
        bin_de_ms: bin_de.as_secs_f64() * 1000.0,
        json_bytes: json_str.len(),
        asun_bytes: asun_str.len(),
        bin_bytes: bin_buf.len(),
    }
}

fn bench_deep_bin(count: usize, iterations: u32) -> BinBenchResult {
    let companies = generate_companies(count);

    let mut json_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        json_str = serde_json::to_string(&companies).unwrap();
    }
    let json_ser = start.elapsed();

    // ASUN: encode vec directly
    let mut asun_str = String::new();
    let start = Instant::now();
    for _ in 0..iterations {
        asun_str = encode(&companies).unwrap();
    }
    let asun_ser = start.elapsed();

    let mut bin_buf: Vec<u8> = Vec::new();
    let start = Instant::now();
    for _ in 0..iterations {
        bin_buf = encode_binary(&companies).unwrap();
    }
    let bin_ser = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = serde_json::from_str(&json_str).unwrap();
    }
    let json_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = decode(&asun_str).unwrap();
    }
    let asun_de = start.elapsed();

    let start = Instant::now();
    for _ in 0..iterations {
        let _: Vec<Company> = decode_binary(&bin_buf).unwrap();
    }
    let bin_de = start.elapsed();

    let decoded: Vec<Company> = decode_binary(&bin_buf).unwrap();
    assert_eq!(companies, decoded, "bin deep {} roundtrip failed", count);

    BinBenchResult {
        name: format!("Deep struct × {} (5-level nested)", count),
        json_ser_ms: json_ser.as_secs_f64() * 1000.0,
        asun_ser_ms: asun_ser.as_secs_f64() * 1000.0,
        bin_ser_ms: bin_ser.as_secs_f64() * 1000.0,
        json_de_ms: json_de.as_secs_f64() * 1000.0,
        asun_de_ms: asun_de.as_secs_f64() * 1000.0,
        bin_de_ms: bin_de.as_secs_f64() * 1000.0,
        json_bytes: json_str.len(),
        asun_bytes: asun_str.len(),
        bin_bytes: bin_buf.len(),
    }
}

// ===========================================================================
// Main
// ===========================================================================

fn main() {
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║            ASUN vs JSON Comprehensive Benchmark              ║");
    println!("╚══════════════════════════════════════════════════════════════╝");

    // System info
    println!(
        "\nSystem: {} {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    let rss_before = get_rss_bytes();
    println!("RSS before benchmarks: {}\n", format_bytes(rss_before));

    let iterations = 100;
    println!("Iterations per test: {}", iterations);

    // ===================================================================
    // Section 1: Flat struct (schema-driven vec encoding)
    // ===================================================================
    println!("\n┌─────────────────────────────────────────────┐");
    println!("│  Section 1: Flat Struct (schema-driven vec) │");
    println!("└─────────────────────────────────────────────┘");

    for count in [100, 500, 1000, 5000] {
        let r = bench_flat(count, iterations);
        r.print();
        println!();
    }

    let rss_after_flat = get_rss_bytes();
    println!(
        "  RSS after flat benchmarks: {} (Δ {})",
        format_bytes(rss_after_flat),
        format_bytes(rss_after_flat.saturating_sub(rss_before))
    );

    // ===================================================================
    // Section 2: All-types struct
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────┐");
    println!("│  Section 2: All-Types Struct (16 fields)     │");
    println!("└──────────────────────────────────────────────┘");

    for count in [100, 500] {
        let r = bench_all_types(count, iterations);
        r.print();
        println!();
    }

    // ===================================================================
    // Section 3: 5-level deep nested struct
    // ===================================================================
    println!("┌──────────────────────────────────────────────────────────┐");
    println!("│  Section 3: 5-Level Deep Nesting (Company hierarchy)     │");
    println!("└──────────────────────────────────────────────────────────┘");

    for count in [10, 50, 100] {
        let r = bench_deep(count, iterations);
        r.print();
        println!();
    }

    let rss_after_deep = get_rss_bytes();
    println!(
        "  RSS after deep benchmarks: {} (Δ {})",
        format_bytes(rss_after_deep),
        format_bytes(rss_after_deep.saturating_sub(rss_before))
    );

    // ===================================================================
    // Section 4: Single struct roundtrip
    // ===================================================================
    println!("┌──────────────────────────────────────────────┐");
    println!("│  Section 4: Single Struct Roundtrip (10000x) │");
    println!("└──────────────────────────────────────────────┘");

    let (asun_flat, json_flat) = bench_single_roundtrip(10000);
    println!(
        "  Flat:  ASUN {:>6.2}ms | JSON {:>6.2}ms | ratio {:.2}x",
        asun_flat,
        json_flat,
        json_flat / asun_flat
    );

    let (asun_deep, json_deep) = bench_deep_single_roundtrip(10000);
    println!(
        "  Deep:  ASUN {:>6.2}ms | JSON {:>6.2}ms | ratio {:.2}x",
        asun_deep,
        json_deep,
        json_deep / asun_deep
    );

    // ===================================================================
    // Section 5: Large payload — 10k flat records
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────┐");
    println!("│  Section 5: Large Payload (10k records)      │");
    println!("└──────────────────────────────────────────────┘");

    let r_large = bench_flat(10000, 10);
    println!("  (10 iterations for large payload)");
    r_large.print();

    let rss_after_large = get_rss_bytes();
    println!(
        "\n  RSS after large payload: {} (Δ {})",
        format_bytes(rss_after_large),
        format_bytes(rss_after_large.saturating_sub(rss_before))
    );

    // ===================================================================
    // Section 6: Annotated vs Unannotated Schema Decoding
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────────────────────┐");
    println!("│  Section 6: Annotated vs Unannotated Schema (decode)          │");
    println!("└──────────────────────────────────────────────────────────────┘");

    {
        // --- (a) Flat struct vec: 1000 records ---
        let users_1k = generate_users(1000);
        let asun_untyped = encode(&users_1k).unwrap(); // e.g. {id,name,...}:...
        // Build typed version by replacing the schema header
        let asun_typed = asun_untyped.replacen(
            "{id,name,email,age,score,active,role,city}:",
            "{id@int,name@str,email@str,age@int,score@float,active@bool,role@str,city@str}:",
            1,
        );
        // Verify both parse identically
        let v1: Vec<User> = decode(&asun_untyped).unwrap();
        let v2: Vec<User> = decode(&asun_typed).unwrap();
        assert_eq!(v1, v2, "typed/untyped flat roundtrip mismatch");

        let de_iters = 200u32;
        let start = Instant::now();
        for _ in 0..de_iters {
            let _: Vec<User> = decode(&asun_untyped).unwrap();
        }
        let untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        for _ in 0..de_iters {
            let _: Vec<User> = decode(&asun_typed).unwrap();
        }
        let typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let ratio = untyped_ms / typed_ms;
        println!("  Flat struct × 1000 ({de_iters} iters, decode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            untyped_ms,
            asun_untyped.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            typed_ms,
            asun_typed.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", ratio);
        println!(
            "    Schema overhead: +{} bytes ({:.1}%)",
            asun_typed.len() - asun_untyped.len(),
            (asun_typed.len() as f64 / asun_untyped.len() as f64 - 1.0) * 100.0
        );
        println!();

        // --- (b) Deep nested single struct ---
        let company = &generate_companies(1)[0];
        let asun_deep_untyped = encode(company).unwrap();
        let asun_deep_typed = asun_deep_untyped.replacen(
            "{name,founded,revenue_m,public,divisions,tags}:",
            "{name@str,founded@int,revenue_m@float,public@bool,divisions,tags}:",
            1,
        );

        let c1: Company = decode(&asun_deep_untyped).unwrap();
        let c2: Company = decode(&asun_deep_typed).unwrap();
        assert_eq!(c1, c2, "typed/untyped deep roundtrip mismatch");

        let deep_iters = 5000u32;
        let start = Instant::now();
        for _ in 0..deep_iters {
            let _: Company = decode(&asun_deep_untyped).unwrap();
        }
        let deep_untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        for _ in 0..deep_iters {
            let _: Company = decode(&asun_deep_typed).unwrap();
        }
        let deep_typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let deep_ratio = deep_untyped_ms / deep_typed_ms;
        println!("  5-level deep single struct ({deep_iters} iters, decode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            deep_untyped_ms,
            asun_deep_untyped.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            deep_typed_ms,
            asun_deep_typed.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", deep_ratio);
        println!(
            "    Schema overhead: +{} bytes ({:.1}%)",
            asun_deep_typed.len() - asun_deep_untyped.len(),
            (asun_deep_typed.len() as f64 / asun_deep_untyped.len() as f64 - 1.0) * 100.0
        );
        println!();

        // --- (c) All-types single struct ---
        let at = &generate_all_types(1)[0];
        let asun_at_untyped = encode(at).unwrap();
        let asun_at_typed = asun_at_untyped.replacen(
            "{b,i8v,i16v,i32v,i64v,u8v,u16v,u32v,u64v,f32v,f64v,s,opt_some,opt_none,vec_int,vec_str}:",
            "{b@bool,i8v@int,i16v@int,i32v@int,i64v@int,u8v@int,u16v@int,u32v@int,u64v@int,f32v@float,f64v@float,s@str,opt_some@int,opt_none@int,vec_int@[int],vec_str@[str]}:",
            1,
        );

        let a1: AllTypes = decode(&asun_at_untyped).unwrap();
        let a2: AllTypes = decode(&asun_at_typed).unwrap();
        assert_eq!(a1, a2, "typed/untyped all-types roundtrip mismatch");

        let at_iters = 10000u32;
        let start = Instant::now();
        for _ in 0..at_iters {
            let _: AllTypes = decode(&asun_at_untyped).unwrap();
        }
        let at_untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        for _ in 0..at_iters {
            let _: AllTypes = decode(&asun_at_typed).unwrap();
        }
        let at_typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let at_ratio = at_untyped_ms / at_typed_ms;
        println!("  All-types single struct ({at_iters} iters, decode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            at_untyped_ms,
            asun_at_untyped.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            at_typed_ms,
            asun_at_typed.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", at_ratio);
        println!(
            "    Schema overhead: +{} bytes ({:.1}%)",
            asun_at_typed.len() - asun_at_untyped.len(),
            (asun_at_typed.len() as f64 / asun_at_untyped.len() as f64 - 1.0) * 100.0
        );

        println!();
        println!("  Summary: Type annotations add a small schema parsing cost but");
        println!("  are negligible in overall decoding. Both produce identical results.");
    }

    // ===================================================================
    // Section 7: Annotated vs Unannotated Schema Encoding
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────────────────────┐");
    println!("│  Section 7: Annotated vs Unannotated Schema (encode)          │");
    println!("└──────────────────────────────────────────────────────────────┘");

    {
        // --- (a) Flat struct vec: 1000 records (encode vs encode_typed) ---
        let users_1k = generate_users(1000);

        let ser_iters = 200u32;
        let start = Instant::now();
        let mut untyped_out = String::new();
        for _ in 0..ser_iters {
            untyped_out = encode(&users_1k).unwrap();
        }
        let untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        let mut typed_out = String::new();
        for _ in 0..ser_iters {
            typed_out = encode_typed(&users_1k).unwrap();
        }
        let typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        // Verify both decode to the same result
        let v1: Vec<User> = decode(&untyped_out).unwrap();
        let v2: Vec<User> = decode(&typed_out).unwrap();
        assert_eq!(v1, v2, "typed/untyped flat encode mismatch");

        let ratio = untyped_ms / typed_ms;
        println!("  Flat struct × 1000 vec ({ser_iters} iters, encode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            untyped_ms,
            untyped_out.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            typed_ms,
            typed_out.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", ratio);
        println!(
            "    Schema overhead: +{} bytes ({:.1}%)",
            typed_out.len() - untyped_out.len(),
            (typed_out.len() as f64 / untyped_out.len() as f64 - 1.0) * 100.0
        );
        println!();

        // --- (b) Single struct: encode vs encode_typed ---
        let single_user = &users_1k[0];
        let single_iters = 10000u32;

        let start = Instant::now();
        let mut single_untyped = String::new();
        for _ in 0..single_iters {
            single_untyped = encode(single_user).unwrap();
        }
        let single_untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        let mut single_typed = String::new();
        for _ in 0..single_iters {
            single_typed = encode_typed(single_user).unwrap();
        }
        let single_typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let single_ratio = single_untyped_ms / single_typed_ms;
        println!("  Single flat struct ({single_iters} iters, encode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            single_untyped_ms,
            single_untyped.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            single_typed_ms,
            single_typed.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", single_ratio);
        println!();

        // --- (c) Deep nested single struct: encode vs encode_typed ---
        let company = &generate_companies(1)[0];

        let deep_iters = 5000u32;
        let start = Instant::now();
        let mut deep_untyped = String::new();
        for _ in 0..deep_iters {
            deep_untyped = encode(company).unwrap();
        }
        let deep_untyped_ms = start.elapsed().as_secs_f64() * 1000.0;

        let start = Instant::now();
        let mut deep_typed = String::new();
        for _ in 0..deep_iters {
            deep_typed = encode_typed(company).unwrap();
        }
        let deep_typed_ms = start.elapsed().as_secs_f64() * 1000.0;

        let deep_ratio = deep_untyped_ms / deep_typed_ms;
        println!("  5-level deep single struct ({deep_iters} iters, encode only)");
        println!(
            "    Unannotated: {:>8.2}ms  ({} B)",
            deep_untyped_ms,
            deep_untyped.len()
        );
        println!(
            "    Annotated:   {:>8.2}ms  ({} B)",
            deep_typed_ms,
            deep_typed.len()
        );
        println!("    Ratio: {:.3}x (unannotated / annotated)", deep_ratio);
        println!(
            "    Schema overhead: +{} bytes ({:.1}%)",
            deep_typed.len() - deep_untyped.len(),
            (deep_typed.len() as f64 / deep_untyped.len() as f64 - 1.0) * 100.0
        );

        // Verify roundtrip
        let c1: Company = decode(&deep_untyped).unwrap();
        let c2: Company = decode(&deep_typed).unwrap();
        assert_eq!(c1, c2, "typed/untyped deep encode mismatch");

        println!();
        println!("  Summary: Typed encoding has minimal overhead. The extra cost");
        println!("  is recording and emitting type hints in the schema header.");
    }

    // ===================================================================
    // Section 8: Throughput summary
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────┐");
    println!("│  Section 8: Throughput Summary               │");
    println!("└──────────────────────────────────────────────┘");

    // Measure raw throughput: 1000 records × 100 iterations
    let users_1k = generate_users(1000);
    let json_1k = serde_json::to_string(&users_1k).unwrap();
    let asun_1k = encode(&users_1k).unwrap();

    let iters = 100u32;

    let start = Instant::now();
    for _ in 0..iters {
        let _ = serde_json::to_string(&users_1k).unwrap();
    }
    let json_ser_dur = start.elapsed();

    let start = Instant::now();
    for _ in 0..iters {
        let _ = encode(&users_1k).unwrap();
    }
    let asun_ser_dur = start.elapsed();

    let start = Instant::now();
    for _ in 0..iters {
        let _: Vec<User> = serde_json::from_str(&json_1k).unwrap();
    }
    let json_de_dur = start.elapsed();

    let start = Instant::now();
    for _ in 0..iters {
        let _: Vec<User> = decode(&asun_1k).unwrap();
    }
    let asun_de_dur = start.elapsed();

    let total_records = 1000.0 * iters as f64;
    let json_ser_rps = total_records / json_ser_dur.as_secs_f64();
    let asun_ser_rps = total_records / asun_ser_dur.as_secs_f64();
    let json_de_rps = total_records / json_de_dur.as_secs_f64();
    let asun_de_rps = total_records / asun_de_dur.as_secs_f64();

    let json_ser_mbps =
        (json_1k.len() as f64 * iters as f64) / json_ser_dur.as_secs_f64() / 1_048_576.0;
    let asun_ser_mbps =
        (asun_1k.len() as f64 * iters as f64) / asun_ser_dur.as_secs_f64() / 1_048_576.0;
    let json_de_mbps =
        (json_1k.len() as f64 * iters as f64) / json_de_dur.as_secs_f64() / 1_048_576.0;
    let asun_de_mbps =
        (asun_1k.len() as f64 * iters as f64) / asun_de_dur.as_secs_f64() / 1_048_576.0;

    println!("  Encode throughput (1000 records × {iters} iters):");
    println!(
        "    JSON: {:.0} records/s  ({:.1} MB/s of JSON)",
        json_ser_rps, json_ser_mbps
    );
    println!(
        "    ASUN: {:.0} records/s  ({:.1} MB/s of ASUN)",
        asun_ser_rps, asun_ser_mbps
    );
    println!(
        "    Speed: {:.2}x {}",
        asun_ser_rps / json_ser_rps,
        if asun_ser_rps > json_ser_rps {
            "✓ ASUN faster"
        } else {
            ""
        }
    );
    println!("  Decode throughput:");
    println!(
        "    JSON: {:.0} records/s  ({:.1} MB/s)",
        json_de_rps, json_de_mbps
    );
    println!(
        "    ASUN: {:.0} records/s  ({:.1} MB/s)",
        asun_de_rps, asun_de_mbps
    );
    println!(
        "    Speed: {:.2}x {}",
        asun_de_rps / json_de_rps,
        if asun_de_rps > json_de_rps {
            "✓ ASUN faster"
        } else {
            ""
        }
    );

    // Peak RSS
    let rss_final = get_rss_bytes();
    println!("\n  Memory:");
    println!("    Initial RSS:  {}", format_bytes(rss_before));
    println!("    Final RSS:    {}", format_bytes(rss_final));
    println!(
        "    Peak delta:   {}",
        format_bytes(rss_final.saturating_sub(rss_before))
    );

    // ===================================================================
    // Section 9: Binary Format (ASUN-BIN)
    // ===================================================================
    println!("\n┌──────────────────────────────────────────────────────────────┐");
    println!("│  Section 9: Binary Format (ASUN-BIN) vs ASUN text vs JSON    │");
    println!("└──────────────────────────────────────────────────────────────┘");

    println!("\n  ── Flat struct ──");
    bench_flat_bin(100, 50).print();
    bench_flat_bin(1000, 20).print();
    bench_flat_bin(5000, 5).print();

    println!("\n  ── Deep struct (5-level nested) ──");
    bench_deep_bin(10, 50).print();
    bench_deep_bin(100, 10).print();
    bench_deep_bin(500, 3).print();

    println!("\n  ── Single User roundtrip ──");
    {
        let user = User {
            id: 42,
            name: "Alice".into(),
            email: "alice@example.com".into(),
            age: 30,
            score: 9.8,
            active: true,
            role: "admin".into(),
            city: "Berlin".into(),
        };
        let iters: u32 = 100_000;

        let start = Instant::now();
        for _ in 0..iters {
            let b = encode_binary(&user).unwrap();
            let _: User = decode_binary(&b).unwrap();
        }
        let bin_ns = start.elapsed().as_nanos() as f64 / iters as f64;

        let start = Instant::now();
        for _ in 0..iters {
            let s = encode(&user).unwrap();
            let _: User = decode(&s).unwrap();
        }
        let asun_ns = start.elapsed().as_nanos() as f64 / iters as f64;

        let start = Instant::now();
        for _ in 0..iters {
            let s = serde_json::to_string(&user).unwrap();
            let _: User = serde_json::from_str(&s).unwrap();
        }
        let json_ns = start.elapsed().as_nanos() as f64 / iters as f64;

        println!(
            "    × {}: BIN {:>6.0}ns | ASUN {:>6.0}ns | JSON {:>6.0}ns",
            iters, bin_ns, asun_ns, json_ns
        );
        println!(
            "    Speedup vs JSON: BIN {:.1}x faster | ASUN {:.1}x faster",
            json_ns / bin_ns,
            json_ns / asun_ns
        );
    }

    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║                    Benchmark Complete                        ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
}
