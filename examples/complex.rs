use ason::{decode, decode_binary, encode, encode_binary, encode_typed};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ===========================================================================
// Basic types (existing)
// ===========================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Department {
    title: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Employee {
    id: i64,
    name: String,
    dept: Department,
    skills: Vec<String>,
    active: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct WithMap {
    name: String,
    attrs: HashMap<String, i64>,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Nested {
    name: String,
    addr: Address,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Address {
    city: String,
    zip: i64,
}

// ===========================================================================
// All-types struct — every primitive and compound type ASON supports
// ===========================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
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
    ch: char,
    s: String,
    opt_some: Option<i64>,
    opt_none: Option<i64>,
    vec_int: Vec<i64>,
    vec_str: Vec<String>,
    nested_vec: Vec<Vec<i64>>,
}

// ===========================================================================
// 5-level deep nesting: Country > Region > City > District > Street > Building
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Building {
    name: String,
    floors: i64,
    residential: bool,
    height_m: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Street {
    name: String,
    length_km: f64,
    buildings: Vec<Building>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct District {
    name: String,
    population: i64,
    streets: Vec<Street>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct City {
    name: String,
    population: i64,
    area_km2: f64,
    districts: Vec<District>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Region {
    name: String,
    cities: Vec<City>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Country {
    name: String,
    code: String,
    population: i64,
    gdp_trillion: f64,
    regions: Vec<Region>,
}

// ===========================================================================
// 7-level deep: Universe > Galaxy > SolarSystem > Planet > Continent > Nation > State
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct State {
    name: String,
    capital: String,
    population: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Nation {
    name: String,
    states: Vec<State>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Continent {
    name: String,
    nations: Vec<Nation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Planet {
    name: String,
    radius_km: f64,
    has_life: bool,
    continents: Vec<Continent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SolarSystem {
    name: String,
    star_type: String,
    planets: Vec<Planet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Galaxy {
    name: String,
    star_count_billions: f64,
    systems: Vec<SolarSystem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Universe {
    name: String,
    age_billion_years: f64,
    galaxies: Vec<Galaxy>,
}

// ===========================================================================
// Enum variants — all forms
// ===========================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum Color {
    Red,
    Green,
    Blue,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum Shape {
    Circle(f64),
    Rectangle(f64, f64),
    Named { name: String, sides: i64 },
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct Drawing {
    title: String,
    color: Color,
    shape: Shape,
    score: f64,
}

// ===========================================================================
// Large config-like struct with maps and optional fields
// ===========================================================================

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct DbConfig {
    host: String,
    port: i64,
    max_connections: i64,
    ssl: bool,
    timeout_ms: f64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct CacheConfig {
    enabled: bool,
    ttl_seconds: i64,
    max_size_mb: i64,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct LogConfig {
    level: String,
    file: Option<String>,
    rotate: bool,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct ServiceConfig {
    name: String,
    version: String,
    db: DbConfig,
    cache: CacheConfig,
    log: LogConfig,
    features: Vec<String>,
    env: HashMap<String, String>,
}

fn main() {
    println!("=== ASON Complex Examples ===\n");

    // -----------------------------------------------------------------------
    // 1. Nested struct (existing)
    // -----------------------------------------------------------------------
    println!("1. Nested struct:");
    let emp: Employee =
        decode("{id,name,dept:{title},skills,active}:(1,Alice,(Manager),[rust],true)").unwrap();
    println!("   {:?}\n", emp);

    // -----------------------------------------------------------------------
    // 2. Vec with nested structs (existing)
    // -----------------------------------------------------------------------
    println!("2. Vec with nested structs:");
    let input = "[{id:int,name:str,dept:{title:str},skills:[str],active:bool}]:
  (1, Alice, (Manager), [Rust, Go], true),
  (2, Bob, (Engineer), [Python], false),
  (3, \"Carol Smith\", (Director), [Leadership, Strategy], true)";
    let employees: Vec<Employee> = decode(input).unwrap();
    for e in &employees {
        println!("   {:?}", e);
    }

    // -----------------------------------------------------------------------
    // 3. Map/Dict field (existing)
    // -----------------------------------------------------------------------
    println!("\n3. Map/Dict field:");
    let input = "{name,attrs}:(Alice,[(age,30),(score,95)])";
    let item: WithMap = decode(input).unwrap();
    println!("   {:?}", item);

    // -----------------------------------------------------------------------
    // 4. Serialize nested struct roundtrip (existing)
    // -----------------------------------------------------------------------
    println!("\n4. Nested struct roundtrip:");
    let nested = Nested {
        name: "Alice".into(),
        addr: Address {
            city: "NYC".into(),
            zip: 10001,
        },
    };
    let s = encode(&nested).unwrap();
    println!("   serialized:   {}", s);
    let deserialized: Nested = decode(&s).unwrap();
    assert_eq!(nested, deserialized);
    println!("   ✓ roundtrip OK");

    // -----------------------------------------------------------------------
    // 5. Escaped strings (existing)
    // -----------------------------------------------------------------------
    println!("\n5. Escaped strings:");
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Note {
        text: String,
    }
    let note = Note {
        text: "say \"hi\", then (wave)\tnewline\nend".into(),
    };
    let s = encode(&note).unwrap();
    println!("   serialized:   {}", s);
    let note2: Note = decode(&s).unwrap();
    assert_eq!(note, note2);
    println!("   ✓ escape roundtrip OK");

    // -----------------------------------------------------------------------
    // 6. Float fields (existing)
    // -----------------------------------------------------------------------
    println!("\n6. Float fields:");
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Measurement {
        id: i64,
        value: f64,
        label: String,
    }
    let m = Measurement {
        id: 2,
        value: 95.0,
        label: "score".into(),
    };
    let s = encode(&m).unwrap();
    println!("   serialized: {}", s);
    let m2: Measurement = decode(&s).unwrap();
    assert_eq!(m, m2);
    println!("   ✓ float roundtrip OK");

    // -----------------------------------------------------------------------
    // 7. Negative numbers (existing)
    // -----------------------------------------------------------------------
    println!("\n7. Negative numbers:");
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Nums {
        a: i64,
        b: f64,
        c: i64,
    }
    let n = Nums {
        a: -42,
        b: -3.15,
        c: i64::MIN + 1,
    };
    let s = encode(&n).unwrap();
    println!("   serialized:   {}", s);
    let n2: Nums = decode(&s).unwrap();
    assert_eq!(n, n2);
    println!("   ✓ negative roundtrip OK");

    // -----------------------------------------------------------------------
    // 8. All types struct
    // -----------------------------------------------------------------------
    println!("\n8. All types struct:");
    let all = AllTypes {
        b: true,
        i8v: -128,
        i16v: -32768,
        i32v: -2147483648,
        i64v: -9223372036854775807,
        u8v: 255,
        u16v: 65535,
        u32v: 4294967295,
        u64v: 18446744073709551615,
        f32v: 3.15,
        f64v: 2.718281828459045,
        ch: 'Z',
        s: "hello, world (test) [arr]".into(),
        opt_some: Some(42),
        opt_none: None,
        vec_int: vec![1, 2, 3, -4, 0],
        vec_str: vec!["alpha".into(), "beta gamma".into(), "delta".into()],
        nested_vec: vec![vec![1, 2], vec![3, 4, 5]],
    };
    let s = encode(&all).unwrap();
    println!("   serialized ({} bytes):", s.len());
    println!("   {}", s);
    let all2: AllTypes = decode(&s).unwrap();
    assert_eq!(all, all2);
    println!("   ✓ all-types roundtrip OK");

    // -----------------------------------------------------------------------
    // 9. Enum variants — unit, newtype, tuple, struct
    // -----------------------------------------------------------------------
    println!("\n9. Enum variants:");
    let d1 = Drawing {
        title: "Circle".into(),
        color: Color::Red,
        shape: Shape::Circle(5.0),
        score: 9.5,
    };
    let s1 = encode(&d1).unwrap();
    println!("   unit+newtype: {}", s1);
    let d1b: Drawing = decode(&s1).unwrap();
    assert_eq!(d1, d1b);

    let d2 = Drawing {
        title: "Rect".into(),
        color: Color::Blue,
        shape: Shape::Rectangle(3.0, 4.0),
        score: 8.2,
    };
    let s2 = encode(&d2).unwrap();
    println!("   tuple variant: {}", s2);
    let d2b: Drawing = decode(&s2).unwrap();
    assert_eq!(d2, d2b);

    let d3 = Drawing {
        title: "Polygon".into(),
        color: Color::Green,
        shape: Shape::Named {
            name: "hexagon".into(),
            sides: 6,
        },
        score: 7.0,
    };
    let s3 = encode(&d3).unwrap();
    println!("   struct variant: {}", s3);
    let d3b: Drawing = decode(&s3).unwrap();
    assert_eq!(d3, d3b);
    println!("   ✓ all enum variants roundtrip OK");

    // -----------------------------------------------------------------------
    // 10. 5-level deep: Country > Region > City > District > Street > Building
    // -----------------------------------------------------------------------
    println!("\n10. Five-level nesting (Country>Region>City>District>Street>Building):");
    let country = Country {
        name: "Rustland".into(),
        code: "RL".into(),
        population: 50_000_000,
        gdp_trillion: 1.5,
        regions: vec![
            Region {
                name: "Northern".into(),
                cities: vec![City {
                    name: "Ferriton".into(),
                    population: 2_000_000,
                    area_km2: 350.5,
                    districts: vec![
                        District {
                            name: "Downtown".into(),
                            population: 500_000,
                            streets: vec![
                                Street {
                                    name: "Main St".into(),
                                    length_km: 2.5,
                                    buildings: vec![
                                        Building {
                                            name: "Tower A".into(),
                                            floors: 50,
                                            residential: false,
                                            height_m: 200.0,
                                        },
                                        Building {
                                            name: "Apt Block 1".into(),
                                            floors: 12,
                                            residential: true,
                                            height_m: 40.5,
                                        },
                                    ],
                                },
                                Street {
                                    name: "Oak Ave".into(),
                                    length_km: 1.2,
                                    buildings: vec![Building {
                                        name: "Library".into(),
                                        floors: 3,
                                        residential: false,
                                        height_m: 15.0,
                                    }],
                                },
                            ],
                        },
                        District {
                            name: "Harbor".into(),
                            population: 150_000,
                            streets: vec![Street {
                                name: "Dock Rd".into(),
                                length_km: 0.8,
                                buildings: vec![Building {
                                    name: "Warehouse 7".into(),
                                    floors: 1,
                                    residential: false,
                                    height_m: 8.0,
                                }],
                            }],
                        },
                    ],
                }],
            },
            Region {
                name: "Southern".into(),
                cities: vec![City {
                    name: "Crabville".into(),
                    population: 800_000,
                    area_km2: 120.0,
                    districts: vec![District {
                        name: "Old Town".into(),
                        population: 200_000,
                        streets: vec![Street {
                            name: "Heritage Ln".into(),
                            length_km: 0.5,
                            buildings: vec![
                                Building {
                                    name: "Museum".into(),
                                    floors: 2,
                                    residential: false,
                                    height_m: 12.0,
                                },
                                Building {
                                    name: "Town Hall".into(),
                                    floors: 4,
                                    residential: false,
                                    height_m: 20.0,
                                },
                            ],
                        }],
                    }],
                }],
            },
        ],
    };
    let s = encode(&country).unwrap();
    println!("   serialized ({} bytes)", s.len());
    println!("   first 200 chars: {}...", &s[..200.min(s.len())]);
    let country2: Country = decode(&s).unwrap();
    assert_eq!(country, country2);
    println!("   ✓ 5-level ASON-text roundtrip OK");

    // ASON binary roundtrip
    let bin = encode_binary(&country).unwrap();
    let country3: Country = decode_binary(&bin).unwrap();
    assert_eq!(country, country3);
    println!("   ✓ 5-level ASON-bin roundtrip OK");

    // Size comparison: ASON-text vs ASON-bin vs JSON
    let json = serde_json::to_string(&country).unwrap();
    println!(
        "   ASON text: {} B | ASON bin: {} B | JSON: {} B",
        s.len(),
        bin.len(),
        json.len()
    );
    println!(
        "   BIN vs JSON: {:.0}% smaller | TEXT vs JSON: {:.0}% smaller",
        (1.0 - bin.len() as f64 / json.len() as f64) * 100.0,
        (1.0 - s.len() as f64 / json.len() as f64) * 100.0
    );

    // -----------------------------------------------------------------------
    // 11. 7-level deep: Universe > Galaxy > SolarSystem > Planet > Continent > Nation > State
    // -----------------------------------------------------------------------
    println!(
        "\n11. Seven-level nesting (Universe>Galaxy>SolarSystem>Planet>Continent>Nation>State):"
    );
    let universe = Universe {
        name: "Observable".into(),
        age_billion_years: 13.8,
        galaxies: vec![Galaxy {
            name: "Milky Way".into(),
            star_count_billions: 250.0,
            systems: vec![SolarSystem {
                name: "Sol".into(),
                star_type: "G2V".into(),
                planets: vec![
                    Planet {
                        name: "Earth".into(),
                        radius_km: 6371.0,
                        has_life: true,
                        continents: vec![
                            Continent {
                                name: "Asia".into(),
                                nations: vec![
                                    Nation {
                                        name: "Japan".into(),
                                        states: vec![
                                            State {
                                                name: "Tokyo".into(),
                                                capital: "Shinjuku".into(),
                                                population: 14_000_000,
                                            },
                                            State {
                                                name: "Osaka".into(),
                                                capital: "Osaka City".into(),
                                                population: 8_800_000,
                                            },
                                        ],
                                    },
                                    Nation {
                                        name: "China".into(),
                                        states: vec![State {
                                            name: "Beijing".into(),
                                            capital: "Beijing".into(),
                                            population: 21_500_000,
                                        }],
                                    },
                                ],
                            },
                            Continent {
                                name: "Europe".into(),
                                nations: vec![Nation {
                                    name: "Germany".into(),
                                    states: vec![
                                        State {
                                            name: "Bavaria".into(),
                                            capital: "Munich".into(),
                                            population: 13_000_000,
                                        },
                                        State {
                                            name: "Berlin".into(),
                                            capital: "Berlin".into(),
                                            population: 3_600_000,
                                        },
                                    ],
                                }],
                            },
                        ],
                    },
                    Planet {
                        name: "Mars".into(),
                        radius_km: 3389.5,
                        has_life: false,
                        continents: vec![],
                    },
                ],
            }],
        }],
    };
    let s = encode(&universe).unwrap();
    println!("   serialized ({} bytes)", s.len());
    let universe2: Universe = decode(&s).unwrap();
    assert_eq!(universe, universe2);
    println!("   ✓ 7-level ASON-text roundtrip OK");

    // ASON binary roundtrip
    let bin = encode_binary(&universe).unwrap();
    let universe3: Universe = decode_binary(&bin).unwrap();
    assert_eq!(universe, universe3);
    println!("   ✓ 7-level ASON-bin roundtrip OK");

    let json = serde_json::to_string(&universe).unwrap();
    println!(
        "   ASON text: {} B | ASON bin: {} B | JSON: {} B",
        s.len(),
        bin.len(),
        json.len()
    );
    println!(
        "   BIN vs JSON: {:.0}% smaller | TEXT vs JSON: {:.0}% smaller",
        (1.0 - bin.len() as f64 / json.len() as f64) * 100.0,
        (1.0 - s.len() as f64 / json.len() as f64) * 100.0
    );

    // -----------------------------------------------------------------------
    // 12. Service config with maps + optional + nested
    // -----------------------------------------------------------------------
    println!("\n12. Complex config struct (nested + map + optional):");
    let mut env = HashMap::new();
    env.insert("RUST_LOG".into(), "debug".into());
    env.insert(
        "DATABASE_URL".into(),
        "postgres://localhost:5432/mydb".into(),
    );
    env.insert("SECRET_KEY".into(), "abc123!@#".into());

    let config = ServiceConfig {
        name: "my-service".into(),
        version: "2.1.0".into(),
        db: DbConfig {
            host: "db.example.com".into(),
            port: 5432,
            max_connections: 100,
            ssl: true,
            timeout_ms: 3000.5,
        },
        cache: CacheConfig {
            enabled: true,
            ttl_seconds: 3600,
            max_size_mb: 512,
        },
        log: LogConfig {
            level: "info".into(),
            file: Some("/var/log/app.log".into()),
            rotate: true,
        },
        features: vec!["auth".into(), "rate-limit".into(), "websocket".into()],
        env,
    };
    let s = encode(&config).unwrap();
    println!("   serialized ({} bytes):", s.len());
    println!("   {}", s);
    let config2: ServiceConfig = decode(&s).unwrap();
    assert_eq!(config, config2);
    println!("   ✓ config roundtrip OK");

    let json = serde_json::to_string(&config).unwrap();
    println!(
        "   ASON text: {} B | JSON: {} B | TEXT vs JSON: {:.0}% smaller",
        s.len(),
        json.len(),
        (1.0 - s.len() as f64 / json.len() as f64) * 100.0
    );
    // Binary roundtrip
    let bin = encode_binary(&config).unwrap();
    let config3: ServiceConfig = decode_binary(&bin).unwrap();
    assert_eq!(config, config3);
    println!("   ✓ config ASON-bin roundtrip OK");
    println!(
        "   ASON bin: {} B | BIN vs JSON: {:.0}% smaller",
        bin.len(),
        (1.0 - bin.len() as f64 / json.len() as f64) * 100.0
    );

    // -----------------------------------------------------------------------
    // 13. Large structure — 100 countries, each with regions/cities/etc
    // -----------------------------------------------------------------------
    println!("\n13. Large structure (100 countries × nested regions):");
    let countries: Vec<Country> = (0..100)
        .map(|i| Country {
            name: format!("Country_{}", i),
            code: format!("C{:02}", i % 100),
            population: 1_000_000 + i * 500_000,
            gdp_trillion: (i as f64) * 0.5,
            regions: (0..3)
                .map(|r| Region {
                    name: format!("Region_{}_{}", i, r),
                    cities: (0..2)
                        .map(|c| City {
                            name: format!("City_{}_{}_{}", i, r, c),
                            population: 100_000 + c * 50_000,
                            area_km2: 50.0 + (c as f64) * 25.5,
                            districts: vec![District {
                                name: format!("Dist_{}", c),
                                population: 50_000 + c * 10_000,
                                streets: vec![Street {
                                    name: format!("St_{}", c),
                                    length_km: 1.0 + c as f64 * 0.5,
                                    buildings: (0..2)
                                        .map(|b| Building {
                                            name: format!("Bldg_{}_{}", c, b),
                                            floors: 5 + b * 3,
                                            residential: b % 2 == 0,
                                            height_m: 15.0 + b as f64 * 10.5,
                                        })
                                        .collect(),
                                }],
                            }],
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect();

    // Serialize each country individually and measure total
    let mut total_ason_bytes = 0usize;
    let mut total_json_bytes = 0usize;
    let mut total_bin_bytes = 0usize;
    for c in &countries {
        let s = encode(c).unwrap();
        let j = serde_json::to_string(c).unwrap();
        let b = encode_binary(c).unwrap();
        // Verify text roundtrip
        let c2: Country = decode(&s).unwrap();
        assert_eq!(c, &c2);
        // Verify binary roundtrip
        let c3: Country = decode_binary(&b).unwrap();
        assert_eq!(c, &c3);
        total_ason_bytes += s.len();
        total_json_bytes += j.len();
        total_bin_bytes += b.len();
    }
    println!("   100 countries with 5-level nesting:");
    println!(
        "   Total ASON text: {} bytes ({:.1} KB)",
        total_ason_bytes,
        total_ason_bytes as f64 / 1024.0
    );
    println!(
        "   Total ASON bin:  {} bytes ({:.1} KB)",
        total_bin_bytes,
        total_bin_bytes as f64 / 1024.0
    );
    println!(
        "   Total JSON:      {} bytes ({:.1} KB)",
        total_json_bytes,
        total_json_bytes as f64 / 1024.0
    );
    println!(
        "   TEXT vs JSON: {:.0}% smaller | BIN vs JSON: {:.0}% smaller",
        (1.0 - total_ason_bytes as f64 / total_json_bytes as f64) * 100.0,
        (1.0 - total_bin_bytes as f64 / total_json_bytes as f64) * 100.0
    );
    println!("   ✓ all 100 countries roundtrip OK (text + bin)");

    // -----------------------------------------------------------------------
    // 14. Deserialize from ASON with deeply nested schema type hints
    // -----------------------------------------------------------------------
    println!("\n14. Deserialize with nested schema type hints:");
    let input = "{name:str,code:str,population:int,gdp_trillion:float,regions:[{name:str,cities:[{name:str,population:int,area_km2:float,districts:[{name:str,population:int,streets:[{name:str,length_km:float,buildings:[{name:str,floors:int,residential:bool,height_m:float}]}]}]}]}]}:(TestLand,TL,1000000,0.5,[(TestRegion,[(TestCity,500000,100.0,[(Central,250000,[(Main St,2.5,[(HQ,10,false,45.0)])])])])])";
    let c: Country = decode(input).unwrap();
    assert_eq!(c.name, "TestLand");
    assert_eq!(
        c.regions[0].cities[0].districts[0].streets[0].buildings[0].name,
        "HQ"
    );
    println!("   ✓ deep schema type-hint parse OK");
    println!(
        "   Building at depth 6: {:?}",
        c.regions[0].cities[0].districts[0].streets[0].buildings[0]
    );

    // -----------------------------------------------------------------------
    // 15. Typed serialization (encode_typed)
    // -----------------------------------------------------------------------
    println!("\n15. Typed serialization (encode_typed):");

    // Simple struct
    let user_typed = encode_typed(&Employee {
        id: 1,
        name: "Alice".into(),
        dept: Department {
            title: "Engineering".into(),
        },
        skills: vec!["Rust".into(), "Go".into()],
        active: true,
    })
    .unwrap();
    println!("   nested struct: {}", user_typed);
    let emp_back: Employee = decode(&user_typed).unwrap();
    assert_eq!(emp_back.name, "Alice");
    println!("   ✓ typed nested struct roundtrip OK");

    // All-types struct
    let all_typed = encode_typed(&all).unwrap();
    println!(
        "   all-types ({} bytes): {}...",
        all_typed.len(),
        &all_typed[..80.min(all_typed.len())]
    );
    let all_back: AllTypes = decode(&all_typed).unwrap();
    assert_eq!(all, all_back);
    println!("   ✓ typed all-types roundtrip OK");

    // Config struct
    let config_typed = encode_typed(&config).unwrap();
    println!(
        "   config ({} bytes): {}...",
        config_typed.len(),
        &config_typed[..100.min(config_typed.len())]
    );
    let config_back: ServiceConfig = decode(&config_typed).unwrap();
    assert_eq!(config, config_back);
    println!("   ✓ typed config roundtrip OK");

    // Compare typed vs untyped output
    let untyped = encode(&config).unwrap();
    println!(
        "   untyped schema: {} bytes | typed schema: {} bytes | overhead: {} bytes",
        untyped.len(),
        config_typed.len(),
        config_typed.len() - untyped.len()
    );

    // -----------------------------------------------------------------------
    // 16. Edge cases — empty collections, special chars
    // -----------------------------------------------------------------------
    println!("\n16. Edge cases:");

    // Empty vec
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct WithVec {
        items: Vec<i64>,
    }
    let wv = WithVec { items: vec![] };
    let s = encode(&wv).unwrap();
    println!("   empty vec: {}", s);
    let wv2: WithVec = decode(&s).unwrap();
    assert_eq!(wv, wv2);

    // String with all special chars
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Special {
        val: String,
    }
    let sp = Special {
        val: "tabs\there, newlines\nhere, quotes\"and\\backslash".into(),
    };
    let s = encode(&sp).unwrap();
    println!("   special chars: {}", s);
    let sp2: Special = decode(&s).unwrap();
    assert_eq!(sp, sp2);

    // Boolean edge
    let sp3 = Special { val: "true".into() };
    let s = encode(&sp3).unwrap();
    println!("   bool-like string: {}", s);
    let sp4: Special = decode(&s).unwrap();
    assert_eq!(sp3, sp4);

    // Number-like string
    let sp5 = Special {
        val: "12345".into(),
    };
    let s = encode(&sp5).unwrap();
    println!("   number-like string: {}", s);
    let sp6: Special = decode(&s).unwrap();
    assert_eq!(sp5, sp6);

    println!("   ✓ all edge cases OK");

    // -----------------------------------------------------------------------
    // 17. Array of arrays of arrays (3-level)
    // -----------------------------------------------------------------------
    println!("\n17. Triple-nested arrays:");
    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Matrix3D {
        data: Vec<Vec<Vec<i64>>>,
    }
    let m3 = Matrix3D {
        data: vec![vec![vec![1, 2], vec![3, 4]], vec![vec![5, 6, 7], vec![8]]],
    };
    let s = encode(&m3).unwrap();
    println!("   {}", s);
    let m3b: Matrix3D = decode(&s).unwrap();
    assert_eq!(m3, m3b);
    println!("   ✓ triple-nested array roundtrip OK");

    // -----------------------------------------------------------------------
    // 18. Comments in ASON
    // -----------------------------------------------------------------------
    println!("\n18. Comments:");
    let _input = "/* Top-level comment */
[{id,name,active}]:
  /* row 1 */ (1, Alice, true)";
    let emp: Employee =
        decode("{id,name,dept:{title},skills,active}:/* inline */ (1,Alice,(HR),[rust],true)")
            .unwrap();
    println!("   with inline comment: {:?}", emp);
    println!("   ✓ comment parsing OK");

    // -----------------------------------------------------------------------
    // Summary
    // -----------------------------------------------------------------------
    println!("\n=== All {} complex examples passed! ===", 18);
}
