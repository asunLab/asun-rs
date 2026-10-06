# Changelog

## 1.3.0

The text codec now follows the ASUN 1.5 grammar
([`GRAMMAR.abnf`](https://github.com/asunLab/asun/blob/main/conformance/GRAMMAR.abnf)).
The binary format is unchanged. `asun-derive` is unchanged and stays at 1.2.0.

### Text output changes

- Strings are quoted only when needed. Spaces, `@`, `:`, `/` and `*` stay
  bare: `(hello world,alice@example.com,12:30)`.
- `"null"` is quoted, since `null` is now a keyword. Capitalised forms such as
  `TRUE` and `True` are no longer quoted, because keywords are case-sensitive.
- Rust tuples and tuple structs encode as arrays (`[3,4]`, previously `(3,4)`).
- Enum variants with data encode as `[Variant,...]` (previously
  `(Variant,...)`). Unit variants are still the bare name.
- `()` and unit structs encode as null (an empty slot) instead of `()`.
- A top-level `None` encodes as `null`, and a one-element `vec![None]` as
  `[null]`.
- Field names outside `[A-Za-z0-9_]` are quoted in every schema header,
  including the `[{...}]:` header of a top-level `Vec<Struct>`.
- In typed output, tuple fields are bound as `@[]` and enum fields carry no
  hint, so the first element's type no longer leaks into the binding.
- A string starting with U+FEFF is quoted, so it cannot be mistaken for a BOM.

### Decoding is strict

- A tuple must have exactly as many slots as its schema. Commas are pure
  separators: `(a,)` is `a` plus a null, and a trailing comma after the last
  row of `[{...}]:` is an error.
- `null` is a keyword (`"null"` is the string).
- Scalar hints are enforced: `@int` rejects `1.5`, `@str` rejects a numeric
  target, and so on. Nested `@{...}` bindings now map fields by name instead
  of being ignored.
- Number errors:
  - Integers must fit the target type.
  - `42abc`, `1.2.3`, `0x10` and `4 2` are not numbers.
  - A float overflowing to infinity is an error.
- Quoted strings follow JSON: raw control characters, unknown escapes and lone
  surrogates are errors. `\/` is accepted.
- Schema errors: duplicate field names, invalid bare field names (`a-b` must
  be written `"a-b"`), and type names other than lowercase
  `int`/`float`/`str`/`bool` (the old `@str?` suffix is gone).
- Document errors: an empty document, a bare top-level tuple, trailing
  content and unclosed comments.
- Layout: comments are allowed anywhere whitespace is, including inside
  tuples. A leading BOM is skipped.
- Nesting deeper than 128 levels is an error, including inside skipped
  unknown fields.
- Inputs that used to hang the decoder (a raw `:` in data, stray `]`/`}`) now
  return an error.
- Unchanged: target fields missing from the source decode to their default,
  and extra source fields are skipped.

### Binary decoding hardening

The wire format is unchanged; hostile input is now rejected instead of
crashing or stalling the process.

- `Vec` nesting deeper than 128 levels returns `DepthLimitExceeded`.
  Previously about 100 KB of input for a recursive type such as
  `struct Node { kids: Vec<Node> }` overflowed the stack and aborted.
- An `Option` tag other than `0`/`1` returns `InvalidTag` (any non-zero byte
  used to mean `Some`).
- Padded varints such as `80 00` for 0 return `VarintOverflow`, so every value
  has exactly one encoding. No official encoder writes them.
- Zero-sized elements (`Vec<()>`, `Vec<EmptyStruct>`) count against one
  per-input budget equal to the sequence limit, so nested sequences of them
  cannot turn a few hundred bytes into billions of iterations.
- The up-front reservation for a sequence is capped at 1 MiB; a claimed count
  no longer reserves `count × size_of::<T>()` bytes before any element is read.

### Fixes

- `f32` values are rounded once. They used to be parsed as `f64` and then
  narrowed, which could be off by one ulp.
- A plain `&str` field holding escapes returns an error instead of the raw
  escaped text.
- `pretty` output keeps a leading null slot when a group is expanded, and no
  longer treats `<`/`>` as brackets.

### Performance

Measured against 1.2.0 with this crate's benchmark example (ASUN speed relative
to `serde_json`): encode went from 2.1× to 2.25× faster than JSON, and decode
from 1.6× to 1.95×.

- Struct decode plans are cached on the parsed schema.
- Short strings are copied without a `memcpy` call.
- Floats use an exact fast path, and integers skip overflow checks below 19
  digits.
- Struct encoding no longer allocates per row.
- Binary codec (format unchanged): fixed-size values and short strings are
  written with a single capacity check and an inline copy, and ASCII strings
  under 64 bytes skip the full UTF-8 validator when decoded.

### Compatibility

- Data written by 1.2 that 1.3 rejects:
  - a trailing comma after the last row of `[{...}]:`;
  - `@type?` hints;
  - unquoted field names containing `+` or `-`;
  - values that only decoded through lenient number parsing.
- Data written by 1.3 that 1.2 cannot read reliably: tuples and enum payloads
  as `[...]`, the `null` keyword, and bare `@`/`:` in values. A 1.2 decoder
  can hang on a bare `:`. Upgrade readers before writers.
- Other ASUN language implementations have not been updated to the 1.5
  grammar yet.
