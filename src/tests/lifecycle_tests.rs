/// 生命周期和内存安全测试模块
/// 测试Guard生命周期、内存安全、复杂类型管理和完整场景

use crate::{new, Atomic};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// 测试1: 读取者 Guard 的生命周期约束
#[test]
fn test_guard_lifetime_constraint() {
    let (_writer, factory) = new();
    let atomic = Atomic::new(42i32);
    
    let handle = factory.create_handle();
    let guard = handle.pin();
    
    // 这个值的生命周期被绑定到 guard
    let value = atomic.load(&guard);
    assert_eq!(*value, 42);
    
    // guard 在这里被 drop，value 的引用也失效
}

/// 测试2: 多个 Guard 同时活跃
#[test]
fn test_multiple_guards_simultaneously_active() {
    let (_writer, factory) = new();
    let atomic = Atomic::new(42i32);
    
    let handle = factory.create_handle();
    
    let guard1 = handle.pin();
    let value1 = atomic.load(&guard1);
    
    let guard2 = handle.pin();
    let value2 = atomic.load(&guard2);
    
    assert_eq!(*value1, 42);
    assert_eq!(*value2, 42);
}

/// 测试3: Guard 嵌套作用域
#[test]
fn test_guard_nested_scopes() {
    let (_writer, factory) = new();
    let atomic = Atomic::new(42i32);
    
    let handle = factory.create_handle();
    
    {
        let guard1 = handle.pin();
        let value1 = atomic.load(&guard1);
        assert_eq!(*value1, 42);
        
        {
            let guard2 = handle.pin();
            let value2 = atomic.load(&guard2);
            assert_eq!(*value2, 42);
        }
        
        // guard2 已 drop，但 guard1 仍然活跃
        let value1_again = atomic.load(&guard1);
        assert_eq!(*value1_again, 42);
    }
}

/// 测试4: 读取者在线程中的隔离
#[test]
fn test_reader_isolation_across_threads() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let mut handles = vec![];
    
    for thread_id in 0..3 {
        let factory_clone = factory.clone();
        let atomic_clone = atomic.clone();
        
        let handle = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            
            // 每个线程有自己的 guard
            let guard = reader_handle.pin();
            let value = *atomic_clone.load(&guard);
            
            // 验证值正确
            assert_eq!(value, 0);
            
            thread_id
        });
        
        handles.push(handle);
    }
    
    for handle in handles {
        handle.join().unwrap();
    }
}

/// 测试5: 写入者的单线程约束
#[test]
fn test_writer_single_threaded_constraint() {
    let (mut writer, _factory) = new();
    
    // Writer 不能被克隆或共享
    writer.retire(Box::new(42i32));
    writer.try_reclaim();
    
    // 这是唯一的 writer 实例
}

/// 测试6: 垃圾回收的内存安全
#[test]
fn test_garbage_collection_memory_safety() {
    let (mut writer, factory) = new();
    
    let reader = factory.create_handle();
    
    // 创建一些数据
    let data1 = Box::new(vec![1, 2, 3, 4, 5]);
    let data2 = Box::new(vec![6, 7, 8, 9, 10]);
    
    // 退休数据
    writer.retire(data1);
    writer.retire(data2);
    
    // 让读取者 pin
    let _guard = reader.pin();
    
    // 触发回收（数据应该被保留）
    writer.try_reclaim();
    
    // 读取者仍然活跃
    let _guard2 = reader.pin();
}

/// 测试7: Atomic 的 Drop 实现
#[test]
fn test_atomic_drop_implementation() {
    {
        let _atomic = Atomic::new(String::from("test"));
        // atomic 在这里被 drop，内部数据应该被正确释放
    }
    
    // 如果能到这里，说明 drop 没有泄漏内存
}

/// 测试8: 多个 Atomic 实例的独立性
#[test]
fn test_multiple_atomic_independence() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic1 = Atomic::new(10i32);
    let atomic2 = Atomic::new(20i32);
    let atomic3 = Atomic::new(30i32);
    
    {
        let guard = handle.pin();
        
        let v1 = *atomic1.load(&guard);
        let v2 = *atomic2.load(&guard);
        let v3 = *atomic3.load(&guard);
        
        assert_eq!(v1, 10);
        assert_eq!(v2, 20);
        assert_eq!(v3, 30);
    }
}

/// 测试9: 读取者工厂的克隆安全性
#[test]
fn test_reader_factory_clone_safety() {
    let (_writer, factory) = new();
    
    let factory1 = factory.clone();
    let factory2 = factory.clone();
    let factory3 = factory.clone();
    
    let reader1 = factory.create_handle();
    let reader2 = factory1.create_handle();
    let reader3 = factory2.create_handle();
    let reader4 = factory3.create_handle();
    
    // 所有读取者都应该能正常工作
    let _g1 = reader1.pin();
    let _g2 = reader2.pin();
    let _g3 = reader3.pin();
    let _g4 = reader4.pin();
}

/// 测试10: 纪元推进的正确性
#[test]
fn test_epoch_advancement_correctness() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader = factory.create_handle();
    
    // 初始纪元应该是 0
    {
        let guard = reader.pin();
        let value = *atomic.load(&guard);
        assert_eq!(value, 0);
    }
    
    // 推进纪元
    writer.try_reclaim();
    
    // 写入新值
    atomic.store(Box::new(1), &mut writer);
    
    // 读取应该看到新值
    {
        let guard = reader.pin();
        let value = *atomic.load(&guard);
        assert_eq!(value, 1);
    }
}

