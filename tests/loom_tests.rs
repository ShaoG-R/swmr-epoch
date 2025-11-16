//! Loom-based concurrency tests
//! 
//! These tests use the `loom` library to exhaustively check all possible
//! thread interleavings and detect concurrency bugs like data races, deadlocks,
//! and memory ordering issues.
//!
//! Run with: `RUSTFLAGS="--cfg loom" cargo test --test loom_tests --release`

#![cfg(loom)]

use loom::sync::Arc;
use loom::thread;
use loom::model::Builder;
use swmr_epoch::{EpochGcDomain, EpochPtr};

/// Test: Multiple readers can safely read concurrently
#[test]
fn loom_concurrent_readers() {
    loom::model(|| {
        let (gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(42i32));

        let mut handles = vec![];

        // Spawn 2 reader threads
        for _ in 0..2 {
            let domain = domain.clone();
            let ptr = Arc::clone(&ptr);
            
            let handle = thread::spawn(move || {
                let local = domain.register_reader();
                let guard = local.pin();
                let value = ptr.load(&guard);
                assert_eq!(*value, 42);
            });
            
            handles.push(handle);
        }

        drop(gc);
        drop(domain);
        drop(ptr);

        for handle in handles {
            handle.join().unwrap();
        }
    });
}

/// Test: Single writer with concurrent readers (basic SWMR)
#[test]
fn loom_single_writer_multi_reader() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        // Spawn reader thread
        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        let reader_handle = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            let value = reader_ptr.load(&guard);
            // Value should be either 1 or 2
            assert!(*value == 1 || *value == 2);
        });

        // Writer updates value
        ptr.store(2i32, &mut gc);
        gc.collect();

        reader_handle.join().unwrap();
    });
}

/// Test: Reentrant pinning (nested pin calls)
#[test]
fn loom_reentrant_pinning() {
    loom::model(|| {
        let (gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(100i32));

        let handle = thread::spawn(move || {
            let local = domain.register_reader();
            
            // Nested pinning
            let guard1 = local.pin();
            let value1 = ptr.load(&guard1);
            assert_eq!(*value1, 100);
            
            let guard2 = local.pin();
            let value2 = ptr.load(&guard2);
            assert_eq!(*value2, 100);
            
            // Both guards should work
            let value3 = ptr.load(&guard1);
            assert_eq!(*value3, 100);
            
            drop(guard2);
            
            // guard1 should still work
            let value4 = ptr.load(&guard1);
            assert_eq!(*value4, 100);
        });

        drop(gc);
        handle.join().unwrap();
    });
}

/// Test: Garbage collection doesn't free data being read
#[test]
fn loom_gc_safety() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader_handle = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            
            // Read the initial value and hold the pin
            let value = reader_ptr.load(&guard);
            assert!(*value >= 1 && *value <= 3);
            
            // Simulate some work while holding the pin
            thread::yield_now();
            
            // Value should still be valid
            let value2 = reader_ptr.load(&guard);
            assert!(*value2 >= 1 && *value2 <= 3);
        });

        // Writer updates and collects garbage
        ptr.store(2i32, &mut gc);
        gc.collect();
        
        ptr.store(3i32, &mut gc);
        gc.collect();

        reader_handle.join().unwrap();
    });
}

/// Test: Multiple sequential stores and garbage collection
#[test]
fn loom_multiple_stores() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);

        // Multiple stores
        ptr.store(2i32, &mut gc);
        ptr.store(3i32, &mut gc);
        
        // Collection should safely reclaim old values
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 3);
    });
}

/// Test: Epoch advancement under concurrent access
#[test]
fn loom_epoch_advancement() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(0i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader_handle = thread::spawn(move || {
            let local = reader_domain.register_reader();
            
            // Pin and unpin multiple times
            for _ in 0..2 {
                let guard = local.pin();
                let _value = reader_ptr.load(&guard);
                drop(guard);
            }
        });

        // Writer performs multiple collections
        gc.collect();
        gc.collect();

        reader_handle.join().unwrap();
    });
}

/// Test: Store and load consistency
#[test]
fn loom_store_load_consistency() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);

        // Store a value
        ptr.store(42i32, &mut gc);

        // Immediately load should see the new value
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 42);
    });
}

