# ason

[![Crates.io](https://img.shields.io/crates/v/ason.svg)](https://crates.io/crates/ason)
[![Documentation](https://docs.rs/ason/badge.svg)](https://docs.rs/ason)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

高性能 [serde](https://serde.rs/) 序列化/反序列化库，用于 [ASON](https://github.com/athxx/ason)（Array-Schema Object Notation）—— 一种面向 LLM 交互和大规模数据传输的高效序列化格式。

[English](README.md)

## 什么是 ASON？

ASON 将 **Schema** 与 **数据** 分离，消除了 JSON 中每个对象都重复出现 Key 的冗余。Schema 只声明一次，数据行仅保留纯值：

```text
JSON (100 tokens):
{"users":[{"id":1,"name":"Alice","active":true},{"id":2,"name":"Bob","active":false}]}

ASON (~35 tokens, 节省 65%):
[{id:int, name:str, active:bool}]:(1,Alice,true),(2,Bob,false)
```

| 方面       | JSON         | ASON           |
| ---------- | ------------ | -------------- |
| Token 效率 | 100%         | 30–70% ✓       |
| Key 重复   | 每个对象都有 | 声明一次 ✓     |
| 人类可读   | 是           | 是 ✓           |
| 嵌套结构   | ✓            | ✓              |
| 类型注解   | 无           | 可选 ✓         |
| 序列化速度 | 1x           | **~2x 更快** ✓ |
| 数据体积   | 100%         | **40–50%** ✓   |

## 快速开始

在 `Cargo.toml` 中添加：

```toml
[dependencies]
ason = "0.1"
```

### 序列化与反序列化结构体

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

    // 序列化
    let s = to_string(&user)?;
    assert_eq!(s, "{id,name,active}:(1,Alice,true)");

    // 反序列化
    let user2: User = from_str(&s)?;
    assert_eq!(user, user2);
    Ok(())
}
```

### 带类型注解序列化

使用 `to_string_typed` 输出带类型注解的 Schema —— 适用于文档生成、LLM 提示词和跨语言数据交换：

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

    // 反序列化同时支持带注解和不带注解的 Schema
    let user2: User = from_str(&s)?;
    assert_eq!(user, user2);
    Ok(())
}
```

### 序列化与反序列化 Vec（Schema 驱动）

对于 `Vec<T>`，ASON 只写入一次 Schema，每个元素以紧凑元组形式输出 —— 这是相比 JSON 的核心优势：

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
        &["int", "str", "bool"]  // 启用 to_string_vec_typed()
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

    // 无注解 Schema
    let s = to_string_vec(&users)?;
    assert_eq!(s, "[{id,name,active}]:(1,Alice,true),(2,Bob,false)");

    // 带类型注解 Schema（需实现 field_types()）
    use ason::to_string_vec_typed;
    let s2 = to_string_vec_typed(&users)?;
    assert_eq!(s2, "[{id:int,name:str,active:bool}]:(1,Alice,true),(2,Bob,false)");

    // 反序列化 —— 两种格式均可
    let users2: Vec<User> = from_str_vec(&s)?;
    let users3: Vec<User> = from_str_vec(&s2)?;
    assert_eq!(users2.len(), 2);
    assert_eq!(users3.len(), 2);
    Ok(())
}
```

## 支持的类型

| 类型                  | ASON 表示                       | 示例                          |
| --------------------- | ------------------------------- | ----------------------------- |
| 整数 (i8–i64, u8–u64) | 纯数字                          | `42`, `-100`                  |
| 浮点 (f32, f64)       | 带小数点                        | `3.14`, `-0.5`                |
| 布尔                  | 字面量                          | `true`, `false`               |
| 字符串                | 无引号或有引号                  | `Alice`, `"Carol Smith"`      |
| Option\<T\>           | 有值或留空                      | `hello` 或 _(空白)_ 表示 None |
| Vec\<T\>              | `[v1,v2,v3]`                    | `[rust,go,python]`            |
| HashMap\<K,V\>        | `[(k1,v1),(k2,v2)]`             | `[(age,30),(score,95)]`       |
| 嵌套结构体            | `(field1,field2)`               | `(Engineering,500000)`        |
| 枚举                  | Unit / Newtype / Tuple / Struct | `Red`, `Circle(5.0)`          |
| 字符                  | 单个字符                        | `A`                           |

### 嵌套结构体

```rust
#[derive(Serialize, Deserialize)]
struct Dept { title: String }

#[derive(Serialize, Deserialize)]
struct Employee { name: String, dept: Dept }

// Schema 自动反映嵌套结构：
// {name:str,dept:{title:str}}:(Alice,(Engineering))
```

### 可选字段

```rust
#[derive(Deserialize)]
struct Item { id: i64, label: Option<String> }

// 有值:    {id:int,label:str}:(1,hello)
// None:   {id:int,label:str}:(1,)
```

### 数组与字典

```rust
#[derive(Deserialize)]
struct Tagged { name: String, tags: Vec<String> }
// {name:str,tags:[str]}:(Alice,[rust,go,python])

use std::collections::HashMap;
#[derive(Deserialize)]
struct Profile { name: String, attrs: HashMap<String, i64> }
// {name:str,attrs:map[str,int]}:(Alice,[(age,30),(score,95)])
```

### 类型注解（可选）

ASON Schema 支持**可选的**类型注解。两种形式完全等价 —— 反序列化器对它们的处理完全一致：

```text
// 不带注解（to_string / to_string_vec 的默认输出）
{id,name,salary,active}:(1,Alice,5000.50,true)

// 带注解（to_string_typed / to_string_vec_typed 的输出）
{id:int,name:str,salary:float,active:bool}:(1,Alice,5000.50,true)
```

注解是**纯粹的装饰性元数据** —— 它们不影响解析和反序列化行为。反序列化器遇到 `:type` 部分时会直接跳过。

**适用场景：**

- LLM 提示词 — 帮助模型理解并生成正确的数据
- API 文档 — 无需外部文档即可自描述 Schema
- 跨语言数据交换 — 消除类型歧义（`42` 是 int 还是 float？）
- 调试 — 一眼看出数据类型

**性能影响：** 可忽略不计（Vec 场景 <0.1%，单结构体 ~3%）。开销是常数级的，不随数据量增长 —— 注解仅影响 Schema 头部，不影响数据体。

### 注释

```text
/* 用户列表 */
[{id:int, name:str, active:bool}]:(1,Alice,true),(2,Bob,false)
```

### 多行格式

```text
[{id:int, name:str, active:bool}]:
  (1, Alice, true),
  (2, Bob, false),
  (3, "Carol Smith", true)
```

## API 参考

| 函数                        | 说明                                                |
| --------------------------- | --------------------------------------------------- |
| `to_string(&T)`             | 序列化结构体 → 无注解 Schema `{id,name}:`           |
| `to_string_typed(&T)`       | 序列化结构体 → 带注解 Schema `{id:int,name:str}:`   |
| `from_str::<T>(s)`          | 反序列化结构体（两种 Schema 格式均可）              |
| `to_string_vec(&[T])`       | 序列化 Vec → 无注解 Schema（需实现 `StructSchema`） |
| `to_string_vec_typed(&[T])` | 序列化 Vec → 带注解 Schema（需实现 `StructSchema`） |
| `from_str_vec::<T>(s)`      | 反序列化 Vec（两种 Schema 格式均可）                |

## 性能

在 Apple Silicon（M 系列）上以 release 模式测试，与 `serde_json` 对比：

### 序列化（ASON 快 1.8–2.4 倍）

| 场景            | JSON    | ASON    | 加速比    |
| --------------- | ------- | ------- | --------- |
| 扁平结构 × 1000 | 11.2 ms | 5.2 ms  | **2.16x** |
| 5 层嵌套 × 100  | 41.2 ms | 22.1 ms | **1.86x** |
| 大数据量 (10k)  | 10.9 ms | 5.0 ms  | **2.19x** |

### 反序列化（ASON 快 1.1–1.3 倍）

| 场景            | JSON    | ASON    | 加速比    |
| --------------- | ------- | ------- | --------- |
| 扁平结构 × 1000 | 29.6 ms | 24.9 ms | **1.19x** |
| 5 层嵌套 × 100  | 93.2 ms | 84.5 ms | **1.10x** |
| 大数据量 (10k)  | 29.9 ms | 25.9 ms | **1.16x** |

### 体积节省

| 场景            | JSON   | ASON   | 节省    |
| --------------- | ------ | ------ | ------- |
| 扁平结构 × 1000 | 121 KB | 57 KB  | **53%** |
| 5 层嵌套 × 100  | 438 KB | 175 KB | **60%** |
| 10k 条记录      | 1.2 MB | 0.6 MB | **53%** |

### 为什么 ASON 更快？

1. **零哈希匹配** — Schema 只解析一次，数据字段通过位置索引 `O(1)` 映射，无需每行对 Key 字符串计算哈希。
2. **模式驱动解析** — 反序列化器通过 Schema 已知每个字段的类型，可以直接调用 `parse_int()` 等方法，而非运行时推断类型。CPU 分支预测命中率接近 100%。
3. **极小内存分配** — 所有数据行共享同一个 Schema 引用，无需为每行重复分配 Key 字符串的内存。

运行基准测试：

```bash
cargo run --release --example bench
```

## 示例

```bash
# 基础用法
cargo run --example basic

# 全面测试（全类型、7 层嵌套、大型结构、边界用例）
cargo run --example complex

# 性能基准（ASON vs JSON，吞吐量，内存占用）
cargo run --release --example bench
```

## ASON 格式规范

完整的 [ASON 规范](https://github.com/athxx/ason/blob/main/docs/ASON_SPEC_CN.md) 包含语法规则、BNF 文法、转义规则、类型系统及 LLM 集成最佳实践。

### 语法速查表

| 元素     | Schema 语法                 | 数据语法            |
| -------- | --------------------------- | ------------------- |
| 对象     | `{field1:type,field2:type}` | `(val1,val2)`       |
| 简单数组 | `field:[type]`              | `[v1,v2,v3]`        |
| 对象数组 | `field:[{f1:type,f2:type}]` | `[(v1,v2),(v3,v4)]` |
| 字典     | `field:map[K,V]`            | `[(k1,v1),(k2,v2)]` |
| 嵌套对象 | `field:{f1:type,f2:type}`   | `(v1,(v3,v4))`      |
| 空值     | —                           | _(空白)_            |
| 空字符串 | —                           | `""`                |
| 注释     | —                           | `/* ... */`         |

## 许可证

MIT
