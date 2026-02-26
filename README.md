# ason

[![Crates.io](https://img.shields.io/crates/v/ason.svg)](https://crates.io/crates/ason)
[![Documentation](https://docs.rs/ason/badge.svg)](https://docs.rs/ason)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A high-performance [serde](https://serde.rs/) serialization/deserialization library for [ASON](https://github.com/athxx/ason) (Array-Schema Object Notation) — a token-efficient, schema-driven data format designed for LLM interactions and large-scale data transmission.

[中文文档](README_CN.md)

## What is ASON?

ASON separates **schema** from **data**, eliminating repetitive keys found in JSON. The schema is declared once, and data rows carry only values:

```text
JSON (100 tokens):
{"users":[{"id":1,"name":"Alice","active":true},{"id":2,"name":"Bob","active":false}]}

ASON (~35 tokens, 65% saving):
[{id:int, name:str, active:bool}]:(1,Alice,true),(2,Bob,false)
```

| Aspect              | JSON         | ASON             |
| ------------------- | ------------ | ---------------- |
| Token efficiency    | 100%         | 30–70% ✓         |
| Key repetition      | Every object | Declared once ✓  |
| Human readable      | Yes          | Yes ✓            |
| Nested structs      | ✓            | ✓                |
| Type annotations    | No           | Optional ✓       |
| Serialization speed | 1x           | **~2x faster** ✓ |
| Data size           | 100%         | **40–50%** ✓     |

## Quick Start

Add to your `Cargo.toml`:

```toml
[dependencies]
ason = "0.1"
```

### Serialize & Deserialize a Struct

```rust
use ason::{to_string, from_str};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

fn main() -> ason::Result<()> {
    let user = User { id: 1, name: "Alice".into(), active: true };

    // Serialize
    let s = to_string(&user)?;
    assert_eq!(s, "{id,name,active}:(1,Alice,true)");

    // Deserialize
    let user2: User = from_str(&s)?;
    assert_eq!(user, user2);
    Ok(())
}
```

### Serialize with Type Annotations

Use `to_string_typed` to output a type-annotated schema — useful for documentation, LLM prompts, and cross-language exchange:

```rust
use ason::{to_string_typed, from_str};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

fn main() -> ason::Result<()> {
    let user = User { id: 1, name: "Alice".into(), active: true };

    let s = to_string_typed(&user)?;
    assert_eq!(s, "{id:int,name:str,active:bool}:(1,Alice,true)");

    // Deserializer accepts both annotated and unannotated schemas
    let user2: User = from_str(&s)?;
    assert_eq!(user, user2);
    Ok(())
}
```

### Serialize & Deserialize a Vec (Schema-Driven)

For `Vec<T>`, ASON writes the schema **once** and emits each element as a compact tuple — the key advantage over JSON:

```rust
use ason::{to_string_vec, from_str_vec, StructSchema};
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
struct User {
    id: i64,
    name: String,
    active: bool,
}

impl StructSchema for User {
    fn field_names() -> &'static [&'static str] {
        &["id", "name", "active"]
    }
    fn field_types() -> &'static [&'static str] {
        &["int", "str", "bool"]  // enables to_string_vec_typed()
    }
    fn serialize_fields(&self, ser: &mut ason::serialize::Serializer) -> ason::Result<()> {
        use serde::Serialize;
        self.id.serialize(&mut *ser)?;
        self.name.serialize(&mut *ser)?;
        self.active.serialize(&mut *ser)?;
        Ok(())
    }
}

fn main() -> ason::Result<()> {
    let users = vec![
        User { id: 1, name: "Alice".into(), active: true },
        User { id: 2, name: "Bob".into(), active: false },
    ];

    // Unannotated schema
    let s = to_string_vec(&users)?;
    assert_eq!(s, "[{id,name,active}]:(1,Alice,true),(2,Bob,false)");

    // Type-annotated schema (requires field_types())
    use ason::to_string_vec_typed;
    let s2 = to_string_vec_typed(&users)?;
    assert_eq!(s2, "[{id:int,name:str,active:bool}]:(1,Alice,true),(2,Bob,false)");

    // Deserialize — accepts both forms
    let users2: Vec<User> = from_str_vec(&s)?;
    let users3: Vec<User> = from_str_vec(&s2)?;
    assert_eq!(users2.len(), 2);
    assert_eq!(users3.len(), 2);
    Ok(())
}
```

## Supported Types

| Type                      | ASON Representation             | Example                       |
| ------------------------- | ------------------------------- | ----------------------------- |
| Integers (i8–i64, u8–u64) | Plain number                    | `42`, `-100`                  |
| Floats (f32, f64)         | Decimal number                  | `3.14`, `-0.5`                |
| Bool                      | Literal                         | `true`, `false`               |
| String                    | Unquoted or quoted              | `Alice`, `"Carol Smith"`      |
| Option\<T\>               | Value or empty                  | `hello` or _(blank)_ for None |
| Vec\<T\>                  | `[v1,v2,v3]`                    | `[rust,go,python]`            |
| HashMap\<K,V\>            | `[(k1,v1),(k2,v2)]`             | `[(age,30),(score,95)]`       |
| Nested struct             | `(field1,field2)`               | `(Engineering,500000)`        |
| Enum variants             | Unit / Newtype / Tuple / Struct | `Red`, `Circle(5.0)`          |
| Char                      | Single character                | `A`                           |

### Nested Structs

```rust
#[derive(Serialize, Deserialize)]
struct Dept { title: String }

#[derive(Serialize, Deserialize)]
struct Employee { name: String, dept: Dept }

// Schema reflects nesting:
// {name:str,dept:{title:str}}:(Alice,(Engineering))
```

### Optional Fields

```rust
#[derive(Deserialize)]
struct Item { id: i64, label: Option<String> }

// With value:   {id:int,label:str}:(1,hello)
// With None:    {id:int,label:str}:(1,)
```

### Arrays & Maps

```rust
#[derive(Deserialize)]
struct Tagged { name: String, tags: Vec<String> }
// {name:str,tags:[str]}:(Alice,[rust,go,python])

use std::collections::HashMap;
#[derive(Deserialize)]
struct Profile { name: String, attrs: HashMap<String, i64> }
// {name:str,attrs:map[str,int]}:(Alice,[(age,30),(score,95)])
```

### Type Annotations (Optional)

ASON schema supports **optional** type annotations. Both forms are fully equivalent — the deserializer handles them identically:

```text
// Without annotations (default output of to_string / to_string_vec)
{id,name,salary,active}:(1,Alice,5000.50,true)

// With annotations (output of to_string_typed / to_string_vec_typed)
{id:int,name:str,salary:float,active:bool}:(1,Alice,5000.50,true)
```

Annotations are **purely decorative metadata** — they do not affect parsing or deserialization behavior. The deserializer simply skips the `:type` portion when present.

**When to use annotations:**

- LLM prompts — helps models understand and generate correct data
- API documentation — self-describing schema without external docs
- Cross-language exchange — eliminates type ambiguity (is `42` an int or float?)
- Debugging — see data types at a glance

**Performance impact:** negligible (<0.1% for Vec, ~3% for single struct). The overhead is constant and does not grow with data volume — annotations only affect the schema header, not the data body.

### Comments

```text
/* user list */
[{id:int, name:str, active:bool}]:(1,Alice,true),(2,Bob,false)
```

### Multiline Format

```text
[{id:int, name:str, active:bool}]:
  (1, Alice, true),
  (2, Bob, false),
  (3, "Carol Smith", true)
```

## API Reference

| Function                    | Description                                                    |
| --------------------------- | -------------------------------------------------------------- |
| `to_string(&T)`             | Serialize a struct → unannotated schema `{id,name}:`           |
| `to_string_typed(&T)`       | Serialize a struct → annotated schema `{id:int,name:str}:`     |
| `from_str::<T>(s)`          | Deserialize a struct (accepts both annotated and unannotated)  |
| `to_string_vec(&[T])`       | Serialize a Vec → unannotated schema (requires `StructSchema`) |
| `to_string_vec_typed(&[T])` | Serialize a Vec → annotated schema (requires `StructSchema`)   |
| `from_str_vec::<T>(s)`      | Deserialize a Vec (accepts both annotated and unannotated)     |

## Performance

Benchmarked on Apple Silicon (M-series), release mode, comparing against `serde_json`:

### Serialization (ASON is 1.8–2.4x faster)

| Scenario            | JSON    | ASON    | Speedup   |
| ------------------- | ------- | ------- | --------- |
| Flat struct × 1000  | 11.2 ms | 5.2 ms  | **2.16x** |
| 5-level deep × 100  | 41.2 ms | 22.1 ms | **1.86x** |
| Large payload (10k) | 10.9 ms | 5.0 ms  | **2.19x** |

### Deserialization (ASON is 1.1–1.3x faster)

| Scenario            | JSON    | ASON    | Speedup   |
| ------------------- | ------- | ------- | --------- |
| Flat struct × 1000  | 29.6 ms | 24.9 ms | **1.19x** |
| 5-level deep × 100  | 93.2 ms | 84.5 ms | **1.10x** |
| Large payload (10k) | 29.9 ms | 25.9 ms | **1.16x** |

### Size Savings

| Scenario           | JSON   | ASON   | Saving  |
| ------------------ | ------ | ------ | ------- |
| Flat struct × 1000 | 121 KB | 57 KB  | **53%** |
| 5-level deep × 100 | 438 KB | 175 KB | **60%** |
| 10k records        | 1.2 MB | 0.6 MB | **53%** |

### Why is ASON Faster?

1. **Zero key-hashing** — Schema is parsed once; data fields are mapped by position index `O(1)`, no per-row key string hashing.
2. **Schema-driven parsing** — The deserializer knows the expected type of each field from the schema, enabling direct parsing (`parse_int()`) instead of runtime type inference. CPU branch prediction hits ~100%.
3. **Minimal memory allocation** — All data rows share one schema reference. No repeated key string allocation.

Run the benchmark yourself:

```bash
cargo run --release --example bench
```

## Examples

```bash
# Basic usage
cargo run --example basic

# Comprehensive (all types, 7-level nesting, large structures, edge cases)
cargo run --example complex

# Performance benchmark (ASON vs JSON, throughput, memory)
cargo run --release --example bench
```

## ASON Format Specification

See the full [ASON Spec](https://github.com/athxx/ason/blob/main/docs/ASON_SPEC_CN.md) for syntax rules, BNF grammar, escape rules, type system, and LLM integration best practices.

### Syntax Quick Reference

| Element       | Schema                      | Data                |
| ------------- | --------------------------- | ------------------- |
| Object        | `{field1:type,field2:type}` | `(val1,val2)`       |
| Array         | `field:[type]`              | `[v1,v2,v3]`        |
| Object array  | `field:[{f1:type,f2:type}]` | `[(v1,v2),(v3,v4)]` |
| Map           | `field:map[K,V]`            | `[(k1,v1),(k2,v2)]` |
| Nested object | `field:{f1:type,f2:type}`   | `(v1,(v3,v4))`      |
| Null          | —                           | _(blank)_           |
| Empty string  | —                           | `""`                |
| Comment       | —                           | `/* ... */`         |

## License

MIT
