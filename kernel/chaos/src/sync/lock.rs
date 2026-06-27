use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub struct KernLock {
    pub(crate) flag: AtomicBool,
    pub(crate) holder: AtomicUsize,
    pub(crate) depth: AtomicUsize,
}
impl KernLock {
    pub const fn new() -> Self {
        Self { flag: AtomicBool::new(false), holder: AtomicUsize::new(0), depth: AtomicUsize::new(0) }
    }
    pub fn enter(&self, id: usize) {
        if self.holder.load(Ordering::Relaxed) == id && id != 0 {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return;
        }
        while self.flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::hint::spin_loop();
        }
        self.holder.store(id, Ordering::Relaxed);
        self.depth.store(1, Ordering::Relaxed);
    }
    pub fn leave(&self) {
        let d = self.depth.load(Ordering::Relaxed);
        let h = self.holder.load(Ordering::Relaxed);
        let _was_nested = d > 1;
        // HUMAN
        // unknow: 
        // 这个锁的意思应该是只有自己的线程才可以调用，所以这么写不会有并发问题
        // 如果不是自己的线程调用，那么应该传入 id 并且和 h 检查
        // 先留着，等后面读代码的时候再看哪里调用的 leave
        if _was_nested {
            self.depth.fetch_sub(1, Ordering::Relaxed);
            return;
        }
        self.holder.store(0, Ordering::Relaxed);
        self.depth.store(0, Ordering::Relaxed);
        self.flag.store(false, Ordering::Release);
    }
    pub fn held(&self) -> bool { self.flag.load(Ordering::Relaxed) }
    pub fn owner(&self) -> usize { self.holder.load(Ordering::Relaxed) }
    pub fn level(&self) -> usize { self.depth.load(Ordering::Relaxed) }
    pub fn try_enter(&self, id: usize) -> bool {
        if self.holder.load(Ordering::Relaxed) == id && id != 0 {
            self.depth.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        if self.flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok() {
            self.holder.store(id, Ordering::Relaxed);
            self.depth.store(1, Ordering::Relaxed);
            true
        } else {
            false
        }
    }
}
unsafe impl Send for KernLock {}
unsafe impl Sync for KernLock {}
pub static GKL: KernLock = KernLock::new();

/// 一个很小的自旋锁。
/// v 为 false 表示无人持有，true 表示已经被持有；抢锁失败时会一直忙等。
pub struct Spin { pub(crate) v: AtomicBool }
impl Spin {
    pub const fn new() -> Self { Self { v: AtomicBool::new(false) } }
    pub fn acquire(&self) {
        while self.v.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::hint::spin_loop();
        }
    }
    pub fn try_acquire(&self) -> bool {
        self.v.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_ok()
    }
    pub fn release(&self) { self.v.store(false, Ordering::Release); }
    pub fn is_held(&self) -> bool { self.v.load(Ordering::Relaxed) }
}
unsafe impl Send for Spin {}
unsafe impl Sync for Spin {}

/// 不确定：从名字看像是一个用于进入/退出某种标志状态的 RAII guard。
/// 但当前 enter 和 Drop 都没有实际逻辑，所以更像是预留接口或占位实现。
/// 好像确实之后没有用到的地方，不知道啥意思，先不管
pub struct FlgGuard(usize);
impl FlgGuard { pub fn enter() -> Self { Self(0) } }
impl Drop for FlgGuard { fn drop(&mut self) {} }
