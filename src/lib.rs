//! # Epoch-Based Garbage Collection
//!
//! This module provides a minimal-locking, single-writer, multi-reader garbage collection system
//! based on epoch-based reclamation. It is designed for scenarios where:
//!
//! - One thread (the writer) owns and updates shared data structures.
//! - Multiple reader threads concurrently access the same data.
//! - Readers need to safely access data without blocking the writer.
//!
//! ## Core Concepts
//!
//! **Epoch**: A logical timestamp that advances monotonically. The writer increments the epoch
//! during garbage collection cycles. Readers "pin" themselves to an epoch, declaring that they
//! are actively reading data from that epoch.
//!
//! **Pin**: When a reader calls `pin()`, it records the current epoch in its slot. This tells
//! the writer: "I am reading data from this epoch; do not reclaim it yet."
//!
//! **Reclamation**: The writer collects retired objects and reclaims those from epochs that
//! are older than the minimum epoch of all active readers.
//!
//! ## Typical Usage
//!
//! ```
//! use swmr_epoch::{EpochGcDomain, EpochPtr};
//!
//! // 1. Create a shared GC domain and get the garbage collector
//! let (mut gc, domain) = EpochGcDomain::new();
//!
//! // 2. Create an epoch-protected pointer
//! let shared_ptr = EpochPtr::new(42i32);
//!
//! // 3. In each reader thread, register a local epoch
//! let local_epoch = domain.register_reader();
//!
//! // 4. Readers pin themselves before accessing shared data
//! let guard = local_epoch.pin();
//! let value = shared_ptr.load(&guard);
//! // ... use value ...
//! // guard is automatically dropped, unpinning the thread
//!
//! // 5. Writer updates shared data and drives garbage collection
//! shared_ptr.store(100i32, &mut gc);
//! gc.collect();  // Reclaim garbage from old epochs
//! ```

#[cfg(loom)]
use loom::cell::Cell;
#[cfg(not(loom))]
use std::cell::Cell;

#[cfg(loom)]
use loom::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
#[cfg(not(loom))]
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};

#[cfg(loom)]
use loom::sync::{Arc, Mutex};
#[cfg(not(loom))]
use std::sync::{Arc, Mutex};

use std::collections::VecDeque;

const AUTO_RECLAIM_THRESHOLD: usize = 64;
const DEFAULT_CLEANUP_INTERVAL: usize = 16;
const INACTIVE_EPOCH: usize = usize::MAX;

type RetiredNode = RetiredObject;

struct RetiredObject {
    ptr: *mut (),
    dtor: unsafe fn(*mut ()),
}

#[inline(always)]
unsafe fn drop_value<T>(ptr: *mut ()) {
    let ptr = ptr as *mut T;
    unsafe {
        drop(Box::from_raw(ptr));
    }
}

impl RetiredObject {
    #[inline(always)]
    fn new<T: 'static>(value: Box<T>) -> Self {
        let ptr = Box::into_raw(value) as *mut ();
        RetiredObject {
            ptr,
            dtor: drop_value::<T>,
        }
    }
}

impl Drop for RetiredObject {
    #[inline(always)]
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                (self.dtor)(self.ptr);
            }
            self.ptr = std::ptr::null_mut();
        }
    }
}

#[derive(Debug)]
#[repr(align(64))]
struct ReaderSlot {
    active_epoch: AtomicUsize,
}

#[derive(Debug)]
#[repr(align(64))]
struct SharedState {
    global_epoch: AtomicUsize,
    readers: Mutex<Vec<Arc<ReaderSlot>>>,
}

/// Manages retired objects and their reclamation.
///
/// This struct encapsulates the logic for:
/// - Storing retired objects in epoch-ordered bags.
/// - Managing a pool of vectors to reduce allocation overhead.
/// - Reclaiming objects when they are safe to delete.
///
/// 管理已退休对象及其回收。
///
/// 此结构体封装了以下逻辑：
/// - 将已退休对象存储在按纪元排序的袋子中。
/// - 管理向量池以减少分配开销。
/// - 当对象可以安全删除时进行回收。
struct GarbageSet {
    /// Queue of garbage bags, ordered by epoch.
    /// Each element is (epoch, bag_of_nodes).
    queue: VecDeque<(usize, Vec<RetiredNode>)>,
    /// Pool of empty vectors to reduce allocation.
    pool: Vec<Vec<RetiredNode>>,
    /// Total number of retired nodes in the queue.
    count: usize,
}

