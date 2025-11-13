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

**ReaderRegistry**
- `ReaderRegistry`: Clone-safe registry for managing thread-local reader state
- `pin()`: Pins the current thread and returns a `Guard`
- `Guard`: RAII guard ensuring safe access during critical sections

**Atomic<T>**
- Type-safe atomic pointer wrapper
- Load operations require active `Guard`
- Store operations trigger garbage collection

### Memory Ordering

- **Acquire/Release semantics**: Ensures proper synchronization between readers and writer
- **SeqCst for epoch advancement**: Guarantees total ordering of epoch transitions
- **Relaxed operations**: Used where ordering is not required for performance

## Usage Example

```rust
use swmr_epoch::{new, Atomic};

fn main() {
    let (mut writer, reader_registry) = new();
    
    // Create an atomic pointer
    let data = Atomic::new(42i32);
    
    // Reader thread
    let registry_clone = reader_registry.clone();
    let reader_thread = std::thread::spawn(move || {
        let guard = registry_clone.pin();
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
| Pin/Unpin | 1.63 ns | 5.57 ns | **3.42x faster** |

SWMR-Epoch's simplified epoch model provides significantly faster pin/unpin operations, now over 3x faster than Crossbeam.

#### 2. Reader Registration (Latency)

| Thread Count | SWMR-Epoch | Crossbeam-Epoch | Ratio |
|-------------|-----------|-----------------|-------|
| 2 threads | 72.02 µs | 78.35 µs | **1.09x faster** |
| 4 threads | 128.11 µs | 137.77 µs | **1.08x faster** |
| 8 threads | 239.55 µs | 251.50 µs | **1.05x faster** |
| 16 threads | 454.38 µs | 479.11 µs | **1.05x faster** |

**Performance Improvements**:
- 2-thread performance improved to 1.09x (from 1.05x)
- 4-thread performance improved to 1.08x (from 1.06x)
- Overall 2-3% performance gain across thread counts

#### 3. Garbage Collection Performance

| Operation | SWMR-Epoch | Crossbeam-Epoch | Ratio |
|-----------|-----------|-----------------|-------|
| Retire 100 items | 3.18 µs | 0.94 µs | **3.38x slower** |
| Retire 1,000 items | 29.95 µs | 14.44 µs | **2.07x slower** |
| Retire 10,000 items | 297.00 µs | 140.87 µs | **2.11x slower** |

**Performance Notes**:
- Small batch (100 items) performance remains stable
- Large batch (10,000 items) performance gap slightly increased (from 1.63x to 2.11x)

**Optimization Opportunities**:
- Consider batching mechanisms for small object reclamation
- Optimize large object reclamation path

#### 4. Atomic Load Operations

| Benchmark | SWMR-Epoch | Crossbeam-Epoch | Advantage |
|-----------|-----------|-----------------|-----------|
| Load | 1.63 ns | 306.63-412.44 ns | **188-253x faster** |

**Significant Improvement**:
- Atomic load performance improved by 73-133%, from 108x to 188-253x faster
- Demonstrates SWMR-Epoch's absolute advantage in read performance

#### 5. Concurrent Reads (Throughput) ⭐ **SWMR-Epoch Leads**

| Thread Count | SWMR-Epoch | Crossbeam-Epoch | Speedup |
|-------------|-----------|-----------------|---------|
| 2 threads | 80.84 µs | 633.65 µs | **7.84x faster** |
| 4 threads | 134.46 µs | 1.26 ms | **9.37x faster** |
| 8 threads | 238.35 µs | 1.29 ms | **5.41x faster** |

**Key Findings**:
- 3-7% performance improvement in 2-4 thread scenarios
- Performance decrease in 8-thread scenario (from 13.28x to 5.41x)
- Possible cause: Increased resource contention under high concurrency

**Optimization Directions**:
- Investigate 8-thread performance regression
- Optimize resource contention in high-concurrency scenarios

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
