# SWMR-Epoch: Lock-Free Single-Writer Multi-Reader Epoch-Based GC

[![Crates.io](https://img.shields.io/crates/v/swmr-epoch.svg)](https://crates.io/crates/swmr-epoch)
[![License](https://img.shields.io/crates/l/swmr-epoch.svg)](https://github.com/ShaoG-R/swmr-epoch#license)
[![Docs.rs](https://docs.rs/swmr-epoch/badge.svg)](https://docs.rs/swmr-epoch)
[![GitHub](https://img.shields.io/badge/github-ShaoG--R/swmr--epoch-blue.svg)](https://github.com/ShaoG-R/swmr-epoch)

[中文文档](./README_CN.md)

A high-performance, lock-free garbage collection system for Rust implementing Single-Writer Multi-Reader (SWMR) epoch-based memory reclamation. Designed for concurrent data structures requiring safe, efficient memory management without locks.

## Features

- **Lock-Free Design**: No mutexes or locks—purely atomic operations and memory ordering
- **Single-Writer Multi-Reader (SWMR)**: One writer thread, unlimited reader threads
- **Epoch-Based Garbage Collection**: Deferred deletion with automatic reclamation
- **Type-Safe**: Full Rust type safety with compile-time guarantees
- **Atomic Pointers**: `Atomic<T>` wrapper for safe concurrent access
- **Zero-Copy Reads**: Readers obtain direct references without allocation
- **Automatic Participant Cleanup**: Weak pointers automatically remove inactive readers

## Architecture

### Core Components

**Writer**
- Single-threaded writer that advances the global epoch
- Manages garbage collection and deferred deletion
- Maintains participant list for tracking active readers

**ReaderFactory & ReaderHandle**
- `ReaderFactory`: Clone-safe factory for creating reader handles
- `ReaderHandle`: Per-thread reader handle with epoch pinning
- `ReaderGuard`: RAII guard ensuring safe access during critical sections

**Atomic<T>**
- Type-safe atomic pointer wrapper
- Load operations require active `ReaderGuard`
- Store operations trigger garbage collection

### Memory Ordering

- **Acquire/Release semantics**: Ensures proper synchronization between readers and writer
- **SeqCst for epoch advancement**: Guarantees total ordering of epoch transitions
- **Relaxed operations**: Used where ordering is not required for performance

## Usage Example

```rust
use swmr_epoch::{new, Atomic};

fn main() {
    let (mut writer, reader_factory) = new();
    
    // Create an atomic pointer
    let data = Atomic::new(42i32);
    
    // Reader thread
    let factory_clone = reader_factory.clone();
    let reader_thread = std::thread::spawn(move || {
        let handle = factory_clone.create_handle();
        let guard = handle.pin();
        let value = data.load(&guard);
        println!("Read value: {}", value);
    });
    
    // Writer thread
    data.store(Box::new(100), &mut writer);
    writer.try_reclaim();
    
    reader_thread.join().unwrap();
}
```

## Performance Analysis

### Benchmark Results

All benchmarks run on a modern multi-core system. Results show median time with 95% confidence intervals.

#### 1. Single-Thread Pin/Unpin Operations

| Benchmark | SWMR-Epoch | Crossbeam-Epoch | Advantage |
|-----------|-----------|-----------------|-----------|
| Pin/Unpin | 3.42 ns | 6.21 ns | **1.82x faster** |

SWMR-Epoch's simpler epoch model provides faster pin/unpin operations compared to Crossbeam's more complex implementation.

#### 2. Reader Registration (Latency)

| Thread Count | SWMR-Epoch | Crossbeam-Epoch | Ratio |
|-------------|-----------|-----------------|-------|
| 2 threads | 152.32 µs | 145.13 µs | 1.05x slower |
| 4 threads | 228.35 µs | 230.32 µs | 0.99x (comparable) |
| 8 threads | 395.54 µs | 394.71 µs | 1.00x (comparable) |
| 16 threads | 708.79 µs | 724.51 µs | **0.98x faster** |

**Trade-off Analysis**: SWMR-Epoch uses a lock-free queue for pending registrations, which adds minimal overhead. At higher thread counts (8+), SWMR-Epoch matches or slightly outperforms Crossbeam due to better scalability of the registration mechanism.

#### 3. Garbage Collection Performance

| Operation | SWMR-Epoch | Crossbeam-Epoch | Ratio |
|-----------|-----------|-----------------|-------|
| Retire 100 items | 5.40 µs | 2.04 µs | **2.65x slower** |
| Retire 1,000 items | 50.18 µs | 38.27 µs | **1.31x slower** |
| Retire 10,000 items | 567.37 µs | 227.83 µs | **2.49x slower** |

**Trade-off Analysis**: SWMR-Epoch's garbage collection is slower because:
- It performs full participant list scans (O(N)) on each reclamation
- Crossbeam uses more sophisticated data structures (e.g., thread-local bags)
- SWMR-Epoch prioritizes simplicity and lock-free guarantees over GC throughput

**Recommendation**: Use SWMR-Epoch when:
- Read-heavy workloads dominate (GC is infrequent)
- Latency predictability is critical
- Lock-free guarantees are essential

#### 4. Atomic Load Operations

| Benchmark | SWMR-Epoch | Crossbeam-Epoch | Advantage |
|-----------|-----------|-----------------|-----------|
| Load | 3.57 ns | 416.19 ns | **116.5x faster** |

SWMR-Epoch's atomic load is nearly 100x faster because it performs a simple `Acquire` load without additional bookkeeping. Crossbeam's overhead comes from its more complex epoch tracking.

#### 5. Concurrent Reads (Throughput) ⭐ **SWMR-Epoch Excels**

| Thread Count | SWMR-Epoch | Crossbeam-Epoch | Speedup |
|-------------|-----------|-----------------|---------|
| 2 threads | 137.33 µs | 992.92 µs | **7.23x faster** |
| 4 threads | 236.07 µs | 1,940.7 µs | **8.22x faster** |
| 8 threads | 397.33 µs | 3,684.8 µs | **9.27x faster** |

**Key Advantage**: SWMR-Epoch demonstrates exceptional performance under concurrent read workloads:
- Linear scalability with thread count
- Minimal contention on shared state
- Lock-free design eliminates reader blocking
- Simple epoch model reduces per-operation overhead

This is the primary strength of SWMR-Epoch—it's purpose-built for read-heavy concurrent workloads.

## Design Decisions

### Why SWMR?

- **Simplicity**: Single writer eliminates write-write conflicts and complex synchronization
- **Performance**: Readers never block readers; writer operations are predictable
- **Safety**: Easier to reason about correctness with one writer

### Why Epoch-Based GC?

- **Lock-Free**: No need for reference counting or atomic CAS loops
- **Predictable**: Deferred deletion provides bounded latency
- **Scalable**: Reader operations are O(1) in the common case

### Why Weak Pointers for Participants?

- **Automatic Cleanup**: Dropped readers are automatically removed from tracking
- **No Explicit Unregistration**: Readers don't need to notify the writer on exit
- **Memory Efficient**: Avoids maintaining stale participant entries

## Limitations

1. **Single Writer**: Only one thread can write at a time
2. **GC Throughput**: Full participant scans make garbage collection slower than specialized systems
3. **Epoch Overflow**: Uses `usize` for epochs; overflow is theoretically possible but impractical

## Building & Testing

```bash
# Build the library
cargo build --release

# Run tests
cargo test

# Run benchmarks
cargo bench --bench epoch_comparison
cargo bench --bench concurrent_workload
```

## Dependencies

- `crossbeam-queue`: Lock-free queue for pending reader registrations
- `criterion`: Benchmarking framework (dev-dependency)

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

## References

- Keir Fraser. "Practical Lock-Freedom" (PhD thesis, 2004)
- Hart, McKenney, Brown. "Performance of Memory Reclamation for Lock-Free Synchronization" (2007)
- Crossbeam Epoch documentation: https://docs.rs/crossbeam-epoch/