impl GarbageSet {
    /// Create a new empty garbage set.
    /// 创建一个新的空垃圾集合。
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            pool: Vec::new(),
            count: 0,
        }
    }

    /// Get the total number of retired objects.
    /// 获取已退休对象的总数。
    #[inline]
    fn len(&self) -> usize {
        self.count
    }

    /// Add a retired node to the set for the current epoch.
    ///
    /// If the last bag belongs to the current epoch, the node is appended to it.
    /// Otherwise, a new bag is created (possibly reused from the pool).
    ///
    /// 将已退休节点添加到当前纪元的集合中。
    ///
    /// 如果最后一个袋子属于当前纪元，则将节点追加到其中。
    /// 否则，创建一个新袋子（可能从池中复用）。
    fn add(&mut self, node: RetiredNode, current_epoch: usize) {
        // Check if we can append to the last bag
        let append_to_last = if let Some((last_epoch, _)) = self.queue.back() {
            *last_epoch == current_epoch
        } else {
            false
        };

        if append_to_last {
            // Safe to unwrap because we checked back() above
            self.queue
                .back_mut()
                .unwrap()
                .1
                .push(node);
        } else {
            // Reuse a vector from the pool if available, or create a new one
            let mut bag = self.pool.pop().unwrap_or_else(|| Vec::with_capacity(16));
            bag.push(node);
            self.queue.push_back((current_epoch, bag));
        }

        self.count += 1;
    }

    /// Reclaim garbage that is safe to delete.
    ///
    /// Garbage from epochs older than `min_active_epoch` (or `min_active_epoch - 1` depending on logic)
    /// is cleared and the vectors are returned to the pool.
    ///
    /// 回收可以安全删除的垃圾。
    ///
    /// 来自比 `min_active_epoch`（或 `min_active_epoch - 1`，取决于逻辑）更旧的纪元的垃圾
    /// 被清除，向量被归还到池中。
    fn collect(&mut self, min_active_epoch: usize, current_epoch: usize) {
        // Helper closure to recycle a bag
        fn recycle_bag(mut bag: Vec<RetiredNode>, pool: &mut Vec<Vec<RetiredNode>>) {
            bag.clear(); // Drops all retired objects inside
            pool.push(bag);
        }

        if min_active_epoch == current_epoch {
            // Reclaim everything
            for (_, bag) in self.queue.drain(..) {
                recycle_bag(bag, &mut self.pool);
            }
        } else if min_active_epoch > 0 {
            let safe_to_reclaim_epoch = min_active_epoch - 1;
            while let Some((epoch, _)) = self.queue.front() {
                if *epoch > safe_to_reclaim_epoch {
                    break;
                }
                // Pop and recycle
                if let Some((_, bag)) = self.queue.pop_front() {
                    recycle_bag(bag, &mut self.pool);
                }
            }
        }

        self.count = self.queue.iter()
            .map(|(_, bag)| bag.len())
            .sum();
    }
}

/// The unique garbage collector handle for an epoch GC domain.
///
/// There should be exactly one `GcHandle` per `EpochGcDomain`, owned by the writer thread.
/// It is responsible for:
/// - Advancing the global epoch during collection cycles.
/// - Receiving retired objects from `EpochPtr::store()`.
/// - Scanning active readers and reclaiming garbage from old epochs.
///
/// **Thread Safety**: `GcHandle` is not thread-safe and must be owned by a single thread.
///
/// 一个 epoch GC 域的唯一垃圾回收器句柄。
/// 每个 `EpochGcDomain` 应该恰好有一个 `GcHandle`，由写入者线程持有。
/// 它负责：
/// - 在回收周期中推进全局纪元。
/// - 从 `EpochPtr::store()` 接收已退休对象。
/// - 扫描活跃读者并回收旧纪元的垃圾。
/// **线程安全性**：`GcHandle` 不是线程安全的，必须由单个线程持有。
pub struct GcHandle {
    shared: Arc<SharedState>,
    garbage: GarbageSet,
    auto_reclaim_threshold: Option<usize>,
    collection_counter: usize,
    cleanup_interval: usize,
}

