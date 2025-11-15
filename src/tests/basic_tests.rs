/// 基础测试模块
/// 测试核心功能的正确性

use crate::{EpochGcDomain, EpochPtr};

/// 测试1: 创建 GcHandle 和 LocalEpoch
#[test]
fn test_create_gc_handle_and_local_epoch() {
    let domain = EpochGcDomain::new();
    
    // 验证 GcHandle 被成功创建
    let (_gc, domain) = domain.with_gc_handle().into_parts();
    
    // 验证 LocalEpoch 被成功创建
    let local_epoch = domain.register_reader();
    let _guard = local_epoch.pin();
    // 如果能 pin，说明 domain 正常工作
}

/// 测试2: 创建单个读取者
#[test]
fn test_create_single_reader() {
    let domain = EpochGcDomain::new();
    
    // 验证可以成功注册和 pin
    let local_epoch = domain.register_reader();
    let _guard = local_epoch.pin();
    // 如果能 pin，说明 domain 正常工作
}

/// 测试3: 读取者 pin/unpin 循环
#[test]
fn test_reader_pin_unpin_cycle() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    // 第一次 pin
    {
        let _guard = local_epoch.pin();
        // guard 在这里活跃
    }
    // guard 在这里被 drop，标记为不活跃
    
    // 第二次 pin
    {
        let _guard = local_epoch.pin();
        // guard 再次活跃
    }
    // guard 再次被 drop
}

/// 测试4: 创建 EpochPtr<T> 并读取
#[test]
fn test_epoch_ptr_create_and_load() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    let ptr = EpochPtr::new(42i32);
    
    let guard = local_epoch.pin();
    let value = ptr.load(&guard);
    assert_eq!(*value, 42);
}

/// 测试5: Writer 存储新值
#[test]
fn test_writer_store() {
    let domain = EpochGcDomain::new();
    let (mut gc, domain) = domain.with_gc_handle().into_parts();
    let local_epoch = domain.register_reader();
    
    let ptr = EpochPtr::new(10i32);
    
    // 读取初始值
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 10);
    }
    
    // Writer 存储新值
    ptr.store(20, &mut gc);
    
    // 读取新值
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(*value, 20);
    }
}

/// 测试6: Writer 回收垃圾
#[test]
fn test_writer_collect() {
    let domain = EpochGcDomain::new();
    let (mut gc, _domain) = domain.with_gc_handle().into_parts();
    
    // 退休一些数据
    gc.retire(Box::new(100i32));
    gc.retire(Box::new(200i32));
    
    // 验证垃圾被添加到本地垃圾桶
    let total_garbage: usize = gc.total_garbage_count();
    assert_eq!(total_garbage, 2);
    
    // 手动触发回收
    gc.collect();
    
    // 回收后，垃圾桶应该被清空（因为没有活跃的读取者）
    let total_garbage_after: usize = gc.total_garbage_count();
    assert_eq!(total_garbage_after, 0);
}

/// 测试7: 嵌套 pin（可重入）
#[test]
fn test_nested_pins() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    // 验证可以创建多个嵌套 guard
    let guard1 = local_epoch.pin();
    let guard2 = guard1.clone();
    let guard3 = guard2.clone();
    
    // 所有 guard 都应该正常工作
    drop(guard3);
    drop(guard2);
    drop(guard1);
}

/// 测试8: 克隆 EpochGcDomain
#[test]
fn test_domain_clone() {
    let domain = EpochGcDomain::new();
    
    let domain_clone = domain.clone();
    
    // 两个 domain 都应该能正常工作
    let local_epoch1 = domain.register_reader();
    let local_epoch2 = domain_clone.register_reader();
    let _guard1 = local_epoch1.pin();
    let _guard2 = local_epoch2.pin();
}

/// 测试9: 字符串类型的 EpochPtr
#[test]
fn test_epoch_ptr_with_string() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    let ptr = EpochPtr::new(String::from("hello"));
    
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(value, "hello");
    }
}

/// 测试10: 结构体类型的 EpochPtr
#[test]
fn test_epoch_ptr_with_struct() {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
    }
    
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    let ptr = EpochPtr::new(Point { x: 10, y: 20 });
    
    {
        let guard = local_epoch.pin();
        let value = ptr.load(&guard);
        assert_eq!(value.x, 10);
        assert_eq!(value.y, 20);
    }
}

/// 测试11: EpochPtr Drop
#[test]
fn test_epoch_ptr_drop() {
    let ptr = EpochPtr::new(42i32);
    drop(ptr);
    // 如果能成功 drop，说明内存管理正确
}

/// 测试12: 多个 EpochPtr 实例
#[test]
fn test_multiple_epoch_ptr_instances() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    let ptr1 = EpochPtr::new(10i32);
    let ptr2 = EpochPtr::new(20i32);
    let ptr3 = EpochPtr::new(30i32);
    
    {
        let guard = local_epoch.pin();
        assert_eq!(*ptr1.load(&guard), 10);
        assert_eq!(*ptr2.load(&guard), 20);
        assert_eq!(*ptr3.load(&guard), 30);
    }
}

/// 测试13: PinGuard 的克隆
#[test]
fn test_pin_guard_clone() {
    let domain = EpochGcDomain::new();
    let local_epoch = domain.register_reader();
    
    let guard1 = local_epoch.pin();
    let guard2 = guard1.clone();
    
    // 两个 guard 都应该正常工作
    let ptr = EpochPtr::new(100i32);
    assert_eq!(*ptr.load(&guard1), 100);
    assert_eq!(*ptr.load(&guard2), 100);
}

/// 测试14: 多线程安全
#[test]
fn test_thread_safety() {
    use std::sync::Arc;
    use std::thread;
    
    let domain = Arc::new(EpochGcDomain::new());
    let ptr = Arc::new(EpochPtr::new(0i32));
    
    // 创建多个线程同时读取
    let mut handles = vec![];
    
    // 启动 5 个读取线程
    for _ in 0..5 {
        let ptr_clone = ptr.clone();
        let domain_clone = domain.clone();
        
        handles.push(thread::spawn(move || {
            let local_epoch = domain_clone.register_reader();
            let guard = local_epoch.pin();
            let value = ptr_clone.load(&guard);
            *value // 返回读取的值
        }));
    }
    
    // 等待所有线程完成
    let results: Vec<i32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    
    // 验证结果
    assert_eq!(results.len(), 5);
    for &result in &results {
        assert_eq!(result, 0);
    }
}
