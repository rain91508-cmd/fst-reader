# FST 格式与算法实现详解

本文档基于 [FST Format Specification](https://blog.timhutt.co.uk/fst_spec/) 和 fst-reader 源代码，详细说明 FST 波形文件的格式、读取机制，以及查找 start 之前最新值的算法实现思路。

## 目录

1. [文件整体结构](#1-文件整体结构)
2. [头部块](#2-头部块-header-block)
3. [变长整数编码](#3-变长整数编码-leb128)
4. [几何块](#4-几何块-geometry-block)
5. [层次结构块](#5-层次结构块-hierarchy-block)
6. [值变化数据块](#6-值变化数据块-value-change-data-block)
7. [高效读取机制](#7-高效读取机制)
8. [关键发现与分析](#8-关键发现与分析)
9. [查找 Start 之前最新值的算法实现思路](#9-查找-start-之前最新值的算法实现思路)
10. [查找时间范围首尾数据的高效算法](#10-查找时间范围首尾数据的高效算法)
11. [关键数据结构](#11-关键数据结构)
12. [压缩算法](#12-压缩算法)
13. [`read_pre_start_values` 函数使用指南](#13-read_pre_start_values-函数使用指南)
14. [`read_range_boundary_values` 函数使用指南](#14-read_range_boundary_values-函数使用指南)

---

## 1. 文件整体结构

FST 文件由一系列 **TLV (Tag, Length, Value)** 块组成：

| 偏移 | 类型 | 描述 |
|------|------|------|
| 0 | u8 | 块类型 (BlockType) |
| 1 | u64 | 块长度（包含长度值本身，不包含块类型字节） |
| 9 | - | 块数据 |

### 块类型定义

| 名称 | 值 | 描述 |
|------|-----|------|
| FST_BL_HDR | 0 | 头部块，位于文件开始 |
| FST_BL_VCDATA | 1 | 值变化数据块，记录波形数据 |
| FST_BL_BLACKOUT | 2 | 记录 $dumpoff/on 调用时间 |
| FST_BL_GEOM | 3 | 几何块，存储变量长度信息 |
| FST_BL_HIER | 4 | 层次结构块（GZip压缩） |
| FST_BL_VCDATA_DYN_ALIAS | 5 | 新版本的值变化数据块 |
| FST_BL_HIER_LZ4 | 6 | 层次结构块（LZ4压缩） |
| FST_BL_HIER_LZ4DUO | 7 | 层次结构块（双重LZ4压缩） |
| FST_BL_VCDATA_DYN_ALIAS2 | 8 | 更新版本的值变化数据块 |
| FST_BL_ZWRAPPER | 254 | 整个文件被 GZip 压缩 |
| FST_BL_SKIP | 255 | 写入时使用的占位块 |

### 文件块顺序

```
┌─────────────────┐
│   Header Block  │  ← 1个，固定329字节
├─────────────────┤
│  Value Change   │  ← 0个或多个
│     Block 1     │
├─────────────────┤
│  Value Change   │
│     Block 2     │
├─────────────────┤
│      ...        │
├─────────────────┤
│  Geometry Block │  ← 1个，变量类型和长度
├─────────────────┤
│ Blackout Block  │  ← 0或1个，可选
├─────────────────┤
│ Hierarchy Block │  ← 0或1个，可选，可压缩
└─────────────────┘
```

---

## 2. 头部块 (Header Block)

头部块长度固定为 **329 字节**。

| 名称 | 偏移 | 类型 | 描述 |
|------|------|------|------|
| header_block_type | 0 | u8 | 块类型 (0) |
| header_block_length | 1 | u64 | 块长度 (329) |
| header_start_time | 9 | u64 | 起始时间 |
| header_end_time | 17 | u64 | 结束时间 |
| header_real_endianness | 25 | f64 | 值 e (2.71828...)，用于字节序检测 |
| header_writer_memory_use | 33 | u64 | 写入器内存使用 |
| header_num_scopes | 41 | u64 | 作用域数量 |
| header_num_hiearchy_vars | 49 | u64 | 层次结构变量数 |
| header_num_vars | 57 | u64 | 不同变量数（去重后）|
| header_num_vc_blocks | 65 | u64 | 值变化块数量 |
| header_timescale | 73 | i8 | 时间刻度指数，0=1s, -9=1ns |
| header_writer | 74 | u8[128] | 仿真器标识字符串 |
| header_date | 202 | u8[119] | 日期字符串 |
| header_filetype | 321 | u8 | 文件类型 |
| header_timezero | 322 | u64 | 时间零点 |

**字节序检测**：使用数学常数 `e` (2.7182818284590452354) 来判断浮点数字节序。

---

## 3. 变长整数编码 (LEB128)

FST 使用 LEB128 编码变长整数：

- 每个字节最高位为 1 表示后续还有字节
- 最高位为 0 表示这是最后一个字节
- 低 7 位存储数据

```
示例：编码值 0x1234 (4660)
  二进制：0001 0010 0011 0100
  
  第1字节：1011 0100 (0xB4)  ← 低7位：0110100，最高位1表示继续
  第2字节：0000 1001 (0x09)  ← 高7位：0001001，最高位0表示结束
  
  解码：
    byte1 & 0x7f = 0110100
    byte2 & 0x7f = 0001001
    result = (0001001 << 7) | 0110100 = 1001000110100 = 0x1234
```

---

## 4. 几何块 (Geometry Block)

存储每个信号的类型和位宽信息。

| 名称 | 类型 | 描述 |
|------|------|------|
| section_length | u64 | 块长度 |
| uncompressed_length | u64 | 解压后长度 |
| max_handle | u64 | 最大信号句柄 |
| compressed_data | - | ZLib压缩的数据 |

解压后的数据是变长整数数组，每个值编码信号信息：
- `value = 0` → Real 类型（64位浮点）
- `value = u32::MAX` → 1位信号
- 其他 → 位宽为 `value + 1` 的位向量

---

## 5. 层次结构块 (Hierarchy Block)

存储设计层次和信号名称，支持多种压缩格式。

### 块结构

| 名称 | 类型 | 描述 |
|------|------|------|
| hierarchy_type | u8 | 块类型 (4/6/7) |
| hierarchy_length | u64 | 块长度 |
| hierarchy_uncompressed_length | u64 | 解压后长度 |
| hierarchy_compressed_once_length | varint | 仅 LZ4DUO，第一次解压后长度 |
| hierarchy_data | - | 压缩数据 |

### 压缩类型

| 块类型 | 压缩方式 |
|--------|----------|
| FST_BL_HIER (4) | GZip |
| FST_BL_HIER_LZ4 (6) | LZ4 |
| FST_BL_HIER_LZ4DUO (7) | 双重 LZ4 |

---

## 6. 值变化数据块 (Value Change Data Block)

存储实际的波形数据，是 FST 文件的核心内容。

### 块结构

| 名称 | 偏移 | 类型 | 描述 |
|------|------|------|------|
| section_length | 0 | u64 | 块长度 |
| start_time | 8 | u64 | 块起始时间 |
| end_time | 16 | u64 | 块结束时间 |
| mem_required_for_traversal | 24 | u64 | 遍历所需内存 |
| frame_data | 32 | - | 初始值帧（压缩，可选） |
| value_changes | - | - | 值变化数据 |
| time_table | - | - | 时间戳表（压缩，位于块末尾） |

**关键优化点**：打开文件时，只读取前 4 个 u64（32 字节）元数据，然后直接 seek 跳过整个块主体，无需读取任何波形数据！

### 时间戳表 (Time Table)

时间戳使用**差分编码**存储：

1. 存储的是相邻时间戳的差值
2. 使用 LEB128 变长编码
3. 解压时累加差值得到实际时间

```
示例：
  原始时间戳：[0, 100, 250, 300]
  差分编码：[0, 100, 150, 50]
  
  解码：
    time[0] = 0
    time[1] = 0 + 100 = 100
    time[2] = 100 + 150 = 250
    time[3] = 250 + 50 = 300
```

时间戳表存储在数据块**末尾**，最后 24 字节是元数据：
- uncompressed_length (u64)
- compressed_length (u64)
- number_of_items (u64)

---

## 7. 高效读取机制

### 7.1 打开文件（关键优化）

```rust
// 1. 检查并解压 GZip 包装（如果需要）
// 2. 读取头部块
// 3. 遍历所有块：
//    - 对于值变化块：只读 32 字节元数据，然后 seek 跳过主体
//    - 对于其他块：完整读取
let reader = FstReader::open(file)?;
```

**关键优化**：打开文件时，值变化块只读取前 32 字节元数据（`section_length`, `start_time`, `end_time`, `mem_required_for_traversal`），然后直接 `seek` 跳过整个数据块主体！这样无论文件多大，打开时间都只与数据块数量线性相关，与数据总量无关。

### 7.2 内存索引建立

文件打开后，内存中建立完整的数据块索引目录：

```rust
Vec<DataSectionInfo> = [
  DataSectionInfo { file_offset: 0x100, start_time: 0, end_time: 1000, ... },
  DataSectionInfo { file_offset: 0x5000, start_time: 1001, end_time: 2000, ... },
  ...
]
```

### 7.3 relevant_sections 详解

```rust
let relevant_sections = sections
    .iter()
    .filter(|s| filter.end >= s.start_time && s.end_time >= filter.start);
```

#### 返回类型

`relevant_sections` 是一个 **延迟计算的迭代器**，类型为 `std::iter::Filter<std::slice::Iter<'_, DataSectionInfo>, _>`。它在被使用时才会实际计算过滤结果。

#### 过滤条件详解

```rust
filter.end >= s.start_time && s.end_time >= filter.start
```

这是**重叠区间检测**条件，判断数据块的时间范围与过滤时间范围是否有重叠：

| 情况 | 条件 | 是否包含 |
|------|------|---------|
| 数据块完全在过滤范围之后 | `s.start_time > filter.end` | ❌ 不包含 |
| 数据块完全在过滤范围之前 | `s.end_time < filter.start` | ❌ 不包含 |
| 数据块与过滤范围有重叠 | 其他情况 | ✅ 包含 |

#### 为什么 O(n) 但很快？

1. **n 是数据块数量**：通常远小于信号变化数量
2. **只做内存比较**：没有 I/O 操作
3. **延迟计算**：迭代器只在被使用时才执行过滤

### 7.4 多信号处理机制

`read_signals` 处理多个信号时采用**分层过滤+联合处理**策略：

#### 第一步：全局查找 relevant_sections（基于时间范围，与信号无关）

所有信号共享相同的 `relevant_sections`，这个过滤只看时间范围，不考虑具体信号。

#### 第二步：对每个数据块，用 BitMask 过滤信号

在 `read_value_changes` 中：

```rust
for entry in signal_offsets.iter() {
    // 检查信号是否在 BitMask 中
    if self.filter.signals.is_set(entry.signal_idx) {
        // 读取该信号的数据
        ...
    }
}
```

`self.filter.signals`（`BitMask` 类型）是第二层过滤，它决定在一个数据块内哪些信号需要被读取，哪些可以直接跳过。

#### 第三步：在同一数据块中同时处理所有需要的信号

使用**时间轮算法**（time-wheel）同时处理多个信号：

```rust
// 按时间顺序遍历
for (time_id, time) in time_table.iter().enumerate() {
    // 处理该时间点的所有信号变化
    while tc_head[time_id] != 0 {
        let signal_id = (tc_head[time_id] - 1) as usize;
        // 读取该信号的值并回调
        ...
    }
}
```

#### 为什么这样设计？

| 优势 |
|------|
| 1. **最小化 I/O**：一个数据块只读取一次，而不是每个信号都读一次 |
| 2. **高效的时间轮**：多个信号的变化在同一时间点上可以一起处理 |
| 3. **利用数据局部性**：相关信号的数据通常在同一个数据块中 |

---

## 8. 关键发现与分析

### 8.1 Start 之前值的处理逻辑

在 `read_value_changes` 中有重要注解（第 708-710 行）：

```rust
// while we cannot ignore signal changes before the start of the window
// (since the signal might retain values for multiple cycles),
// signal changes after our window are completely useless
```

#### 处理逻辑说明

1. **处理 start 之前的信号变化，但没有选择性回调**

```rust
for (time_id, time) in time_table.iter().enumerate() {
    // ✅ 只对 end 之后的 break
    if *time > self.filter.end {
        break;
    }
    
    // ❌ 但对 start 之前的不 skip，全部处理！
    // 处理该时间点的所有信号变化...
    // ⚠️ 直接回调，没有时间过滤！
    (self.callback)(*time, signal_handle, value);
}
```

2. **为什么这样设计？**

**关键原因：信号值在没有变化时会持续多个时钟周期！**

图示说明：
```
假设 filter.start = 200

时间:     0    100    200    300    400
信号A:    0 ────── 1 ──────────── 0
          ↑      ↑              ↑
       (需要处理) (需要处理)  (需要处理)
       虽然 <200   虽然 <200    虽然>200
       
为什么？因为信号A从100到300都是1，如果不处理100的变化，
在200时就不知道信号值是1了！
```

3. **当前实现的限制**

注意：虽然处理了 start 之前的信号变化（更新时间轮指针），但是**没有选择性地回调**！

当前实现：所有处理到的信号变化都会回调，不管是否在 start 之前。

如果想要选择性地回调，需要添加类似这样的逻辑：

```rust
let should_callback = *time >= self.filter.start;
if should_callback {
    (self.callback)(*time, signal_handle, value);
}
```

| 问题 | 答案 |
|------|------|
| 有获取 start 点之前值的逻辑吗？ | ✅ 有注解，也有处理逻辑（更新时间轮指针） |
| 这个逻辑是做什么的？ | 处理 start 之前的信号变化，以建立正确的信号状态 |
| 会回调 start 之前的值吗？ | ❌ 当前实现没有过滤，所有值都会回调 |

### 8.2 时间轮（Time-Wheel）算法详解

#### 核心数据结构

```rust
// 时间轮头指针数组
let mut tc_head = vec![0u32; time_table.len()];

// 每个信号的链接指针
let mut scatter_pointer = vec![0u32; max_handle as usize];

// 信号数据位置和长度
let mut head_pointer = vec![0u32; max_handle as usize];
let mut length_remaining = vec![0u32; max_handle as usize];
```

#### 多信号同时处理的图示

```
假设：
- time_table = [0, 100, 200, 300]
- filter.start = 200
- 信号A在 0, 300 变化
- 信号B在 0 变化
- 信号C在 200 变化

时间轮结构：
tc_head[0]  → 信号A → 信号B → 0
tc_head[1]  → 0
tc_head[2]  → 信号C → 0
tc_head[3]  → 信号A → 0

处理流程：
─────────────────────────────────────────────
时间0 (time=0 < start=200)
  ↓
处理信号A → 回调(time=0, 信号A)
  ↓
处理信号B → 回调(time=0, 信号B)
─────────────────────────────────────────────
时间100 (time=100 < start=200)
  ↓
无信号，跳过
─────────────────────────────────────────────
时间200 (time=200 >= start=200)
  ↓
处理信号C → 回调(time=200, 信号C)
─────────────────────────────────────────────
时间300 (time=300 >= start=200)
  ↓
处理信号A → 回调(time=300, 信号A)
─────────────────────────────────────────────
```

#### 关键设计点

| 特性 | 说明 |
|------|------|
| **统一遍历** | 所有信号在同一个时间循环中处理 |
| **时间轮链接** | 同一时间点的多个信号用链表链接 |
| **逐个处理** | 每个时间点处理完所有挂在该点的信号 |
| **状态延续** | 处理 < start 的时间点是为了建立正确的信号状态 |

### 8.3 现有代码的可复用性分析

#### 代码位置关系

```
FstReader::read_signals()  (src/reader.rs:226-265)
  ↓
read_signals() 内部函数  (src/reader.rs:325-338)
  ↓
DataReader::read()  (src/reader.rs:794-843)
  ↓
DataReader::read_value_changes()  ← 这是核心处理代码！
  (src/reader.rs:650-792)
```

#### 能不能直接复用？

| 问题 | 答案 |
|------|------|
| 这段代码独立于 read_signals 吗？ | ❌ **不是**，它是 `read_signals` 的内部实现 |
| 能不能直接复用？ | ❌ **不能**，有回调依赖、没有状态追踪 |
| 能不能借鉴其架构？ | ✅ **可以！** 数据读取、时间轮、解码逻辑都能借鉴 |

#### 可借鉴的部分

| 可借鉴的部分 | 用途 |
|--------------|------|
| 时间轮指针建立（678-702 行） | 解析信号位置表，读取数据到内存 |
| 时间轮遍历（707-789 行） | 按时间顺序处理多个信号 |
| 信号值解码（729-763 行） | 解析不同类型的信号值 |

---

## 9. 查找 Start 之前最新值的算法实现思路

### 9.1 需求定义

我们需要实现一个独立函数，接口类似 `read_signals`，但是：
- 只回调 `filter.start` 之前的**最新值**
- 或者说，`start` 之前最近的一个 transition 的值
- 高效，参考 `read_signals` 的实现

### 9.2 核心设计思路

#### 设计原则

1. **借鉴但不复制**：复用数据读取和解码逻辑，但重新设计流程
2. **反向遍历**：从最新数据块开始找，而不是从最旧的开始
3. **状态追踪**：为每个信号记录最新值，而不是边处理边回调
4. **提前终止**：所有信号找到值后立即停止，不用读完所有数据

#### 核心数据结构

```rust
struct SignalState {
    has_value: bool,
    time: u64,
    value: Vec<u8>,      // 存储数字信号值
    real_value: f64,     // 存储浮点信号值
    is_real: bool,
}

struct PreStartValueReader<'a, R: Read + Seek> {
    input: &'a mut R,
    meta: &'a MetaData,
    filter: &'a DataFilter,
    // 新增：追踪每个信号的最新值
    signal_states: Vec<Option<SignalState>>,
    signals_found: usize,
    target_time: u64,
}
```

### 9.3 完整算法流程

#### 阶段 1：准备阶段

```
输入：target_time, include_signals
输出：每个信号在 target_time 之前的最新值

1. 构建 BitMask，标识哪些信号需要查找
2. 初始化 signal_states 数组
3. 计算需要查找的信号总数
```

#### 阶段 2：反向遍历数据块

```
for section in meta.data_sections.iter().rev() {  // ← 反向！从最新的开始
    
    // 快速跳过：如果这个块完全在 target_time 之后，且所有信号还没找到值
    if section.start_time > target_time && signals_found < total_signals {
        continue;
    }
    
    // 处理这个数据块
    process_section(section);
    
    // 提前终止：所有信号都找到了！
    if signals_found >= total_signals {
        break;
    }
}
```

#### 阶段 3：处理单个数据块（核心）

这部分借鉴 `read_value_changes`，但做关键修改：

```
function process_section(section):
    
    // 3.1 借鉴：读取时间戳表
    time_table = read_time_table(...)
    
    // 3.2 借鉴：读取信号位置表
    signal_offsets = read_signal_locs(...)
    
    // 3.3 借鉴：读取信号数据到内存，建立时间轮
    // 这部分完全复用 read_value_changes 的 678-702 行
    mu, head_pointer, length_remaining, tc_head = build_time_wheel(signal_offsets)
    
    // 3.4 修改：反向遍历时间点！
    for (time_id, time) in time_table.iter().enumerate().rev() {
        
        // 跳过：这个时间点在 target_time 之后
        if *time > target_time {
            continue;
        }
        
        // 3.5 修改：处理这个时间点的信号，但更新 signal_states，而不是回调
        while tc_head[time_id] != 0 {
            signal_id = (tc_head[time_id] - 1) as usize
            
            // 如果这个信号还没找到值
            if !signal_states[signal_id].has_value {
                
                // 读取信号值
                value = decode_signal_value(signal_id, ...)
                
                // 更新状态
                signal_states[signal_id] = SignalState {
                    has_value: true,
                    time: *time,
                    value: value,
                    ...
                }
                
                signals_found += 1
            }
            
            // 关键：我们不需要继续处理这个信号的更早变化！
            // 因为我们是反向遍历，第一个找到的就是最新的！
            // 所以不更新时间轮指针到下一个变化点
            tc_head[time_id] = 0  // ← 直接清零，不处理更早的变化
        }
        
        // 3.6 检查：是否所有信号都找到了？
        if signals_found >= total_signals {
            return;  // 提前终止！
        }
    }
    
    // 3.7 如果这个块有 Frame（初始值帧）
    // 对于还没找到值的信号，检查 Frame
    if is_first_section_in_reverse && has_frame {
        read_frame_and_update_states()
    }
}
```

### 9.4 关键优化点

| 优化 | 说明 |
|------|------|
| **反向遍历数据块** | 从最新的开始，找到值就能早停止 |
| **反向遍历时间点** | 在一个数据块内也从最新的时间点开始 |
| **找到就停止** | 一个信号找到最新值后，不再处理它的更早变化 |
| **提前终止** | 所有信号找到值后，立即停止整个流程 |
| **复用解码逻辑** | 信号值解码完全复用现有代码 |

### 9.5 时间复杂度分析

| 情况 | 时间复杂度 |
|------|-----------|
| 最好情况（所有信号都在最新块） | O(1) 个数据块 + O(m) 个时间点 |
| 平均情况 | O(k) 个数据块，k << 总数据块数 |
| 最坏情况（要找文件开头） | O(n) 个数据块，n = 总数据块数 |

### 9.6 与 read_signals 的对比

| 特性 | read_signals | 我们的新函数 |
|------|-------------|-------------|
| 遍历方向 | 正向（旧→新） | 反向（新→旧） |
| 回调方式 | 每个变化都回调 | 只回调最新值 |
| 状态追踪 | 无（边处理边回调） | 有（记录每个信号最新值） |
| 提前终止 | 无（处理到 filter.end） | 有（所有信号找到就停） |
| 处理 start 之前 | 为了状态正确 | 专门为了找最新值 |

---

## 10. 查找时间范围首尾数据的高效算法

### 10.1 问题背景

当需要分析一个大时间范围的波形数据时，通常只需要知道：
- 时间范围开始时的信号状态
- 时间范围结束时的信号状态

如果使用 `read_signals` 函数，需要遍历整个时间范围的所有数据，这对于大时间范围来说非常低效。

### 10.2 核心优化思路

#### 10.2.1 数据块级别快速定位

FST 文件中的每个数据块（DataSection）都有：
- `start_time`：块的起始时间
- `end_time`：块的结束时间

利用这些信息，我们可以：
1. **快速找到第一个相关块**：找到第一个 `end_time >= filter.start` 的块
2. **快速找到最后一个相关块**：找到最后一个 `start_time <= filter.end` 的块

无需遍历所有数据块！

#### 10.2.2 时间戳表二分查找

每个数据块都有完整的时间戳表（Time Table），它是一个**有序数组**！

利用二分查找：
- **找第一个时间点**：在时间戳表中二分查找第一个 `>= filter.start` 的时间
- **找最后一个时间点**：在时间戳表中二分查找最后一个 `<= filter.end` 的时间

时间复杂度：O(log N)，其中 N 是时间戳数量

#### 10.2.3 只处理目标时间点

一旦找到目标时间点，我们只需要：
1. 构建时间轮数据结构（和 `read_signals` 一样）
2. 遍历时间戳，直到到达目标时间点
3. 在目标时间点收集所有信号值
4. 立即停止，不需要继续处理后续数据

即使需要遍历到目标时间点，也只处理：
- 时间范围开始到目标时间点的数据
- 不是整个大时间范围的数据！

### 10.3 算法流程

```
输入：时间范围 [start, end]，信号列表

阶段 1：找第一个时间点
  ├─ 找到第一个包含 start 的数据块
  ├─ 读取该块的时间戳表
  ├─ 二分查找第一个 >= start 的时间点 T1
  └─ 收集 T1 时刻的所有信号值

阶段 2：找最后一个时间点
  ├─ 找到最后一个包含 end 的数据块
  ├─ 读取该块的时间戳表
  ├─ 二分查找最后一个 <= end 的时间点 T2
  └─ 收集 T2 时刻的所有信号值

输出：(T1时刻的值, T2时刻的值)
```

### 10.4 关键优化点

| 优化技术 | 传统方法 | 优化后 | 提升 |
|---------|---------|--------|------|
| 数据块定位 | O(N) 遍历 | O(1) 直接找 | 巨大 |
| 时间点定位 | O(N) 遍历 | O(log N) 二分 | 对数级 |
| 数据处理 | O(M) 全范围 | O(K) 仅目标点 | 线性级 |

其中：
- N = 数据块数量
- M = 时间范围内的总时间点数
- K = 从块开始到目标时间点的时间点数

### 10.5 边界情况处理

1. **时间范围太小**：start 和 end 在同一个数据块
   - 优化：只读取一个数据块

2. **没有找到第一个时间点**：start 之后没有数据
   - 返回 None

3. **没有找到最后一个时间点**：end 之前没有数据
   - 返回 None

4. **目标时间点正好是边界**
   - 二分查找能正确处理

### 10.6 与 read_signals 的对比

| 特性 | read_signals | read_range_boundary_values |
|------|-------------|---------------------------|
| 目标 | 读取范围内所有数据 | 只读取首尾两个时间点 |
| 数据块处理 | 所有相关块 | 最多 2 个块 |
| 时间点处理 | 所有时间点 | 最多 2 个时间点 |
| 适用场景 | 需要详细分析 | 只需要首尾状态 |
| 大时间范围性能 | 差（线性） | 优（对数+常数） |

---

## 11. 关键数据结构

### SignalInfo

```rust
pub(crate) enum SignalInfo {
    BitVec(NonZeroU32),  // 位向量，存储位宽+1
    Real,                // 64位浮点数
}
```

### DataSectionInfo

```rust
pub(crate) struct DataSectionInfo {
    pub(crate) file_offset: u64,        // 块在文件中的偏移
    pub(crate) start_time: u64,         // 块起始时间
    pub(crate) end_time: u64,           // 块结束时间
    pub(crate) kind: DataSectionKind,   // 块类型
    pub(crate) mem_required_for_traversal: u64,  // 遍历所需内存
}
```

### BitMask

```rust
pub(crate) struct BitMask {
    inner: Vec<BitMaskWord>,  // BitMaskWord = u64
}

impl BitMask {
    pub(crate) fn repeat(value: bool, size: usize) -> Self
    pub(crate) fn set(&mut self, index: usize, value: bool)
    pub(crate) fn is_set(&self, index: usize) -> bool
}
```

---

## 12. 压缩算法

FST 格式使用多种压缩算法：

| 算法 | 用途 |
|------|------|
| ZLib | 头部、几何块、层次结构块、帧数据、时间戳表 |
| LZ4 | 层次结构块、值变化数据 |
| FastLZ | 值变化数据 |
| GZip | 整个文件包装 |

---

## 13. `read_pre_start_values` 函数使用指南

### 概述

`read_pre_start_values` 函数用于在 FST 文件中查找指定时间点之前的最新信号值。这个函数是我们本次实现的新功能，它采用反向遍历和提前终止的优化策略，能够高效地找到所需的信号值。

### API 定义

```rust
pub fn read_pre_start_values(&mut self, filter: &FstFilter) -> Result<PreStartValues>
```

### 参数说明

- `filter: &FstFilter` - 过滤器，包含以下字段：
  - `start: u64` - 查找的截止时间点（函数会查找在此时间之前发生的最新值）
  - `end: Option<u64>` - （在本函数中未使用，但为了保持与 `read_signals` 接口一致）
  - `include: Option<Vec<FstSignalHandle>>` - 要查找的信号列表，如果为 `None` 则查找所有信号

### 返回值

返回 `Result<PreStartValues>`，其中 `PreStartValues` 结构体定义如下：

```rust
pub struct PreStartValues {
    pub string_values: Vec<PreStartSignalValue>,
    pub real_values: Vec<PreStartRealValue>,
}
```

- `string_values` - 包含所有字符串/位向量类型的信号值
- `real_values` - 包含所有实数类型的信号值

其中 `PreStartSignalValue` 和 `PreStartRealValue` 的定义：

```rust
pub struct PreStartSignalValue {
    pub handle: FstSignalHandle,  // 信号句柄
    pub value: Vec<u8>,            // 信号值（字节数组）
    pub time: u64,                 // 值发生的时间
}

pub struct PreStartRealValue {
    pub handle: FstSignalHandle,  // 信号句柄
    pub value: f64,                // 信号值（浮点数）
    pub time: u64,                 // 值发生的时间
}
```

### 使用示例

#### 示例 1：查找所有信号在时间 1000 之前的最新值

```rust
use fst_reader::{FstReader, FstFilter};
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("waveform.fst")?;
    let reader = BufReader::new(file);
    let mut fst = FstReader::open(reader)?;

    let filter = FstFilter {
        start: 1000,
        end: None,
        include: None,  // 查找所有信号
    };

    let pre_start_values = fst.read_pre_start_values(&filter)?;

    println!("Found {} string values:", pre_start_values.string_values.len());
    for val in &pre_start_values.string_values {
        println!("  Signal {:?} at time {}: {:?}", 
                 val.handle, val.time, val.value);
    }

    println!("Found {} real values:", pre_start_values.real_values.len());
    for val in &pre_start_values.real_values {
        println!("  Signal {:?} at time {}: {}", 
                 val.handle, val.time, val.value);
    }

    Ok(())
}
```

#### 示例 2：查找特定信号在时间 500 之前的最新值

```rust
use fst_reader::{FstReader, FstFilter, FstSignalHandle};
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("waveform.fst")?;
    let reader = BufReader::new(file);
    let mut fst = FstReader::open(reader)?;

    // 假设我们有一些信号句柄
    let signals = vec![
        FstSignalHandle::from_index(0),
        FstSignalHandle::from_index(5),
        FstSignalHandle::from_index(10),
    ];

    let filter = FstFilter {
        start: 500,
        end: None,
        include: Some(signals),  // 只查找指定的信号
    };

    let pre_start_values = fst.read_pre_start_values(&filter)?;

    // 处理结果...
    Ok(())
}
```

### 性能特点

1. **反向遍历**：从 `filter.start` 时间点向后搜索，而不是从文件开头向前搜索
2. **提前终止**：一旦所有请求的信号都找到，立即停止搜索
3. **分层过滤**：先过滤数据块，再过滤时间点，最后只处理需要的信号

### 注意事项

1. 如果某个信号在 `filter.start` 之前没有任何变化，则该信号不会出现在结果中
2. 结果中的值是每个信号在 `filter.start` 之前的**最新**值
3. 函数返回的时间是值实际发生变化的时间，而不是 `filter.start`
4. 字符串值以字节数组形式返回，可能需要根据信号类型进行解释

---

## 14. `read_range_boundary_values` 函数使用指南

### 概述

`read_range_boundary_values` 函数用于高效地查找指定时间范围内的第一个和最后一个信号值。这个函数特别优化了大时间范围的场景，通过二分查找和选择性数据处理，避免了遍历整个时间范围的开销。

### API 定义

```rust
pub fn read_range_boundary_values(&mut self, filter: &FstFilter) -> Result<RangeBoundaryValues>
```

### 参数说明

- `filter: &FstFilter` - 过滤器，包含以下字段：
  - `start: u64` - 时间范围的起始时间
  - `end: Option<u64>` - 时间范围的结束时间
  - `include: Option<Vec<FstSignalHandle>>` - 要查找的信号列表，如果为 `None` 则查找所有信号

### 返回值

返回 `Result<RangeBoundaryValues>`，其中 `RangeBoundaryValues` 结构体定义如下：

```rust
pub struct RangeBoundaryValues {
    pub first: Option<TimePointValues>,
    pub last: Option<TimePointValues>,
}
```

- `first` - 时间范围内第一个时间点的信号值（如果存在）
- `last` - 时间范围内最后一个时间点的信号值（如果存在）

其中 `TimePointValues` 的定义：

```rust
pub struct TimePointValues {
    pub time: u64,
    pub string_values: Vec<PreStartSignalValue>,
    pub real_values: Vec<PreStartRealValue>,
}
```

### 使用示例

#### 示例 1：查找大时间范围的首尾数据

```rust
use fst_reader::{FstReader, FstFilter};
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("large_waveform.fst")?;
    let reader = BufReader::new(file);
    let mut fst = FstReader::open(reader)?;

    // 定义一个很大的时间范围
    let filter = FstFilter {
        start: 1000,
        end: Some(1_000_000),  // 非常大的结束时间
        include: None,            // 查找所有信号
    };

    let boundary_values = fst.read_range_boundary_values(&filter)?;

    // 处理第一个时间点的数据
    if let Some(first) = &boundary_values.first {
        println!("First time point at {}", first.time);
        println!("  String values: {}", first.string_values.len());
        println!("  Real values: {}", first.real_values.len());
    }

    // 处理最后一个时间点的数据
    if let Some(last) = &boundary_values.last {
        println!("Last time point at {}", last.time);
        println!("  String values: {}", last.string_values.len());
        println!("  Real values: {}", last.real_values.len());
    }

    Ok(())
}
```

#### 示例 2：查找特定信号的首尾数据

```rust
use fst_reader::{FstReader, FstFilter, FstSignalHandle};
use std::fs::File;
use std::io::BufReader;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let file = File::open("waveform.fst")?;
    let reader = BufReader::new(file);
    let mut fst = FstReader::open(reader)?;

    // 只关注特定的信号
    let signals = vec![
        FstSignalHandle::from_index(0),
        FstSignalHandle::from_index(10),
        FstSignalHandle::from_index(20),
    ];

    let filter = FstFilter {
        start: 5000,
        end: Some(50000),
        include: Some(signals),
    };

    let boundary_values = fst.read_range_boundary_values(&filter)?;

    // 比较首尾状态的变化
    if let (Some(first), Some(last)) = (&boundary_values.first, &boundary_values.last) {
        println!("Signal changes from {} to {}", first.time, last.time);
        
        // 可以在这里对比每个信号的变化
        for (first_val, last_val) in first.string_values.iter().zip(&last.string_values) {
            if first_val.value != last_val.value {
                println!("Signal {:?} changed!", first_val.handle);
            }
        }
    }

    Ok(())
}
```

### 性能特点

1. **数据块级快速定位**：利用 DataSectionInfo 的 start_time 和 end_time 快速定位相关块
2. **时间戳表二分查找**：在有序的时间戳表上使用二分查找，O(log N) 复杂度
3. **选择性数据处理**：只处理到目标时间点，不处理整个范围的数据
4. **信号过滤优化**：使用 BitMask 只处理感兴趣的信号

### 时间复杂度分析

| 操作 | 时间复杂度 | 说明 |
|------|-----------|------|
| 数据块定位 | O(1) | 直接利用元数据 |
| 时间点查找 | O(log N) | 二分查找时间戳表 |
| 数据读取 | O(K) | K = 到目标时间点的数据量 |

其中 N = 时间戳总数，K << 总时间点数（对于大时间范围）

### 注意事项

1. **适用于大时间范围**：当时间范围跨越很多数据块时，优化效果最明显
2. **首尾可能相同**：如果时间范围内只有一个时间点，first 和 last 会指向同一个时间点
3. **可能返回 None**：如果时间范围内没有数据，first 和 last 都可能是 None
4. **只返回变化的信号**：在目标时间点没有变化的信号不会出现在结果中

### 与 read_signals 的性能对比

假设我们有一个包含 100 个数据块、每个块 1000 个时间点的文件：

| 场景 | read_signals | read_range_boundary_values |
|------|-------------|---------------------------|
| 小范围（1-2个块） | 快 | 类似 |
| 中等范围（10个块） | 中等 | 快 |
| 大范围（50个块） | 慢 | 快 |
| 极大范围（全部100个块） | 非常慢 | 快（只处理2个块） |

---

## 参考资料

- [FST Format Specification](https://blog.timhutt.co.uk/fst_spec/)
- [GTKWave FST Documentation](https://gtkwave.sourceforge.net/)
- fst-reader 源代码：`src/reader.rs`, `src/io.rs`, `src/types.rs`
