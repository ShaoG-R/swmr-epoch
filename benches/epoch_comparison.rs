use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

// Import our epoch-based GC implementation
use swmr_epoch::{EpochGcDomain, EpochPtr};

// Benchmark 1: Single-threaded pin/unpin overhead
fn bench_single_thread_pin_unpin(c: &mut Criterion) {
    c.bench_function("swmr_epoch_single_thread_pin_unpin", |b| {
        let domain = EpochGcDomain::new();
        let local_epoch = domain.register_reader();
        
        b.iter(|| {
            let _guard = local_epoch.pin();
            black_box(());
        });
    });

    c.bench_function("crossbeam_epoch_single_thread_pin_unpin", |b| {
        b.iter(|| {
            let _guard = crossbeam_epoch::pin();
            black_box(());
        });
    });
}

// Benchmark 2: Multi-threaded reader registration
fn bench_reader_registration(c: &mut Criterion) {
    let mut group = c.benchmark_group("reader_registration");
    
    for num_readers in [2, 4, 8, 16].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_readers),
            num_readers,
            |b, &num_readers| {
                b.iter(|| {
                    let domain = EpochGcDomain::new();
                    let domain = Arc::new(domain);
                    
                    let handles: Vec<_> = (0..num_readers)
                        .map(|_| {
                            let d = domain.clone();
                            thread::spawn(move || {
                                let local_epoch = d.register_reader();
                                let _guard = local_epoch.pin();
                            })
                        })
                        .collect();
                    
                    for handle in handles {
                        let _ = handle.join();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch", num_readers),
            num_readers,
            |b, &num_readers| {
                b.iter(|| {
                    let handles: Vec<_> = (0..num_readers)
                        .map(|_| {
                            thread::spawn(|| {
                                let _guard = crossbeam_epoch::pin();
                            })
                        })
                        .collect();
                    
                    for handle in handles {
                        let _ = handle.join();
                    }
                });
            },
        );
    }
    
    group.finish();
}

// Benchmark 3: Garbage collection overhead
fn bench_garbage_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("garbage_collection");
    
    for num_items in [100, 1000, 10000].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch_retire", num_items),
            num_items,
            |b, &num_items| {
                b.iter_custom(|iters| {
                    let mut total_duration = std::time::Duration::ZERO;
                    
                    for _ in 0..iters {
                        let domain = EpochGcDomain::new();
                        let mut gc = domain.gc_handle();
                        let local_epoch = domain.register_reader();
                        
                        let start = std::time::Instant::now();
                        
                        for i in 0..num_items {
                            let _guard = local_epoch.pin();
                            gc.retire(Box::new(i as u64));
                        }
                        gc.collect();
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
    }
    
    // NOTE: crossbeam_epoch_defer benchmark cannot be implemented fairly because:
    // 1. crossbeam_epoch defers closures but doesn't provide explicit garbage collection API
    // 2. Deferred closures only execute when all active pins are dropped
    // 3. This creates a fundamental difference in GC semantics:
    //    - swmr_epoch: explicit collect() with deterministic cleanup
    //    - crossbeam_epoch: implicit GC tied to pin lifecycle, causing memory accumulation
    //      during benchmark iterations if not properly managed
    // 4. Any attempt to measure crossbeam_epoch_defer fairly would require either:
    //    - Including GC overhead in measurements (unfair comparison)
    //    - Forcing GC outside measurements (doesn't reflect real usage)
    
    group.finish();
}

// Benchmark 4: Epoch pointer operations
fn bench_atomic_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("atomic_operations");
    
    group.bench_function("swmr_epoch_load", |b| {
        let domain = EpochGcDomain::new();
        let local_epoch = domain.register_reader();
        let epoch_ptr = EpochPtr::new(42u64);
        
        b.iter(|| {
            let guard = local_epoch.pin();
            let val = epoch_ptr.load(&guard);
            black_box(val);
        });
    });

    group.bench_function("crossbeam_epoch_load", |b| {
        let atomic = crossbeam_epoch::Atomic::new(42u64);
        
        b.iter(|| {
            let guard = crossbeam_epoch::pin();
            let val = atomic.load(Ordering::Acquire, &guard);
            black_box(val);
        });
    });
    
    group.finish();
}

// Benchmark 5: Concurrent read-heavy workload
fn bench_concurrent_reads(c: &mut Criterion) {
    let mut group = c.benchmark_group("concurrent_reads");
    group.sample_size(10);
    
    for num_threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let domain = EpochGcDomain::new();
                    let epoch_ptr = Arc::new(EpochPtr::new(0u64));
                    let counter = Arc::new(AtomicUsize::new(0));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let d = domain.clone();
                            let ep = epoch_ptr.clone();
                            let c = counter.clone();
                            
                            thread::spawn(move || {
                                let local_epoch = d.register_reader();
                                for _ in 0..1000 {
                                    let guard = local_epoch.pin();
                                    let _val = ep.load(&guard);
                                    c.fetch_add(1, Ordering::Relaxed);
                                }
                            })
                        })
                        .collect();
                    
                    for handle in handles {
                        let _ = handle.join();
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let atomic = Arc::new(crossbeam_epoch::Atomic::new(0u64));
                    let counter = Arc::new(AtomicUsize::new(0));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let a = atomic.clone();
                            let c = counter.clone();
                            
                            thread::spawn(move || {
                                for _ in 0..1000 {
                                    let guard = crossbeam_epoch::pin();
                                    let _val = a.load(Ordering::Acquire, &guard);
                                    c.fetch_add(1, Ordering::Relaxed);
                                }
                            })
                        })
                        .collect();
                    
                    for handle in handles {
                        let _ = handle.join();
                    }
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_single_thread_pin_unpin,
    bench_reader_registration,
    bench_garbage_collection,
    bench_atomic_operations,
    bench_concurrent_reads
);
criterion_main!(benches);