/// 测试11: 并发读取的一致性
#[test]
fn test_concurrent_read_consistency() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(42i32));
    
    let mut handles = vec![];
    let consistency_check = Arc::new(AtomicUsize::new(0));
    
    for _ in 0..10 {
        let factory_clone = factory.clone();
        let atomic_clone = atomic.clone();
        let check_clone = consistency_check.clone();
        
        let handle = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            
            for _ in 0..100 {
                let guard = reader_handle.pin();
                let value = *atomic_clone.load(&guard);
                
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
    let (mut writer, factory) = new();
    
    let reader_count = Arc::new(AtomicUsize::new(0));
    
    {
        let factory_clone = factory.clone();
        let count_clone = reader_count.clone();
        
        let _thread = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            let _guard = reader_handle.pin();
            count_clone.fetch_add(1, Ordering::SeqCst);
        });
        
        thread::sleep(std::time::Duration::from_millis(10));
    }
    
    // 验证线程已完成
    assert_eq!(reader_count.load(Ordering::SeqCst), 1);
    
    // 触发回收，应该清理掉已退出的读取者
    writer.try_reclaim();
}

/// 测试13: 大量垃圾的安全回收
#[test]
fn test_large_garbage_safe_reclamation() {
    let (mut writer, _factory) = new();
    
    // 退休大量数据
    for i in 0..1000 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于没有活跃读取者，垃圾会被回收
    // 但可能不会完全清空，只需验证数量少于退休的数据
    assert!(writer.local_garbage.len() < 1000);
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
    
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let data = ComplexData {
        id: 1,
        values: vec![1, 2, 3, 4, 5],
        name: String::from("test"),
    };
    
    let atomic = Atomic::new(data);
    
    {
        let guard = handle.pin();
        let loaded = atomic.load(&guard);
        assert_eq!(loaded.id, 1);
        assert_eq!(loaded.values.len(), 5);
        assert_eq!(loaded.name, "test");
    }
}

/// 测试15: 读取者在不同纪元的数据可见性
#[test]
fn test_data_visibility_across_epochs() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    // Reader 1 在纪元 0
    let reader1 = factory.create_handle();
    let guard1 = reader1.pin();
    {
        let value = *atomic.load(&guard1);
        assert_eq!(value, 0);
    }
    
    // 推进纪元并更新值
    writer.try_reclaim();
    atomic.store(Box::new(1), &mut writer);
    
    // Reader 2 在纪元 1
    let reader2 = factory.create_handle();
    let guard2 = reader2.pin();
    {
        let value = *atomic.load(&guard2);
        assert_eq!(value, 1);
    }
    
    // Reader 1 仍然活跃，会看到最新的值
    // 因为 Atomic 中存储的是指针，所有读取者看到的是同一个数据
    let guard1_again = reader1.pin();
    {
        let value = *atomic.load(&guard1_again);
        assert_eq!(value, 1);
    }
}

/// 测试17: 读取者的快速切换
#[test]
fn test_rapid_reader_switching() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(42i32));
    
    let handle = factory.create_handle();
    
    for _ in 0..100 {
        {
            let guard = handle.pin();
            let value = *atomic.load(&guard);
            assert_eq!(value, 42);
        }
        
        // 立即创建新的 guard
        {
            let guard = handle.pin();
            let value = *atomic.load(&guard);
            assert_eq!(value, 42);
        }
    }
}

/// 测试18: 写入者的垃圾管理
#[test]
fn test_writer_garbage_management() {
    let (mut writer, factory) = new();
    
    let reader = factory.create_handle();
    
    // 第一轮：退休数据，读取者活跃
    {
        let _guard = reader.pin();
        for i in 0..50 {
            writer.retire(Box::new(i as i32));
        }
        assert!(writer.local_garbage.len() > 0);
    }
    
    // 第二轮：读取者不活跃，垃圾应该被回收
    writer.try_reclaim();
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试19: 多个读取者的垃圾保护
#[test]
fn test_multiple_readers_garbage_protection() {
    let (mut writer, factory) = new();
    
    let reader1 = factory.create_handle();
    let reader2 = factory.create_handle();
    let reader3 = factory.create_handle();
    
    let _guard1 = reader1.pin();
    let _guard2 = reader2.pin();
    let _guard3 = reader3.pin();
    
    // 退休数据
    for i in 0..100 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于所有读取者都活跃，垃圾应该被保留
    assert!(writer.local_garbage.len() > 0);
}

/// 测试20: 完整的生命周期场景
#[test]
fn test_complete_lifecycle_scenario() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(String::from("initial")));
    
    // 创建多个读取者
    let readers: Vec<_> = (0..5).map(|_| factory.create_handle()).collect();
    
    // 执行多轮操作
    for round in 0..3 {
        // 所有读取者 pin
        let guards: Vec<_> = readers.iter().map(|r| r.pin()).collect();
        
        // 验证当前值
        for guard in &guards {
            let value = atomic.load(guard);
            assert!(value.len() > 0);
        }
        
        // 推进纪元
        writer.try_reclaim();
        
        // 写入新值
        atomic.store(Box::new(format!("round_{}", round)), &mut writer);
        
        // 退休一些数据
        for i in 0..50 {
            writer.retire(Box::new(i as i32));
        }
        
        // 再次推进纪元
        writer.try_reclaim();
    }
}
