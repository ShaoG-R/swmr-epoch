use crate::sync::{Arc, Ordering};
use crate::state::{SharedState, INACTIVE_EPOCH};
use std::collections::VecDeque;
use std::vec::Vec;
use std::boxed::Box;

/// Alias for the retired object type used in garbage lists.
/// 垃圾列表中使用的已退休对象类型的别名。
type RetiredNode = RetiredObject;

/// An object that has been retired (removed from shared view) but not yet deleted.
/// It stores the raw pointer and a destructor function to safely drop the concrete type.
///
/// 一个已被退休（从共享视图中移除）但尚未删除的对象。
/// 它存储原始指针和析构函数，以安全地 drop 具体类型。
struct RetiredObject {
    /// The raw pointer to the data.
    /// 数据的原始指针。
    ptr: *mut (),
    /// Function pointer to the type-specific destructor.
    /// 类型特定析构函数的函数指针。
    dtor: unsafe fn(*mut ()),
}

/// Generic destructor for retired objects.
/// Converts the raw pointer back to Box<T> and drops it.
///
/// 已退休对象的通用析构函数。
/// 将原始指针转换回 Box<T> 并将其 drop。
#[inline(always)]
unsafe fn drop_value<T>(ptr: *mut ()) {
    let ptr = ptr as *mut T;
    unsafe {
        drop(Box::from_raw(ptr));
    }
}

impl RetiredObject {
    /// Create a new retired object from a Box<T>.
    /// 从 Box<T> 创建一个新的已退休对象。
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
    /// Executes the type-erased destructor.
    /// 执行类型擦除的析构函数。
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
pub(crate) struct GarbageSet {
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
    pub(crate) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            pool: Vec::new(),
            count: 0,
        }
    }

    /// Get the total number of retired objects.
    /// 获取已退休对象的总数。
    #[inline]
    pub(crate) fn len(&self) -> usize {
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
    #[inline]
    fn add(&mut self, node: RetiredNode, current_epoch: usize) {
        // Check if we can append to the last bag
        let append_to_last = if let Some((last_epoch, _)) = self.queue.back() {
            *last_epoch == current_epoch
        } else {
            false
        };

        if append_to_last {
            // Safe to unwrap because we checked back() above
            self.queue.back_mut().unwrap().1.push(node);
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
    pub(crate) fn collect(&mut self, min_active_epoch: usize, current_epoch: usize) {
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

        self.count = self.queue.iter().map(|(_, bag)| bag.len()).sum();
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
    pub(crate) shared: Arc<SharedState>,
    pub(crate) garbage: GarbageSet,
    pub(crate) auto_reclaim_threshold: Option<usize>,
    pub(crate) collection_counter: usize,
    pub(crate) cleanup_interval: usize,
}

impl GcHandle {
    #[inline]
    pub(crate) fn total_garbage_count(&self) -> usize {
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
        let new_epoch = self.shared.global_epoch.fetch_add(1, Ordering::AcqRel) + 1;

        let mut min_active_epoch = new_epoch;
        self.collection_counter += 1;

        let should_cleanup =
            self.cleanup_interval > 0 && self.collection_counter % self.cleanup_interval == 0;

        let mut shared_readers = self.shared.readers.lock();

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

        self.shared
            .min_active_epoch
            .store(min_active_epoch, Ordering::Release);
        self.garbage.collect(min_active_epoch, new_epoch);
    }
}