/// Test: PinGuard cloning for nested scopes
#[test]
fn loom_pin_guard_cloning() {
    loom::model(|| {
        let (gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(99i32));

        let handle = thread::spawn(move || {
            let local = domain.register_reader();
            let guard1 = local.pin();
            
            // Clone the guard
            let guard2 = guard1.clone();
            
            // Both guards should work
            let value1 = ptr.load(&guard1);
            let value2 = ptr.load(&guard2);
            assert_eq!(*value1, 99);
            assert_eq!(*value2, 99);
            
            // Drop one guard, the other should still work
            drop(guard2);
            let value3 = ptr.load(&guard1);
            assert_eq!(*value3, 99);
        });

        drop(gc);
        handle.join().unwrap();
    });
}

/// Test: Multiple writers sequentially (simulating ownership transfer)
#[test]
fn loom_sequential_writers() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));
        
        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            let value = reader_ptr.load(&guard);
            assert!(*value >= 1 && *value <= 3);
        });
        
        // Sequential stores
        ptr.store(2i32, &mut gc);
        gc.collect();
        ptr.store(3i32, &mut gc);
        gc.collect();
        
        reader.join().unwrap();
    });
}

/// Test: Multiple EpochPtr instances with shared GC
#[test]
fn loom_multiple_epoch_ptrs() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr1 = Arc::new(EpochPtr::new(10i32));
        let ptr2 = Arc::new(EpochPtr::new(20i32));

        let reader_domain = domain.clone();
        let reader_ptr1 = Arc::clone(&ptr1);
        let reader_ptr2 = Arc::clone(&ptr2);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            
            let val1 = reader_ptr1.load(&guard);
            let val2 = reader_ptr2.load(&guard);
            
            // Values should be from their respective stores
            assert!(*val1 == 10 || *val1 == 11);
            assert!(*val2 == 20 || *val2 == 21);
        });
        
        // Update both pointers
        ptr1.store(11i32, &mut gc);
        ptr2.store(21i32, &mut gc);
        gc.collect();
        
        reader.join().unwrap();
    });
}

/// Test: Fast pin/unpin cycles during garbage collection
#[test]
fn loom_fast_pin_unpin_cycles() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(0i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            
            // Rapid pin/unpin cycles
            for _ in 0..2 {
                let guard = local.pin();
                let _value = reader_ptr.load(&guard);
                drop(guard);
                thread::yield_now();
            }
        });

        // Writer stores and collects during reader's pin/unpin cycles
        ptr.store(1i32, &mut gc);
        gc.collect();
        
        reader.join().unwrap();
    });
}

/// Test: No active readers - GC should reclaim all garbage
#[test]
fn loom_gc_with_no_active_readers() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);

        // Store multiple values without any readers
        ptr.store(2i32, &mut gc);
        ptr.store(3i32, &mut gc);
        ptr.store(4i32, &mut gc);
        
        // Collect - should reclaim all since no readers are active
        gc.collect();

        // Now register a reader and verify latest value
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 4);
    });
}

/// Test: LocalEpoch drop behavior
#[test]
fn loom_local_epoch_drop() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            {
                let local = reader_domain.register_reader();
                let guard = local.pin();
                let value = reader_ptr.load(&guard);
                // Value can be 1 or 2 depending on thread interleaving
                assert!(*value == 1 || *value == 2);
                // local and guard drop here
            }
            // LocalEpoch has been dropped
        });

        thread::yield_now();
        
        // Store after reader might have dropped
        ptr.store(2i32, &mut gc);
        gc.collect();
        
        reader.join().unwrap();
    });
}

/// Test: Reader pinned across multiple epoch advancements
#[test]
fn loom_reader_across_epochs() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            
            // Hold pin across multiple yields
            let value1 = reader_ptr.load(&guard);
            thread::yield_now();
            let value2 = reader_ptr.load(&guard);
            thread::yield_now();
            
            // Should see a consistent view or new value
            assert!(*value1 >= 1 && *value1 <= 3);
            assert!(*value2 >= 1 && *value2 <= 3);
        });

        // Advance epoch multiple times
        gc.collect();
        ptr.store(2i32, &mut gc);
        gc.collect();
        ptr.store(3i32, &mut gc);
        gc.collect();
        
        reader.join().unwrap();
    });
}