impl GcHandle {
    #[inline]
    fn total_garbage_count(&self) -> usize {
        self.garbage.len()
    }

    /// Retire (defer deletion) of a value.
    ///
    /// The value is stored in a garbage bin associated with the current epoch.
    /// It will be reclaimed once the epoch becomes older than all active readers' epochs.
    ///
    /// This is an internal method used by `EpochPtr::store()`.
    ///
    /// **Automatic Reclamation**: If automatic reclamation is enabled (via `new_with_threshold()`),
    /// and the total garbage count exceeds the configured threshold after this call,
    /// `collect()` is automatically invoked. The default threshold is `AUTO_RECLAIM_THRESHOLD` (64).
    /// To disable automatic reclamation, pass `None` to `new_with_threshold()`.
    ///
    /// 退休（延迟删除）一个值。
    ///
    /// 该值被存储在与当前纪元关联的垃圾桶中。
    /// 一旦该纪元比所有活跃读者的纪元都更旧，它就会被回收。
    ///
    /// 这是 `EpochPtr::store()` 使用的内部方法。
    ///
    /// **自动回收**：如果启用了自动回收（通过 `new_with_threshold()`），
    /// 且在此调用后总垃圾计数超过配置的阈值，`collect()` 会被自动调用。
    /// 默认阈值是 `AUTO_RECLAIM_THRESHOLD`（64）。
    /// 要禁用自动回收，请向 `new_with_threshold()` 传递 `None`。
    #[inline]
    pub(crate) fn retire<T: 'static>(&mut self, data: Box<T>) {
        let current_epoch = self.shared.global_epoch.load(Ordering::Relaxed);

        self.garbage.add(RetiredObject::new(data), current_epoch);

        if let Some(threshold) = self.auto_reclaim_threshold {
            if self.total_garbage_count() > threshold {
                self.collect();
            }
        }
    }

    /// Perform a garbage collection cycle.
    ///
    /// This method:
    /// 1. Advances the global epoch.
    /// 2. Scans all active readers to find the minimum active epoch.
    /// 3. Reclaims garbage from epochs older than the minimum active epoch.
    ///
    /// **Garbage Reclamation Logic**:
    /// - If there are no active readers (min_active_epoch == new_epoch), all garbage is eligible for reclamation.
    /// - Otherwise, garbage from epochs older than `min_active_epoch - 1` is reclaimed.
    /// - This ensures that readers pinned to the minimum epoch can still safely access data from that epoch.
    ///
    /// Can be called periodically or after significant updates.
    /// Safe to call even if there is no garbage to reclaim.
    ///
    /// 执行一个垃圾回收周期。
    /// 此方法：
    /// 1. 推进全局纪元。
    /// 2. 扫描所有活跃读者以找到最小活跃纪元。
    /// 3. 回收比最小活跃纪元更旧的纪元中的垃圾。
    ///
    /// **垃圾回收逻辑**：
    /// - 如果没有活跃读者（min_active_epoch == new_epoch），所有垃圾都可以被回收。
    /// - 否则，回收来自比 `min_active_epoch - 1` 更旧的纪元中的垃圾。
    /// - 这确保了被钉住到最小纪元的读者仍然可以安全地访问该纪元的数据。
    ///
    /// 可以定期调用或在重大更新后调用。
    /// 即使没有垃圾要回收也可以安全调用。
    pub fn collect(&mut self) {
        let new_epoch = self.shared.global_epoch.fetch_add(1, Ordering::SeqCst) + 1;

        let mut min_active_epoch = new_epoch;
        self.collection_counter += 1;
        
        let should_cleanup = self.cleanup_interval > 0 && self.collection_counter % self.cleanup_interval == 0;

        let mut shared_readers = self.shared.readers.lock()
            .expect("Failed to acquire readers lock in collect: mutex poisoned");
        
        let mut dead_count = 0;
        
        for arc_slot in shared_readers.iter() {
            let epoch = arc_slot.active_epoch.load(Ordering::Acquire);
            if epoch != INACTIVE_EPOCH {
                min_active_epoch = min_active_epoch.min(epoch);
            } else if should_cleanup && Arc::strong_count(arc_slot) == 1 {
                // Only this Vec holds a reference, the LocalEpoch was dropped
                dead_count += 1;
            }
        }

        if should_cleanup && dead_count > 0 {
            // Keep only slots that have external references (strong_count > 1)
            shared_readers.retain(|arc_slot| Arc::strong_count(arc_slot) > 1);
        }
        
        drop(shared_readers);

        self.garbage.collect(min_active_epoch, new_epoch);
    }
}

