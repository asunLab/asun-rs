# asun

[![Crates.io](https://img.shields.io/crates/v/asun.svg)](https://crates.io/crates/asun)
[![Documentation](https://docs.rs/asun/badge.svg)](https://docs.rs/asun)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Rust support for [ASUN](https://github.com/asunLab/asun), a schema-driven format for compact structured data. Serde-free: types opt in with the crate's own `#[derive(AsunEncode, AsunDecode)]` macros.

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

- Serde-free — its own `#[derive(AsunEncode, AsunDecode)]` macros, no serde dependency
- Text and binary wire formats from a single pair of derives
- Zero-copy binary decode (`&str` fields borrow from the input)
- Current API uses `encode` / `decode`, not the older `to_string` / `from_str` names
- Optional scalar-hint schema output
- Pretty text output
- Works well for structs, vectors, options, enums, nested data, and entry-list based keyed collections
- serde-aligned field attributes: `rename`, `skip`, `skip_serializing`, `skip_deserializing`, `skip_serializing_if`, `default`

## Install

```toml
[dependencies]
asun = "1.2"
```

No `serde` in your dependency tree — ASUN ships its own derive macros. If you
already use `serde` elsewhere it stays independent; the two never interact.

## Usage

### 1. Derive the traits

You opt a type into ASUN with the crate's **own** derives — `#[derive(AsunEncode, AsunDecode)]`.
There is no `serde` involved: `AsunEncode` produces both the text and binary
*encoders*, and `AsunDecode` produces both the text and binary *decoders*, from a
single annotation. Derive only the direction you need (both is the common case).

```rust
use asun::{AsunEncode, AsunDecode};

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct User {
    id: i64,
    name: String,
    active: bool,
}
```

`AsunEncode` / `AsunDecode` are re-exported from the `asun` crate root, so a
single `use asun::{AsunEncode, AsunDecode};` brings in both the trait and the
matching derive macro — you do not depend on `asun-derive` directly.

### 2. Text: `encode` / `decode`

`encode` writes the compact schema-driven text; `decode` reads it back into your
type. The turbofish (`::<User>`) or an explicit binding tells `decode` what to
build.

```rust
use asun::{encode, decode};

let user = User { id: 1, name: "Alice".into(), active: true };

let text: String = encode(&user)?;            // "{id,name,active}:(1,Alice,true)"
let back: User   = decode(&text)?;            // decode infers the type from the binding
assert_eq!(user, back);
```

### 3. Self-describing text: `encode_typed`

`encode_typed` embeds scalar hints (`@int`, `@str`, `@bool`, …) in the schema so
the payload is readable without the Rust type on hand. `decode` accepts both the
plain and the annotated forms.

```rust
use asun::{encode_typed, decode};

let typed = encode_typed(&user)?;
assert_eq!(typed, "{id@int,name@str,active@bool}:(1,Alice,true)");

let back: User = decode(&typed)?;             // annotated text decodes the same way
assert_eq!(user, back);
```

### 4. Vectors, options, nested types

The same two calls work for any derived type — vectors share one schema across
all rows, `Option` maps to an empty slot, and nested structs/enums compose
automatically.

```rust
let users = vec![
    User { id: 1, name: "Alice".into(), active: true },
    User { id: 2, name: "Bob".into(),   active: false },
];

let text: String     = encode(&users)?;       // "[{id,name,active}]:(1,Alice,true),(2,Bob,false)"
let back: Vec<User>  = decode(&text)?;
assert_eq!(users, back);
```

Enums derive the same way and encode by variant:

```rust
#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
enum Event {
    Ping,
    Login { user: String },
    Score(u32),
}

let text = encode(&Event::Login { user: "Alice".into() })?;
let back: Event = decode(&text)?;
```

### 5. Pretty text: `encode_pretty` / `encode_pretty_typed`

For logs and human review, the pretty encoders add indentation and line breaks.
Output stays valid ASUN and round-trips through `decode`.

```rust
use asun::{encode_pretty, encode_pretty_typed, decode};

let pretty = encode_pretty(&users)?;          // indented, one row per line
let pretty_typed = encode_pretty_typed(&users)?;
let back: Vec<User> = decode(&pretty)?;
```

### 6. Binary: `encode_binary` / `decode_binary`

The binary format is the smallest and fastest wire form (LEB128 varints, zigzag
signed integers, fixed-width floats). It is schema-less: fields are read in
declaration order, so **encoder and decoder must share the same type definition.**
`&str` / `&[u8]` fields decode **zero-copy**, borrowing straight from the input
buffer.

```rust
use asun::{encode_binary, decode_binary};

let bytes: Vec<u8> = encode_binary(&users)?;
let back: Vec<User> = decode_binary(&bytes)?;
assert_eq!(users, back);
```

For hot loops or untrusted input there are two extra entry points:

```rust
use asun::{encode_binary_into, decode_binary_exact};

// Reuse one buffer across many encodes — keeps the allocation, no per-call Vec.
let mut buf = Vec::new();
encode_binary_into(&users, &mut buf)?;        // buf is cleared, then filled
encode_binary_into(&user,  &mut buf)?;        // same allocation reused

// Decode exactly one value and reject any trailing bytes (stricter than decode_binary).
let one: User = decode_binary_exact(&encode_binary(&user)?)?;
```

`decode_binary` caps decoded sequence lengths at `DEFAULT_MAX_SEQUENCE_LEN`
(16 MiB) as a guard against hostile length prefixes; construct a
`BinaryDecoder::with_max_sequence_len` if your protocol needs a different bound.

### 7. Errors

Every fallible call returns `asun::Result<T>` (alias for `Result<T, asun::Error>`).
`Error` implements `std::error::Error` + `Display`, so it slots into `?` and any
error-handling crate.

```rust
fn load(text: &str) -> asun::Result<Vec<User>> {
    let users: Vec<User> = decode(text)?;     // propagates asun::Error on malformed input
    Ok(users)
}
```

## API Reference

| Function                                | Purpose                                              |
| --------------------------------------- | ---------------------------------------------------- |
| `encode`                                | Encode to compact text                               |
| `encode_typed`                          | Encode to text with scalar type hints                |
| `decode`                                | Decode from text (plain or annotated)                |
| `encode_pretty` / `encode_pretty_typed` | Pretty (indented) text output                        |
| `encode_binary`                         | Encode to binary                                     |
| `encode_binary_into`                    | Encode to binary into a reused buffer                |
| `decode_binary`                         | Decode from binary (accepts trailing bytes)          |
| `decode_binary_exact`                   | Decode one binary value, reject trailing bytes       |

Traits & derives: `AsunEncode`, `AsunDecode` (text + binary via one derive each).
Types: `Error`, `Result<T>`, `DEFAULT_MAX_SEQUENCE_LEN`.

### Which format?

- **`encode` / `decode`** — human-readable, token-efficient text. Best for LLM
  prompts, APIs, config, anywhere you want to eyeball the payload.
- **`encode_typed`** — same text, but self-describing; use when the reader may
  not have the Rust type.
- **`encode_binary` / `decode_binary`** — smallest and fastest; use for storage
  and service-to-service traffic where both ends share the type.

## Field Attributes

Fields and enum variants accept `#[asun(...)]` attributes, aligned with serde:

| Attribute                          | Effect                                                                       |
| ---------------------------------- | ---------------------------------------------------------------------------- |
| `rename = "name"`                  | Use `name` on the wire instead of the Rust identifier                        |
| `skip`                             | Never written, never read; decodes to the default                            |
| `skip_serializing`                 | Not written; still read from text when present                               |
| `skip_deserializing`               | Not read; always decodes to the default                                      |
| `skip_serializing_if = "path"`     | Omit from **text** when the predicate `fn(&T) -> bool` returns `true`         |
| `default = "path"`                 | Value source `fn() -> T` for a field skipped on decode (else `Default`)      |

```rust
#[derive(AsunEncode, AsunDecode)]
struct Config {
    host: String,
    #[asun(rename = "p")]
    port: u16,
    #[asun(skip)]
    cached: Vec<u8>,
    #[asun(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}
```

**Binary note:** the binary format has no schema and reads fields in declaration
order, so any skip is forced **symmetric** — a field skipped on one side is
skipped on both. `skip_serializing_if` is ignored in binary (the field is always
written) to keep fixed-order decoding reliable. A custom `default` applies only
to fields skipped on decode (`skip` / `skip_deserializing`).

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
