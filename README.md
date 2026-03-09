# ason

[![Crates.io](https://img.shields.io/crates/v/ason.svg)](https://crates.io/crates/ason)
[![Documentation](https://docs.rs/ason/badge.svg)](https://docs.rs/ason)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust support for [ASON](https://github.com/ason-lab/ason), a schema-driven format for compact structured data with serde-based encoding and decoding.

[中文文档](README_CN.md)

## Why ASON

ASON writes schema once and keeps each row positional:

```json
[
  {"id": 1, "name": "Alice", "active": true},
  {"id": 2, "name": "Bob", "active": false}
]
```

```text
[{id:int,name:str,active:bool}]:(1,Alice,true),(2,Bob,false)
```

That makes repeated records shorter and easier to transport or feed into models.

## Highlights

- Serde-based text encoding and decoding
- Current API uses `encode` / `decode`, not the older `to_string` / `from_str` names
- Optional typed schema output
- Pretty text output and binary format
- Works well for structs, vectors, options, maps, enums, and nested data

## Install

```toml
[dependencies]
ason = "0.1"
serde = { version = "1", features = ["derive"] }
```

## Quick Start

```rust
use ason::{decode, encode, encode_typed};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

fn main() -> ason::Result<()> {
    let user = User { id: 1, name: "Alice".into(), active: true };

    let text = encode(&user)?;
    let typed = encode_typed(&user)?;
    let decoded: User = decode(&text)?;

    assert_eq!(decoded.id, 1);
    assert_eq!(typed, "{id:int,name:str,active:bool}:(1,Alice,true)");
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
use ason::{decode_binary, encode_binary, encode_pretty, encode_pretty_typed};

let pretty = encode_pretty(&users)?;
let pretty_typed = encode_pretty_typed(&users)?;
let bin = encode_binary(&users)?;
let decoded: Vec<User> = decode_binary(&bin)?;
```

## Current API

| Function | Purpose |
| --- | --- |
| `encode` / `encode_typed` | Encode to text |
| `decode` | Decode from text |
| `encode_pretty` / `encode_pretty_typed` | Pretty text output |
| `encode_binary` | Encode to binary |
| `decode_binary` | Decode from binary |

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

Measured on this machine with:

```bash
CARGO_BUILD_RUSTC_WRAPPER='' CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc RUSTFLAGS='' cargo run --example bench --release
```

Headline numbers:

- Flat 1,000-record dataset: ASON serialize `57.46ms` vs JSON `73.06ms`, deserialize `84.37ms` vs JSON `111.17ms`
- Flat 10,000-record dataset: ASON serialize `46.91ms` vs JSON `57.75ms`, deserialize `72.26ms` vs JSON `93.22ms`
- Deep 100-record dataset: ASON serialize `234.43ms` vs JSON `252.71ms`, deserialize `211.58ms` vs JSON `313.06ms`
- Throughput summary on 1,000 records: ASON text was `1.12x` faster than JSON for serialize and `1.34x` faster for deserialize
- Binary summary on 1,000 flat records: BIN serialize `2.13ms` vs JSON `12.79ms`, BIN deserialize `5.86ms` vs JSON `19.87ms`

## License

MIT
