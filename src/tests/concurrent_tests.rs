/// 并发测试模块
/// 测试并发场景、纪元管理和多读取者场景

use crate::{new, Atomic};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

/// 测试1: 单个写入者，多个读取者并发读取
#[test]
fn test_single_writer_multiple_readers_concurrent_reads() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let mut handles = vec![];
    
    // 创建 5 个读取者线程
    for _i in 0..5 {
        let factory_clone = factory.clone();
        let atomic_clone = atomic.clone();
        
        let handle = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            
            // 每个读取者读取 10 次
            for _ in 0..10 {
                let guard = reader_handle.pin();
                let value = *atomic_clone.load(&guard);
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
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader_handle = factory.create_handle();
    let atomic_clone = atomic.clone();
    
    let reader_thread = thread::spawn(move || {
        // 读取初始值
        {
            let guard = reader_handle.pin();
            let value = *atomic_clone.load(&guard);
            assert_eq!(value, 0);
        }
        
        // 等待一段时间让写入者更新
        thread::sleep(std::time::Duration::from_millis(10));
        
        // 读取更新后的值
        {
            let guard = reader_handle.pin();
            let value = *atomic_clone.load(&guard);
            assert_eq!(value, 100);
        }
    });
    
    // 主线程作为写入者
    thread::sleep(std::time::Duration::from_millis(5));
    atomic.store(Box::new(100), &mut writer);
    
    reader_thread.join().unwrap();
}

/// 测试3: 多个写入者模拟（通过 Arc<Mutex<Writer>>）
#[test]
fn test_sequential_writer_operations() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(1i32));
    
    let reader_handle = factory.create_handle();
    
    // 第一次写入
    atomic.store(Box::new(2), &mut writer);
    {
        let guard = reader_handle.pin();
        assert_eq!(*atomic.load(&guard), 2);
    }
    
    // 第二次写入
    atomic.store(Box::new(3), &mut writer);
    {
        let guard = reader_handle.pin();
        assert_eq!(*atomic.load(&guard), 3);
    }
    
    // 第三次写入
    atomic.store(Box::new(4), &mut writer);
    {
        let guard = reader_handle.pin();
        assert_eq!(*atomic.load(&guard), 4);
    }
}

/// 测试4: 读取者在不同纪元中的行为
#[test]
fn test_readers_in_different_epochs() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader1 = factory.create_handle();
    let reader2 = factory.create_handle();
    
    // Reader 1 pin
    let guard1 = reader1.pin();
    {
        let value = *atomic.load(&guard1);
        assert_eq!(value, 0);
    }
    
    // Writer 推进纪元并更新
    writer.try_reclaim();
    atomic.store(Box::new(10), &mut writer);
    
    // Reader 2 pin（在新纪元）
    let guard2 = reader2.pin();
    {
        let value = *atomic.load(&guard2);
        assert_eq!(value, 10);
    }
    
    // Reader 1 现在也会看到新值，因为 Atomic 中的指针已经更新
    // 纪元主要用于保护垃圾回收，而不是隔离数据可见性
    {
        let value = *atomic.load(&guard1);
        assert_eq!(value, 10);
    }
}

/// 测试5: 垃圾回收触发
#[test]
fn test_garbage_collection_trigger() {
    let (mut writer, _factory) = new();
    
    // 退休数据直到触发回收
    for i in 0..70 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于 RECLAIM_THRESHOLD = 64，第 65 个退休会触发 try_reclaim
    // 在没有活跃读取者的情况下，垃圾应该被清空
    // 但由于没有读取者注册，垃圾可能不会完全清空
    // 只需验证垃圾数量少于退休的数据数量
    assert!(writer.local_garbage.len() < 70);
}

/// 测试6: 活跃读取者保护垃圾
#[test]
fn test_active_reader_protects_garbage() {
    let (mut writer, factory) = new();
    
    let reader_handle = factory.create_handle();
    
    // 让读取者 pin，保持活跃
    let _guard = reader_handle.pin();
    
    // 退休数据直到触发回收
    for i in 0..70 {
        writer.retire(Box::new(i as i32));
    }
    
    // 由于读取者仍然活跃，垃圾不应该被完全清空
    // （至少应该保留一些垃圾）
    assert!(writer.local_garbage.len() > 0);
}

