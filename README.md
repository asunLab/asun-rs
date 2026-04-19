# asun

[![Crates.io](https://img.shields.io/crates/v/asun.svg)](https://crates.io/crates/asun)
[![Documentation](https://docs.rs/asun/badge.svg)](https://docs.rs/asun)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust support for [ASUN](https://github.com/asunLab/asun), a schema-driven format for compact structured data with serde-based encoding and decoding.

[中文文档](https://github.com/asunLab/asun-rs/blob/main/README_CN.md)

## Why ASUN?

**json**

Standard JSON repeats every field name in every record. When you send structured data to an LLM, over an API, or across services, that repetition wastes tokens, bytes, and attention:

```json
[
  { "id": 1, "name": "Alice", "active": true },
  { "id": 2, "name": "Bob", "active": false },
  { "id": 3, "name": "Carol", "active": true }
]
```

**asun**

ASUN declares the schema **once** and streams data as compact tuples:

```asun
[{id, name, active}]:
  (1,Alice,true),
  (2,Bob,false),
  (3,Carol,true)
```

**Fewer tokens. Smaller payloads. Clearer structure, and faster parsing than repeated-object JSON.**

---

## Highlights

- Serde-based text encoding and decoding
- Current API uses `encode` / `decode`, not the older `to_string` / `from_str` names
- Optional scalar-hint schema output
- Pretty text output and binary format
- Works well for structs, vectors, options, enums, nested data, and entry-list based keyed collections

## Install

```toml
[dependencies]
asun = "*"
serde = { version = "1", features = ["derive"] }
```

## Quick Start

```rust
use asun::{decode, encode, encode_typed};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

fn main() -> asun::Result<()> {
    let user = User { id: 1, name: "Alice".into(), active: true };

    let text = encode(&user)?;
    let typed = encode_typed(&user)?;
    let decoded: User = decode(&text)?;

    assert_eq!(decoded.id, 1);
    assert_eq!(typed, "{id@int,name@str,active@bool}:(1,Alice,true)");
    Ok(())
}
```

### Encode a vector

```rust
let users = vec![
    User { id: 1, name: "Alice".into(), active: true },
    User { id: 2, name: "Bob".into(), active: false },
];

let text = encode(&users)?;
let typed = encode_typed(&users)?;
let decoded: Vec<User> = decode(&text)?;
```

### Pretty and binary output

```rust
use asun::{decode_binary, encode_binary, encode_pretty, encode_pretty_typed};

let pretty = encode_pretty(&users)?;
let pretty_typed = encode_pretty_typed(&users)?;
let bin = encode_binary(&users)?;
let decoded: Vec<User> = decode_binary(&bin)?;
```

## Current API

| Function                                | Purpose            |
| --------------------------------------- | ------------------ |
| `encode` / `encode_typed`               | Encode to text     |
| `decode`                                | Decode from text   |
| `encode_pretty` / `encode_pretty_typed` | Pretty text output |
| `encode_binary`                         | Encode to binary   |
| `decode_binary`                         | Decode from binary |

## Run Examples

```bash
cargo test
cargo run --example basic
cargo run --example complex
cargo run --example bench
```

## Contributors

- [Athan](https://github.com/athxx)

## Benchmark Snapshot

Run the benchmark example with:

```bash
cargo run --example bench --release
```

The Rust benchmark now uses the same two-line summary style as the Go example:

```text
Flat struct × 1000 (8 fields, vec)
  Serialize:   JSON   411.05ms /   121675 B | ASUN   175.25ms (2.3x) /    56718 B (46.6%) | BIN    41.32ms (9.9x) /    74454 B (61.2%)
  Deserialize: JSON   287.06ms | ASUN   195.57ms (1.5x) | BIN    64.62ms (4.4x)
```

`ASUN` / `BIN` ratios are measured against JSON, and size percentages show the remaining size relative to JSON.

## License

MIT
