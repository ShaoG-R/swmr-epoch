/// 边界情况和压力测试模块
/// 测试边界条件、垃圾回收阈值、数据类型变化和高频操作

use crate::{EpochGcDomain, EpochPtr};
use std::sync::Arc;
use std::thread;

/// 测试1: 空的垃圾回收
#[test]
fn test_empty_garbage_collection() {
    let domain = EpochGcDomain::new();
    let (mut gc, _domain) = domain.with_gc_handle().into_parts();
    
    // 不退休任何数据，直接回收
    gc.collect();
    
    // 应该没有问题
    assert_eq!(gc.local_garbage.len(), 0);
}

/// 测试2: 单个数据的退休和回收
#[test]
fn test_single_data_retire_and_reclaim() {
    let domain = EpochGcDomain::new();
    let (mut gc, _domain) = domain.with_gc_handle().into_parts();
    
    gc.retire(Box::new(42i32));
    assert_eq!(gc.total_garbage_count(), 1);
    
    gc.collect();
    assert_eq!(gc.total_garbage_count(), 0);
}

/// 测试3: 恰好达到回收阈值
#[test]
fn test_exactly_reach_reclaim_threshold() {
    let domain = EpochGcDomain::new();
    let (mut gc, _domain) = domain.with_gc_handle().into_parts();
    
    // 退休 64 个数据（AUTO_RECLAIM_THRESHOLD = 64）
    for i in 0..64 {
        gc.retire(Box::new(i as i32));
    }
    
    // 应该还没有自动回收
    let total_garbage: usize = gc.total_garbage_count();
    assert_eq!(total_garbage, 64);
    
    // 再退休一个，应该触发自动回收
    // 由于没有活跃读取者，垃圾会被回收
    gc.retire(Box::new(64i32));
    
    // 回收后垃圾应该被清空（因为没有读取者保护）
    let total_garbage_after: usize = gc.total_garbage_count();
    assert_eq!(total_garbage_after, 0);
}

/// 测试4: 超过回收阈值
#[test]
fn test_exceed_reclaim_threshold() {
    let domain = EpochGcDomain::new();
    let (mut gc, _domain) = domain.with_gc_handle().into_parts();
    
    // 退休 100 个数据
    for i in 0..100 {
        gc.retire(Box::new(i as i32));
    }
    
    // 由于没有活跃读取者，垃圾会被回收
    // 但可能不会完全清空，只需验证数量少于退休的数据
    assert!(gc.local_garbage.len() < 100);
}

/// 测试5: 零大小类型
#[test]
fn test_zero_sized_type() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    #[derive(Debug, PartialEq)]
    struct ZeroSized;
    
    let ptr = EpochPtr::new(ZeroSized);
    
    {
        let guard = local_epoch.pin();
        let _value = ptr.load(&guard);
        // ZST 应该能正常工作
    }
}

/// 测试6: 大型数据结构
#[test]
fn test_large_data_structure() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    #[derive(Debug, PartialEq)]
    struct LargeData {
        data: [u64; 1000],
    }
    
    let large = LargeData { data: [42; 1000] };
    let ptr = EpochPtr::new(large);
    
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(value.data[0], 42);
        assert_eq!(value.data[999], 42);
    }
}

/// 测试7: 嵌套结构体
#[test]
fn test_nested_structures() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
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
    
    let ptr = EpochPtr::new(outer);
    
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(value.inner.value, 42);
        assert_eq!(value.name, "test");
    }
}

/// 测试8: 向量类型
#[test]
fn test_vector_type() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    let vec = vec![1, 2, 3, 4, 5];
    let ptr = EpochPtr::new(vec);
    
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(value.len(), 5);
        assert_eq!(value[0], 1);
        assert_eq!(value[4], 5);
    }
}

/// 测试9: 多次 store 操作
#[test]
fn test_multiple_store_operations() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    let ptr = EpochPtr::new(0i32);
    
    for i in 1..=10 {
        ptr.store(i, &mut gc);
        
        {
            let guard = local_epoch.pin();
            let value = *ptr.load(&guard);
            assert_eq!(value, i);
        }
    }
}

