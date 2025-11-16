/// 并发测试模块
/// 测试并发场景、纪元管理和多读取者场景
use crate::{EpochGcDomain, EpochPtr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// 测试1: 单个写入者，多个读取者并发读取
#[test]
fn test_single_writer_multiple_readers_concurrent_reads() {
    let (mut _gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0i32));

    let mut handles = vec![];

    // 创建 5 个读取者线程
    for _i in 0..5 {
        let domain_clone = domain.clone();
        let ptr_clone = ptr.clone();

        let handle = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            // 每个读取者读取 10 次
            for _ in 0..10 {
                let guard = local_epoch.pin();
                let value = *ptr_clone.load(&guard);
                assert!(value >= 0);
            }
        });

        handles.push(handle);
    }

    // 等待所有读取者完成
    for handle in handles {
        handle.join().unwrap();
    }
}

/// 测试2: 写入者更新，读取者观察
#[test]
fn test_writer_updates_readers_observe() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0i32));

    let domain_clone = domain.clone();
    let ptr_clone = ptr.clone();

    let reader_thread = thread::spawn(move || {
        let local_epoch = domain_clone.register_reader();
        // 读取初始值
        {
            let guard = local_epoch.pin();
            let value = *ptr_clone.load(&guard);
            assert_eq!(value, 0);
        }

        // 等待一段时间让写入者更新
        thread::sleep(std::time::Duration::from_millis(10));

        // 读取更新后的值
        {
            let guard = local_epoch.pin();
            let value = *ptr_clone.load(&guard);
            assert_eq!(value, 100);
        }
    });

    // 主线程作为写入者
    thread::sleep(std::time::Duration::from_millis(5));
    ptr.store(100, &mut gc);

    reader_thread.join().unwrap();
}

/// 测试3: 顺序写入操作
#[test]
fn test_sequential_writer_operations() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = Arc::new(EpochPtr::new(1i32));

    // 第一次写入
    ptr.store(2, &mut gc);
    {
        let guard = local_epoch.pin();
        assert_eq!(*ptr.load(&guard), 2);
    }

    // 第二次写入
    ptr.store(3, &mut gc);
    {
        let guard = local_epoch.pin();
        assert_eq!(*ptr.load(&guard), 3);
    }

    // 第三次写入
    ptr.store(4, &mut gc);
    {
        let guard = local_epoch.pin();
        assert_eq!(*ptr.load(&guard), 4);
    }
}

/// 测试4: 读取者在不同纪元中的行为
#[test]
fn test_readers_in_different_epochs() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0i32));

    // 创建两个不同的 LocalEpoch 实例来模拟不同的读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();

    // Reader 1 pin
    let guard1 = local_epoch1.pin();
    {
        let value = *ptr.load(&guard1);
        assert_eq!(value, 0);
    }

    // Writer 推进纪元并更新
    gc.collect();
    ptr.store(10, &mut gc);

    // Reader 2 pin（在新纪元）
    let guard2 = local_epoch2.pin();
    {
        let value = *ptr.load(&guard2);
        assert_eq!(value, 10);
    }

    // Reader 1 现在也会看到新值，因为 EpochPtr 中的指针已经更新
    // 纪元主要用于保护垃圾回收，而不是隔离数据可见性
    {
        let value = *ptr.load(&guard1);
        assert_eq!(value, 10);
    }
}

/// 测试5: 垃圾回收触发
#[test]
fn test_garbage_collection_trigger() {
    let (mut gc, _domain) = EpochGcDomain::new();

    // 退休数据直到触发回收
    for i in 0..70 {
        gc.retire(Box::new(i as i32));
    }

    // 由于 AUTO_RECLAIM_THRESHOLD = 64，第 65 个退休会触发 collect
    // 在没有活跃读取者的情况下，垃圾应该被清空
    // 只需验证垃圾数量少于退休的数据数量
    assert!(gc.local_garbage.len() < 70);
}

/// 测试6: 活跃读取者保护垃圾
#[test]
fn test_active_reader_protects_garbage() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    // 让读取者 pin，保持活跃
    let _guard = local_epoch.pin();

    // 退休数据直到触发回收
    for i in 0..70 {
        gc.retire(Box::new(i as i32));
    }

    // 由于读取者仍然活跃，垃圾不应该被完全清空
    // （至少应该保留一些垃圾）
    assert!(gc.local_garbage.len() > 0);
}

