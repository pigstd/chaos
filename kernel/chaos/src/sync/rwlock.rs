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

pub struct RwReadGuard<'a> {
    lock: &'a RwLock,
}

pub struct RwWriteGuard<'a> {
    lock: &'a RwLock,
}

impl RwLock {
    pub fn new() -> Self {
        RwLock { status: AtomicUsize::new(0) }
    }
    pub fn read_guard(&self) -> RwReadGuard<'_> {
        self.read_lock();
        RwReadGuard { lock: self }
    }
    pub fn write_guard(&self) -> RwWriteGuard<'_> {
        self.write_lock();
        RwWriteGuard { lock: self }
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

impl Drop for RwReadGuard<'_> {
    fn drop(&mut self) {
        self.lock.read_unlock();
    }
}

impl Drop for RwWriteGuard<'_> {
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}
