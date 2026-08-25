# asun

[![Crates.io](https://img.shields.io/crates/v/asun.svg)](https://crates.io/crates/asun)
[![Documentation](https://docs.rs/asun/badge.svg)](https://docs.rs/asun)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

面向 [ASUN](https://github.com/asunLab/asun) 的 Rust 实现，为紧凑的结构化数据提供编码与解码。**无需 serde**：类型通过本 crate 自带的 `#[derive(AsunEncode, AsunDecode)]` 宏接入。

[English](https://github.com/asunLab/asun-rs/blob/main/README.md)

## 为什么用 ASUN

标准 JSON 会在每条记录里重复写一遍字段名。当你把结构化数据发给大模型、经过 API 或跨服务传输时，这种重复浪费 token、字节和注意力：

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

**更少 token、更小体积、结构更清晰，解析也比重复对象的 JSON 更快。**

## 特性

- 无 serde —— 使用自带的 `#[derive(AsunEncode, AsunDecode)]` 宏，不依赖 serde
- 一对 derive 同时生成文本与二进制两种线格式
- 二进制零拷贝解码（`&str` 字段直接借用输入缓冲区）
- 当前 API 是 `encode` / `decode`，不再是旧文档里的 `to_string` / `from_str`
- 支持可选的带基本类型提示 Schema 输出
- 支持更易读的 pretty 文本
- 适用于结构体、向量、Option、枚举、嵌套数据，以及基于条目列表的键值集合
- 与 serde 对齐的字段属性：`rename`、`skip`、`skip_serializing`、`skip_deserializing`、`skip_serializing_if`、`default`

## 安装

```toml
[dependencies]
asun = "1.2"
```

依赖树中**不会引入 serde**，ASUN 自带 derive 宏。如果你的项目别处已经用了 serde，两者互不影响、互不交互。

## 使用方法

### 1. 派生 trait

通过本 crate **自带的** derive —— `#[derive(AsunEncode, AsunDecode)]` —— 让类型接入 ASUN，全程不涉及 serde。一次标注即可：`AsunEncode` 同时生成文本和二进制的**编码器**，`AsunDecode` 同时生成文本和二进制的**解码器**。只需要某一个方向时可以只派生对应的那个（通常两个都派生）。

```rust
use asun::{AsunEncode, AsunDecode};

#[derive(Debug, PartialEq, AsunEncode, AsunDecode)]
struct User {
    id: i64,
    name: String,
    active: bool,
}
```

`AsunEncode` / `AsunDecode` 都从 `asun` crate 根部重导出，所以一行 `use asun::{AsunEncode, AsunDecode};` 就同时带入了 trait 和对应的 derive 宏——你不需要直接依赖 `asun-derive`。

### 2. 文本：`encode` / `decode`

`encode` 输出紧凑的、由 Schema 驱动的文本；`decode` 再读回你的类型。用 turbofish（`::<User>`）或显式的绑定类型来告诉 `decode` 要构造什么。

```rust
use asun::{encode, decode};

let user = User { id: 1, name: "Alice".into(), active: true };

let text: String = encode(&user)?;            // "{id,name,active}:(1,Alice,true)"
let back: User   = decode(&text)?;            // decode 从绑定类型推断目标
assert_eq!(user, back);
```

### 3. 自描述文本：`encode_typed`

`encode_typed` 会在 Schema 中嵌入标量类型提示（`@int`、`@str`、`@bool` 等），即使手上没有 Rust 类型也能读懂载荷。`decode` 同时接受不带提示和带提示两种形式。

```rust
use asun::{encode_typed, decode};

let typed = encode_typed(&user)?;
assert_eq!(typed, "{id@int,name@str,active@bool}:(1,Alice,true)");

let back: User = decode(&typed)?;             // 带类型提示的文本也能同样解码
assert_eq!(user, back);
```

### 4. 向量、Option、嵌套类型

同样的两个调用适用于任何派生过的类型——向量在所有行之间共享同一个 Schema，`Option` 对应一个空槽位，嵌套的结构体/枚举会自动组合。

```rust
let users = vec![
    User { id: 1, name: "Alice".into(), active: true },
    User { id: 2, name: "Bob".into(),   active: false },
];

let text: String     = encode(&users)?;       // "[{id,name,active}]:(1,Alice,true),(2,Bob,false)"
let back: Vec<User>  = decode(&text)?;
assert_eq!(users, back);
```

枚举也以同样的方式派生，按变体编码：

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

### 5. Pretty 文本：`encode_pretty` / `encode_pretty_typed`

用于日志和人工检查时，pretty 编码器会加上缩进和换行。输出仍是合法的 ASUN，可通过 `decode` 完整还原。

```rust
use asun::{encode_pretty, encode_pretty_typed, decode};

let pretty = encode_pretty(&users)?;          // 缩进、每行一条记录
let pretty_typed = encode_pretty_typed(&users)?;
let back: Vec<User> = decode(&pretty)?;
```

### 6. 二进制：`encode_binary` / `decode_binary`

二进制格式是体积最小、速度最快的线格式（LEB128 变长整数、有符号整数用 zigzag、浮点定宽）。它**没有 Schema**：字段按声明顺序读取，因此**编码端和解码端必须共享同一份类型定义**。`&str` / `&[u8]` 字段**零拷贝**解码，直接从输入缓冲区借用。

```rust
use asun::{encode_binary, decode_binary};

let bytes: Vec<u8> = encode_binary(&users)?;
let back: Vec<User> = decode_binary(&bytes)?;
assert_eq!(users, back);
```

针对热路径或不可信输入，另有两个入口：

```rust
use asun::{encode_binary_into, decode_binary_exact};

// 在多次编码之间复用同一个缓冲区——保留分配，避免每次都新建 Vec。
let mut buf = Vec::new();
encode_binary_into(&users, &mut buf)?;        // buf 会先被清空再写入
encode_binary_into(&user,  &mut buf)?;        // 复用同一块分配

// 精确解码一个值，并拒绝任何多余的尾部字节（比 decode_binary 更严格）。
let one: User = decode_binary_exact(&encode_binary(&user)?)?;
```

`decode_binary` 会把解码序列的长度上限约束在 `DEFAULT_MAX_SEQUENCE_LEN`（16 MiB），以防御恶意的长度前缀；如果你的协议需要不同的上限，可用 `BinaryDecoder::with_max_sequence_len` 构造。

### 7. 错误处理

所有可能失败的调用都返回 `asun::Result<T>`（即 `Result<T, asun::Error>` 的别名）。`Error` 实现了 `std::error::Error` + `Display`，因此可直接用 `?` 传播，也能接入任意错误处理 crate。

```rust
fn load(text: &str) -> asun::Result<Vec<User>> {
    let users: Vec<User> = decode(text)?;     // 输入非法时传播 asun::Error
    Ok(users)
}
```

## API 参考

| 函数                                    | 作用                                       |
| --------------------------------------- | ------------------------------------------ |
| `encode`                                | 编码为紧凑文本                             |
| `encode_typed`                          | 编码为带标量类型提示的文本                 |
| `decode`                                | 从文本解码（不带提示或带提示均可）         |
| `encode_pretty` / `encode_pretty_typed` | 生成缩进后的 pretty 文本                   |
| `encode_binary`                         | 编码为二进制                               |
| `encode_binary_into`                    | 编码为二进制，写入复用的缓冲区             |
| `decode_binary`                         | 从二进制解码（允许尾部多余字节）           |
| `decode_binary_exact`                   | 只解码一个二进制值，拒绝尾部多余字节       |

trait 与 derive：`AsunEncode`、`AsunDecode`（各一个 derive 同时覆盖文本 + 二进制）。
类型：`Error`、`Result<T>`、`DEFAULT_MAX_SEQUENCE_LEN`。

### 该选哪种格式？

- **`encode` / `decode`** —— 可读、省 token 的文本。适合大模型提示词、API、配置，以及任何你想直接肉眼查看载荷的场景。
- **`encode_typed`** —— 同样是文本，但带自描述；当读取方可能没有 Rust 类型时使用。
- **`encode_binary` / `decode_binary`** —— 体积最小、速度最快；适合两端共享类型的存储和服务间通信。

## 字段属性

字段和枚举变体支持 `#[asun(...)]` 属性，与 serde 对齐：

| 属性                               | 效果                                                              |
| ---------------------------------- | ----------------------------------------------------------------- |
| `rename = "name"`                  | 线格式上用 `name` 取代 Rust 标识符                                 |
| `skip`                             | 既不写也不读；解码为默认值                                          |
| `skip_serializing`                 | 不写入；文本中存在时仍会读取                                        |
| `skip_deserializing`               | 不读取；始终解码为默认值                                            |
| `skip_serializing_if = "path"`     | 当谓词 `fn(&T) -> bool` 返回 `true` 时，在**文本**中省略该字段      |
| `default = "path"`                 | 为解码时被跳过的字段提供取值来源 `fn() -> T`（否则用 `Default`）    |

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

**二进制说明：** 二进制格式没有 Schema，按声明顺序读取字段，因此任何 skip 都被强制为**对称**——一端跳过的字段两端都会跳过。`skip_serializing_if` 在二进制中被忽略（字段始终写入），以保证定序解码的可靠性。自定义 `default` 只作用于解码时被跳过的字段（`skip` / `skip_deserializing`）。

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