/// A reader thread's local epoch state.
///
/// Each reader thread should create exactly one `LocalEpoch` via `EpochGcDomain::register_reader()`.
/// It is `!Sync` (due to `Cell`) and must be stored per-thread.
///
/// The `LocalEpoch` is used to:
/// - Pin the thread to the current epoch via `pin()`.
/// - Obtain a `PinGuard` that protects access to `EpochPtr` values.
///
/// **Thread Safety**: `LocalEpoch` is not `Sync` and must be used by only one thread.
///
/// 读者线程的本地纪元状态。
/// 每个读者线程应该通过 `EpochGcDomain::register_reader()` 创建恰好一个 `LocalEpoch`。
/// 它是 `!Sync` 的（因为 `Cell`），必须在每个线程中存储。
/// `LocalEpoch` 用于：
/// - 通过 `pin()` 将线程钉住到当前纪元。
/// - 获取保护对 `EpochPtr` 值的访问的 `PinGuard`。
/// **线程安全性**：`LocalEpoch` 不是 `Sync` 的，必须仅由一个线程使用。
pub struct LocalEpoch {
    slot: Arc<ReaderSlot>,
    shared: Arc<SharedState>,
    pin_count: Cell<usize>,
}

impl LocalEpoch {
    /// Pin this thread to the current epoch.
    ///
    /// Returns a `PinGuard` that keeps the thread pinned for its lifetime.
    ///
    /// **Reentrancy**: This method is reentrant. Multiple calls can be nested, and the thread
    /// remains pinned until all returned guards are dropped. You can also clone a guard to create
    /// additional references: `let guard2 = guard1.clone();`
    ///
    /// **Example**:
    /// ```ignore
    /// let guard1 = local_epoch.pin();
    /// let guard2 = local_epoch.pin();  // Reentrant call
    /// let guard3 = guard1.clone();     // Clone for nested scope
    /// // Thread remains pinned until all three guards are dropped
    /// ```
    ///
    /// While pinned, the thread is considered "active" at a particular epoch,
    /// and the garbage collector will not reclaim data from that epoch.
    ///
    /// 将此线程钉住到当前纪元。
    ///
    /// 返回一个 `PinGuard`，在其生命周期内保持线程被钉住。
    ///
    /// **可重入性**：此方法是可重入的。多个调用可以嵌套，线程在所有返回的守卫被 drop 之前保持被钉住。
    /// 你也可以克隆一个守卫来创建额外的引用：`let guard2 = guard1.clone();`
    ///
    /// **示例**：
    /// ```ignore
    /// let guard1 = local_epoch.pin();
    /// let guard2 = local_epoch.pin();  // 可重入调用
    /// let guard3 = guard1.clone();     // 克隆用于嵌套作用域
    /// // 线程保持被钉住直到所有三个守卫被 drop
    /// ```
    ///
    /// 当被钉住时，线程被认为在特定纪元"活跃"，垃圾回收器不会回收该纪元的数据。
    #[inline]
    pub fn pin(&self) -> PinGuard<'_> {
        let pin_count = self.pin_count.get();

        if pin_count == 0 {
            let current_epoch = self.shared.global_epoch.load(Ordering::Acquire);
            self.slot
                .active_epoch
                .store(current_epoch, Ordering::Release);
        }

