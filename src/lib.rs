//! # Epoch-Based Garbage Collection
//!
//! This module provides a lock-free, single-writer, multi-reader garbage collection system
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
//! ```ignore
//! // 1. Create a shared GC domain (can be cloned across threads)
//! let domain = EpochGcDomain::new();
//!
//! // 2. Create the unique garbage collector in the writer thread
//! let mut gc = domain.gc_handle();
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
//! shared_ptr.store(new_value, &mut gc);
//! gc.collect();  // Reclaim garbage from old epochs
//! ```

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Weak;

use crossbeam_queue::SegQueue;

const AUTO_RECLAIM_THRESHOLD: usize = 64;
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
struct ReaderSlot {
    active_epoch: AtomicUsize,
}

#[derive(Debug)]
struct SharedState {
    global_epoch: AtomicUsize,
    pending_registrations: SegQueue<Arc<ReaderSlot>>,
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
    local_garbage: BTreeMap<usize, Vec<RetiredNode>>,
    local_garbage_count: usize,
    readers: Vec<Weak<ReaderSlot>>,
}

impl GcHandle {
    #[inline]
    fn total_garbage_count(&self) -> usize {
        self.local_garbage_count
    }

    /// Retire (defer deletion) of a value.
    ///
    /// The value is stored in a garbage bin associated with the current epoch.
    /// It will be reclaimed once the epoch becomes older than all active readers' epochs.
    ///
    /// This is an internal method used by `EpochPtr::store()`.
    /// If garbage count exceeds `AUTO_RECLAIM_THRESHOLD`, automatic collection is triggered.
    ///
    /// 退休（延迟删除）一个值。
    /// 该值被存储在与当前纪元关联的垃圾桶中。
    /// 一旦该纪元比所有活跃读者的纪元都更旧，它就会被回收。
    /// 这是 `EpochPtr::store()` 使用的内部方法。
    /// 如果垃圾计数超过 `AUTO_RECLAIM_THRESHOLD`，自动回收被触发。
    #[inline]
    pub(crate) fn retire<T: 'static>(&mut self, data: Box<T>) {
        let current_epoch = self.shared.global_epoch.load(Ordering::Relaxed);

        self.local_garbage
            .entry(current_epoch)
            .or_default()
            .push(RetiredObject::new(data));

        self.local_garbage_count += 1;

