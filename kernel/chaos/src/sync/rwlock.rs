use core::sync::atomic::{AtomicUsize, Ordering};
use core::hint::{spin_loop};

// HUMAN
// 一个基于原子操作的读写锁实现

const WRITER_LOCKED: usize = usize::MAX;

pub struct RwLock {
    pub status: AtomicUsize,
    // status = WRITER_LOCKED: write lock held
    // status = 0: unlocked
    // status = n: read lock held by n readers
}

impl RwLock {
    pub fn new() -> Self {
        RwLock { status: AtomicUsize::new(0) }
    }
    pub fn read_lock(&self) {
        loop {
            let current = self.status.load(Ordering::Acquire);
            if current == WRITER_LOCKED {
                spin_loop();
                continue;
            }
            if self.status
                .compare_exchange_weak(current, current + 1, Ordering::Acquire, Ordering::Relaxed)
                .is_ok() {
                break;
            }
        }
    }
    pub fn read_unlock(&self) {
        self.status.fetch_sub(1, Ordering::Release);
    }
    pub fn write_lock(&self) {
        loop {
            let current = self.status.load(Ordering::Acquire);
            if current != 0 {
                spin_loop();
                continue;
            }
            if self.status
                .compare_exchange_weak(0, WRITER_LOCKED, Ordering::Acquire, Ordering::Relaxed)
                .is_ok() {
                break;
            }
        }
    }
    pub fn write_unlock(&self) {
        self.status.store(0, Ordering::Release);
    }
}