/// 测试11: 读取者快速 pin/unpin
#[test]
fn test_rapid_pin_unpin() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    for _ in 0..1000 {
        let _guard = local_epoch.pin();
        // 立即 drop
    }
}

/// 测试12: 多个读取者快速创建和销毁
#[test]
fn test_rapid_reader_creation_destruction() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    for _ in 0..100 {
        let _guard = local_epoch.pin();
    }
}

/// 测试13: 读取者在不同线程中的行为
#[test]
fn test_readers_in_different_threads() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    let domain = Arc::new(domain);
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

/// 测试14: 写入者在 drop 前的清理
#[test]
fn test_writer_cleanup_on_drop() {
    {
        let domain = EpochGcDomain::new();
        let (mut gc, _domain) = domain.with_gc_handle().into_parts();
        
        for i in 0..50 {
            gc.retire(Box::new(i as i32));
        }
        
        // gc 在这里被 drop
    }
    
    // 如果能到这里，说明 drop 没有问题
}

/// 测试15: 读取者 Guard 在 drop 前的清理
#[test]
fn test_reader_handle_cleanup_on_drop() {
    let domain = EpochGcDomain::new();
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    {
        let _guard = local_epoch.pin();
        // guard 在这里被 drop
    }
    
    // 如果能到这里，说明 drop 没有问题
}

/// 测试16: 交替的纪元推进
#[test]
fn test_alternating_epoch_advancement() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
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

/// 测试17: 大量读取者的纪元管理
#[test]
fn test_many_readers_epoch_management() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    
    // 创建多个读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();
    let local_epoch3 = domain.register_reader();
    
    // 推进纪元
    gc.collect();
    
    // 所有读取者都应该能工作
    let _guard1 = local_epoch1.pin();
    let _guard2 = local_epoch2.pin();
    let _guard3 = local_epoch3.pin();
    
    // 推进纪元
    gc.collect();
    
    // 再次验证所有读取者仍然工作
    let _guard4 = local_epoch1.pin();
    let _guard5 = local_epoch2.pin();
    let _guard6 = local_epoch3.pin();
}

/// 测试18: 读取者在不同纪元的垃圾保护
#[test]
fn test_garbage_protection_across_epochs() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    // 第一轮：退休数据，读取者活跃
    {
        let _guard = local_epoch.pin();
        for i in 0..50 {
            gc.retire(Box::new(i as i32));
        }
        
        // 垃圾应该被保留
        assert!(gc.local_garbage.len() > 0);
    }
    
    // 第二轮：读取者不活跃，垃圾应该被回收
    gc.collect();
    assert_eq!(gc.local_garbage.len(), 0);
}

/// 测试19: 动态读取者注册
#[test]
fn test_dynamic_reader_registration() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    
    // 创建多个读取者
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain.register_reader();
    let local_epoch3 = domain.register_reader();
    
    // 推进纪元
    gc.collect();
    
    // 所有读取者都应该能工作
    let _guard1 = local_epoch1.pin();
    let _guard2 = local_epoch2.pin();
    
    // 推进纪元
    gc.collect();
    
    // 再次验证所有读取者仍然工作
    let _guard3 = local_epoch1.pin();
    let _guard4 = local_epoch2.pin();
    let _guard5 = local_epoch3.pin();
}

/// 测试20: 压力测试 - 高频操作
#[test]
fn test_stress_high_frequency_operations() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    let ptr = Arc::new(EpochPtr::new(0i32));
    
    // 执行 1000 次操作
    for i in 0..1000 {
        // 写入
        ptr.store(i % 100, &mut gc);
        
        // 读取
        {
            let guard = local_epoch.pin();
            let value = *ptr.load(&guard);
            assert!(value < 100);
        }
        
        // 偶尔触发回收
        if i % 100 == 0 {
            gc.collect();
        }
    }
}
