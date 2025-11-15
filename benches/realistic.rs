use criterion::{criterion_group, criterion_main, Criterion, BenchmarkId};
use std::hint::black_box;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

// Import our epoch-based GC implementation
use swmr_epoch::{EpochGcDomain, EpochPtr};

// ==================== Scenario 1: Realistic SWMR Workload ====================
// 模拟真实的单写多读场景：一个写者持续更新配置，多个读者频繁访问

fn bench_realistic_swmr_workload(c: &mut Criterion) {
    let mut group = c.benchmark_group("realistic_swmr_workload");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));

    for num_readers in [2, 4, 8, 16].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", num_readers),
            num_readers,
            |b, &num_readers| {
                b.iter(|| {
                    let domain = EpochGcDomain::new();
                    let mut gc = domain.gc_handle();
                    
                    let config = Arc::new(EpochPtr::new(ConfigData {
                        version: 0,
                        settings: vec![0; 100],
                    }));
                    
                    let running = Arc::new(AtomicBool::new(true));
                    let total_reads = Arc::new(AtomicUsize::new(0));
                    let total_writes = Arc::new(AtomicUsize::new(0));

                    // Spawn reader threads
                    let reader_handles: Vec<_> = (0..num_readers)
                        .map(|_| {
                            let d = domain.clone();
                            let cfg = config.clone();
                            let r = running.clone();
                            let reads = total_reads.clone();
                            
                            thread::spawn(move || {
                                let local_epoch = d.register_reader();
                                let mut local_reads = 0;
                                
                                while r.load(Ordering::Relaxed) {
                                    // Simulate realistic access pattern: 
                                    // frequently access config with short-lived guards
                                    for _ in 0..100 {
                                        let guard = local_epoch.pin();
                                        let data = cfg.load(&guard);
                                        black_box(data.version);
                                        black_box(&data.settings[0]);
                                        local_reads += 1;
                                    }
                                }
                                
                                reads.fetch_add(local_reads, Ordering::Relaxed);
                            })
                        })
                        .collect();

                    // Writer thread updates config periodically
                    for i in 0..100 {
                        config.store(
                            ConfigData {
                                version: i + 1,
                                settings: vec![i; 100],
                            },
                            &mut gc,
                        );
                        
                        total_writes.fetch_add(1, Ordering::Relaxed);
                    }

                    running.store(false, Ordering::Relaxed);
                    
                    for handle in reader_handles {
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
                    let config = Arc::new(crossbeam_epoch::Atomic::new(ConfigData {
                        version: 0,
                        settings: vec![0; 100],
                    }));
                    
                    let running = Arc::new(AtomicBool::new(true));
                    let total_reads = Arc::new(AtomicUsize::new(0));

                    // Spawn reader threads
                    let reader_handles: Vec<_> = (0..num_readers)
                        .map(|_| {
                            let cfg = config.clone();
                            let r = running.clone();
                            let reads = total_reads.clone();
                            
                            thread::spawn(move || {
                                let mut local_reads = 0;
                                
                                while r.load(Ordering::Relaxed) {
                                    for _ in 0..100 {
                                        let guard = crossbeam_epoch::pin();
                                        let data_ptr = cfg.load(Ordering::Acquire, &guard);
                                        let data = unsafe { data_ptr.as_ref().unwrap() };
                                        black_box(data.version);
                                        black_box(&data.settings[0]);
                                        local_reads += 1;
                                    }
                                }
                                
                                reads.fetch_add(local_reads, Ordering::Relaxed);
                            })
                        })
                        .collect();

                    // Writer thread updates config
                    for i in 0..100 {
                        let guard = crossbeam_epoch::pin();
                        let old = config.swap(
                            crossbeam_epoch::Owned::new(ConfigData {
                                version: i + 1,
                                settings: vec![i; 100],
                            }),
                            Ordering::Release,
                            &guard,
                        );
                        
                        unsafe {
                            guard.defer_destroy(old);
                        }
                    }

                    running.store(false, Ordering::Relaxed);
                    
                    for handle in reader_handles {
                        let _ = handle.join();
                    }
                });
            },
        );
    }

    group.finish();
}

// Helper struct for config data
#[derive(Clone)]
struct ConfigData {
    version: usize,
    settings: Vec<usize>,
}

// ==================== Scenario 2: Linked List Node Reclamation ====================
// 测试链表节点的内存回收效率