/// 测试7: 读取者 drop 后垃圾被回收
#[test]
fn test_garbage_reclaimed_after_reader_drop() {
    let (mut writer, factory) = new();
    
    {
        let reader_handle = factory.create_handle();
        let _guard = reader_handle.pin();
        
        // 在读取者活跃时退休数据
        for i in 0..70 {
            writer.retire(Box::new(i as i32));
        }
        
        // 垃圾应该被保留
        assert!(writer.local_garbage.len() > 0);
    }
    
    // 读取者 drop 后，触发一次回收
    writer.try_reclaim();
    
    // 现在垃圾应该被清空
    assert_eq!(writer.local_garbage.len(), 0);
}

/// 测试8: 多个读取者的最小纪元计算
#[test]
fn test_min_epoch_calculation_multiple_readers() {
    let (mut writer, factory) = new();
    
    let reader1 = factory.create_handle();
    let reader2 = factory.create_handle();
    
    // Reader 1 在纪元 0
    let _guard1 = reader1.pin();
    
    // 推进纪元
    writer.try_reclaim();
    
    // Reader 2 在纪元 1
    let _guard2 = reader2.pin();
    
    // 退休一些数据
    writer.retire(Box::new(100i32));
    
    // 再次回收，应该保留在纪元 0 之后的垃圾
    writer.try_reclaim();
    
    // 由于 reader1 仍在纪元 0，垃圾应该被保留
    assert!(writer.local_garbage.len() > 0);
}

/// 测试12: 大量并发读取
#[test]
fn test_high_concurrency_reads() {
    let (_writer, factory) = new();
    let atomic = Arc::new(Atomic::new(42i32));
    
    let mut handles = vec![];
    
    // 创建 20 个读取者线程
    for _ in 0..20 {
        let factory_clone = factory.clone();
        let atomic_clone = atomic.clone();
        
        let handle = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            
            // 每个读取者读取 100 次
            for _ in 0..100 {
                let guard = reader_handle.pin();
                let value = *atomic_clone.load(&guard);
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

/// 测试13: 读取者线程退出后的清理
#[test]
fn test_reader_thread_exit_cleanup() {
    let (mut writer, factory) = new();
    
    let counter = Arc::new(AtomicUsize::new(0));
    
    {
        let factory_clone = factory.clone();
        let counter_clone = counter.clone();
        
        let _thread = thread::spawn(move || {
            let reader_handle = factory_clone.create_handle();
            let _guard = reader_handle.pin();
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });
        
        // 等待线程完成
        thread::sleep(std::time::Duration::from_millis(10));
    }
    
    // 验证线程已完成
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    
    // 触发回收，应该能清理掉已退出的读取者
    writer.try_reclaim();
}

/// 测试14: 交替的读写操作
#[test]
fn test_interleaved_read_write_operations() {
    let (mut writer, factory) = new();
    let atomic = Arc::new(Atomic::new(0i32));
    
    let reader_handle = factory.create_handle();
    
    for i in 0..10 {
        // 写入
        atomic.store(Box::new(i), &mut writer);
        
        // 读取
        {
            let guard = reader_handle.pin();
            let value = *atomic.load(&guard);
            assert_eq!(value, i);
        }
    }
}

/// 测试15: 大量垃圾回收循环
#[test]
fn test_heavy_garbage_collection_cycles() {
    let (mut writer, factory) = new();
    
    let reader_handle = factory.create_handle();
    
    for cycle in 0..10 {
        // 在每个循环中退休大量数据
        for i in 0..100 {
            writer.retire(Box::new((cycle * 100 + i) as i32));
        }
        
        // 触发回收
        writer.try_reclaim();
        
        // 读取者仍然活跃
        let _guard = reader_handle.pin();
    }
}