        self.pin_count.set(pin_count + 1);

        PinGuard { reader: self }
    }
}

/// Builder for configuring an `EpochGcDomain`.
///
/// Use this builder to customize garbage collection behavior:
/// - `auto_reclaim_threshold`: Set garbage count threshold for automatic collection
/// - `cleanup_interval`: Set how often to cleanup dead reader slots
///
/// # Example
/// ```
/// use swmr_epoch::EpochGcDomain;
///
/// let (gc, domain) = EpochGcDomain::builder()
///     .auto_reclaim_threshold(128)
///     .cleanup_interval(32)
///     .build();
/// ```
///
/// 用于配置 `EpochGcDomain` 的构建器。
pub struct EpochGcDomainBuilder {
    auto_reclaim_threshold: Option<usize>,
    cleanup_interval: usize,
}

impl EpochGcDomainBuilder {
    /// Create a new builder with default settings.
    /// 创建一个带有默认设置的新构建器。
    #[inline]
    pub fn new() -> Self {
        Self {
            auto_reclaim_threshold: Some(AUTO_RECLAIM_THRESHOLD),
            cleanup_interval: DEFAULT_CLEANUP_INTERVAL,
        }
    }

    /// Set the automatic reclamation threshold.
    ///
    /// When garbage count exceeds this threshold, `collect()` is automatically called.
    /// Pass `None` to disable automatic reclamation.
    ///
    /// Default: `Some(64)`
    ///
    /// 设置自动回收阈值。
    /// 当垃圾计数超过此阈值时，会自动调用 `collect()`。
    /// 传递 `None` 可禁用自动回收。
    #[inline]
    pub fn auto_reclaim_threshold(mut self, threshold: impl Into<Option<usize>>) -> Self {
        self.auto_reclaim_threshold = threshold.into();
        self
    }

    /// Set the cleanup interval for dead reader slots.
    ///
    /// Dead reader slots are cleaned up every N collection cycles to reduce overhead.
    /// Set to `0` to disable periodic cleanup (not recommended).
    ///
    /// Default: `16`
    ///
    /// 设置死读者槽的清理间隔。
    /// 死读者槽每 N 个回收周期清理一次，以减少开销。
    /// 设置为 `0` 可禁用定期清理（不推荐）。
    #[inline]
    pub fn cleanup_interval(mut self, interval: usize) -> Self {
        self.cleanup_interval = interval;
        self
    }

    /// Build the `EpochGcDomain` with the configured settings.
    ///
    /// Returns both the `GcHandle` and the `EpochGcDomain`.
    ///
    /// 使用配置的设置构建 `EpochGcDomain`。
    /// 返回 `GcHandle` 和 `EpochGcDomain`。
    #[inline]
    pub fn build(self) -> (GcHandle, EpochGcDomain) {
        let shared = Arc::new(SharedState {
            global_epoch: AtomicUsize::new(0),
            readers: Mutex::new(Vec::new()),
        });

        let gc = GcHandle {
            shared: shared.clone(),
            garbage: GarbageSet::new(),
            auto_reclaim_threshold: self.auto_reclaim_threshold,
            collection_counter: 0,
            cleanup_interval: self.cleanup_interval,
        };

        let domain = EpochGcDomain { shared };

        (gc, domain)
    }
}

impl Default for EpochGcDomainBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// An epoch-based garbage collection domain.
///
/// `EpochGcDomain` is the entry point for creating an epoch-based GC system.
/// It manages:
/// - The global epoch counter.
/// - Registration of reader threads.
/// - Creation of the unique garbage collector.
///
/// This design uses the type system to enforce at compile-time that only one `GcHandle` is created.
///
/// `EpochGcDomain` is `Clone` and can be safely shared across threads.
/// Typically, you create one domain at startup and clone it to threads that need it.
///
/// **Typical Usage**:
/// ```
/// use swmr_epoch::EpochGcDomain;
///
/// // Main thread: create the domain and get the garbage collector
/// let (mut gc, domain) = EpochGcDomain::new();
///
/// // Reader threads: register and pin
/// let local_epoch = domain.register_reader();
/// let guard = local_epoch.pin();
/// ```
///
/// 基于纪元的垃圾回收域。
/// `EpochGcDomain` 是创建基于纪元的 GC 系统的入口点。
/// 它管理：
/// - 全局纪元计数器。
/// - 读者线程的注册。
/// - 唯一垃圾回收器的创建。
/// 这个设计使用类型系统在编译时强制只创建一个 `GcHandle`。
/// `EpochGcDomain` 是 `Clone` 的，可以安全地在线程间共享。
/// 通常，你在启动时创建一个域并将其克隆到需要它的线程。
#[derive(Clone)]
pub struct EpochGcDomain {
    shared: Arc<SharedState>,
}

