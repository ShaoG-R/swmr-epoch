use std::any::Any;
use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
use std::sync::Arc;

use crossbeam_queue::SegQueue;
use std::sync::Weak;

// Garbage collection trigger threshold
// 垃圾回收的触发阈值
const RECLAIM_THRESHOLD: usize = 64;

// Represents an "inactive" epoch
// 代表"不活跃"的纪元
const INACTIVE_EPOCH: usize = usize::MAX;

// Type-erased wrapper for a "retired" object
// It knows how to drop itself
// 一个被"退休"的对象的类型擦除包装
// 它知道如何 drop 自己
type ErasedGarbage = Box<dyn Any + Send>;

// --- 1. Internal shared state ---
// --- 1. 内部共享状态 ---

/// Slot for each reader thread in the global table
///
/// The writer reads `active_epoch` to determine the safe reclamation point
///
/// 每个读取者线程在全局表格中对应的"槽位"
/// 
/// 写入者会读取 `active_epoch` 来决定安全回收点
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
    /// Readers push Arc<ParticipantSlot> into this queue
    /// 一个无锁队列，用于接收新读取者的注册请求
    /// 读取者将 Arc<ParticipantSlot> 放入这里
    pending_registrations: SegQueue<Arc<ParticipantSlot>>,
}

// --- 2. Writer ---
// --- 2. 写入者 (Writer) ---

/// The unique writer handle
///
/// It is `!Clone` and `!Sync` to guarantee "single writer"
///
/// 唯一的写入者句柄
/// 
/// 它是 `!Clone` 和 `!Sync` 的，以保证"单写入者"
pub struct Writer {
    shared: Arc<SharedState>,
    
    /// Writer's private garbage bin.
    /// Tuple: (epoch when retired, data to be dropped)
    /// 写入者私有的垃圾桶。
    /// 元组: (退休时的纪元, 要被drop的数据)
    local_garbage: Vec<(usize, ErasedGarbage)>,

    /// Participant list.
    /// We store Weak pointers to automatically detect when Handle is dropped.
    /// 参与者列表。
    /// 我们存储 Weak 指针，以便自动检测 Handle 何时被 drop。
    participants: Vec<Weak<ParticipantSlot>>,
}

impl Writer {
    /// Retire (defer deletion) a pointer
    ///
    /// This method is generic and can accept any Box<T>
    ///
    /// 退休（延迟删除）一个指针
    ///
    /// 这个方法是通用的，可以接受任何 Box<T>
    pub fn retire<T: Send + 'static>(&mut self, data: Box<T>) {
        let current_epoch = self.shared.global_epoch.load(Ordering::Relaxed);
        
        // Put into garbage bin
        // 放入垃圾桶
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
        
        // Create a new Vec to hold participants for the next round
        // We can pre-estimate capacity
        // 创建一个新的 Vec 来存放下一轮的参与者
        // 我们可以预估容量
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
            // else: offline reader, don't push to new_participants, auto-remove
            // else: 掉线的读取者，不 push 到 new_participants，自动移除
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
// --- 3. 读取者 (Reader) ---

/// Cloneable reader factory
///
/// `Clone`, `Send`, `Sync`.
/// It can be wrapped in `Arc` and distributed to all new threads.
///
/// 可克隆的读取者工厂
///
/// `Clone`, `Send`, `Sync`。
/// 它可以被 `Arc` 并分发给所有新线程。
#[derive(Clone)]
pub struct ReaderFactory {
    shared: Arc<SharedState>,
}

/// Reader handle
/// 读取者句柄
pub struct ReaderHandle {
    // It "owns" its own slot
    // 它"拥有"自己的槽位
    slot: Arc<ParticipantSlot>, 
    shared: Arc<SharedState>,
}

impl ReaderFactory {
    /// Create a new reader handle (lock-free operation)
    /// 创建一个新的读取者句柄（无锁操作）
    pub fn create_handle(&self) -> ReaderHandle {
        let slot = Arc::new(ParticipantSlot {
            active_epoch: AtomicUsize::new(INACTIVE_EPOCH),
        });

        // Push the slot into the queue, waiting for Writer to register (lock-free)
        // 将槽位推入队列，等待 Writer 注册（无锁）
        self.shared.pending_registrations.push(slot.clone());

        // The returned Handle holds both slot and shared
        // 返回的 Handle 同时持有 slot 和 shared
        ReaderHandle {
            slot,
            shared: self.shared.clone(),
        }
    }
}

impl ReaderHandle {
    /// Pin the current thread and get a temporary `ReaderGuard`
    ///
    /// Lock-free operation
    ///
    /// As long as `ReaderGuard` exists, the writer cannot reclaim
    /// any data retired in the current epoch or later.
    ///
    /// "钉住"当前线程，获取一个临时的 `ReaderGuard`
    ///
    /// 无锁操作
    ///
    /// 只要 `ReaderGuard` 存在，写入者就不能回收
    /// 在当前纪元或之后被退休的任何数据。
    pub fn pin(&self) -> ReaderGuard<'_> {
        // Step 1: Read the current global epoch (lock-free)
        // Use Acquire semantics to ensure we observe Writer's epoch advancement (fetch_add)
        // and all memory operations before that epoch.
        // 步骤 1: 读取当前全局纪元（无锁）
        // 使用 Acquire 语义，确保我们能观察到 Writer 推进纪元 (fetch_add)
        // 以及在该纪元之前的所有内存操作。
        let current_epoch = self.shared.global_epoch.load(Ordering::Acquire);
        
