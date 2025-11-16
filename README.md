# SWMR-Epoch: Single-Writer Multi-Reader Epoch-Based GC with Minimal Locking

[![Crates.io](https://img.shields.io/crates/v/swmr-epoch.svg)](https://crates.io/crates/swmr-epoch)
[![License](https://img.shields.io/crates/l/swmr-epoch.svg)](https://github.com/ShaoG-R/swmr-epoch#license)
[![Docs.rs](https://docs.rs/swmr-epoch/badge.svg)](https://docs.rs/swmr-epoch)
[![GitHub](https://img.shields.io/badge/github-ShaoG--R/swmr--epoch-blue.svg)](https://github.com/ShaoG-R/swmr-epoch)

[中文文档](./README_CN.md)

A high-performance garbage collection system for Rust implementing Single-Writer Multi-Reader (SWMR) epoch-based memory reclamation. Designed for concurrent data structures requiring safe, efficient memory management. Uses minimal locking (a single Mutex for reader tracking) combined with atomic operations for the core epoch mechanism.

## Features

- **Minimal Locking**: Uses a single Mutex only for reader registration tracking; atomic operations for core epoch mechanism
- **Single-Writer Multi-Reader (SWMR)**: One writer thread, unlimited reader threads
- **Epoch-Based Garbage Collection**: Deferred deletion with automatic reclamation
- **Type-Safe**: Full Rust type safety with compile-time guarantees
- **Epoch-Protected Pointers**: `EpochPtr<T>` wrapper for safe concurrent access
- **Zero-Copy Reads**: Readers obtain direct references without allocation
- **Automatic Participant Cleanup**: Weak pointers automatically remove inactive readers
- **Reentrant Pinning**: Nested pin guards supported via reference counting

## Architecture

### Core Components

**EpochGcDomain**
- Entry point for creating an epoch-based GC system
- Clone-safe and can be shared across threads
- Manages the global epoch counter and reader registration

**GcHandle**
- The unique garbage collector for a domain, owned by the writer thread
- Advances the global epoch during collection cycles
- Receives retired objects and scans active readers for reclamation
- Not thread-safe; must be owned by a single thread

**LocalEpoch**
- Reader thread's local epoch state
- Not `Sync` (due to `Cell`) and must be stored per-thread
- Used to pin threads and obtain `PinGuard` for safe access

**PinGuard**
- RAII guard that keeps the current thread pinned to an epoch
- Prevents the writer from reclaiming data during the read
- Supports cloning for nested pinning via reference counting
- Lifetime bound to the `LocalEpoch` it came from

**EpochPtr<T>**
- Type-safe atomic pointer wrapper
- Load operations require active `PinGuard`
- Store operations may trigger automatic garbage collection if the garbage count exceeds the configured threshold
- Safely manages memory across writer and reader threads

### Memory Ordering

- **Acquire/Release semantics**: Ensures proper synchronization between readers and writer
- **Acquire for epoch loads**: Readers synchronize with writer's epoch advances
- **Release for epoch stores**: Writer ensures visibility to all readers
- **Relaxed operations**: Used where ordering is not required for performance

## Usage Example

```rust
use swmr_epoch::{EpochGcDomain, EpochPtr};
use std::sync::Arc;

fn main() {
    // 1. Create a shared GC domain and get the garbage collector
    let (mut gc, domain) = EpochGcDomain::new();
    
    // 2. Create an epoch-protected pointer wrapped in Arc for thread-safe sharing
    let data = Arc::new(EpochPtr::new(42i32));
    
    // 3. Reader thread
    let domain_clone = domain.clone();
    let data_clone = data.clone();
    let reader_thread = std::thread::spawn(move || {
        let local_epoch = domain_clone.register_reader();
        let guard = local_epoch.pin();
        let value = data_clone.load(&guard);
        println!("Read value: {}", value);
    });
    
    // 4. Writer thread: update and collect garbage
    data.store(100, &mut gc);
    gc.collect();
    
    reader_thread.join().unwrap();
}
```

## Advanced Usage

### Custom Garbage Collection Threshold

By default, automatic garbage collection is triggered when garbage count exceeds 64 items. You can customize this:

```rust
use swmr_epoch::EpochGcDomain;

// Create with custom threshold (e.g., 128 items)
let (mut gc, domain) = EpochGcDomain::new_with_threshold(Some(128));

// Or disable automatic collection entirely
let (mut gc, domain) = EpochGcDomain::new_with_threshold(None);
gc.collect();  // Manually trigger collection when needed
```

### Nested Pinning

`PinGuard` supports cloning for nested pinning scenarios:

```rust
let guard1 = local_epoch.pin();
let guard2 = guard1.clone();  // Nested pin - thread remains pinned
let guard3 = guard1.clone();  // Multiple nested pins are supported

// Thread remains pinned until all guards are dropped
drop(guard3);
drop(guard2);
drop(guard1);
```

## Core Concepts

### Epoch

A logical timestamp that advances monotonically. The writer increments the epoch during garbage collection cycles. Readers "pin" themselves to an epoch, declaring that they are actively reading data from that epoch.

### Pin

When a reader calls `pin()`, it records the current epoch in its slot. This tells the writer: "I am reading data from this epoch; do not reclaim it yet."

### Reclamation

The writer collects retired objects and reclaims those from epochs that are older than the minimum epoch of all active readers.

## Design Decisions

### Why SWMR?

- **Simplicity**: Single writer eliminates write-write conflicts and complex synchronization
- **Performance**: Readers don't block each other during normal reads; writer operations are predictable
- **Safety**: Easier to reason about correctness with one writer

### Why Epoch-Based GC?

- **Minimal Synchronization**: Epoch mechanism uses atomic operations; only reader tracking uses a Mutex during collection
- **Predictable**: Deferred deletion provides bounded latency
- **Scalable**: Reader operations are O(1) in the common case (no CAS loops or reference counting overhead)

### Why Weak Pointers for Readers?

- **Automatic Cleanup**: Dropped readers are automatically removed from tracking
- **No Explicit Unregistration**: Readers don't need to notify the writer on exit
- **Memory Efficient**: Avoids maintaining stale reader entries

### Why Reentrant Pinning?

- **Flexibility**: Allows nested critical sections without explicit guard management
- **Safety**: Pin count ensures correct unpinning order
- **Simplicity**: Developers don't need to manually track pin depth

## Limitations

1. **Single Writer**: Only one thread can write at a time
2. **GC Throughput**: Full reader scans make garbage collection slower than specialized systems
3. **Epoch Overflow**: Uses `usize` for epochs; overflow is theoretically possible but impractical
4. **Automatic Reclamation**: Garbage collection is triggered automatically when threshold is exceeded, which may cause latency spikes. This can be disabled by passing `None` to `new_with_threshold()`, or customized by passing a different threshold value
5. **Reader Tracking Mutex**: A single Mutex is used to track active readers during garbage collection. While this is a minimal synchronization point, it is not fully lock-free. Performance testing showed that lock-free alternatives (e.g., SegQueue) resulted in worse performance due to contention and memory ordering overhead

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

- `criterion`: Benchmarking framework (dev-dependency)

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.

## References

- Keir Fraser. "Practical Lock-Freedom" (PhD thesis, 2004)
- Hart, McKenney, Brown. "Performance of Memory Reclamation for Lock-Free Synchronization" (2007)
- Crossbeam Epoch documentation: https://docs.rs/crossbeam-epoch/