impl EpochGcDomain {
    /// Create a new epoch GC domain with default auto-reclaim threshold.
    /// Returns both the GcHandle and the EpochGcDomain.
    /// 创建一个新的 epoch GC 域，带有默认自动回收阈值。
    /// 返回 GcHandle 和 EpochGcDomain。
    #[inline]
    pub fn new() -> (GcHandle, Self) {
        Self::builder().build()
    }

    /// Create a builder for configuring the epoch GC domain.
    /// 
    /// # Example
    /// ```
    /// use swmr_epoch::EpochGcDomain;
    /// 
    /// let (gc, domain) = EpochGcDomain::builder()
    ///     .auto_reclaim_threshold(128)
    ///     .cleanup_interval(32)
    ///     .build();
    /// ```
    /// 
    /// 创建一个用于配置 epoch GC 域的构建器。
    #[inline]
    pub fn builder() -> EpochGcDomainBuilder {
        EpochGcDomainBuilder::new()
    }

    /// Register a new reader for the current thread.
    ///
    /// Returns a `LocalEpoch` that should be stored per-thread.
    /// The caller is responsible for ensuring that each `LocalEpoch` is used
    /// by only one thread.
    ///
    /// 为当前线程注册一个新的读者。
    /// 返回一个应该在每个线程中存储的 `LocalEpoch`。
    /// 调用者有责任确保每个 `LocalEpoch` 仅由一个线程使用。
    #[inline]
    pub fn register_reader(&self) -> LocalEpoch {
        let slot = Arc::new(ReaderSlot {
            active_epoch: AtomicUsize::new(INACTIVE_EPOCH),
        });

        // Register the reader immediately in the shared readers list
        self.shared.readers.lock()
            .expect("Failed to acquire readers lock in register_reader: mutex poisoned")
            .push(Arc::clone(&slot));

        LocalEpoch {
            slot,
            shared: self.shared.clone(),
            pin_count: Cell::new(0),
        }
    }
}

/// A guard that keeps the current thread pinned to an epoch.
///
/// `PinGuard` is obtained by calling `LocalEpoch::pin()`.
/// It is `!Send` and `!Sync` because it references a `!Sync` `LocalEpoch`.
/// Its lifetime is bound to the `LocalEpoch` it came from.
///
/// While a `PinGuard` is held, the thread is considered "active" at a particular epoch,
/// and the garbage collector will not reclaim data from that epoch.
///
/// `PinGuard` supports internal cloning via reference counting (increments the pin count),
/// allowing nested pinning. The thread remains pinned until all cloned guards are dropped.
///
/// **Safety**: The `PinGuard` is the mechanism that ensures safe concurrent access to
/// `EpochPtr` values. Readers must always hold a valid `PinGuard` when accessing
/// shared data through `EpochPtr::load()`.
///
/// 一个保持当前线程被钉住到一个纪元的守卫。
/// `PinGuard` 通过调用 `LocalEpoch::pin()` 获得。
/// 它是 `!Send` 和 `!Sync` 的，因为它引用了一个 `!Sync` 的 `LocalEpoch`。
/// 它的生命周期被绑定到它来自的 `LocalEpoch`。
/// 当 `PinGuard` 被持有时，线程被认为在特定纪元"活跃"，
/// 垃圾回收器不会回收该纪元的数据。
/// `PinGuard` 支持通过引用计数的内部克隆（增加 pin 计数），允许嵌套 pinning。
/// 线程保持被钉住直到所有克隆的守卫被 drop。
/// **安全性**：`PinGuard` 是确保对 `EpochPtr` 值安全并发访问的机制。
/// 读者在通过 `EpochPtr::load()` 访问共享数据时必须始终持有有效的 `PinGuard`。
#[must_use]
pub struct PinGuard<'a> {
    reader: &'a LocalEpoch,
}

