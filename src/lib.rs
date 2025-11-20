//! # Epoch-Based Garbage Collection
//!
//! This module provides a minimal-locking, single-writer, multi-reader garbage collection system
//! based on epoch-based reclamation. It is designed for scenarios where:
//!
//! - One thread (the writer) owns and updates shared data structures.
//! - Multiple reader threads concurrently access the same data.
//! - Readers need to safely access data without blocking the writer.
//!
//! ## Core Concepts
//!
//! **Epoch**: A logical timestamp that advances monotonically. The writer increments the epoch
//! during garbage collection cycles. Readers "pin" themselves to an epoch, declaring that they
//! are actively reading data from that epoch.
//!
//! **Pin**: When a reader calls `pin()`, it records the current epoch in its slot. This tells
//! the writer: "I am reading data from this epoch; do not reclaim it yet."
//!
//! **Reclamation**: The writer collects retired objects and reclaims those from epochs that
//! are older than the minimum epoch of all active readers.
//!
//! ## Typical Usage
//!
//! ```
//! use swmr_epoch::{EpochGcDomain, EpochPtr};
//!
//! // 1. Create a shared GC domain and get the garbage collector
//! let (mut gc, domain) = EpochGcDomain::new();
//!
//! // 2. Create an epoch-protected pointer
//! let shared_ptr = EpochPtr::new(42i32);
//!
//! // 3. In each reader thread, register a local epoch
//! let local_epoch = domain.register_reader();
//!
//! // 4. Readers pin themselves before accessing shared data
//! let guard = local_epoch.pin();
//! let value = shared_ptr.load(&guard);
//! // ... use value ...
//! // guard is automatically dropped, unpinning the thread
//!
//! // 5. Writer updates shared data and drives garbage collection
//! shared_ptr.store(100i32, &mut gc);
//! gc.collect();  // Reclaim garbage from old epochs
//! ```

mod sync;
pub(crate) mod state;
pub(crate) mod garbage;
pub(crate) mod reader;
pub(crate) mod domain;
pub(crate) mod ptr;

#[cfg(test)]
mod tests;

pub use domain::{EpochGcDomain, EpochGcDomainBuilder};
pub use garbage::GcHandle;
pub use ptr::EpochPtr;
pub use reader::{LocalEpoch, PinGuard};
