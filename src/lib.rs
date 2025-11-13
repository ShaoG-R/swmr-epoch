use std::any::Any;
use std::cell::Cell;
// Used for the thread-local pin count
// 用于线程本地的 pin 计数
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::Weak;

use crossbeam_queue::SegQueue;
use thread_local::ThreadLocal;

// Garbage collection trigger threshold
// 垃圾回收的触发阈值
const RECLAIM_THRESHOLD: usize = 64;

// Represents an "inactive" epoch
// 代表"不活跃"的纪元
const INACTIVE_EPOCH: usize = usize::MAX;

// Type-erased wrapper for a "retired" object
// 一个被"退休"的对象的类型擦除包装
type ErasedGarbage = Box<dyn Any + Send>;

// --- 1. Internal shared state ---
// --- 1. 内部共享状态 ---

/// Slot for each reader thread in the global table
/// 每个读取者线程在全局表格中对应的"槽位"
#[derive(Debug)]
struct ParticipantSlot {
    // `usize::MAX` (INACTIVE_EPOCH) indicates inactive
    // `usize::MAX` (INACTIVE_EPOCH) 表示不活跃
    active_epoch: AtomicUsize,
}

/// Global state shared by all threads
/// 所有线程共享的全局状态
#[derive(Debug)]
struct SharedState {
    /// Global epoch, only Writer can advance it
    /// 全局纪元，只有 Writer 可以推进
    global_epoch: AtomicUsize,

    /// Lock-free queue for receiving registration requests from new readers
    /// 用于接收新读取者注册请求的无锁队列
    pending_registrations: SegQueue<Arc<ParticipantSlot>>,
}

// --- 2. Writer ---
// --- 2. 写入者 ---

/// The unique writer handle
/// 唯一的写入者句柄
pub struct Writer {
    shared: Arc<SharedState>,
    
    /// Writer's private garbage bin
    /// 写入者私有的垃圾桶
    local_garbage: Vec<(usize, ErasedGarbage)>,

    /// Participant list
    /// 参与者列表
    participants: Vec<Weak<ParticipantSlot>>,
}

impl Writer {
    /// Retire (defer deletion) a pointer
    /// 退休（延迟删除）一个指针
    pub fn retire<T: Send + 'static>(&mut self, data: Box<T>) {
        let current_epoch = self.shared.global_epoch.load(Ordering::Relaxed);
        self.local_garbage.push((current_epoch, data));

        if self.local_garbage.len() > RECLAIM_THRESHOLD {
            self.try_reclaim();
        }
    }

    /// Try to reclaim garbage
    /// 尝试回收垃圾
    pub fn try_reclaim(&mut self) {
        // Step 1: Advance global epoch
        // 步骤 1: 推进全局纪元
        let new_epoch = self.shared.global_epoch.fetch_add(1, Ordering::SeqCst) + 1;

        let mut min_active_epoch = new_epoch;
        let mut new_participants = Vec::with_capacity(self.participants.len());

        // Step 2.A: Scan old participants (O(N))
        // 步骤 2.A: 扫描旧的参与者 (O(N))
        for weak_slot in self.participants.iter() {
            if let Some(slot) = weak_slot.upgrade() {
                // Reader is still active
                // 读取者仍然活跃
                let epoch = slot.active_epoch.load(Ordering::Acquire);
                min_active_epoch = min_active_epoch.min(epoch);
                new_participants.push(weak_slot.clone());
            }
            // else: offline reader, auto-remove
            // else: 掉线的读取者，自动移除
        }

        // Step 2.B: Register all new readers (O(P))
        // 步骤 2.B: 注册所有新来的读取者 (O(P))
        while let Some(new_slot_arc) = self.shared.pending_registrations.pop() {
            let epoch = new_slot_arc.active_epoch.load(Ordering::Acquire);
            min_active_epoch = min_active_epoch.min(epoch);
            new_participants.push(Arc::downgrade(&new_slot_arc));
        }

        // Step 2.C: Replace old list
        // 步骤 2.C: 替换旧列表
        self.participants = new_participants;

        // Step 3: Calculate safe reclamation point
        // 步骤 3: 计算安全回收点
        let safe_to_reclaim_epoch = min_active_epoch.saturating_sub(1);

        // Step 4: Release garbage
        // 步骤 4: 释放垃圾
        self.local_garbage.retain(|(epoch, _)| *epoch > safe_to_reclaim_epoch);
    }
}

