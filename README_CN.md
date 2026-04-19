# asun

[![Crates.io](https://img.shields.io/crates/v/asun.svg)](https://crates.io/crates/asun)
[![Documentation](https://docs.rs/asun/badge.svg)](https://docs.rs/asun)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

面向 [ASUN](https://github.com/asunLab/asun) 的 Rust 实现，基于 serde 提供紧凑结构化数据的编码与解码。

[English](https://github.com/asunLab/asun-rs/blob/main/README.md)

## 为什么用 ASUN

ASUN 只写一次 Schema，后续每行数据按位置保存：

```json
[
  { "id": 1, "name": "Alice", "active": true },
  { "id": 2, "name": "Bob", "active": false }
]
```

**asun**
ASUN 只声明 **一次** Schema，数据以紧凑元组方式流式传输：

```asun
[{id,name,active}]:
    (1,Alice,true),
    (2,Bob,false)
```

这让重复记录更短，更适合传输、存储或送入模型。

## 特性

- 基于 serde 的文本编码和解码
- 当前 API 是 `encode` / `decode`，不再是旧文档里的 `to_string` / `from_str`
- 支持可选的带基本类型提示 Schema 输出
- 支持更易读的 pretty 文本和二进制格式
- 适用于结构体、向量、Option、枚举、嵌套数据，以及基于条目列表的键值集合

## 安装

```toml
[dependencies]
asun = "0.1"
serde = { version = "1", features = ["derive"] }
```

## 快速开始

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

### 编码向量

```rust
let users = vec![
    User { id: 1, name: "Alice".into(), active: true },
    User { id: 2, name: "Bob".into(), active: false },
];

let text = encode(&users)?;
let typed = encode_typed(&users)?;
let decoded: Vec<User> = decode(&text)?;
```

### Pretty 文本和二进制

```rust
use asun::{decode_binary, encode_binary, encode_pretty, encode_pretty_typed};

let pretty = encode_pretty(&users)?;
let pretty_typed = encode_pretty_typed(&users)?;
let bin = encode_binary(&users)?;
let decoded: Vec<User> = decode_binary(&bin)?;
```

## 当前 API

| 函数                                    | 作用             |
| --------------------------------------- | ---------------- |
| `encode` / `encode_typed`               | 编码为文本       |
| `decode`                                | 从文本解码       |
| `encode_pretty` / `encode_pretty_typed` | 生成更易读的文本 |
| `encode_binary`                         | 编码为二进制     |
| `decode_binary`                         | 从二进制解码     |

## 运行示例

```bash
cargo test
cargo run --example basic
cargo run --example complex
cargo run --example bench
```

## Contributors

- [Athan](https://github.com/athxx)

## Benchmark Snapshot

可以通过下面命令运行 benchmark 示例：

```bash
cargo run --example bench --release
```

Rust 版 benchmark 现在和 Go 版保持同一种两行汇总样式：

```text
Flat struct × 1000 (8 fields, vec)
  Serialize:   JSON   411.05ms /   121675 B | ASUN   175.25ms (2.3x) /    56718 B (46.6%) | BIN    41.32ms (9.9x) /    74454 B (61.2%)
  Deserialize: JSON   287.06ms | ASUN   195.57ms (1.5x) | BIN    64.62ms (4.4x)
```

其中 `ASUN` / `BIN` 后面的倍率都是相对 JSON 计算的，大小百分比表示“占 JSON 的剩余比例”。

## 许可证

MIT
