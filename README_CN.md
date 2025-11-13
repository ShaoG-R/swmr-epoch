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

**ReaderRegistry（读取者注册表）**
- `ReaderRegistry`：可克隆的注册表，用于管理线程本地读取者状态
- `pin()`：钉住当前线程并返回 `Guard`
- `Guard`：RAII 守卫，确保临界区安全访问

**Atomic<T>**
- 类型安全的原子指针包装器
- 读取操作需要活跃的 `Guard`
- 存储操作触发垃圾回收

### 内存排序

- **Acquire/Release 语义**：确保读取者和写入者之间的正确同步
- **纪元推进使用 SeqCst**：保证纪元转换的全序关系
- **Relaxed 操作**：在不需要排序的地方用于性能优化

## 使用示例

```rust
use swmr_epoch::{new, Atomic};

fn main() {
    let (mut writer, reader_registry) = new();
    
    // 创建原子指针
    let data = Atomic::new(42i32);
    
    // 读取者线程
    let registry_clone = reader_registry.clone();
    let reader_thread = std::thread::spawn(move || {
        let guard = registry_clone.pin();
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
| 钉住/取消 | 1.63 ns | 5.57 ns | **快 3.42 倍** |

SWMR-Epoch 的简化纪元模型提供比 Crossbeam 更快的钉住/取消操作，性能提升了 3 倍以上。

#### 2. 读取者注册（延迟）

| 线程数 | SWMR-Epoch | Crossbeam-Epoch | 比率 |
|-------|-----------|-----------------|------|
| 2 线程 | 72.02 µs | 78.35 µs | **快 1.09 倍** |
| 4 线程 | 128.11 µs | 137.77 µs | **快 1.08 倍** |
| 8 线程 | 239.55 µs | 251.50 µs | **快 1.05 倍** |
| 16 线程 | 454.38 µs | 479.11 µs | **快 1.05 倍** |

**性能提升**：
- 2 线程性能提升至 1.09 倍（原 1.05 倍）
- 4 线程性能提升至 1.08 倍（原 1.06 倍）
- 整体性能提升约 2-3%

#### 3. 垃圾回收性能

| 操作 | SWMR-Epoch | Crossbeam-Epoch | 比率 |
|-----|-----------|-----------------|------|
| 回收 100 项 | 3.18 µs | 0.94 µs | **慢 3.38 倍** |
| 回收 1,000 项 | 29.95 µs | 14.44 µs | **慢 2.07 倍** |
| 回收 10,000 项 | 297.00 µs | 140.87 µs | **慢 2.11 倍** |

**性能变化**：
- 小批量回收（100 项）性能保持稳定
- 大批量回收（10,000 项）性能差距略有扩大（从 1.63x 到 2.11x）

**优化建议**：
- 考虑批量回收机制减少小对象回收开销
- 优化大对象回收路径

#### 4. 原子加载操作

| 基准测试 | SWMR-Epoch | Crossbeam-Epoch | 优势 |
|---------|-----------|-----------------|------|
| 加载 | 1.63 ns | 306.63-412.44 ns | **快 188-253 倍** |

**显著提升**：
- 原子加载性能提升 73-133%，从 108x 提升至 188-253x
- 证明 SWMR-Epoch 在读取性能上的绝对优势

#### 5. 并发读取（吞吐量）⭐ **SWMR-Epoch 领先**

| 线程数 | SWMR-Epoch | Crossbeam-Epoch | 加速比 |
|-------|-----------|-----------------|--------|
| 2 线程 | 80.84 µs | 633.65 µs | **快 7.84 倍** |
| 4 线程 | 134.46 µs | 1.26 ms | **快 9.37 倍** |
| 8 线程 | 238.35 µs | 1.29 ms | **快 5.41 倍** |

**关键发现**：
- 2-4 线程下性能提升 3-7%
- 8 线程下性能有所下降（从 13.28x 降至 5.41x）
- 可能原因：高并发下资源争用增加

**优化方向**：
- 分析 8 线程性能下降原因
- 优化高并发场景下的资源争用

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