// --- 3. Reader ---
// --- 3. 读取者 ---

/// Holds the thread-local state for a reader
/// 持有读取者的线程本地状态
///
/// Contains the participant slot and the reentrant pin count.
/// 包含参与者槽位和可重入的 pin 计数。
struct ThreadLocalParticipant {
    /// This thread's participant slot
    /// 此线程的参与者槽位
    slot: Arc<ParticipantSlot>,
    /// Reentrant pin count for this thread
    /// 此线程的可重入 pin 计数
    pin_count: Cell<usize>,
}

/// Cloneable reader registry
/// 可克隆的读取者注册表
///
/// This replaces ReaderFactory. It is Clone, Send, Sync.
/// 它取代了 ReaderFactory。它是 Clone, Send, Sync。
/// It can be shared across all threads.
/// 可以在所有线程间共享。
/// It manages the thread-local participant slots.
/// 它管理线程本地的参与者槽位。
#[derive(Clone)]
pub struct ReaderRegistry {
    shared: Arc<SharedState>,
    /// Thread-local storage for participant slots.
    /// 用于参与者槽位的线程本地存储。
    /// Each thread gets its own ThreadLocalParticipant.
    /// 每个线程获得自己的 ThreadLocalParticipant。
    local_participant: Arc<ThreadLocal<ThreadLocalParticipant>>,
}

impl ReaderRegistry {
    /// Pins the current thread.
    /// "钉住"当前线程。
    ///
    /// This method is reentrant. It returns a Guard that, when dropped,
    /// will unpin the thread if it's the last remaining guard.
    /// 此方法是可重入的。它返回一个 Guard，当 Guard 被 drop 时，
    /// 如果这是最后一个 Guard，它将"解钉"线程。
    pub fn pin(&self) -> Guard {
        // Get or create the thread-local participant for this thread
        // 获取或创建此线程的本地参与者
        let participant = self.local_participant.get_or(|| {
            // This closure runs only once per thread to initialize
            // 这个闭包每个线程只在初始化时运行一次
            let slot = Arc::new(ParticipantSlot {
                active_epoch: AtomicUsize::new(INACTIVE_EPOCH),
            });

            // Register this new slot with the writer
            // 向写入者注册这个新槽位
            self.shared.pending_registrations.push(slot.clone());

            ThreadLocalParticipant {
                slot,
                pin_count: Cell::new(0),
            }
        });

        let pin_count = participant.pin_count.get();
        if pin_count == 0 {
            // This is the first pin on this thread. Mark as active.
            // 这是此线程上的第一个 pin。标记为活跃。
            // Use Acquire to see the new epoch from the Writer.
            // 使用 Acquire 来观察写入者的新纪元。
            let current_epoch = self.shared.global_epoch.load(Ordering::Acquire);
            // Use Release to ensure this store is visible to the Writer.
            // 使用 Release 来确保此存储对写入者可见。
            participant
                .slot
                .active_epoch
                .store(current_epoch, Ordering::Release);
        }

        // Increment the reentrant pin count
        // 增加可重入 pin 计数
        participant.pin_count.set(pin_count + 1);

        // Return a new guard pointing to the thread-local data
        // 返回一个指向线程本地数据的 Guard
        Guard {
            local: participant as *const ThreadLocalParticipant,
        }
    }
}

/// A guard that keeps the current thread pinned.
/// 一个保持当前线程被"钉住"的守卫。
///
/// This guard is !Send and !Sync because it references thread-local data.
/// 此守卫是 !Send 和 !Sync 的，因为它引用了线程本地数据。
/// It holds a raw pointer *const to the thread's ThreadLocalParticipant.
/// 它持有一个指向线程的 ThreadLocalParticipant 的裸指针 *const。
///
/// Dropping the guard decrements the thread-local pin count and unpins
/// the thread if the count reaches zero.
/// Drop 此守卫会减少线程本地的 pin 计数，并在计数达到零时"解钉"线程。
#[must_use]
pub struct Guard {
    local: *const ThreadLocalParticipant,
}

