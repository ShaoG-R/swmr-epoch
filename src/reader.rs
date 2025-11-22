use crate::state::{INACTIVE_EPOCH, ReaderSlot, SharedState};
use crate::sync::{Arc, AtomicUsize, Cell, Ordering};

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
    pub(crate) fn new(shared: Arc<SharedState>) -> Self {
        let slot = Arc::new(ReaderSlot {
            active_epoch: AtomicUsize::new(INACTIVE_EPOCH),
        });

        // Register the reader immediately in the shared readers list
        shared.readers.lock().push(Arc::clone(&slot));

        LocalEpoch {
            slot,
            shared,
            pin_count: Cell::new(0),
        }
    }

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
            loop {
                let current_epoch = self.shared.global_epoch.load(Ordering::Acquire);
                self.slot
                    .active_epoch
                    .store(current_epoch, Ordering::Release);

                let min_active = self.shared.min_active_epoch.load(Ordering::Acquire);
                if current_epoch >= min_active {
                    break;
                }
                std::hint::spin_loop();
            }
        }

        self.pin_count.set(pin_count + 1);

        PinGuard { reader: self }
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
