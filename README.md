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
- Store operations trigger automatic garbage collection
- Safely manages memory across writer and reader threads

### Memory Ordering

- **Acquire/Release semantics**: Ensures proper synchronization between readers and writer
- **Acquire for epoch loads**: Readers synchronize with writer's epoch advances
- **Release for epoch stores**: Writer ensures visibility to all readers
- **Relaxed operations**: Used where ordering is not required for performance

## Usage Example

```rust
use swmr_epoch::{EpochGcDomain, EpochPtr};

fn main() {
    // 1. Create a shared GC domain
    let domain = EpochGcDomain::new();
    
    // 2. Create the unique garbage collector in the writer thread
    let mut gc = domain.gc_handle();
    
    // 3. Create an epoch-protected pointer
    let data = EpochPtr::new(42i32);
    
    // 4. Reader thread
    let domain_clone = domain.clone();
    let data_clone = &data;
    let reader_thread = std::thread::spawn(move || {
        let local_epoch = domain_clone.register_reader();
        let guard = local_epoch.pin();
        let value = data_clone.load(&guard);
        println!("Read value: {}", value);
    });
    
    // 5. Writer thread: update and collect garbage
    data.store(100, &mut gc);
    gc.collect();
    
    reader_thread.join().unwrap();
}
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
- **Performance**: Readers never block readers; writer operations are predictable
- **Safety**: Easier to reason about correctness with one writer

### Why Epoch-Based GC?

- **Lock-Free**: No need for reference counting or atomic CAS loops
- **Predictable**: Deferred deletion provides bounded latency
- **Scalable**: Reader operations are O(1) in the common case

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
4. **Automatic Reclamation**: Garbage collection is triggered automatically when threshold is exceeded, which may cause latency spikes

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
