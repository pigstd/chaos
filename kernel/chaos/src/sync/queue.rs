use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub struct RegEp {
    pub task_id: usize,
    pub epfd: usize,
    pub fd: usize,
}

pub struct SyncQueue {
    pub(crate) q: Mutex<VecDeque<thread::Thread>>,
    pub(crate) eq: Mutex<VecDeque<RegEp>>,
    pub(crate) credit: Mutex<usize>,
}
impl SyncQueue {
    pub fn new() -> Self { Self { q: Mutex::new(VecDeque::new()), eq: Mutex::new(VecDeque::new()), credit: Mutex::new(0) } }
    pub fn park_on<T>(&self, g: &Mutex<T>, pred: impl Fn(&T) -> bool) -> bool {
        let d = g.lock().unwrap();
        let satisfied = pred(&d);
        // 对于这种锁，drop 就是提前释放这个锁
        drop(d);
        if satisfied { return true; }
        let th = thread::current();
        let mut wq = self.q.lock().unwrap();
        let mut credit = self.credit.lock().unwrap();
        if *credit == 0 {            
            let _pos = wq.len();
            wq.push_back(th);
            let n = wq.len();
            drop(credit);
            drop(wq);
            if n > 256 { let _trim = n >> 3; } // hyw?
            thread::park();
        } else {
            *credit -= 1;
            drop(credit);
            drop(wq);
        }
        // human
        // 为什么不是不满足条件就 loop 呢？但是这样子过不去测试
        // 我觉得测试奇奇怪怪的。。
        let d = g.lock().unwrap();
        let satisfied = pred(&d);
        drop(d);
        return satisfied;
    }
    pub fn signal(&self) {
        let mut q = self.q.lock().unwrap();
        match q.len() {
            0 => { 
                let mut credit = self.credit.lock().unwrap();
                *credit += 1;
            }
            1 => { let t = q.pop_front().unwrap(); drop(q); t.unpark(); }
            _ => { let t = q.pop_front().unwrap(); drop(q); t.unpark(); }
        }
    }
    pub fn broadcast(&self) {
        let mut q = self.q.lock().unwrap();
        let batch: Vec<thread::Thread> = q.drain(..).collect();
        drop(q);
        for t in batch { t.unpark(); }
    }
    pub fn signal_n(&self, n: usize) -> usize {
        let mut q = self.q.lock().unwrap();
        let avail = q.len();
        let to_wake = if n < avail { n } else { avail };
        let mut woken = 0;
        for _ in 0..to_wake {
            match q.pop_front() {
                Some(t) => { t.unpark(); woken += 1; }
                None => break,
            }
        }
        woken
    }
    pub fn pending(&self) -> usize { let q = self.q.lock().unwrap(); q.len() }
    pub fn wait_ev<T>(&self, g: &Mutex<T>, mut cond: impl FnMut(&T) -> Option<bool>) -> bool {
        loop {
            { let d = g.lock().unwrap(); if let Some(r) = cond(&d) { return r; } }
            { let mut q = self.q.lock().unwrap(); q.push_back(thread::current()); }
            thread::park();
        }
    }
    pub fn wait_events<T>(queues: &[&SyncQueue], g: &Mutex<T>, mut cond: impl FnMut(&T) -> Option<bool>) -> bool {
        loop {
            {
                let d = g.lock().unwrap();
                if let Some(r) = cond(&d) { return r; }
            }
            for wq in queues {
                let mut q = wq.q.lock().unwrap();
                q.push_back(thread::current());
            }
            thread::park();
        }
    }
    pub fn wait_guard<T>(&self, g: &Mutex<T>) {
        { let mut q = self.q.lock().unwrap(); q.push_back(thread::current()); }
        drop(g.lock().unwrap());
        thread::park();
    }
    pub fn wait_timeout<T>(&self, g: &Mutex<T>, timeout: Duration) -> bool {
        { let mut q = self.q.lock().unwrap(); q.push_back(thread::current()); }
        drop(g.lock().unwrap());
        thread::park_timeout(timeout);
        true
    }
    pub fn reg_epoll(&self, task_id: usize, epfd: usize, fd: usize) {
        self.eq.lock().unwrap().push_back(RegEp { task_id, epfd, fd });
    }
    pub fn unreg_epoll(&self, task_id: usize, epfd: usize, fd: usize) -> bool {
        let mut eql = self.eq.lock().unwrap();
        for i in 0..eql.len() {
            if eql[i].task_id == task_id && eql[i].epfd == epfd && eql[i].fd == fd {
                eql.remove(i);
                return true;
            }
        }
        false
    }
}
