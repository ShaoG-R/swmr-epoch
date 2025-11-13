/// 基础测试模块
/// 测试核心功能的正确性

use crate::{new, Atomic};

/// 测试1: 创建 Writer 和 ReaderRegistry
#[test]
fn test_create_writer_and_registry() {
    let (writer, registry) = new();
    
    // 验证 Writer 被成功创建
    assert_eq!(writer.local_garbage.len(), 0);
    
    // 验证 ReaderRegistry 被成功创建
    let _guard = registry.pin();
    // 如果能 pin，说明 registry 正常工作
}

/// 测试2: 创建单个读取者
#[test]
fn test_create_single_reader() {
    let (_writer, registry) = new();
    
    // 验证可以成功 pin
    let _guard = registry.pin();
    // 如果能 pin，说明 registry 正常工作
}

/// 测试3: 读取者 pin/unpin 循环
#[test]
fn test_reader_pin_unpin_cycle() {
    let (_writer, registry) = new();
    
    // 第一次 pin
    {
        let _guard = registry.pin();
        // guard 在这里活跃
    }
    // guard 在这里被 drop，标记为不活跃
    
    // 第二次 pin
    {
        let _guard = registry.pin();
        // guard 再次活跃
    }
    // guard 再次被 drop
}

/// 测试4: 创建 Atomic<T> 并读取
#[test]
fn test_atomic_create_and_load() {
    let (_writer, registry) = new();
    
    let atomic = Atomic::new(42i32);
    
    let guard = registry.pin();
    let value = atomic.load(&guard);
    assert_eq!(*value, 42);
}

/// 测试5: Writer 存储新值
#[test]
fn test_writer_store() {
    let (mut writer, registry) = new();
    
    let atomic = Atomic::new(10i32);
    
    // 读取初始值
    {
        let guard = registry.pin();
        let value = atomic.load(&guard);
        assert_eq!(*value, 10);
    }
    
    // Writer 存储新值
    atomic.store(Box::new(20), &mut writer);
    
    // 读取新值
    {
        let guard = registry.pin();
        let value = atomic.load(&guard);
        assert_eq!(*value, 20);
    }
}

/// 测试6: Writer 回收垃圾
#[test]
fn test_writer_try_reclaim() {
    let (mut writer, _registry) = new();
    
    // 退休一些数据
    writer.retire(Box::new(100i32));
    writer.retire(Box::new(200i32));
    
    // 验证垃圾被添加到本地垃圾桶
    // With BTreeMap, we check the total garbage count across all epochs
    // 使用 BTreeMap，我们检查所有 epoch 中的垃圾总数
    let total_garbage: usize = writer.local_garbage_count;
    assert_eq!(total_garbage, 2);
    
    // 手动触发回收
    writer.try_reclaim();
    
    // 回收后，垃圾桶应该被清空（因为没有活跃的读取者）
    let total_garbage_after: usize = writer.local_garbage_count;
    assert_eq!(total_garbage_after, 0);
}

/// 测试7: 多个读取者
#[test]
fn test_multiple_readers() {
    let (_writer, registry) = new();
    
    // 验证可以创建多个 guard
    let _guard1 = registry.pin();
    let _guard2 = registry.pin();
    let _guard3 = registry.pin();
}

/// 测试8: 克隆 ReaderRegistry
#[test]
fn test_registry_clone() {
    let (_writer, registry) = new();
    
    let registry_clone = registry.clone();
    
    // 两个 registry 都应该能正常工作
    let _guard1 = registry.pin();
    let _guard2 = registry_clone.pin();
}

/// 测试9: 字符串类型的 Atomic
#[test]
fn test_atomic_with_string() {
    let (_writer, registry) = new();
    
    let atomic = Atomic::new(String::from("hello"));
    
    {
        let guard = registry.pin();
        let value = atomic.load(&guard);
        assert_eq!(value, "hello");
    }
}

/// 测试10: 结构体类型的 Atomic
#[test]
fn test_atomic_with_struct() {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
    }
    
    let (_writer, registry) = new();
    
    let atomic = Atomic::new(Point { x: 10, y: 20 });
    
    {
        let guard = registry.pin();
        let value = atomic.load(&guard);
        assert_eq!(value.x, 10);
        assert_eq!(value.y, 20);
    }
}

/// 测试11: Atomic Drop
#[test]
fn test_atomic_drop() {
    let atomic = Atomic::new(42i32);
    drop(atomic);
    // 如果能成功 drop，说明内存管理正确
}

/// 测试12: 多个 Atomic 实例
#[test]
fn test_multiple_atomic_instances() {
    let (_writer, registry) = new();
    
    let atomic1 = Atomic::new(10i32);
    let atomic2 = Atomic::new(20i32);
    let atomic3 = Atomic::new(30i32);
    
    {
        let guard = registry.pin();
        assert_eq!(*atomic1.load(&guard), 10);
        assert_eq!(*atomic2.load(&guard), 20);
        assert_eq!(*atomic3.load(&guard), 30);
    }
}

/// 测试13: Guard 的克隆
#[test]
fn test_guard_clone() {
    let (_writer, registry) = new();
    
    let guard1 = registry.pin();
    let guard2 = guard1.clone();
    
    // 两个 guard 都应该正常工作
    let atomic = Atomic::new(100i32);
    assert_eq!(*atomic.load(&guard1), 100);
    assert_eq!(*atomic.load(&guard2), 100);
}

/// 测试14: 多线程安全
#[test]
fn test_thread_safety() {
    use std::sync::Arc;
    use std::thread;
    
    let (mut writer, registry) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    // 创建多个线程同时读取和写入
    let mut handles = vec![];
    
    // 启动 5 个读取线程
    for _ in 0..5 {
        let atomic_clone = atomic.clone();
        let registry_clone = registry.clone();
        
        handles.push(thread::spawn(move || {
            let guard = registry_clone.pin();
            let value = atomic_clone.load(&guard);
            *value // 返回读取的值
        }));
    }
    
    // 主线程更新值
    atomic.store(Box::new(42), &mut writer);
    
    // 等待所有线程完成
    let results: Vec<i32> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    
    // 验证结果
    assert_eq!(results.len(), 5);
    for &result in &results {
        assert!(result == 0 || result == 42);
    }
}
