/// 边界情况和压力测试模块
/// 测试边界条件、垃圾回收阈值、数据类型变化和高频操作

use crate::{new, Atomic};
use std::sync::Arc;
use std::thread;

/// 测试1: 空的垃圾回收
#[test]
fn test_empty_garbage_collection() {
    let (mut writer, _factory) = new();
    
    // 不退休任何数据，直接回收
    writer.try_reclaim();
    
    // 应该没有问题
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试2: 单个数据的退休和回收
#[test]
fn test_single_data_retire_and_reclaim() {
    let (mut writer, _factory) = new();
    
    writer.retire(Box::new(42i32));
    assert_eq!(writer.local_garbage.len(), 1);
    
    writer.try_reclaim();
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试3: 恰好达到回收阈值
#[test]
fn test_exactly_reach_reclaim_threshold() {
    let (mut writer, _factory) = new();
    
    // 退休 64 个数据（RECLAIM_THRESHOLD = 64）
    for i in 0..64 {
        writer.retire(Box::new(i as i32));
    }
    
    // 应该还没有自动回收
    assert_eq!(writer.local_garbage.len(), 64);
    
    // 再退休一个，应该触发自动回收
    writer.retire(Box::new(64i32));
    
    // 回收后应该清空
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试4: 超过回收阈值
#[test]
fn test_exceed_reclaim_threshold() {
    let (mut writer, _factory) = new();
    
    // 退休 100 个数据
    for i in 0..100 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于没有活跃读取者，垃圾会被回收
    // 但可能不会完全清空，只需验证数量少于退休的数据
    assert!(writer.local_garbage.len() < 100);
}

/// 测试5: 零大小类型
#[test]
fn test_zero_sized_type() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    #[derive(Debug, PartialEq)]
    struct ZeroSized;
    
    let atomic = Atomic::new(ZeroSized);
    
    {
        let guard = handle.pin();
        let _value = atomic.load(&guard);
        // ZST 应该能正常工作
    }
}

/// 测试6: 大型数据结构
#[test]
fn test_large_data_structure() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    #[derive(Debug, PartialEq)]
    struct LargeData {
        data: [u64; 1000],
    }
    
    let large = LargeData { data: [42; 1000] };
    let atomic = Atomic::new(large);
    
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(value.data[0], 42);
        assert_eq!(value.data[999], 42);
    }
}

/// 测试7: 嵌套结构体
#[test]
fn test_nested_structures() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    #[derive(Debug, PartialEq)]
    struct Inner {
        value: i32,
    }
    
    #[derive(Debug, PartialEq)]
    struct Outer {
        inner: Inner,
        name: String,
    }
    
    let outer = Outer {
        inner: Inner { value: 42 },
        name: String::from("test"),
    };
    
    let atomic = Atomic::new(outer);
    
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(value.inner.value, 42);
        assert_eq!(value.name, "test");
    }
}

/// 测试8: 向量类型
#[test]
fn test_vector_type() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let vec = vec![1, 2, 3, 4, 5];
    let atomic = Atomic::new(vec);
    
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(value.len(), 5);
        assert_eq!(value[0], 1);
        assert_eq!(value[4], 5);
    }
}

/// 测试9: 多次 store 操作
#[test]
fn test_multiple_store_operations() {
    let (mut writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic = Atomic::new(0i32);
    
    for i in 1..=10 {
        atomic.store(Box::new(i), &mut writer);
        
        {
            let guard = handle.pin();
            let value = *atomic.load(&guard);
            assert_eq!(value, i);
        }
    }
}

/// 测试11: 读取者快速 pin/unpin
#[test]
fn test_rapid_pin_unpin() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    for _ in 0..1000 {
        let _guard = handle.pin();
        // 立即 drop
    }
}

/// 测试12: 多个读取者快速创建和销毁
#[test]
fn test_rapid_reader_creation_destruction() {
    let (_writer, factory) = new();
    
    for _ in 0..100 {
        let handle = factory.create_handle();
        let _guard = handle.pin();
    }
}