        if self.total_garbage_count() > AUTO_RECLAIM_THRESHOLD {
            self.collect();
        }
    }

    /// Perform a garbage collection cycle.
    ///
    /// This method:
    /// 1. Advances the global epoch.
    /// 2. Scans all active readers to find the minimum active epoch.
    /// 3. Reclaims garbage from epochs older than the minimum active epoch.
    ///
    /// Can be called periodically or after significant updates.
    /// Safe to call even if there is no garbage to reclaim.
    ///
    /// 执行一个垃圾回收周期。
    /// 此方法：
    /// 1. 推进全局纪元。
    /// 2. 扫描所有活跃读者以找到最小活跃纪元。
    /// 3. 回收比最小活跃纪元更旧的纪元中的垃圾。
    /// 可以定期调用或在重大更新后调用。
    /// 即使没有垃圾要回收也可以安全调用。
    pub(crate) fn collect(&mut self) {
        let new_epoch = self.shared.global_epoch.fetch_add(1, Ordering::Acquire) + 1;

        let mut min_active_epoch = new_epoch;
        let mut new_readers = Vec::with_capacity(self.readers.len());

        for weak_slot in self.readers.iter() {
            if let Some(slot) = weak_slot.upgrade() {
                let epoch = slot.active_epoch.load(Ordering::Acquire);
                if epoch != INACTIVE_EPOCH {
                    min_active_epoch = min_active_epoch.min(epoch);
                }
                new_readers.push(weak_slot.clone());
            }
        }

        while let Some(new_slot_arc) = self.shared.pending_registrations.pop() {
            let epoch = new_slot_arc.active_epoch.load(Ordering::Acquire);
            if epoch != INACTIVE_EPOCH {
                min_active_epoch = min_active_epoch.min(epoch);
            }
            new_readers.push(Arc::downgrade(&new_slot_arc));
        }

        self.readers = new_readers;

        let safe_to_reclaim_epoch = if min_active_epoch == new_epoch {
            usize::MAX
        } else {
            min_active_epoch.saturating_sub(1)
        };

        let mut retained_count = 0;

        self.local_garbage.retain(|&epoch, bag| {
            if epoch > safe_to_reclaim_epoch {
                retained_count += bag.len();
                true
            } else {
                false
            }
        });

        self.local_garbage_count = retained_count;
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
    /// This method is reentrant: multiple calls can be nested, and the thread
    /// remains pinned until all returned guards are dropped.
    ///
    /// While pinned, the thread is considered "active" at a particular epoch,
    /// and the garbage collector will not reclaim data from that epoch.
    ///
    /// 将此线程钉住到当前纪元。
    /// 返回一个 `PinGuard`，在其生命周期内保持线程被钉住。
    /// 此方法是可重入的：多个调用可以嵌套，线程
    /// 在所有返回的守卫被 drop 之前保持被钉住。
    /// 当被钉住时，线程被认为在特定纪元"活跃"，
    /// 垃圾回收器不会回收该纪元的数据。
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

        PinGuard {
            reader: self,
        }
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
/// `EpochGcDomain` is `Clone` and can be safely shared across threads.
/// Typically, you create one domain at startup and clone it to threads that need it.
///
/// **Typical Usage**:
/// ```ignore
/// // Main thread: create the domain
/// let domain = EpochGcDomain::new();
///
/// // Writer thread: create the garbage collector
/// let mut gc = domain.gc_handle();
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
/// `EpochGcDomain` 是 `Clone` 的，可以安全地在线程间共享。
/// 通常，你在启动时创建一个域并将其克隆到需要它的线程。
#[derive(Clone)]
pub struct EpochGcDomain {
    shared: Arc<SharedState>,
}

impl EpochGcDomain {
    /// Create a new epoch GC domain.
    /// 创建一个新的 epoch GC 域。
    #[inline]
    pub fn new() -> Self {
        EpochGcDomain {
            shared: Arc::new(SharedState {
                global_epoch: AtomicUsize::new(0),
                pending_registrations: SegQueue::new(),
            }),
        }
    }

    /// Create the unique garbage collector handle for this domain.
    ///
    /// There should be exactly one `GcHandle` per domain, owned by the writer thread.
    /// Calling this multiple times will create multiple independent handles,
    /// which is not recommended and may lead to incorrect behavior.
    ///
    /// 为此域创建唯一的垃圾回收器句柄。
    /// 每个域应该恰好有一个 `GcHandle`，由写入者线程持有。
    /// 多次调用此方法会创建多个独立的句柄，
    /// 不推荐这样做，可能导致不正确的行为。
    #[inline]
    pub fn gc_handle(&self) -> GcHandle {
        GcHandle {
            shared: self.shared.clone(),
            local_garbage: BTreeMap::new(),
            local_garbage_count: 0,
            readers: Vec::new(),
        }
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

        self.shared.pending_registrations.push(slot.clone());

        LocalEpoch {
            slot,
            shared: self.shared.clone(),
            pin_count: Cell::new(0),
        }
    }
}

impl Default for EpochGcDomain {
    fn default() -> Self {
        Self::new()
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
/// `PinGuard` can be cloned (which increments the pin count), allowing nested pinning.
/// The thread remains pinned until all cloned guards are dropped.
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
/// `PinGuard` 可以被克隆（增加 pin 计数），允许嵌套 pinning。
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
    /// Cloning increments the pin count, and the thread remains pinned
    /// until all cloned guards are dropped.
    ///
    /// 克隆此守卫以创建嵌套 pin。
    /// 克隆会增加 pin 计数，线程保持被钉住
    /// 直到所有克隆的守卫被 drop。
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
/// ```ignore
/// let shared = EpochPtr::new(initial_value);
///
/// // Reader thread:
/// let guard = local_epoch.pin();
/// let value = shared.load(&guard);
/// // use value...
/// drop(guard);
///
/// // Writer thread:
/// shared.store(new_value, &mut gc);
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
    /// The `guard` parameter ensures that the calling thread is pinned to an epoch,
    /// preventing the writer from reclaiming the data during the read.
    /// The lifetime of the returned reference is bound to the guard's lifetime.
    ///
    /// # Panics
    /// This method does not panic, but the `guard` parameter is required for safety.
    /// If you call this without a valid `PinGuard`, you are violating the API contract.
    ///
    /// 读取者 load：安全地读取当前值。
    /// `guard` 参数确保调用线程被钉住到一个纪元，
    /// 防止写入者在读取期间回收数据。
    /// 返回的引用的生命周期被绑定到守卫的生命周期。
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
    /// 写入者 store：安全地更新值并退休旧值。
    /// 此方法原子地用新指针替换当前指针，
    /// 并将旧值入队进行垃圾回收。
    /// 旧值将在安全时被回收（即，在所有读者都已超过
    /// 退休该值的纪元之后）。
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