impl Clone for Guard {
    /// Cloning a guard is a valid way to re-pin.
    /// 克隆一个守卫是合法的重"钉" (re-pin) 方式。
    /// This increments the thread-local pin count.
    /// 这会增加线程本地的 pin 计数。
    fn clone(&self) -> Self {
        // SAFETY: local points to this thread's valid TLS data.
        // SAFETY: local 指向此线程的有效 TLS 数据。
        let participant = unsafe { &*self.local };
        let pin_count = participant.pin_count.get();

        // We must be in a pinned state to clone
        // 克隆时必须处于"钉住"状态
        assert!(pin_count > 0, "Cloning a guard in an unpinned state");

        // Increment pin count
        // 增加 pin 计数
        participant.pin_count.set(pin_count + 1);

        // Return a new guard pointing to the same data
        // 返回一个指向相同数据的新守卫
        Guard { local: self.local }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: local points to this thread's valid TLS data.
        // SAFETY: local 指向此线程的有效 TLS 数据。
        let participant = unsafe { &*self.local };
        let pin_count = participant.pin_count.get();

        // We must be in a pinned state to drop
        // Drop 时必须处于"钉住"状态
        assert!(pin_count > 0, "Dropping a guard in an unpinned state");

        if pin_count == 1 {
            // This is the last guard. Mark the thread as inactive.
            // 这是最后一个守卫。标记线程为不活跃。
            // Use Release to ensure this is visible to the Writer.
            // 使用 Release 确保这对写入者可见。
            participant
                .slot
                .active_epoch
                .store(INACTIVE_EPOCH, Ordering::Release);
        }

        // Decrement the reentrant pin count
        // 减少可重入 pin 计数
        participant.pin_count.set(pin_count - 1);
    }
}

// --- 4. Entry point ---
// --- 4. 入口点 ---

/// Create a new SWMR epoch system
/// 创建一个新的 SWMR 纪元系统
///
/// Returns the Writer and the ReaderRegistry.
/// 返回 Writer 和 ReaderRegistry。
pub fn new() -> (Writer, ReaderRegistry) {
    let shared = Arc::new(SharedState {
        global_epoch: AtomicUsize::new(0),
        pending_registrations: SegQueue::new(),
    });

    let writer = Writer {
        shared: shared.clone(),
        participants: Vec::new(),
        local_garbage: Vec::new(),
    };

    let registry = ReaderRegistry {
        shared,
        local_participant: Arc::new(ThreadLocal::new()),
    };

    (writer, registry)
}

/// An epoch-protected atomic pointer
/// 一个受 epoch 保护的原子指针
pub struct Atomic<T> {
    ptr: AtomicPtr<T>,
}

impl<T: Send + 'static> Atomic<T> {
    /// Create a new atomic pointer, initialized with the given data
    /// 创建一个新的原子指针，初始化为给定的数据
    pub fn new(data: T) -> Self {
        Self {
            ptr: AtomicPtr::new(Box::into_raw(Box::new(data))),
        }
    }

    /// Reader load
    /// 读取者 load
    ///
    /// Must provide a &Guard.
    /// 必须提供一个 &Guard。
    /// The lifetime of the returned reference &T is bound to the lifetime
    /// of the Guard.
    /// 返回的引用 &T 的生命周期被绑定到 Guard 的生命周期。
    pub fn load<'guard>(&self, _guard: &'guard Guard) -> &'guard T {
        let ptr = self.ptr.load(Ordering::Acquire);
        // SAFETY:
        // 1. ptr is always valid.
        // 1. ptr 总是有效的。
        // 2. The _guard guarantees the thread is pinned, so the
        //    writer will not reclaim the data ptr points to.
        // 2. _guard 保证了线程被"钉住"，所以写入者
        //    不会回收 ptr 指向的数据。
        // 3. The lifetime 'guard ensures the reference cannot outlive the pin.
        // 3. 'guard 生命周期确保了引用不会比"钉"存活更久。
        unsafe { &*ptr }
    }

    /// Writer store
    /// 写入者 store
    pub fn store(&self, data: Box<T>, writer: &mut Writer) {
        let new_ptr = Box::into_raw(data);
        let old_ptr = self.ptr.swap(new_ptr, Ordering::Release);

        // Give the old pointer to GC
        // 将旧指针交给 GC
        if !old_ptr.is_null() {
            unsafe {
                writer.retire(Box::from_raw(old_ptr));
            }
        }
    }
}

impl<T> Drop for Atomic<T> {
    fn drop(&mut self) {
        // At `drop` time, we assume no other threads are accessing
        // 在 `drop` 时，我们假设没有其他线程在访问
        // So we can safely take back and `drop` the final `Box`
        // 所以我们可以安全地拿回并 `drop` 最后的 `Box`
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