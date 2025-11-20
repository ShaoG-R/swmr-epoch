use crate::sync::{Arc, AtomicUsize, Mutex};
use crate::state::{SharedState, AUTO_RECLAIM_THRESHOLD, DEFAULT_CLEANUP_INTERVAL};
use crate::garbage::{GcHandle, GarbageSet};
use crate::reader::LocalEpoch;
use std::vec::Vec;

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
            min_active_epoch: AtomicUsize::new(0),
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
        LocalEpoch::new(self.shared.clone())
    }
}