fn bench_linked_list_reclamation(c: &mut Criterion) {
    let mut group = c.benchmark_group("linked_list_reclamation");
    group.sample_size(10);

    for list_size in [100, 1000].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", list_size),
            list_size,
            |b, &list_size| {
                b.iter(|| {
                    let domain = EpochGcDomain::new();
                    let mut gc = domain.gc_handle();
                    let local_epoch = domain.register_reader();

                    // Build a linked list
                    let mut head = Box::new(Node {
                        value: 0,
                        next: None,
                    });

                    for i in 1..list_size {
                        let new_head = Box::new(Node {
                            value: i,
                            next: Some(head),
                        });
                        head = new_head;
                    }

                    // Build an EpochPtr to hold the linked list head
                    let list_ptr = EpochPtr::new(head);
                    
                    // Simulate removing nodes one by one by updating the pointer
                    for _ in 0..list_size {
                        let _guard = local_epoch.pin();
                        let current = list_ptr.load(&_guard);
                        
                        if let Some(next) = current.next.clone() {
                            list_ptr.store(next, &mut gc);
                        } else {
                            break;
                        }
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch", list_size),
            list_size,
            |b, &list_size| {
                b.iter(|| {
                    // Build linked list using crossbeam
                    let head = crossbeam_epoch::Atomic::new(Box::new(Node {
                        value: 0,
                        next: None,
                    }));

                    for i in 1..list_size {
                        let guard = crossbeam_epoch::pin();
                        let old_head = head.load(Ordering::Acquire, &guard);
                        
                        let new_node = Box::new(Node {
                            value: i,
                            next: unsafe { old_head.as_ref().and_then(|n| (**n).next.clone()) },
                        });
                        
                        let old = head.swap(crossbeam_epoch::Owned::new(new_node), Ordering::Release, &guard);
                        
                        if !old.is_null() {
                            unsafe {
                                guard.defer_destroy(old);
                            }
                        }
                    }

                    // Cleanup
                    let guard = crossbeam_epoch::pin();
                    let final_head = head.load(Ordering::Acquire, &guard);
                    if !final_head.is_null() {
                        unsafe {
                            guard.defer_destroy(final_head);
                        }
                    }
                });
            },
        );
    }

    group.finish();
}

#[derive(Clone)]
struct Node {
    #[allow(dead_code)]
    value: usize,
    next: Option<Box<Node>>,
}

// ==================== Scenario 3: Pin Guard Lifetime Impact ====================
// 测试不同生命周期的 Guard 对性能的影响

fn bench_pin_lifetime_impact(c: &mut Criterion) {
    let mut group = c.benchmark_group("pin_lifetime_impact");

    // Short-lived guards (typical case)
    group.bench_function("swmr_epoch_short_lived", |b| {
        let domain = EpochGcDomain::new();
        let local_epoch = domain.register_reader();
        let ptr = EpochPtr::new(42u64);

        b.iter(|| {
            for _ in 0..1000 {
                let guard = local_epoch.pin();
                let _val = ptr.load(&guard);
                black_box(_val);
                // Guard drops immediately
            }
        });
    });

    group.bench_function("crossbeam_epoch_short_lived", |b| {
        let atomic = crossbeam_epoch::Atomic::new(42u64);

        b.iter(|| {
            for _ in 0..1000 {
                let guard = crossbeam_epoch::pin();
                let _val = atomic.load(Ordering::Acquire, &guard);
                black_box(_val);
            }
        });
    });

    // Long-lived guards (might block GC)
    group.bench_function("swmr_epoch_long_lived", |b| {
        let domain = EpochGcDomain::new();
        let local_epoch = domain.register_reader();
        let ptr = EpochPtr::new(42u64);

        b.iter(|| {
            let guard = local_epoch.pin();
            for _ in 0..1000 {
                let _val = ptr.load(&guard);
                black_box(_val);
            }
            // Guard held for entire iteration
        });
    });

    group.bench_function("crossbeam_epoch_long_lived", |b| {
        let atomic = crossbeam_epoch::Atomic::new(42u64);

        b.iter(|| {
            let guard = crossbeam_epoch::pin();
            for _ in 0..1000 {
                let _val = atomic.load(Ordering::Acquire, &guard);
                black_box(_val);
            }
        });
    });

    group.finish();
}

// ==================== Scenario 4: Memory Pressure Test ====================
// 大量对象的快速分配和回收，测试内存压力下的性能

fn bench_memory_pressure(c: &mut Criterion) {
    let mut group = c.benchmark_group("memory_pressure");
    group.sample_size(10);

    for object_size in [64, 256, 1024].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch_allocations", object_size),
            object_size,
            |b, &object_size| {
                b.iter(|| {
                    let domain = EpochGcDomain::new();
                    let mut gc = domain.gc_handle();
                    let local_epoch = domain.register_reader();
                    let ptr = EpochPtr::new(vec![0u8; object_size]);

                    // Rapidly allocate and store large objects
                    for i in 0..1000 {
                        let _guard = local_epoch.pin();
                        let data = vec![i as u8; object_size];
                        ptr.store(data, &mut gc);
                    }
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("crossbeam_epoch_allocations", object_size),
            object_size,
            |b, &object_size| {
                b.iter(|| {
                    for _ in 0..1000 {
                        let guard = crossbeam_epoch::pin();
                        let data = vec![0u8; object_size];
                        guard.defer(move || drop(data));
                    }
                });
            },
        );
    }

    group.finish();
}

fn bench_read_heavy_burst_writes(c: &mut Criterion) {
    let mut group = c.benchmark_group("read_heavy_burst_writes");
    group.sample_size(10);

    group.bench_function("swmr_epoch", |b| {
        b.iter(|| {
            let domain = Arc::new(EpochGcDomain::new());
            let mut gc = domain.gc_handle();
            let data = Arc::new(EpochPtr::new(CacheEntry {
                key: String::from("cache_key"),
                value: vec![0u8; 256],
                timestamp: 0,
            }));

            let running = Arc::new(AtomicBool::new(true));

            // Multiple reader threads
            let reader_handles: Vec<_> = (0..4)
                .map(|_| {
                    let d = domain.clone();
                    let dat = data.clone();
                    let r = running.clone();

                    thread::spawn(move || {
                        let local_epoch = d.register_reader();
                        while r.load(Ordering::Relaxed) {
                            for _ in 0..1000 {
                                let guard = local_epoch.pin();
                                let entry = dat.load(&guard);
                                black_box(&entry.key);
                                black_box(&entry.value);
                            }
                        }
                    })
                })
                .collect();

            // Writer performs burst writes
            for burst in 0..10 {
                thread::sleep(Duration::from_micros(100));
                
                // Burst of 10 writes
                for i in 0..10 {
                    data.store(
                        CacheEntry {
                            key: format!("cache_key_{}", burst * 10 + i),
                            value: vec![i as u8; 256],
                            timestamp: burst * 10 + i,
                        },
                        &mut gc,
                    );
                }
            }

            running.store(false, Ordering::Relaxed);

            for handle in reader_handles {
                let _ = handle.join();
            }
        });
    });

    group.bench_function("crossbeam_epoch", |b| {
        b.iter(|| {
            let data = Arc::new(crossbeam_epoch::Atomic::new(CacheEntry {
                key: String::from("cache_key"),
                value: vec![0u8; 256],
                timestamp: 0,
            }));

            let running = Arc::new(AtomicBool::new(true));

            // Multiple reader threads
            let reader_handles: Vec<_> = (0..4)
                .map(|_| {
                    let dat = data.clone();
                    let r = running.clone();

                    thread::spawn(move || {
                        while r.load(Ordering::Relaxed) {
                            for _ in 0..1000 {
                                let guard = crossbeam_epoch::pin();
                                let entry_ptr = dat.load(Ordering::Acquire, &guard);
                                if let Some(entry) = unsafe { entry_ptr.as_ref() } {
                                    black_box(&entry.key);
                                    black_box(&entry.value);
                                }
                            }
                        }
                    })
                })
                .collect();

            // Writer performs burst writes
            for burst in 0..10 {
                thread::sleep(Duration::from_micros(100));
                
                for i in 0..10 {
                    let guard = crossbeam_epoch::pin();
                    let old = data.swap(
                        crossbeam_epoch::Owned::new(CacheEntry {
                            key: format!("cache_key_{}", burst * 10 + i),
                            value: vec![i as u8; 256],
                            timestamp: burst * 10 + i,
                        }),
                        Ordering::Release,
                        &guard,
                    );

                    unsafe {
                        guard.defer_destroy(old);
                    }
                }
            }

            running.store(false, Ordering::Relaxed);

            for handle in reader_handles {
                let _ = handle.join();
            }
        });
    });

    group.finish();
}

#[derive(Clone)]
struct CacheEntry {
    key: String,
    value: Vec<u8>,
    #[allow(dead_code)]
    timestamp: usize,
}

// ==================== Scenario 6: Nested Pin Guards ====================
// 测试嵌套的 pin guard 对性能的影响

fn bench_nested_pins(c: &mut Criterion) {
    let mut group = c.benchmark_group("nested_pins");

    group.bench_function("swmr_epoch_nested", |b| {
        let domain = EpochGcDomain::new();
        let local_epoch = domain.register_reader();
        let ptr1 = EpochPtr::new(1u64);
        let ptr2 = EpochPtr::new(2u64);
        let ptr3 = EpochPtr::new(3u64);

        b.iter(|| {
            for _ in 0..100 {
                let guard1 = local_epoch.pin();
                let val1 = ptr1.load(&guard1);
                black_box(val1);
                
                {
                    let guard2 = local_epoch.pin();
                    let val2 = ptr2.load(&guard2);
                    black_box(val2);
                    
                    {
                        let guard3 = local_epoch.pin();
                        let val3 = ptr3.load(&guard3);
                        black_box(val3);
                    }
                }
            }
        });
    });

    group.bench_function("crossbeam_epoch_nested", |b| {
        let atomic1 = crossbeam_epoch::Atomic::new(1u64);
        let atomic2 = crossbeam_epoch::Atomic::new(2u64);
        let atomic3 = crossbeam_epoch::Atomic::new(3u64);

        b.iter(|| {
            for _ in 0..100 {
                let guard1 = crossbeam_epoch::pin();
                let val1 = atomic1.load(Ordering::Acquire, &guard1);
                black_box(val1);
                
                {
                    let guard2 = crossbeam_epoch::pin();
                    let val2 = atomic2.load(Ordering::Acquire, &guard2);
                    black_box(val2);
                    
                    {
                        let guard3 = crossbeam_epoch::pin();
                        let val3 = atomic3.load(Ordering::Acquire, &guard3);
                        black_box(val3);
                    }
                }
            }
        });
    });

    group.finish();
}

// ==================== Scenario 7: Variable Reader Contention ====================
// 测试不同读者争用程度下的性能

fn bench_variable_contention(c: &mut Criterion) {
    let mut group = c.benchmark_group("variable_contention");
    group.sample_size(10);

    for contention_level in [1, 4, 16, 64].iter() {
        group.bench_with_input(
            BenchmarkId::new("swmr_epoch", contention_level),
            contention_level,
            |b, &contention_level| {
                b.iter(|| {
                    let domain = Arc::new(EpochGcDomain::new());
                    let data = Arc::new(EpochPtr::new(SharedData {
                        counters: vec![0usize; 16],
                    }));

                    let handles: Vec<_> = (0..contention_level)
                        .map(|_| {
                            let d = domain.clone();
                            let dat = data.clone();

                            thread::spawn(move || {
                                let local_epoch = d.register_reader();
                                for _ in 0..1000 {
                                    let guard = local_epoch.pin();
                                    let shared = dat.load(&guard);
                                    black_box(&shared.counters);
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
            BenchmarkId::new("crossbeam_epoch", contention_level),
            contention_level,
            |b, &contention_level| {
                b.iter(|| {
                    let data = Arc::new(crossbeam_epoch::Atomic::new(SharedData {
                        counters: vec![0usize; 16],
                    }));

                    let handles: Vec<_> = (0..contention_level)
                        .map(|_| {
                            let dat = data.clone();

                            thread::spawn(move || {
                                for _ in 0..1000 {
                                    let guard = crossbeam_epoch::pin();
                                    let shared_ptr = dat.load(Ordering::Acquire, &guard);
                                    unsafe {
                                        let shared = shared_ptr.as_ref().unwrap();
                                        black_box(&shared.counters);
                                    }
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

#[derive(Clone)]
struct SharedData {
    counters: Vec<usize>,
}

criterion_group!(
    benches,
    bench_realistic_swmr_workload,
    bench_linked_list_reclamation,
    bench_pin_lifetime_impact,
    bench_memory_pressure,
    bench_read_heavy_burst_writes,
    bench_nested_pins,
    bench_variable_contention
);
criterion_main!(benches);