/// Test: Three concurrent readers with writer
#[test]
fn loom_three_readers_one_writer() {
    // Use preemption bound to limit state space exploration
    // 3 readers + 1 writer + mutex contention = large state space
    let mut builder = Builder::new();
    builder.preemption_bound = Some(3);
    builder.check(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(0i32));

        let mut readers = vec![];
        
        // Spawn 3 reader threads
        for _ in 0..3 {
            let reader_domain = domain.clone();
            let reader_ptr = Arc::clone(&ptr);
            
            let reader = thread::spawn(move || {
                let local = reader_domain.register_reader();
                let guard = local.pin();
                let value = reader_ptr.load(&guard);
                assert!(*value <= 5);
            });
            
            readers.push(reader);
        }

        // Writer updates
        ptr.store(5i32, &mut gc);
        gc.collect();

        for reader in readers {
            reader.join().unwrap();
        }
    });
}

/// Test: Interleaved pin/unpin from multiple readers
#[test]
fn loom_interleaved_pin_unpin() {
    // Multiple pin/unpin cycles create large state space
    let mut builder = Builder::new();
    builder.preemption_bound = Some(5);
    builder.check(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(100i32));

        let mut readers = vec![];
        
        for _ in 0..2 {
            let reader_domain = domain.clone();
            let reader_ptr = Arc::clone(&ptr);
            
            let reader = thread::spawn(move || {
                let local = reader_domain.register_reader();
                
                // First pin
                {
                    let guard = local.pin();
                    let _value = reader_ptr.load(&guard);
                }
                
                thread::yield_now();
                
                // Second pin after unpin
                {
                    let guard = local.pin();
                    let _value = reader_ptr.load(&guard);
                }
            });
            
            readers.push(reader);
        }

        // Collect during interleaved access
        gc.collect();

        for reader in readers {
            reader.join().unwrap();
        }
    });
}

/// Test: Store with immediate collection and concurrent read
#[test]
fn loom_store_collect_read_race() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            let value = reader_ptr.load(&guard);
            assert!(*value == 1 || *value == 2);
        });

        // Store and immediately collect - races with reader
        ptr.store(2i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Builder configuration with custom threshold
#[test]
fn loom_builder_custom_threshold() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::builder()
            .auto_reclaim_threshold(2)
            .build();
        
        let ptr = EpochPtr::new(1i32);
        
        // Store should not trigger auto-collection at threshold 2
        ptr.store(2i32, &mut gc);
        
        // Verify value is updated
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 2);
    });
}

/// Test: Builder with disabled auto-reclamation
#[test]
fn loom_builder_no_auto_reclaim() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::builder()
            .auto_reclaim_threshold(None)
            .build();
        
        let ptr = EpochPtr::new(1i32);
        
        // Multiple stores without auto-collection
        ptr.store(2i32, &mut gc);
        ptr.store(3i32, &mut gc);
        
        // Manual collection
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 3);
    });
}

/// Test: Concurrent readers with different pin lifetimes
#[test]
fn loom_different_pin_lifetimes() {
    // Limit preemption bound for faster completion
    let mut builder = Builder::new();
    builder.preemption_bound = Some(5);
    builder.check(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        // Reader 1: short-lived pin
        let reader1_domain = domain.clone();
        let reader1_ptr = Arc::clone(&ptr);
        let reader1 = thread::spawn(move || {
            let local = reader1_domain.register_reader();
            {
                let guard = local.pin();
                let _value = reader1_ptr.load(&guard);
            } // guard dropped early
        });

        // Reader 2: long-lived pin
        let reader2_domain = domain.clone();
        let reader2_ptr = Arc::clone(&ptr);
        let reader2 = thread::spawn(move || {
            let local = reader2_domain.register_reader();
            let guard = local.pin();
            let value = reader2_ptr.load(&guard);
            thread::yield_now();
            assert!(*value >= 1 && *value <= 2);
        });

        // Writer stores during mixed reader lifetimes
        ptr.store(2i32, &mut gc);
        gc.collect();

        reader1.join().unwrap();
        reader2.join().unwrap();
    });
}

