# ason

[![Crates.io](https://img.shields.io/crates/v/ason.svg)](https://crates.io/crates/ason)
[![Documentation](https://docs.rs/ason/badge.svg)](https://docs.rs/ason)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

面向 [ASON](https://github.com/ason-lab/ason) 的 Rust 实现，基于 serde 提供紧凑结构化数据的编码与解码。

[English](README.md)

## 为什么用 ASON

ASON 只写一次 Schema，后续每行数据按位置保存：

```json
[
  {"id": 1, "name": "Alice", "active": true},
  {"id": 2, "name": "Bob", "active": false}
]
```

```text
[{id:int,name:str,active:bool}]:(1,Alice,true),(2,Bob,false)
```

这让重复记录更短，更适合传输、存储或送入模型。

## 特性

- 基于 serde 的文本编码和解码
- 当前 API 是 `encode` / `decode`，不再是旧文档里的 `to_string` / `from_str`
- 支持可选的带类型 Schema 输出
- 支持更易读的 pretty 文本和二进制格式
- 适用于结构体、向量、Option、Map、枚举和嵌套数据

## 安装

```toml
[dependencies]
ason = "0.1"
serde = { version = "1", features = ["derive"] }
```

## 快速开始

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
use ason::{decode_binary, encode_binary, encode_pretty, encode_pretty_typed};

let pretty = encode_pretty(&users)?;
let pretty_typed = encode_pretty_typed(&users)?;
let bin = encode_binary(&users)?;
let decoded: Vec<User> = decode_binary(&bin)?;
```

## 当前 API

| 函数 | 作用 |
| --- | --- |
| `encode` / `encode_typed` | 编码为文本 |
| `decode` | 从文本解码 |
| `encode_pretty` / `encode_pretty_typed` | 生成更易读的文本 |
| `encode_binary` | 编码为二进制 |
| `decode_binary` | 从二进制解码 |

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

在当前机器上通过下面命令实测：

```bash
CARGO_BUILD_RUSTC_WRAPPER='' CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=cc RUSTFLAGS='' cargo run --example bench --release
```

关键结果：

- 扁平 1,000 条记录：ASON 序列化 `57.46ms`，JSON `73.06ms`；反序列化 ASON `84.37ms`，JSON `111.17ms`
- 扁平 10,000 条记录：ASON 序列化 `46.91ms`，JSON `57.75ms`；反序列化 ASON `72.26ms`，JSON `93.22ms`
- 深层 100 条数据：ASON 序列化 `234.43ms`，JSON `252.71ms`；反序列化 ASON `211.58ms`，JSON `313.06ms`
- 1,000 条记录吞吐总结：ASON 文本序列化比 JSON 快 `1.12x`，反序列化快 `1.34x`
- 1,000 条扁平记录二进制总结：BIN 序列化 `2.13ms`，JSON `12.79ms`；BIN 反序列化 `5.86ms`，JSON `19.87ms`

## 许可证

MIT