/// 测试13: 读取者在不同线程中的行为
#[test]
fn test_readers_in_different_threads() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(42i32));
    
    let mut handles = vec![];
    
    for thread_id in 0..5 {
        let factory_clone = factory.clone();
        let atomic_clone = atomic.clone();
        
        let handle = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            
            for _ in 0..10 {
                let guard = reader_handle.pin();
                let value = *atomic_clone.load(&guard);
                assert_eq!(value, 42);
            }
            
            thread_id
        });
        
        handles.push(handle);
    }
    
    for handle in handles {
        let thread_id = handle.join().unwrap();
        assert!(thread_id < 5);
    }
}

/// 测试14: 写入者在 drop 前的清理
#[test]
fn test_writer_cleanup_on_drop() {
    {
        let (mut writer, _factory) = new();
        
        for i in 0..50 {
            writer.retire(Box::new(i as i32));
        }
        
        // writer 在这里被 drop
    }
    
    // 如果能到这里，说明 drop 没有问题
}

/// 测试15: 读取者 Handle 在 drop 前的清理
#[test]
fn test_reader_handle_cleanup_on_drop() {
    let (_writer, factory) = new();
    
    {
        let handle = factory.create_handle();
        let _guard = handle.pin();
        // handle 在这里被 drop
    }
    
    // 如果能到这里，说明 drop 没有问题
}

/// 测试16: 交替的纪元推进
#[test]
fn test_alternating_epoch_advancement() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader = factory.create_handle();
    
    for i in 0..5 {
        // 推进纪元
        writer.try_reclaim();
        
        // 写入新值
        atomic.store(Box::new(i), &mut writer);
        
        // 读取
        {
            let guard = reader.pin();
            let value = *atomic.load(&guard);
            assert_eq!(value, i);
        }
    }
}

/// 测试17: 大量读取者的纪元管理
#[test]
fn test_many_readers_epoch_management() {
    let (mut writer, factory) = new();
    
    let mut readers = vec![];
    
    // 创建 10 个读取者
    for _ in 0..10 {
        readers.push(factory.create_handle());
    }
    
    // 让所有读取者 pin
    let _guards: Vec<_> = readers.iter().map(|r| r.pin()).collect();
    
    // 推进纪元
    writer.try_reclaim();
    
    // 验证所有读取者仍然能工作
    for (_i, reader) in readers.iter().enumerate() {
        let guard = reader.pin();
        // 应该能成功 pin
        drop(guard);
    }
}

/// 测试18: 读取者在不同纪元的垃圾保护
#[test]
fn test_garbage_protection_across_epochs() {
    let (mut writer, factory) = new();
    
    let reader1 = factory.create_handle();
    let _guard1 = reader1.pin();
    
    // 推进纪元
    writer.try_reclaim();
    
    let reader2 = factory.create_handle();
    let _guard2 = reader2.pin();
    
    // 推进纪元
    writer.try_reclaim();
    
    let reader3 = factory.create_handle();
    let _guard3 = reader3.pin();
    
    // 退休数据
    for i in 0..100 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于有活跃的读取者，垃圾应该被保留
    assert!(writer.local_garbage.len() > 0);
}

/// 测试19: 动态读取者注册
#[test]
fn test_dynamic_reader_registration() {
    let (mut writer, factory) = new();
    
    // 创建第一批读取者
    let readers1: Vec<_> = (0..5).map(|_| factory.create_handle()).collect();
    
    // 推进纪元
    writer.try_reclaim();
    
    // 创建第二批读取者
    let readers2: Vec<_> = (0..5).map(|_| factory.create_handle()).collect();
    
    // 推进纪元
    writer.try_reclaim();
    
    // 所有读取者都应该能工作
    for reader in readers1.iter().chain(readers2.iter()) {
        let _guard = reader.pin();
    }
}

/// 测试20: 压力测试 - 高频操作
#[test]
fn test_stress_high_frequency_operations() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader_handle = factory.create_handle();
    
    // 执行 1000 次操作
    for i in 0..1000 {
        // 写入
        atomic.store(Box::new(i % 100), &mut writer);
        
        // 读取
        {
            let guard = reader_handle.pin();
            let value = *atomic.load(&guard);
            assert!(value < 100);
        }
        
        // 偶尔触发回收
        if i % 100 == 0 {
            writer.try_reclaim();
        }
    }
}