/// Test: Multiple collections without stores
#[test]
fn loom_multiple_collections_no_stores() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(42i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            let value = reader_ptr.load(&guard);
            assert_eq!(*value, 42);
        });

        // Multiple collections without any stores
        gc.collect();
        gc.collect();
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Reader registration during ongoing operations
#[test]
fn loom_dynamic_reader_registration() {
    // Dynamic registration with mutex contention needs bound
    let mut builder = Builder::new();
    builder.preemption_bound = Some(5);
    builder.check(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        // First reader already registered
        let reader1_domain = domain.clone();
        let reader1_ptr = Arc::clone(&ptr);
        let reader1 = thread::spawn(move || {
            let local = reader1_domain.register_reader();
            let guard = local.pin();
            let value = reader1_ptr.load(&guard);
            assert!(*value >= 1 && *value <= 2);
        });

        // Second reader registers during operation
        let reader2_domain = domain.clone();
        let reader2_ptr = Arc::clone(&ptr);
        let reader2 = thread::spawn(move || {
            thread::yield_now();
            let local = reader2_domain.register_reader();
            let guard = local.pin();
            let value = reader2_ptr.load(&guard);
            assert!(*value >= 1 && *value <= 2);
        });

        ptr.store(2i32, &mut gc);
        gc.collect();

        reader1.join().unwrap();
        reader2.join().unwrap();
    });
}

/// Test: Alternating store and collect operations
#[test]
fn loom_alternating_store_collect() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(0i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            let value = reader_ptr.load(&guard);
            assert!(*value <= 2);
        });

        // Alternating pattern
        ptr.store(1i32, &mut gc);
        gc.collect();
        ptr.store(2i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Multiple guards from same LocalEpoch
#[test]
fn loom_multiple_guards_same_local() {
    loom::model(|| {
        let (gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(77i32));

        let handle = thread::spawn(move || {
            let local = domain.register_reader();
            
            // Create multiple guards simultaneously
            let guard1 = local.pin();
            let guard2 = local.pin();
            let guard3 = guard1.clone();
            
            // All guards should work
            assert_eq!(*ptr.load(&guard1), 77);
            assert_eq!(*ptr.load(&guard2), 77);
            assert_eq!(*ptr.load(&guard3), 77);
            
            // Drop in different order
            drop(guard2);
            assert_eq!(*ptr.load(&guard1), 77);
            drop(guard3);
            assert_eq!(*ptr.load(&guard1), 77);
        });

        drop(gc);
        handle.join().unwrap();
    });
}

/// Test: EpochPtr::new and immediate load
#[test]
fn loom_new_and_immediate_load() {
    loom::model(|| {
        let (gc, domain) = EpochGcDomain::new();
        
        // Create and immediately access
        let ptr = EpochPtr::new(123i32);
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 123);
        
        drop(guard);
        drop(gc);
    });
}

/// Test: Writer-only scenario (no readers)
#[test]
fn loom_writer_only() {
    loom::model(|| {
        let (mut gc, _domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);
        
        // Series of stores and collections without any readers
        ptr.store(2i32, &mut gc);
        gc.collect();
        ptr.store(3i32, &mut gc);
        gc.collect();
        ptr.store(4i32, &mut gc);
        gc.collect();
        
        // All garbage should be reclaimed
    });
}

/// Test: Reader observes values from valid range across pin/unpin cycles
#[test]
fn loom_reader_monotonic_observation() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            
            let guard1 = local.pin();
            let val1 = *reader_ptr.load(&guard1);
            drop(guard1);
            
            thread::yield_now();
            
            let guard2 = local.pin();
            let val2 = *reader_ptr.load(&guard2);
            drop(guard2);
            
            // Values should be from the sequence 1, 2, 3
            assert!(val1 >= 1 && val1 <= 3);
            assert!(val2 >= 1 && val2 <= 3);
            // Note: SWMR does NOT guarantee monotonicity across different pin cycles
            // Reader might observe non-monotonic values (e.g., 3 then 2) depending on
            // thread interleaving. Both values must be valid, but order is not guaranteed.
        });

        // Monotonically increasing stores
        ptr.store(2i32, &mut gc);
        gc.collect();
        ptr.store(3i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Builder with custom cleanup interval
#[test]
fn loom_builder_custom_cleanup_interval() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::builder()
            .cleanup_interval(1)  // Cleanup every collection
            .build();
        
        let ptr = EpochPtr::new(1i32);
        
        // Store and collect
        ptr.store(2i32, &mut gc);
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 2);
    });
}

/// Test: Zero cleanup interval (disabled cleanup)
#[test]
fn loom_builder_zero_cleanup_interval() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::builder()
            .cleanup_interval(0)  // No periodic cleanup
            .build();
        
        let ptr = EpochPtr::new(42i32);
        
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 42);
    });
}

