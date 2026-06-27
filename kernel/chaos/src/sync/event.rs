use crate::prelude::*;
use crate::consts::*;
use crate::*;

/// 事件标志位集合。Ev 是 event 的缩写。
/// 每个常量占一个 bit，用来表示“可读、可写、出错、关闭、进程退出、收到信号”等事件。
pub struct EvFlag;
impl EvFlag {
    pub const READABLE: u32 = 1 << 0;
    pub const WRITABLE: u32 = 1 << 1;
    pub const ERROR: u32 = 1 << 2;
    pub const CLOSED: u32 = 1 << 3;
    pub const PROC_QUIT: u32 = 1 << 10;
    pub const CHILD_QUIT: u32 = 1 << 11;
    pub const RECV_SIG: u32 = 1 << 12;
    pub const SEM_RM: u32 = 1 << 20;
    pub const SEM_ACQ: u32 = 1 << 21;
}

/// 事件回调函数类型。
/// 参数是当前事件位图；返回 true 表示这个回调已经处理完，会从事件总线里移除。
pub type EvCb = Box<dyn Fn(u32) -> bool + Send>;

/// 一个简单的事件总线。
/// ev 保存当前事件位图，cbs 保存订阅者回调；事件变化时会调用回调，
/// 返回 true 的回调会被删除，返回 false 的回调会继续保留。
#[derive(Default)]
pub struct EvBus {
    pub ev: u32,
    pub cbs: Vec<Box<dyn Fn(u32) -> bool + Send>>,
}
impl EvBus {
    pub fn make() -> Arc<Mutex<Self>> { Arc::new(Mutex::new(Self::default())) }
    pub fn set(&mut self, s: u32) { self.change(0, s); }
    pub fn clear(&mut self, s: u32) { self.change(s, 0); }
    pub fn change(&mut self, rst: u32, s: u32) {
        let orig = self.ev;
        self.ev = (self.ev & !rst) | s;
        let ev = self.ev;
        if ev != orig { self.cbs.retain(|f| !f(ev)); }
    }
    pub fn sub(&mut self, cb: Box<dyn Fn(u32) -> bool + Send>) { self.cbs.push(cb); }
    pub fn cb_len(&self) -> usize { self.cbs.len() }
}

pub fn wait_ev(bus: &Arc<Mutex<EvBus>>, mask: u32) -> u32 {
    loop {
        { let g = bus.lock().unwrap(); if (g.ev & mask) != 0 { return g.ev; } }
        thread::yield_now();
    }
}
