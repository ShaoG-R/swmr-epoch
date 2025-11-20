use crate::sync::{AtomicUsize, Mutex, Arc};
use std::vec::Vec;

/// Default threshold for automatic garbage reclamation (count of retired nodes).
/// 自动垃圾回收的默认阈值（已退休节点的数量）。
pub(crate) const AUTO_RECLAIM_THRESHOLD: usize = 64;

/// Default interval for cleaning up dead reader slots (in collection cycles).
/// 清理死读者槽的默认间隔（以回收周期为单位）。
pub(crate) const DEFAULT_CLEANUP_INTERVAL: usize = 16;

/// Represents a reader that is not currently pinned to any epoch.
/// 表示当前未被钉住到任何纪元的读者。
pub(crate) const INACTIVE_EPOCH: usize = usize::MAX;

/// A slot allocated for a reader thread to record its active epoch.
///
/// Cache-aligned to prevent false sharing between readers.
///
/// 为读者线程分配的槽，用于记录其活跃纪元。
/// 缓存对齐以防止读者之间的伪共享。
#[derive(Debug)]
#[repr(align(64))]
pub(crate) struct ReaderSlot {
    /// The epoch currently being accessed by the reader, or INACTIVE_EPOCH.
    /// 读者当前访问的纪元，或 INACTIVE_EPOCH。
    pub(crate) active_epoch: AtomicUsize,
}

/// Global shared state for the epoch GC domain.
///
/// Contains the global epoch, the minimum active epoch, and the list of reader slots.
///
/// epoch GC 域的全局共享状态。
/// 包含全局纪元、最小活跃纪元和读者槽列表。
#[derive(Debug)]
#[repr(align(64))]
pub(crate) struct SharedState {
    /// The global monotonic epoch counter.
    /// 全局单调纪元计数器。
    pub(crate) global_epoch: AtomicUsize,
    /// The minimum epoch among all active readers (cached for performance).
    /// 所有活跃读者中的最小纪元（为性能而缓存）。
    pub(crate) min_active_epoch: AtomicUsize,
    /// List of all registered reader slots. Protected by a Mutex.
    /// 所有注册读者槽的列表。由 Mutex 保护。
    pub(crate) readers: Mutex<Vec<Arc<ReaderSlot>>>,
}