impl<'a> Clone for PinGuard<'a> {
    /// Clone this guard to create a nested pin.
    ///
    /// Cloning increments the pin count, and the thread remains pinned until all cloned guards
    /// are dropped. This allows multiple scopes to hold pins simultaneously.
    ///
    /// 克隆此守卫以创建嵌套 pin。
    ///
    /// 克隆会增加 pin 计数，线程保持被钉住直到所有克隆的守卫被 drop。
    /// 这允许多个作用域同时持有 pin。
    #[inline]
    fn clone(&self) -> Self {
        let pin_count = self.reader.pin_count.get();

        assert!(
            pin_count > 0,
            "BUG: Cloning a PinGuard in an unpinned state (pin_count = 0). \
             This indicates incorrect API usage or a library bug."
        );

        self.reader.pin_count.set(pin_count + 1);

        PinGuard {
            reader: self.reader,
        }
    }
}

impl<'a> Drop for PinGuard<'a> {
    #[inline]
    fn drop(&mut self) {
        let pin_count = self.reader.pin_count.get();

        assert!(
            pin_count > 0,
            "BUG: Dropping a PinGuard in an unpinned state (pin_count = 0). \
             This indicates incorrect API usage or a library bug."
        );

        if pin_count == 1 {
            self.reader
                .slot
                .active_epoch
                .store(INACTIVE_EPOCH, Ordering::Release);
        }

        self.reader.pin_count.set(pin_count - 1);
    }
}

/// An epoch-protected shared pointer for safe concurrent access.
///
/// `EpochPtr<T>` is an atomic pointer that can be safely read by multiple readers
/// (via `load()` with a `PinGuard`) and safely written by a single writer
/// (via `store()` with a `GcHandle`).
///
/// **Safety Contract**:
/// - Readers must hold a `PinGuard` when calling `load()`. The `PinGuard` ensures
///   that the reader is pinned to an epoch, and the writer will not reclaim the
///   data until the guard is dropped.
/// - Writers must use the same `GcHandle` for all `store()` calls on pointers
///   that may be accessed by the same readers. This ensures proper garbage collection.
/// - The lifetime of the returned reference from `load()` is bound to the `PinGuard`.
///
/// **Typical Usage**:
/// ```
/// use swmr_epoch::{EpochGcDomain, EpochPtr};
///
/// let (mut gc, domain) = EpochGcDomain::new();
/// let shared = EpochPtr::new(42i32);
///
/// // Reader thread:
/// let local_epoch = domain.register_reader();
/// let guard = local_epoch.pin();
/// let value = shared.load(&guard);
/// // use value...
/// drop(guard);
///
/// // Writer thread:
/// shared.store(100i32, &mut gc);
/// gc.collect();
/// ```
///
/// 一个受 epoch 保护的共享指针，用于安全的并发访问。
/// `EpochPtr<T>` 是一个原子指针，可以被多个读者安全读取
/// （通过 `load()` 和 `PinGuard`），也可以被单个写入者安全写入
/// （通过 `store()` 和 `GcHandle`）。
/// **安全合约**：
/// - 读者在调用 `load()` 时必须持有 `PinGuard`。`PinGuard` 确保
///   读者被钉住到一个纪元，写入者不会在守卫被 drop 之前回收数据。
/// - 写入者必须对所有可能被相同读者访问的指针使用相同的 `GcHandle`。
///   这确保了正确的垃圾回收。
/// - 从 `load()` 返回的引用的生命周期被绑定到 `PinGuard`。
pub struct EpochPtr<T> {
    ptr: AtomicPtr<T>,
}