/// 测试7: 读取者 drop 后垃圾被回收
#[test]
fn test_garbage_reclaimed_after_reader_drop() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    {
        let _guard = local_epoch.pin();

        // 在读取者活跃时退休数据
        for i in 0..70 {
            gc.retire(Box::new(i as i32));
        }

        // 垃圾应该被保留
        assert!(gc.local_garbage.len() > 0);
    }

    // 读取者 drop 后，触发一次回收
    gc.collect();

    // 现在垃圾应该被清空
    assert_eq!(gc.local_garbage.len(), 0);
}

/// 测试8: 多个读取者的最小纪元计算
#[test]
fn test_min_epoch_calculation_multiple_readers() {
    let (mut gc, domain) = EpochGcDomain::new();

    // 创建两个不同的 LocalEpoch 来模拟不同的读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();

    // Reader 1 在纪元 0
    let _guard1 = local_epoch1.pin();

    // 推进纪元
    gc.collect();

    // Reader 2 在纪元 1
    let _guard2 = local_epoch2.pin();

    // 退休一些数据
    gc.retire(Box::new(100i32));

    // 再次回收，应该保留在纪元 0 之后的垃圾
    gc.collect();

    // 由于 reader1 仍在纪元 0，垃圾应该被保留
    assert!(gc.local_garbage.len() > 0);
}

/// 测试9: 大量并发读取
#[test]
fn test_high_concurrency_reads() {
    let (mut _gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(42i32));

    let mut handles = vec![];

    // 创建 20 个读取者线程
    for _ in 0..20 {
        let domain_clone = domain.clone();
        let ptr_clone = ptr.clone();

        let handle = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            // 每个读取者读取 100 次
            for _ in 0..100 {
                let guard = local_epoch.pin();
                let value = *ptr_clone.load(&guard);
                assert_eq!(value, 42);
            }
        });

        handles.push(handle);
    }

    // 等待所有线程完成
    for handle in handles {
        handle.join().unwrap();
    }
}

/// 测试10: 读取者线程退出后的清理
#[test]
fn test_reader_thread_exit_cleanup() {
    let (mut gc, domain) = EpochGcDomain::new();

    let counter = Arc::new(AtomicUsize::new(0));

    {
        let domain_clone = domain.clone();
        let counter_clone = counter.clone();

        let _thread = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            let _guard = local_epoch.pin();
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        // 等待线程完成
        thread::sleep(std::time::Duration::from_millis(10));
    }

    // 验证线程已完成
    assert_eq!(counter.load(Ordering::SeqCst), 1);

    // 触发回收，应该能清理掉已退出的读取者
    gc.collect();
}

/// 测试11: 交替的读写操作
#[test]
fn test_interleaved_read_write_operations() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = Arc::new(EpochPtr::new(0i32));

    for i in 0..10 {
        // 写入
        ptr.store(i, &mut gc);

        // 读取
        {
            let guard = local_epoch.pin();
            let value = *ptr.load(&guard);
            assert_eq!(value, i);
        }
    }
}

/// 测试12: 大量垃圾回收循环
#[test]
fn test_heavy_garbage_collection_cycles() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    for cycle in 0..10 {
        // 在每个循环中退休大量数据
        for i in 0..100 {
            gc.retire(Box::new((cycle * 100 + i) as i32));
        }

        // 触发回收
        gc.collect();

        // 读取者仍然活跃
        let _guard = local_epoch.pin();
    }
}

/// 测试13: 读取者在写入者更新时持有 guard
/// Test reader holds guard while writer updates
#[test]
fn test_reader_holds_guard_during_updates() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0));
    let num_updates = 50;

    let domain_clone = domain.clone();
    let ptr_clone = ptr.clone();

    // Reader thread: holds guard and a reference for a while
    let reader = thread::spawn(move || {
        let reader_epoch = domain_clone.register_reader();
        // Hold a guard for a while
        // 持有 guard 一段时间
        let guard = reader_epoch.pin();
        // Get a reference to the current value
        // 获取当前值的引用
        let value_ref = ptr_clone.load(&guard);
        let initial_value = *value_ref;
        thread::sleep(std::time::Duration::from_millis(10));
        // The same reference should still be valid and consistent
        // even though writer has updated the pointer
        // 即使写入者已更新指针，相同的引用仍应有效且一致
        assert_eq!(*value_ref, initial_value);
        // Value should be in valid range
        // 值应在有效范围内
        assert!(initial_value >= 0 && initial_value <= num_updates);
    });

    // Writer on main thread: performs multiple updates
    // 主线程上的写入者：执行多次更新
    for i in 1..=num_updates {
        ptr.store(i, &mut gc);
    }

    reader.join().unwrap();
}
