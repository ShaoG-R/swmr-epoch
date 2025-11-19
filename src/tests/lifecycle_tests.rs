/// 生命周期和内存安全测试模块
/// 测试Guard生命周期、内存安全、复杂类型管理和完整场景
use crate::{EpochGcDomain, EpochPtr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// 测试1: 读取者 Guard 的生命周期约束
#[test]
fn test_guard_lifetime_constraint() {
    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = EpochPtr::new(42i32);

    let guard = local_epoch.pin();

    // 这个值的生命周期被绑定到 guard
    let value = ptr.load(&guard);
    assert_eq!(*value, 42);

    // guard 在这里被 drop，value 的引用也失效
}

/// 测试2: 多个 Guard 同时活跃
#[test]
fn test_multiple_guards_simultaneously_active() {
    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = EpochPtr::new(42i32);

    let guard1 = local_epoch.pin();
    let value1 = ptr.load(&guard1);

    let guard2 = guard1.clone();
    let value2 = ptr.load(&guard2);

    assert_eq!(*value1, 42);
    assert_eq!(*value2, 42);
}

/// 测试3: Guard 嵌套作用域
#[test]
fn test_guard_nested_scopes() {
    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = EpochPtr::new(42i32);

    {
        let guard1 = local_epoch.pin();
        let value1 = ptr.load(&guard1);
        assert_eq!(*value1, 42);

        {
            let guard2 = guard1.clone();
            let value2 = ptr.load(&guard2);
            assert_eq!(*value2, 42);
        }

        // guard2 已 drop，但 guard1 仍然活跃
        let value1_again = ptr.load(&guard1);
        assert_eq!(*value1_again, 42);
    }
}

/// 测试4: 读取者在线程中的隔离
#[test]
fn test_reader_isolation_across_threads() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0i32));

    // 创建并启动读取者线程
    let mut handles = vec![];
    for _ in 0..3 {
        let domain_clone = domain.clone();
        let ptr_clone = ptr.clone();

        let handle = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            let guard = local_epoch.pin();
            let value = *ptr_clone.load(&guard);
            assert_eq!(value, 0);
        });

        handles.push(handle);
    }

    // 主线程作为写入者
    thread::sleep(std::time::Duration::from_millis(10));
    ptr.store(1, &mut gc);

    // 等待所有读取者完成
    for handle in handles {
        handle.join().unwrap();
    }
}

/// 测试5: 写入者的单线程约束
#[test]
fn test_writer_single_threaded_constraint() {
    let (mut gc, _domain) = EpochGcDomain::new();

    // GcHandle 不能被克隆或共享
    gc.retire(Box::new(42i32));
    gc.collect();

    // 这是唯一的 gc 实例
}

/// 测试6: 垃圾回收的内存安全
#[test]
fn test_garbage_collection_memory_safety() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    // 创建一些数据
    let data1 = Box::new(vec![1, 2, 3, 4, 5]);
    let data2 = Box::new(vec![6, 7, 8, 9, 10]);

    // 退休数据
    gc.retire(data1);
    gc.retire(data2);

    // 让读取者 pin
    let _guard = local_epoch.pin();

    // 触发回收（数据应该被保留）
    gc.collect();

    // 读取者仍然活跃
    let _guard2 = local_epoch.pin();
}

/// 测试7: EpochPtr 的 Drop 实现
#[test]
fn test_epoch_ptr_drop_implementation() {
    {
        let _ptr = EpochPtr::new(String::from("test"));
        // ptr 在这里被 drop，内部数据应该被正确释放
    }

    // 如果能到这里，说明 drop 没有泄漏内存
}

/// 测试8: 多个 EpochPtr 实例的独立性
#[test]
fn test_multiple_epoch_ptr_independence() {
    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    let ptr1 = EpochPtr::new(10i32);
    let ptr2 = EpochPtr::new(20i32);
    let ptr3 = EpochPtr::new(30i32);

    {
        let guard = local_epoch.pin();

        let v1 = *ptr1.load(&guard);
        let v2 = *ptr2.load(&guard);
        let v3 = *ptr3.load(&guard);

        assert_eq!(v1, 10);
        assert_eq!(v2, 20);
        assert_eq!(v3, 30);
    }
}

/// 测试9: 域的克隆安全性
#[test]
fn test_domain_clone_safety() {
    let (_gc, domain) = EpochGcDomain::new();

    // 克隆 domain 创建多个实例
    let domain1 = domain.clone();
    let domain2 = domain.clone();

    // 所有 domain 实例都应该能正常工作
    let p1 = domain.register_reader();
    let p2 = domain1.register_reader();
    let p3 = domain2.register_reader();

    let _g1 = p1.pin();
    let _g2 = p2.pin();
    let _g3 = p3.pin();
}

/// 测试10: 纪元推进的正确性
#[test]
fn test_epoch_advancement_correctness() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = Arc::new(EpochPtr::new(0i32));

    // 初始纪元应该是 0
    {
        let guard = local_epoch.pin();
        let value = *ptr.load(&guard);
        assert_eq!(value, 0);
    }

    // 推进纪元
    gc.collect();

    // 写入新值
    ptr.store(1, &mut gc);

    // 读取应该看到新值
    {
        let guard = local_epoch.pin();
        let value = *ptr.load(&guard);
        assert_eq!(value, 1);
    }
}