impl<T: 'static> EpochPtr<T> {
    /// Create a new epoch-protected pointer, initialized with the given value.
    /// 创建一个新的受 epoch 保护的指针，初始化为给定的值。
    #[inline]
    pub fn new(data: T) -> Self {
        Self {
            ptr: AtomicPtr::new(Box::into_raw(Box::new(data))),
        }
    }

    /// Reader load: safely read the current value.
    ///
    /// The `guard` parameter is required for **compile-time safety verification**.
    /// It ensures that the calling thread is pinned to an epoch,
    /// preventing the writer from reclaiming the data during the read.
    ///
    /// **Compile-Time Safety**: The lifetime of the returned reference is bound to the guard's lifetime.
    /// This is enforced by the Rust compiler, ensuring that:
    /// - You cannot use the reference after the guard is dropped.
    /// - The writer cannot reclaim the data while the guard (and thus the reference) is alive.
    /// - This creates a compile-time guarantee of memory safety without runtime overhead.
    ///
    /// # Panics
    /// This method does not panic. The `guard` parameter is used only for type-level safety.
    /// If you call this without a valid `PinGuard`, you are violating the API contract.
    ///
    /// 读取者 load：安全地读取当前值。
    ///
    /// `guard` 参数用于**编译时安全验证**。
    /// 它确保调用线程被钉住到一个纪元，防止写入者在读取期间回收数据。
    ///
    /// **编译时安全**：返回的引用的生命周期被绑定到守卫的生命周期。
    /// 这由 Rust 编译器强制执行，确保：
    /// - 你不能在守卫被 drop 后使用该引用。
    /// - 当守卫（以及引用）活跃时，写入者不能回收数据。
    /// - 这在没有运行时开销的情况下创建了内存安全的编译时保证。
    #[inline]
    pub fn load<'guard>(&self, _guard: &'guard PinGuard) -> &'guard T {
        let ptr = self.ptr.load(Ordering::Acquire);
        unsafe { &*ptr }
    }

    /// Writer store: safely update the value and retire the old one.
    ///
    /// This method atomically replaces the current pointer with a new one,
    /// and enqueues the old value for garbage collection.
    /// The old value will be reclaimed once it is safe to do so (i.e., after
    /// all readers have moved past the epoch in which it was retired).
    ///
    /// **Automatic Reclamation**: This operation may trigger automatic garbage collection
    /// if the garbage threshold is exceeded.
    ///
    /// 写入者 store：安全地更新值并退休旧值。
    /// 此方法原子地用新指针替换当前指针，
    /// 并将旧值入队进行垃圾回收。
    /// 旧值将在安全时被回收（即，在所有读者都已超过
    /// 退休该值的纪元之后）。
    ///
    /// **自动回收**：如果超过垃圾阈值，此操作可能会触发自动垃圾回收。
    #[inline]
    pub fn store(&self, data: T, gc: &mut GcHandle) {
        let new_ptr = Box::into_raw(Box::new(data));
        let old_ptr = self.ptr.swap(new_ptr, Ordering::Release);

        if !old_ptr.is_null() {
            unsafe {
                gc.retire(Box::from_raw(old_ptr));
            }
        }
    }
}

impl<T> std::fmt::Debug for EpochPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ptr = self.ptr.load(Ordering::Relaxed);
        f.debug_tuple("EpochPtr").field(&ptr).finish()
    }
}

impl<T> Drop for EpochPtr<T> {
    /// When an `EpochPtr` is dropped, it safely drops the current value.
    ///
    /// At drop time, we assume no other threads are accessing the pointer,
    /// so we can safely take back and drop the final value.
    ///
    /// 当 `EpochPtr` 被 drop 时，它安全地 drop 当前值。
    /// 在 drop 时，我们假设没有其他线程在访问该指针，
    /// 所以我们可以安全地拿回并 drop 最后的值。
    #[inline]
    fn drop(&mut self) {
        let ptr = self.ptr.load(Ordering::Relaxed);
        if !ptr.is_null() {
            unsafe {
                drop(Box::from_raw(ptr));
            }
        }
    }
}

#[cfg(test)]
mod tests;
