use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

// Import our SMR epoch implementation
use swmr_epoch::{new, Atomic};

// Benchmark 1: Single-threaded pin/unpin overhead
fn bench_single_thread_pin_unpin(c: &mut Criterion) {
    c.bench_function("swmr_epoch_single_thread_pin_unpin", |b| {
        let (_, registry) = new();
        
        b.iter(|| {
            let _guard = registry.pin();
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
                    let (_, registry) = new();
                    let registry = Arc::new(registry);
                    
                    let handles: Vec<_> = (0..num_readers)
                        .map(|_| {
                            let r = registry.clone();
                            thread::spawn(move || {
                                let _guard = r.pin();
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
                        let (mut writer, registry) = new();
                        
                        let start = std::time::Instant::now();
                        
                        for i in 0..num_items {
                            let _guard = registry.pin();
                            writer.retire(Box::new(i as u64));
                        }
                        writer.try_reclaim();
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch_defer", num_items),
            num_items,
            |b, &num_items| {
                b.iter_custom(|iters| {
                    let mut total_duration = std::time::Duration::ZERO;
                    
                    for _ in 0..iters {
                        let start = std::time::Instant::now();
                        let guard = crossbeam_epoch::pin();
                        
                        for i in 0..num_items {
                            guard.defer(move || {
                                let _ = i;
                            });
                        }
                        
                        total_duration += start.elapsed();
                    }
                    
                    total_duration
                });
            },
        );
    }
    
    group.finish();
}

// Benchmark 4: Atomic pointer operations
fn bench_atomic_operations(c: &mut Criterion) {
    let mut group = c.benchmark_group("atomic_operations");
    
    group.bench_function("swmr_epoch_load", |b| {
        let (_, registry) = new();
        let atomic = Atomic::new(42u64);
        
        b.iter(|| {
            let guard = registry.pin();
            let val = atomic.load(&guard);
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
                    let (_, registry) = new();
                    let registry = Arc::new(registry);
                    let atomic = Arc::new(Atomic::new(0u64));
                    let counter = Arc::new(AtomicUsize::new(0));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let r = registry.clone();
                            let a = atomic.clone();
                            let c = counter.clone();
                            
                            thread::spawn(move || {
                                for _ in 0..1000 {
                                    let guard = r.pin();
                                    let _val = a.load(&guard);
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
