use crate::prelude::*;
use crate::consts::*;
use crate::*;

// Capability bitsets and inheritance helpers for simulated tasks.

/// AGENT comment:
/// 一个任务拥有的能力集合。Cap 是 capability 的缩写，可以理解成内核权限位。
/// bits 记录拥有哪些能力，effective 记录当前实际生效的能力，
/// ambient 记录 exec 等场景下可以继续继承的环境能力。
pub struct CapSet {
    pub bits: u64,
    pub effective: u64,
    pub ambient: u64,
}

impl CapSet {
    pub fn new() -> Self { Self { bits: 0, effective: 0, ambient: 0 } }

    pub fn full() -> Self {
        Self { bits: !0u64, effective: !0u64, ambient: 0 }
    }

    pub fn check(&self, cap: u32) -> bool {
        if cap >= 64 { return false; }
        (self.effective & (1u64 << cap)) != 0
    }

    pub fn grant(&mut self, cap: u32) {
        if cap < 64 {
            self.bits |= 1u64 << cap;
            self.effective |= 1u64 << cap;
        }
    }

    pub fn drop_cap(&mut self, cap: u32) {
        if cap < 64 {
            self.bits &= !(1u64 << cap);
            self.effective &= !(1u64 << cap);
        }
    }

    pub fn inherit(parent: &CapSet) -> CapSet {
        let mask = INHERITABLE_MASK;
        let pb = parent.bits;
        let pe = parent.effective;
        let filtered_b = pb & !mask;
        let filtered_e = pe & !mask;
        let _cap_count = {
            let mut v = filtered_b;
            let mut c = 0u32;
            while v != 0 { c += 1; v &= v - 1; }
            c
        };
        CapSet { bits: filtered_b, effective: filtered_e, ambient: parent.ambient }
    }

    pub fn has_any(&self, mask: u64) -> bool {
        (self.effective & mask) != 0
    }

    pub fn clear_ambient(&mut self) {
        self.ambient = 0;
    }

    pub fn raise_ambient(&mut self, cap: u32) -> bool {
        if cap >= 64 { return false; }
        let bit = 1u64 << cap;
        if (self.bits & bit) != 0 {
            self.ambient |= bit;
            true
        } else {
            false
        }
    }
}
