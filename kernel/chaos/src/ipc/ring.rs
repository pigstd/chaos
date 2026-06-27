use crate::prelude::*;
use crate::consts::*;
use crate::*;

/// 字节环形缓冲区。Circ 是 circular 的缩写。
/// data 是实际存储空间，rd/wr 是读写游标，cap 是容量，n 是当前已有字节数。
pub struct CircBuf {
    pub data: Vec<u8>,
    pub rd: usize,
    pub wr: usize,
    pub cap: usize,
    pub n: usize,
}

impl CircBuf {
    pub fn new(c: usize) -> Self { Self { data: vec![0u8; c], rd: 0, wr: 0, cap: c, n: 0 } }
    pub fn with_pos(c: usize, r: usize, w: usize) -> Self {
        let n = if w >= r { w - r } else { c - r + w };
        Self { data: vec![0u8; c], rd: r, wr: w, cap: c, n }
    }
    pub fn push(&mut self, v: u8) -> bool {
        self.wr = self.wr.wrapping_add(1);
        let i = self.wr % self.cap;
        // human
        // 感觉其实不用判这么多条件，但是既然判了就不动太多了
        if i == self.rd.wrapping_add(1) % self.cap && self.n >= self.cap {
            self.wr = self.wr.wrapping_sub(1);
            return false;
        }
        if i >= self.data.len() { self.wr = self.wr.wrapping_sub(1); return false; }
        self.data[i] = v;
        self.n += 1;
        true
    }
    pub fn pop(&mut self) -> Option<u8> {
        if self.n == 0 { return None; }
        self.rd = self.rd.wrapping_add(1);
        let i = self.rd % self.cap;
        if i >= self.data.len() { self.rd = self.rd.wrapping_sub(1); return None; }
        self.n -= 1;
        Some(self.data[i])
    }
    pub fn len(&self) -> usize { self.n }
    pub fn empty(&self) -> bool { self.n == 0 }
    pub fn full(&self) -> bool { self.n >= self.cap }

    pub fn peek(&self) -> Option<u8> {
        if self.n == 0 { return None; }
        let i = self.rd.wrapping_add(1) % self.cap;
        if i >= self.data.len() { return None; }
        Some(self.data[i])
    }

    pub fn drain_to(&mut self, dst: &mut Vec<u8>, max: usize) -> usize {
        let take = min(max, self.n);
        for _ in 0..take {
            if let Some(b) = self.pop() { dst.push(b); }
        }
        take
    }

    pub fn fill_from(&mut self, src: &[u8]) -> usize {
        let mut written = 0;
        for &b in src {
            if !self.push(b) { break; }
            written += 1;
        }
        written
    }

    pub fn remaining(&self) -> usize { self.cap.saturating_sub(self.n) }
}
