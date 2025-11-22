use crate::garbage::GcHandle;
use crate::reader::PinGuard;
use crate::sync::{AtomicPtr, Ordering};
use std::boxed::Box;

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
