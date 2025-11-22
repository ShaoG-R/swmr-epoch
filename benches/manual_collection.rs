use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;
use swmr_epoch::{EpochGcDomain, EpochPtr};

/// Benchmark: Manual collection performance with varying garbage counts
///
/// This benchmark measures the performance of manually collecting garbage when
/// automatic reclamation is disabled. It tests how collection performance scales
/// with the number of retired objects.
fn bench_manual_collection(c: &mut Criterion) {
    let mut group = c.benchmark_group("manual_collection");

    // Test with different garbage counts
    for garbage_count in [10, 50, 100, 500, 1000, 5000].iter() {
        group.bench_with_input(
            BenchmarkId::new("collect_n_garbage", garbage_count),
            garbage_count,
            |b, &garbage_count| {
                b.iter(|| {
                    // Create GC domain with automatic reclamation disabled
                    let (mut gc, _domain) = EpochGcDomain::builder()
                        .auto_reclaim_threshold(None)
                        .build();
                    let epoch_ptr = EpochPtr::new(0u64);

                    // Retire N objects without automatic collection
                    for i in 0..garbage_count {
                        epoch_ptr.store(i, &mut gc);
                    }

                    // Manually trigger collection and measure performance
                    gc.collect();
                    black_box(&gc);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Collection overhead with active readers
///
/// This benchmark measures the performance of manual collection when there are
/// active readers pinned to various epochs. This tests the worst-case scenario
/// where the collector must scan multiple active readers.
fn bench_collection_with_readers(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_with_readers");

    for num_readers in [0, 2, 4, 8, 16].iter() {
        group.bench_with_input(
            BenchmarkId::new("readers", num_readers),
            num_readers,
            |b, &num_readers| {
                b.iter(|| {
                    // Create GC domain with automatic reclamation disabled
                    let (mut gc, domain) = EpochGcDomain::builder()
                        .auto_reclaim_threshold(None)
                        .build();
                    let epoch_ptr = EpochPtr::new(0u64);

                    // Register readers and pin them
                    let local_epochs: Vec<_> =
                        (0..num_readers).map(|_| domain.register_reader()).collect();
                    let _guards: Vec<_> = local_epochs.iter().map(|le| le.pin()).collect();

                    // Retire 100 objects
                    for i in 0..100 {
                        epoch_ptr.store(i, &mut gc);
                    }

                    // Manually trigger collection
                    gc.collect();
                    black_box(&gc);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Multiple collection cycles
///
/// This benchmark measures the performance of multiple collection cycles in sequence.
/// It tests how well the collector handles repeated collection operations.
fn bench_multiple_collections(c: &mut Criterion) {
    let mut group = c.benchmark_group("multiple_collections");

    for num_cycles in [5, 10, 20, 50].iter() {
        group.bench_with_input(
            BenchmarkId::new("cycles", num_cycles),
            num_cycles,
            |b, &num_cycles| {
                b.iter(|| {
                    // Create GC domain with automatic reclamation disabled
                    let (mut gc, _domain) = EpochGcDomain::builder()
                        .auto_reclaim_threshold(None)
                        .build();
                    let epoch_ptr = EpochPtr::new(0u64);

                    for cycle in 0..num_cycles {
                        // Retire some objects
                        for i in 0..20 {
                            epoch_ptr.store(cycle * 100 + i, &mut gc);
                        }

                        // Manually collect
                        gc.collect();
                    }

                    black_box(&gc);
                });
            },
        );
    }

    group.finish();
}

/// Benchmark: Collection latency distribution
///
/// This benchmark measures the latency of a single collection operation
/// with a fixed amount of garbage, providing insight into the consistency
/// of collection performance.
fn bench_collection_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("collection_latency");
    group.sample_size(100);

    group.bench_function("collect_100_objects", |b| {
        b.iter(|| {
            // Create GC domain with automatic reclamation disabled
            let (mut gc, _domain) = EpochGcDomain::builder()
                .auto_reclaim_threshold(None)
                .build();
            let epoch_ptr = EpochPtr::new(0u64);

            // Retire 100 objects
            for i in 0..100 {
                epoch_ptr.store(i, &mut gc);
            }

            // Measure collection latency
            gc.collect();
            black_box(&gc);
        });
    });

    group.finish();
}

/// Benchmark: Compare auto vs manual collection
///
/// This benchmark compares the performance of automatic collection (with default threshold)
/// versus manual collection. It helps understand the overhead of automatic triggering.
fn bench_auto_vs_manual(c: &mut Criterion) {
    let mut group = c.benchmark_group("auto_vs_manual");

    group.bench_function("auto_collection_default_threshold", |b| {
        b.iter(|| {
            // Create GC domain with default automatic reclamation
            let (mut gc, _domain) = EpochGcDomain::new();
            let epoch_ptr = EpochPtr::new(0u64);

            // Retire 200 objects (will trigger automatic collection)
            for i in 0..200 {
                epoch_ptr.store(i, &mut gc);
            }

            black_box(&gc);
        });
    });

    group.bench_function("manual_collection_200_objects", |b| {
        b.iter(|| {
            // Create GC domain with automatic reclamation disabled
            let (mut gc, _domain) = EpochGcDomain::builder()
                .auto_reclaim_threshold(None)
                .build();
            let epoch_ptr = EpochPtr::new(0u64);

            // Retire 200 objects
            for i in 0..200 {
                epoch_ptr.store(i, &mut gc);
            }

            // Manually collect once at the end
            gc.collect();
            black_box(&gc);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_manual_collection,
    bench_collection_with_readers,
    bench_multiple_collections,
    bench_collection_latency,
    bench_auto_vs_manual
);
criterion_main!(benches);
