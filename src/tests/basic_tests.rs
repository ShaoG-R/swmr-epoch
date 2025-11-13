/// 基础测试模块
/// 测试核心功能的正确性

use crate::{new, Atomic};

/// 测试1: 创建 Writer 和 ReaderFactory
#[test]
fn test_create_writer_and_reader_factory() {
    let (writer, factory) = new();
    
    // 验证 Writer 被成功创建
    assert_eq!(writer.local_garbage.len(), 0);
    
    // 验证 ReaderFactory 被成功创建
    let _handle = factory.create_handle();
    // 如果能创建 handle，说明 factory 正常工作
}

/// 测试2: 创建单个读取者 Handle
#[test]
fn test_create_single_reader_handle() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    // 验证 handle 被成功创建
    let _guard = handle.pin();
    // 如果能 pin，说明 handle 正常工作
}

/// 测试3: 读取者 pin/unpin 循环
#[test]
fn test_reader_pin_unpin_cycle() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    // 第一次 pin
    {
        let _guard = handle.pin();
        // guard 在这里活跃
    }
    // guard 在这里被 drop，标记为不活跃
    
    // 第二次 pin
    {
        let _guard = handle.pin();
        // guard 再次活跃
    }
    // guard 再次被 drop
}

/// 测试4: 创建 Atomic<T> 并读取
#[test]
fn test_atomic_create_and_load() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic = Atomic::new(42i32);
    
    let guard = handle.pin();
    let value = atomic.load(&guard);
    assert_eq!(*value, 42);
}

/// 测试5: Writer 存储新值
#[test]
fn test_writer_store() {
    let (mut writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic = Atomic::new(10i32);
    
    // 读取初始值
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(*value, 10);
    }
    
    // Writer 存储新值
    atomic.store(Box::new(20), &mut writer);
    
    // 读取新值
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(*value, 20);
    }
}

/// 测试6: Writer 回收垃圾
#[test]
fn test_writer_try_reclaim() {
    let (mut writer, _factory) = new();
    
    // 退休一些数据
    writer.retire(Box::new(100i32));
    writer.retire(Box::new(200i32));
    
    // 验证垃圾被添加到本地垃圾桶
    assert_eq!(writer.local_garbage.len(), 2);
    
    // 手动触发回收
    writer.try_reclaim();
    
    // 回收后，垃圾桶应该被清空（因为没有活跃的读取者）
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试7: 多个读取者 Handle
#[test]
fn test_multiple_reader_handles() {
    let (_writer, factory) = new();
    
    let handle1 = factory.create_handle();
    let handle2 = factory.create_handle();
    let handle3 = factory.create_handle();
    
    // 验证三个 handle 都能 pin
    let _guard1 = handle1.pin();
    let _guard2 = handle2.pin();
    let _guard3 = handle3.pin();
}

/// 测试8: 读取者克隆 Factory
#[test]
fn test_reader_factory_clone() {
    let (_writer, factory) = new();
    
    let factory_clone = factory.clone();
    
    let handle1 = factory.create_handle();
    let handle2 = factory_clone.create_handle();
    
    // 两个 handle 都应该能正常工作
    let _guard1 = handle1.pin();
    let _guard2 = handle2.pin();
}


/// 测试12: 字符串类型的 Atomic
#[test]
fn test_atomic_with_string() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic = Atomic::new(String::from("hello"));
    
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(value, "hello");
    }
}

/// 测试13: 结构体类型的 Atomic
#[test]
fn test_atomic_with_struct() {
    #[derive(Debug, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
    }
    
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic = Atomic::new(Point { x: 10, y: 20 });
    
    {
        let guard = handle.pin();
        let value = atomic.load(&guard);
        assert_eq!(value.x, 10);
        assert_eq!(value.y, 20);
    }
}

/// 测试14: Atomic Drop
#[test]
fn test_atomic_drop() {
    let atomic = Atomic::new(42i32);
    drop(atomic);
    // 如果能成功 drop，说明内存管理正确
}

/// 测试15: 多个 Atomic 实例
#[test]
fn test_multiple_atomic_instances() {
    let (_writer, factory) = new();
    let handle = factory.create_handle();
    
    let atomic1 = Atomic::new(10i32);
    let atomic2 = Atomic::new(20i32);
    let atomic3 = Atomic::new(30i32);
    
    {
        let guard = handle.pin();
        assert_eq!(*atomic1.load(&guard), 10);
        assert_eq!(*atomic2.load(&guard), 20);
        assert_eq!(*atomic3.load(&guard), 30);
    }
}
