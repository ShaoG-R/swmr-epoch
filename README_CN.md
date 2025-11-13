# SWMR-Epoch: 无锁单写多读纪元式垃圾回收系统

[![Crates.io](https://img.shields.io/crates/v/swmr-epoch.svg)](https://crates.io/crates/swmr-epoch)
[![License](https://img.shields.io/crates/l/swmr-epoch.svg)](https://github.com/ShaoG-R/swmr-epoch#license)
[![Docs.rs](https://docs.rs/swmr-epoch/badge.svg)](https://docs.rs/swmr-epoch)
[![GitHub](https://img.shields.io/badge/github-ShaoG--R/swmr--epoch-blue.svg)](https://github.com/ShaoG-R/swmr-epoch)

[English Documentation](./README.md)

一个高性能的无锁垃圾回收系统，实现单写多读（SWMR）纪元式内存回收机制。专为需要安全、高效内存管理的并发数据结构设计。

## 特性

- **无锁设计**：完全基于原子操作和内存排序，无互斥锁
- **单写多读（SWMR）**：一个写入线程，无限个读取线程
- **纪元式垃圾回收**：延迟删除，自动回收
- **类型安全**：完整的 Rust 类型安全保证
- **原子指针**：`Atomic<T>` 包装器用于安全并发访问
- **零复制读取**：读取者直接获取引用，无分配
- **自动参与者清理**：弱指针自动移除不活跃的读取者

## 架构

### 核心组件

**Writer（写入者）**
- 单线程写入者，推进全局纪元
- 管理垃圾回收和延迟删除
- 维护参与者列表以跟踪活跃读取者

**ReaderFactory & ReaderHandle**
- `ReaderFactory`：可克隆的读取者工厂
- `ReaderHandle`：每线程读取者句柄，支持纪元钉住
- `ReaderGuard`：RAII 守卫，确保临界区安全访问

**Atomic<T>**
- 类型安全的原子指针包装器
- 读取操作需要活跃的 `ReaderGuard`
- 存储操作触发垃圾回收

### 内存排序

- **Acquire/Release 语义**：确保读取者和写入者之间的正确同步
- **纪元推进使用 SeqCst**：保证纪元转换的全序关系
- **Relaxed 操作**：在不需要排序的地方用于性能优化

## 使用示例

```rust
use swmr_epoch::{new, Atomic};

fn main() {
    let (mut writer, reader_factory) = new();
    
    // 创建原子指针
    let data = Atomic::new(42i32);
    
    // 读取者线程
    let factory_clone = reader_factory.clone();
    let reader_thread = std::thread::spawn(move || {
        let handle = factory_clone.create_handle();
        let guard = handle.pin();
        let value = data.load(&guard);
        println!("读取值: {}", value);
    });
    
    // 写入者线程
    data.store(Box::new(100), &mut writer);
    writer.try_reclaim();
    
    reader_thread.join().unwrap();
}
```

## 性能分析

### 基准测试结果

所有基准测试在现代多核系统上运行。结果显示中位数时间及 95% 置信区间。

#### 1. 单线程钉住/取消钉住操作

| 基准测试 | SWMR-Epoch | Crossbeam-Epoch | 优势 |
|---------|-----------|-----------------|------|
| 钉住/取消 | 3.42 ns | 6.21 ns | **快 1.82 倍** |

SWMR-Epoch 的简化纪元模型提供比 Crossbeam 更快的钉住/取消操作。

#### 2. 读取者注册（延迟）

| 线程数 | SWMR-Epoch | Crossbeam-Epoch | 比率 |
|-------|-----------|-----------------|------|
| 2 线程 | 152.32 µs | 145.13 µs | 慢 1.05 倍 |
| 4 线程 | 228.35 µs | 230.32 µs | 0.99 倍（相当） |
| 8 线程 | 395.54 µs | 394.71 µs | 1.00 倍（相当） |
| 16 线程 | 708.79 µs | 724.51 µs | **快 0.98 倍** |

**取舍分析**：SWMR-Epoch 使用无锁队列处理待注册读取者，开销最小。在高线程数（8+）时，SWMR-Epoch 由于注册机制可扩展性更好，性能相当或略优于 Crossbeam。

#### 3. 垃圾回收性能

| 操作 | SWMR-Epoch | Crossbeam-Epoch | 比率 |
|-----|-----------|-----------------|------|
| 回收 100 项 | 5.40 µs | 2.04 µs | **慢 2.65 倍** |
| 回收 1,000 项 | 50.18 µs | 38.27 µs | **慢 1.31 倍** |
| 回收 10,000 项 | 567.37 µs | 227.83 µs | **慢 2.49 倍** |

**取舍分析**：SWMR-Epoch 的垃圾回收较慢，原因如下：
- 每次回收时执行完整的参与者列表扫描（O(N)）
- Crossbeam 使用更复杂的数据结构（如线程本地袋）
- SWMR-Epoch 优先考虑简洁性和无锁保证，而非 GC 吞吐量

**使用建议**：在以下场景使用 SWMR-Epoch：
- 读取密集型工作负载（GC 不频繁）
- 需要延迟可预测性
- 需要无锁保证

#### 4. 原子加载操作

| 基准测试 | SWMR-Epoch | Crossbeam-Epoch | 优势 |
|---------|-----------|-----------------|------|
| 加载 | 3.57 ns | 416.19 ns | **快 116.5 倍** |

SWMR-Epoch 的原子加载快 100 倍左右，因为它只执行简单的 `Acquire` 加载，无额外开销。Crossbeam 的开销来自其更复杂的纪元跟踪机制。

#### 5. 并发读取（吞吐量）⭐ **SWMR-Epoch 领先**

| 线程数 | SWMR-Epoch | Crossbeam-Epoch | 加速比 |
|-------|-----------|-----------------|--------|
| 2 线程 | 137.33 µs | 992.92 µs | **快 7.23 倍** |
| 4 线程 | 236.07 µs | 1,940.7 µs | **快 8.22 倍** |
| 8 线程 | 397.33 µs | 3,684.8 µs | **快 9.27 倍** |

**关键优势**：SWMR-Epoch 在并发读取工作负载下表现出色：
- 随线程数线性扩展
- 共享状态上的争用最小
- 无锁设计消除读取者阻塞
- 简化的纪元模型降低每次操作开销

这是 SWMR-Epoch 的主要优势——专为读取密集型并发工作负载设计。

## 设计决策

### 为什么选择 SWMR？

- **简洁性**：单写入者消除写-写冲突和复杂同步
- **性能**：读取者永不阻塞读取者；写入者操作可预测
- **安全性**：单写入者更易于正确性推理

### 为什么选择纪元式 GC？

- **无锁**：无需引用计数或原子 CAS 循环
- **可预测**：延迟删除提供有界延迟
- **可扩展**：读取操作在常见情况下为 O(1)

### 为什么对参与者使用弱指针？

- **自动清理**：已删除的读取者自动从跟踪中移除
- **无显式注销**：读取者退出时无需通知写入者
- **内存高效**：避免维护过时的参与者条目

## 限制

1. **单写入者**：同时只有一个线程可以写入
2. **GC 吞吐量**：完整参与者扫描使垃圾回收比专用系统慢
3. **纪元溢出**：使用 `usize` 表示纪元；溢出理论上可能但实际不可行

## 构建与测试

```bash
# 构建库
cargo build --release

# 运行测试
cargo test

# 运行基准测试
cargo bench --bench epoch_comparison
cargo bench --bench concurrent_workload
```

## 依赖

- `crossbeam-queue`：用于待注册读取者的无锁队列
- `criterion`：基准测试框架（开发依赖）

## 许可证

在 Apache License, Version 2.0 或 MIT 许可证下授权，任选其一。

## 参考资源

- Keir Fraser. "Practical Lock-Freedom"（博士论文，2004）
- Hart, McKenney, Brown. "Performance of Memory Reclamation for Lock-Free Synchronization"（2007）
- Crossbeam Epoch 文档：https://docs.rs/crossbeam-epoch/
