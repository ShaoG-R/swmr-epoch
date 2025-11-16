# SWMR-Epoch: 单写多读纪元式垃圾回收系统（最小化锁定）

[![Crates.io](https://img.shields.io/crates/v/swmr-epoch.svg)](https://crates.io/crates/swmr-epoch)
[![License](https://img.shields.io/crates/l/swmr-epoch.svg)](https://github.com/ShaoG-R/swmr-epoch#license)
[![Docs.rs](https://docs.rs/swmr-epoch/badge.svg)](https://docs.rs/swmr-epoch)
[![GitHub](https://img.shields.io/badge/github-ShaoG--R/swmr--epoch-blue.svg)](https://github.com/ShaoG-R/swmr-epoch)

[English Documentation](./README.md)

一个高性能的垃圾回收系统，实现单写多读（SWMR）纪元式内存回收机制。专为需要安全、高效内存管理的并发数据结构设计。采用最小化锁定方案（单个 Mutex 用于读取者跟踪）结合原子操作实现核心纪元机制。

## 特性

- **最小化锁定**：仅在读取者注册跟踪中使用单个 Mutex；核心纪元机制使用原子操作
- **单写多读（SWMR）**：一个写入线程，无限个读取线程
- **纪元式垃圾回收**：延迟删除，自动回收
- **类型安全**：完整的 Rust 类型安全保证
- **纪元保护指针**：`EpochPtr<T>` 包装器用于安全并发访问
- **零复制读取**：读取者直接获取引用，无分配
- **自动读取者清理**：弱指针自动移除不活跃的读取者
- **可重入钉住**：通过引用计数支持嵌套 pin 守卫

## 架构

### 核心组件

**EpochGcDomain（纪元 GC 域）**
- 创建基于纪元的 GC 系统的入口点
- 可克隆，可在线程间安全共享
- 管理全局纪元计数器和读取者注册

**GcHandle（垃圾回收句柄）**
- 域的唯一垃圾回收器，由写入者线程持有
- 在回收周期中推进全局纪元
- 接收已退休对象并扫描活跃读取者进行回收
- 不是线程安全的；必须由单个线程持有

**LocalEpoch（本地纪元）**
- 读取者线程的本地纪元状态
- 不是 `Sync` 的（因为 `Cell`），必须在每个线程中存储
- 用于钉住线程并获取 `PinGuard` 以安全访问

**PinGuard（钉住守卫）**
- RAII 守卫，保持当前线程被钉住到一个纪元
- 防止写入者在读取期间回收数据
- 支持克隆以通过引用计数实现嵌套钉住
- 生命周期绑定到它来自的 `LocalEpoch`

**EpochPtr<T>（纪元保护指针）**
- 类型安全的原子指针包装器
- 读取操作需要活跃的 `PinGuard`
- 存储操作在垃圾计数超过配置阈值时可能触发自动垃圾回收
- 在写入者和读取者线程间安全管理内存

### 内存排序

- **Acquire/Release 语义**：确保读取者和写入者之间的正确同步
- **纪元加载使用 Acquire**：读取者与写入者的纪元推进同步
- **纪元存储使用 Release**：写入者确保对所有读取者的可见性
- **Relaxed 操作**：在不需要排序的地方用于性能优化

## 使用示例

```rust
use swmr_epoch::{EpochGcDomain, EpochPtr};
use std::sync::Arc;

fn main() {
    // 1. 创建共享 GC 域并获取垃圾回收器
    let (mut gc, domain) = EpochGcDomain::new();
    
    // 2. 创建纪元保护指针，用 Arc 包装以便线程间共享
    let data = Arc::new(EpochPtr::new(42i32));
    
    // 3. 读取者线程
    let domain_clone = domain.clone();
    let data_clone = data.clone();
    let reader_thread = std::thread::spawn(move || {
        let local_epoch = domain_clone.register_reader();
        let guard = local_epoch.pin();
        let value = data_clone.load(&guard);
        println!("读取值: {}", value);
    });
    
    // 4. 写入者线程：更新并回收垃圾
    data.store(100, &mut gc);
    gc.collect();
    
    reader_thread.join().unwrap();
}
```

## 高级用法

### 使用构建器模式配置

使用构建器模式自定义 GC 行为：

```rust
use swmr_epoch::EpochGcDomain;

// 使用构建器模式配置
let (mut gc, domain) = EpochGcDomain::builder()
    .auto_reclaim_threshold(128)    // 在 128 项时触发回收
    .cleanup_interval(32)            // 每 32 次回收清理死读者
    .build();

// 完全禁用自动回收
let (mut gc, domain) = EpochGcDomain::builder()
    .auto_reclaim_threshold(None)   // 不自动回收
    .build();
gc.collect();  // 需要时手动触发回收
```

**配置选项**：
- `auto_reclaim_threshold(n)`：当垃圾计数超过 `n` 时触发自动 GC（默认：64）。传递 `None` 可禁用
- `cleanup_interval(n)`：每 `n` 个回收周期清理死读者槽（默认：16）

### 嵌套钉住

`PinGuard` 支持克隆以实现嵌套钉住场景：

```rust
let guard1 = local_epoch.pin();
let guard2 = guard1.clone();  // 嵌套钉住 - 线程保持被钉住
let guard3 = guard1.clone();  // 支持多个嵌套钉住

// 线程保持被钉住直到所有守卫被 drop
drop(guard3);
drop(guard2);
drop(guard1);
```

## 核心概念

### 纪元（Epoch）

一个单调递增的逻辑时间戳。写入者在垃圾回收周期中推进纪元。读取者"钉住"自己到一个纪元，声明他们正在从该纪元读取数据。

### 钉住（Pin）

当读取者调用 `pin()` 时，它在其槽中记录当前纪元。这告诉写入者："我正在从这个纪元读取数据；不要回收它。"

### 回收（Reclamation）

写入者收集已退休的对象，并回收来自比所有活跃读取者的最小纪元更旧的纪元的对象。

## 设计决策

### 为什么选择 SWMR？

- **简洁性**：单写入者消除写-写冲突和复杂同步
- **性能**：读取者在正常读取期间不相互阻塞；写入者操作可预测
- **安全性**：单写入者更易于正确性推理

### 为什么选择纪元式 GC？

- **最小化同步**：纪元机制使用原子操作；仅在回收期间使用 Mutex 跟踪读取者
- **可预测**：延迟删除提供有界延迟
- **可扩展**：读取操作在常见情况下为 O(1)（无 CAS 循环或引用计数开销）
- **优化的回收**：批量清理死读者减少每次回收的开销

### 为什么对读取者使用弱指针？

- **自动清理**：已删除的读取者自动从跟踪中移除
- **无显式注销**：读取者退出时无需通知写入者
- **内存高效**：避免维护过时的读取者条目

### 为什么支持可重入钉住？

- **灵活性**：允许嵌套临界区，无需显式守卫管理
- **安全性**：钉住计数确保正确的解钉顺序
- **简洁性**：开发者无需手动跟踪钉住深度

## 限制

1. **单写入者**：同时只有一个线程可以写入
2. **GC 吞吐量**：回收期间需要扫描读取者；通过批量清理死读者槽优化性能
3. **纪元溢出**：使用 `usize` 表示纪元；溢出理论上可能但实际不可行
4. **自动回收**：当超过阈值时自动触发垃圾回收，可能导致延迟尖峰。可以使用构建器模式禁用或自定义
5. **读取者跟踪 Mutex**：使用单个 Mutex 来跟踪垃圾回收期间的活跃读取者。虽然这是最小的同步点，但不是完全无锁的。性能测试表明，无锁替代方案（例如 SegQueue）由于竞争和内存排序开销而导致性能更差。为了最小化开销，死读者清理采用批量处理（可通过构建器中的 `cleanup_interval` 配置）

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

- `criterion`：基准测试框架（开发依赖）

## 许可证

在 Apache License, Version 2.0 或 MIT 许可证下授权，任选其一。

## 参考资源

- Keir Fraser. "Practical Lock-Freedom"（博士论文，2004）
- Hart, McKenney, Brown. "Performance of Memory Reclamation for Lock-Free Synchronization"（2007）
- Crossbeam Epoch 文档：https://docs.rs/crossbeam-epoch/