        // Step 2: Mark our slot as "active in the current epoch" (lock-free)
        // Use Release semantics to ensure any read operations before this store
        // (such as the upcoming Atomic<T>::load)
        // "happen-before" this marking.
        // This prevents Writer (reading this value with Acquire) from incorrectly reclaiming
        // data we are about to access.
        // 步骤 2: 将自己的槽位标记为"活跃在当前纪元"（无锁）
        // 使用 Release 语义，确保在此次 store 之前的任何读取操作
        // (比如即将发生的 Atomic<T>::load)
        // 都 "happen-before" 这个标记。
        // 这能防止 Writer (使用 Acquire 读取此值) 错误地回收我们
        // 即将访问的数据。
        self.slot.active_epoch.store(current_epoch, Ordering::Release);
        
        // Step 3: Return the guard
        // 步骤 3: 返回守卫
        ReaderGuard { slot: &self.slot }
    }
}

/// Temporary reader guard
///
/// `!Send`, `!Sync`. It must be used on the stack of `pin()`.
/// Its lifetime `'handle` guarantees it cannot outlive `ReaderHandle`.
///
/// 临时的读取者守卫
///
/// `!Send`, `!Sync`。它必须在 `pin()` 的栈上使用。
/// 它的生命周期 `'handle` 保证了它不能活得比 `ReaderHandle` 更久。
#[must_use]
pub struct ReaderGuard<'handle> {
    slot: &'handle ParticipantSlot,
}

impl<'handle> Drop for ReaderGuard<'handle> {
    /// When `ReaderGuard` is destroyed, mark the thread as "inactive"
    /// 当 `ReaderGuard` 被销毁时，标记线程为"不活跃"
    fn drop(&mut self) {
        self.slot.active_epoch.store(INACTIVE_EPOCH, Ordering::Release);
    }
}

// --- 4. Entry point ---
// --- 4. 入口点 ---

/// Create a new SWMR epoch system
/// 创建一个新的 SWMR 纪元系统
pub fn new() -> (Writer, ReaderFactory) {
    let shared = Arc::new(SharedState {
        global_epoch: AtomicUsize::new(0),
        pending_registrations: SegQueue::new(),
    });

    let writer = Writer {
        shared: shared.clone(),
        participants: Vec::new(),
        local_garbage: Vec::new(),
    };

    let factory = ReaderFactory {
        shared,
    };

    (writer, factory)
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

    /// Reader `load`
    ///
    /// Must provide a `ReaderGuard`.
    /// The lifetime of the returned reference `&T` is bound to the lifetime of `ReaderGuard`,
    /// which guarantees at compile time that we won't use this reference outside the `Guard`.
    ///
    /// 读取者 `load`
    ///
    /// 必须提供一个 `ReaderGuard`。
    /// 返回的引用 `&T` 的生命周期被绑定到 `ReaderGuard` 的生命周期，
    /// 这在编译期保证了我们不会在 `Guard` 之外使用这个引用。
    pub fn load<'guard>(&self, _guard: &'guard ReaderGuard) -> &'guard T {
        let ptr = self.ptr.load(Ordering::Acquire);
        // SAFETY:
        // 1. `ptr` will never be null (because writer always maintains a valid pointer)
        // 2. As long as `_guard` exists, `Writer` will not reclaim the memory pointed to by `ptr`
        // 3. The returned reference `&'guard T` ensures this reference cannot be used outside the `Guard`
        // SAFETY:
        // 1. `ptr` 永远不会是 null (因为 writer 总是维护有效指针)
        // 2. 只要 `_guard` 存在，`Writer` 就不会回收 `ptr` 指向的内存
        // 3. 返回的引用 `&'guard T` 确保了在 `Guard` 之外无法使用此引用
        unsafe { &*ptr }
    }

    /// Writer `store`
    ///
    /// Must provide a `&mut Writer`.
    /// This atomically replaces the pointer and gives the old pointer to `Writer` for retirement.
    ///
    /// 写入者 `store`
    ///
    /// 必须提供一个 `&mut Writer`。
    /// 这会原子地替换指针，并将旧指针交给 `Writer` 退休。
    pub fn store(&self, data: Box<T>, writer: &mut Writer) {
        let new_ptr = Box::into_raw(data);
        
        // Atomically swap the pointer
        // 原子地交换指针
        let old_ptr = self.ptr.swap(new_ptr, Ordering::Release);

        // Give the old pointer to GC
        // SAFETY:
        // `old_ptr` is created by `Box::into_raw`, which is valid.
        // `writer` will ensure it is not freed before no readers reference it.
        // 将旧指针交给 GC
        // SAFETY:
        // `old_ptr` 是由 `Box::into_raw` 创建的，是有效的。
        // `writer` 会确保在没有读取者引用它之前，不会释放它。
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
        // So we can safely take back and `drop` the final `Box`
        // 在 `drop` 时，我们假设没有其他线程在访问
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