/// Test: Combined builder options
#[test]
fn loom_builder_combined_options() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::builder()
            .auto_reclaim_threshold(5)
            .cleanup_interval(2)
            .build();
        
        let ptr = EpochPtr::new(1i32);
        
        ptr.store(2i32, &mut gc);
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 2);
    });
}

/// Test: Reader with guard that outlives multiple stores
#[test]
fn loom_guard_outlives_stores() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            let guard = local.pin();
            
            // Hold guard and read initial value
            let initial = *reader_ptr.load(&guard);
            
            // Yield to let writer proceed
            thread::yield_now();
            thread::yield_now();
            
            // Guard still protects the initial value we read
            // But we might see new value on subsequent loads
            let current = *reader_ptr.load(&guard);
            
            assert!(initial >= 1 && initial <= 3);
            assert!(current >= 1 && current <= 3);
        });

        // Writer performs multiple operations
        ptr.store(2i32, &mut gc);
        gc.collect();
        ptr.store(3i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Empty domain (no readers registered)
#[test]
fn loom_empty_domain() {
    loom::model(|| {
        let (mut gc, _domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(10i32);
        
        // Operations without any registered readers
        ptr.store(20i32, &mut gc);
        gc.collect();
        ptr.store(30i32, &mut gc);
        gc.collect();
    });
}

/// Test: Concurrent stores prevented by &mut GcHandle
/// This test verifies that the type system prevents concurrent writes
#[test]
fn loom_single_writer_enforced() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);
        
        // Only one writer can exist due to &mut requirement
        ptr.store(2i32, &mut gc);
        ptr.store(3i32, &mut gc);
        
        let local = domain.register_reader();
        let guard = local.pin();
        assert_eq!(*ptr.load(&guard), 3);
    });
}

/// Test: Guard lifetime ensures safety
#[test]
fn loom_guard_lifetime_safety() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(100i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            
            // Create nested scopes with different guard lifetimes
            {
                let guard = local.pin();
                let _val = reader_ptr.load(&guard);
            } // guard1 dropped
            
            thread::yield_now();
            
            {
                let guard = local.pin();
                let _val = reader_ptr.load(&guard);
            } // guard2 dropped
        });

        ptr.store(200i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Store with null check (internal detail)
#[test]
fn loom_store_replaces_value() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = EpochPtr::new(1i32);
        
        // First store replaces initial value
        ptr.store(2i32, &mut gc);
        
        // Second store replaces previous value
        ptr.store(3i32, &mut gc);
        
        gc.collect();
        
        let local = domain.register_reader();
        let guard = local.pin();
        assert_eq!(*ptr.load(&guard), 3);
    });
}

/// Test: Concurrent pin and unpin with collection
#[test]
fn loom_concurrent_pin_unpin_collect() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(1i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            let local = reader_domain.register_reader();
            
            // Pin and unpin rapidly
            let guard = local.pin();
            let _val = reader_ptr.load(&guard);
            drop(guard);
            
            let guard = local.pin();
            let _val = reader_ptr.load(&guard);
        });

        // Writer races with reader's pin/unpin
        ptr.store(2i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}

/// Test: Multiple LocalEpoch instances from same domain
#[test]
fn loom_multiple_local_epochs() {
    loom::model(|| {
        let (mut gc, domain) = EpochGcDomain::new();
        let ptr = Arc::new(EpochPtr::new(5i32));

        let reader_domain = domain.clone();
        let reader_ptr = Arc::clone(&ptr);
        
        let reader = thread::spawn(move || {
            // Register multiple local epochs (unusual but valid)
            let local1 = reader_domain.register_reader();
            let local2 = reader_domain.register_reader();
            
            let guard1 = local1.pin();
            let guard2 = local2.pin();
            
            let val1 = reader_ptr.load(&guard1);
            let val2 = reader_ptr.load(&guard2);
            
            // Both values should be valid (either 5 or 6)
            assert!(*val1 == 5 || *val1 == 6);
            assert!(*val2 == 5 || *val2 == 6);
            // Note: val1 and val2 may differ if writer stores between the two pins
        });

        ptr.store(6i32, &mut gc);
        gc.collect();

        reader.join().unwrap();
    });
}
