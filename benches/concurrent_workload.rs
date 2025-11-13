use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::thread;

use swmr_epoch::{new, Atomic};

// Benchmark 1: Mixed read-write workload (80% reads)
fn bench_mixed_workload_80(c: &mut Criterion) {
    let mut group = c.benchmark_group("mixed_workload_80");
    group.sample_size(10);
    
    for num_threads in [2, 4, 8].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let (mut writer, factory) = new();
                    let factory = Arc::new(factory);
                    let atomic = Arc::new(Atomic::new(0u64));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let f = factory.clone();
                            let a = atomic.clone();
                            
                            thread::spawn(move || {
                                let handle = f.create_handle();
                                
                                for _ in 0..500 {
                                    let guard = handle.pin();
                                    let _val = a.load(&guard);
                                }
                            })
                        })
                        .collect();
                    
                    for handle in handles {
                        let _ = handle.join();
                    }
                    
                    writer.try_reclaim();
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let atomic = Arc::new(crossbeam_epoch::Atomic::new(0u64));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let a = atomic.clone();
                            
                            thread::spawn(move || {
                                for _ in 0..500 {
                                    let guard = crossbeam_epoch::pin();
                                    let _val = a.load(Ordering::Acquire, &guard);
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

// Benchmark 2: Scalability test - varying thread count
fn bench_scalability(c: &mut Criterion) {
    let mut group = c.benchmark_group("scalability");
    group.sample_size(10);
    
    for num_threads in [1, 2, 4, 8, 16].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_threads),
            num_threads,
            |b, &num_threads| {
                b.iter(|| {
                    let (_, factory) = new();
                    let factory = Arc::new(factory);
                    let atomic = Arc::new(Atomic::new(0u64));
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let f = factory.clone();
                            let a = atomic.clone();
                            
                            thread::spawn(move || {
                                let handle = f.create_handle();
                                for _ in 0..100 {
                                    let guard = handle.pin();
                                    let _val = a.load(&guard);
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
                    
                    let handles: Vec<_> = (0..num_threads)
                        .map(|_| {
                            let a = atomic.clone();
                            
                            thread::spawn(move || {
                                for _ in 0..100 {
                                    let guard = crossbeam_epoch::pin();
                                    let _val = a.load(Ordering::Acquire, &guard);
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

// Benchmark 3: Garbage collection pressure
fn bench_gc_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("gc_pressure");
    group.sample_size(10);
    
    for num_retires in [100, 500, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_retires),
            num_retires,
            |b, &num_retires| {
                b.iter(|| {
                    let (mut writer, factory) = new();
                    let handle = factory.create_handle();
                    
                    for i in 0..num_retires {
                        let _guard = handle.pin();
                        writer.retire(Box::new(i as u64));
                    }
                    writer.try_reclaim();
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch", num_retires),
            num_retires,
            |b, &num_retires| {
                b.iter(|| {
                    let guard = crossbeam_epoch::pin();
                    for i in 0..num_retires {
                        guard.defer(move || {
                            let _ = i;
                        });
                    }
                });
            },
        );
    }
    
    group.finish();
}

// Benchmark 4: Pin/Unpin latency distribution
fn bench_pin_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("pin_latency");
    group.sample_size(100);
    
    group.bench_function("swmr_epoch_pin_latency", |b| {
        let (_, factory) = new();
        let handle = factory.create_handle();
        
        b.iter(|| {
            let guard = handle.pin();
            black_box(&guard);
            drop(guard);
        });
    });

    group.bench_function("crossbeam_epoch_pin_latency", |b| {
        b.iter(|| {
            let guard = crossbeam_epoch::pin();
            black_box(&guard);
            drop(guard);
        });
    });
    
    group.finish();
}

// Benchmark 5: Contention under high load
fn bench_high_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("high_contention");
    group.sample_size(5);
    
    group.bench_function("swmr_epoch_high_contention", |b| {
        b.iter(|| {
            let (_, factory) = new();
            let factory = Arc::new(factory);
            let atomic = Arc::new(Atomic::new(0u64));
            
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let f = factory.clone();
                    let a = atomic.clone();
                    
                    thread::spawn(move || {
                        let handle = f.create_handle();
                        for _ in 0..1000 {
                            let guard = handle.pin();
                            let _val = a.load(&guard);
                        }
                    })
                })
                .collect();
            
            for handle in handles {
                let _ = handle.join();
            }
        });
    });

    group.bench_function("crossbeam_epoch_high_contention", |b| {
        b.iter(|| {
            let atomic = Arc::new(crossbeam_epoch::Atomic::new(0u64));
            
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    let a = atomic.clone();
                    
                    thread::spawn(move || {
                        for _ in 0..1000 {
                            let guard = crossbeam_epoch::pin();
                            let _val = a.load(Ordering::Acquire, &guard);
                        }
                    })
                })
                .collect();
            
            for handle in handles {
                let _ = handle.join();
            }
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    bench_mixed_workload_80,
    bench_scalability,
    bench_gc_pressure,
    bench_pin_latency,
    bench_high_contention
);
criterion_main!(benches);