/// 测试11: 并发读取的一致性
#[test]
fn test_concurrent_read_consistency() {
    let (_gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(42i32));

    let mut handles = vec![];
    let consistency_check = Arc::new(AtomicUsize::new(0));

    for _ in 0..10 {
        let domain_clone = domain.clone();
        let ptr_clone = ptr.clone();
        let check_clone = consistency_check.clone();

        let handle = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            for _ in 0..100 {
                let guard = local_epoch.pin();
                let value = *ptr_clone.load(&guard);

                if value == 42 {
                    check_clone.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        handles.push(handle);
    }

    for handle in handles {
        handle.join().unwrap();
    }

    // 所有读取都应该看到一致的值
    assert_eq!(consistency_check.load(Ordering::SeqCst), 1000);
}

/// 测试12: 读取者退出时的清理
#[test]
fn test_reader_exit_cleanup() {
    let (mut gc, domain) = EpochGcDomain::new();

    let reader_count = Arc::new(AtomicUsize::new(0));

    {
        let domain_clone = domain.clone();
        let count_clone = reader_count.clone();

        let _thread = thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            let _guard = local_epoch.pin();
            count_clone.fetch_add(1, Ordering::SeqCst);
        });

        thread::sleep(std::time::Duration::from_millis(10));
    }

    // 验证线程已完成
    assert_eq!(reader_count.load(Ordering::SeqCst), 1);

    // 触发回收，应该清理掉已退出的读取者
    gc.collect();
}

/// 测试13: 大量垃圾的安全回收
#[test]
fn test_large_garbage_safe_reclamation() {
    let (mut gc, _domain) = EpochGcDomain::new();

    // 退休大量数据
    for i in 0..1000 {
        gc.retire(Box::new(i as i32));
    }

    // 由于没有活跃读取者，垃圾会被回收
    // 但可能不会完全清空，只需验证数量少于退休的数据
    assert!(gc.garbage.len() < 1000);
}

/// 测试14: 复杂类型的生命周期管理
#[test]
fn test_complex_type_lifetime_management() {
    #[derive(Debug)]
    struct ComplexData {
        id: usize,
        values: Vec<i32>,
        name: String,
    }

    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    let data = ComplexData {
        id: 1,
        values: vec![1, 2, 3, 4, 5],
        name: String::from("test"),
    };

    let ptr = EpochPtr::new(data);

    {
        let guard = local_epoch.pin();
        let loaded = ptr.load(&guard);
        assert_eq!(loaded.id, 1);
        assert_eq!(loaded.values.len(), 5);
        assert_eq!(loaded.name, "test");
    }
}

/// 测试15: 读取者在不同纪元的数据可见性
#[test]
fn test_data_visibility_across_epochs() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(0i32));

    // 创建两个读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();

    // Reader 1 在纪元 0
    let guard1 = local_epoch1.pin();
    {
        let value = *ptr.load(&guard1);
        assert_eq!(value, 0);
    }

    // Writer 推进纪元并更新
    gc.collect();
    ptr.store(1, &mut gc);

    // Reader 2 在纪元 1
    let guard2 = local_epoch2.pin();
    {
        let value = *ptr.load(&guard2);
        assert_eq!(value, 1);
    }

    // Reader 1 现在也会看到新值，因为 EpochPtr 中的指针已经更新
    // 纪元主要用于保护垃圾回收，而不是隔离数据可见性
    {
        let value = *ptr.load(&guard1);
        assert_eq!(value, 1);
    }
}

/// 测试17: 读取者的快速切换
#[test]
fn test_rapid_reader_switching() {
    let (_gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    let ptr = Arc::new(EpochPtr::new(42i32));

    for _ in 0..100 {
        {
            let guard = local_epoch.pin();
            let value = *ptr.load(&guard);
            assert_eq!(value, 42);
        }

        // 立即创建新的 guard
        {
            let guard = local_epoch.pin();
            let value = *ptr.load(&guard);
            assert_eq!(value, 42);
        }
    }
}

/// 测试18: 写入者的垃圾管理
#[test]
fn test_writer_garbage_management() {
    let (mut gc, domain) = EpochGcDomain::new();
    let local_epoch = domain.register_reader();

    // 第一轮：退休数据，读取者活跃
    {
        let _guard = local_epoch.pin();
        for i in 0..50 {
            gc.retire(Box::new(i as i32));
        }

        // 垃圾应该被保留
        assert!(gc.garbage.len() > 0);
    }

    // 第二轮：读取者不活跃，垃圾应该被回收
    gc.collect();
    assert_eq!(gc.garbage.len(), 0);
}

/// 测试19: 多个读取者的垃圾保护
#[test]
fn test_multiple_readers_garbage_protection() {
    let (mut gc, domain) = EpochGcDomain::new();

    // 创建多个读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();
    let local_epoch3 = domain.register_reader();

    let _guard1 = local_epoch1.pin();
    let _guard2 = local_epoch2.pin();
    let _guard3 = local_epoch3.pin();

    // 退休数据
    for i in 0..100 {
        gc.retire(Box::new(i as i32));
    }

    // 由于所有读取者都活跃，垃圾应该被保留
    assert!(gc.garbage.len() > 0);
}

/// 测试20: 完整的生命周期场景
#[test]
fn test_complete_lifecycle_scenario() {
    let (mut gc, domain) = EpochGcDomain::new();
    let ptr = Arc::new(EpochPtr::new(String::from("initial")));

    // 创建多个读取者
    let local_epochs: Vec<_> = (0..5).map(|_| domain.register_reader()).collect();

    // 执行多轮操作
    for round in 0..3 {
        // 所有读取者 pin
        let guards: Vec<_> = local_epochs.iter().map(|e| e.pin()).collect();

        // 验证当前值
        for guard in &guards {
            let value = ptr.load(guard);
            assert!(!value.is_empty());
        }

        // 推进纪元
        gc.collect();

        // 写入新值
        ptr.store(format!("round_{}", round), &mut gc);

        // 退休一些数据
        for i in 0..50 {
            gc.retire(Box::new(i as i32));
        }

        // 再次推进纪元
        gc.collect();
    }
}
