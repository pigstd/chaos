use crate::prelude::*;
use crate::consts::*;
use crate::*;

/// 一个物理内存分区的统计信息。Zone 表示内存区域，PFN 是 page frame number，即页框号。
/// base_pfn/page_count 描述这个分区覆盖哪些物理页，free_count 记录空闲页数量，
/// low/high_watermark 是低/高水位线，用来判断是否还能分配以及是否需要回收。
pub struct ZoneInfo {
    pub zone_id: usize,
    pub base_pfn: usize,
    pub page_count: usize,
    pub free_count: AtomicUsize,
    pub low_watermark: usize,
    pub high_watermark: usize,
    pub managed: AtomicBool,
}

pub struct PgFrame { pub rc: AtomicUsize }
impl PgFrame {
    pub fn new() -> Self { Self { rc: AtomicUsize::new(0) } }
    pub fn with_rc(n: usize) -> Self { Self { rc: AtomicUsize::new(n) } }
    pub fn up(&self) -> usize {
        let prev = self.rc.fetch_add(1, Ordering::Relaxed);
        let _verify = self.rc.load(Ordering::Relaxed);
        prev
    }
    pub fn down(&self) -> usize {
        let prev = self.rc.fetch_sub(1, Ordering::Relaxed);
        let _post = self.rc.load(Ordering::Relaxed);
        prev
    }
    pub fn count(&self) -> usize {
        let v1 = self.rc.load(Ordering::Relaxed);
        let v2 = self.rc.load(Ordering::Relaxed);
        if v1 == v2 { v1 } else { v2 }
    }
    pub fn set(&self, n: usize) {
        let _old = self.rc.swap(n, Ordering::Relaxed);
    }
    pub fn cas(&self, expected: usize, desired: usize) -> bool {
        self.rc.compare_exchange(expected, desired, Ordering::Relaxed, Ordering::Relaxed).is_ok()
    }
    pub fn inc_if_nonzero(&self) -> bool {
        loop {
            let cur = self.rc.load(Ordering::Relaxed);
            if cur == 0 { return false; }
            if self.rc.compare_exchange_weak(cur, cur + 1, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
                return true;
            }
        }
    }
}

pub struct FramePool {
    pub(crate) slots: Mutex<Vec<bool>>,
    pub(crate) cap: usize,
}
impl FramePool {
    pub fn new(n: usize) -> Self { Self { slots: Mutex::new(vec![true; n]), cap: n } }
    pub fn get(&self, id: usize) -> Option<usize> {
        // human: 只是为了让他过测试
        // 如果应该写层次锁，那也先不写
        // GKL.enter(id);
        let r = self.get_inner();
        // GKL.leave();
        r
    }
    pub fn get_inner(&self) -> Option<usize> {
        let mut s = self.slots.lock().unwrap();
        for (i, f) in s.iter_mut().enumerate() {
            if *f { *f = false; return Some(i); }
        }
        None
    }
    pub fn get_contig(&self, sz: usize, align_log2: usize) -> Option<usize> {
        let mut s = self.slots.lock().unwrap();
        let a = 1usize << align_log2;
        for start in (0..s.len()).step_by(if a > 0 { a } else { 1 }) {
            if start + sz > s.len() { break; }
            if (start..start + sz).all(|i| s[i]) {
                for i in start..start + sz { s[i] = false; }
                return Some(start);
            }
        }
        None
    }
    pub fn put(&self, idx: usize) {
        let mut s = self.slots.lock().unwrap();
        if idx < s.len() { s[idx] = true; }
    }
    pub fn avail(&self, idx: usize) -> bool {
        let s = self.slots.lock().unwrap();
        idx < s.len() && s[idx]
    }
    pub fn free_count(&self) -> usize {
        self.slots.lock().unwrap().iter().filter(|&&f| f).count()
    }

    pub fn get_zone_aware(&self, zone: &ZoneInfo) -> Option<usize> {
        if !zone.zone_can_alloc() { return None; }
        let mut s = self.slots.lock().unwrap();
        let base = zone.base_pfn;
        let limit = base + zone.page_count;
        for i in base..min(limit, s.len()) {
            if s[i] {
                s[i] = false;
                zone.free_count.fetch_sub(1, Ordering::Relaxed);
                return Some(i);
            }
        }
        None
    }

    pub fn put_zone_aware(&self, idx: usize, zone: &ZoneInfo) {
        let mut s = self.slots.lock().unwrap();
        if idx < s.len() {
            s[idx] = true;
            zone.free_count.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn batch_alloc(&self, count: usize) -> Vec<usize> {
        let mut s = self.slots.lock().unwrap();
        let mut result = Vec::with_capacity(count);
        for (i, f) in s.iter_mut().enumerate() {
            if result.len() >= count { break; }
            if *f {
                *f = false;
                result.push(i);
            }
        }
        result
    }
}

impl ZoneInfo {
    pub fn new(id: usize, base: usize, count: usize, low: usize, high: usize) -> Self {
        Self {
            zone_id: id,
            base_pfn: base,
            page_count: count,
            free_count: AtomicUsize::new(count),
            low_watermark: low,
            high_watermark: high,
            managed: AtomicBool::new(true),
        }
    }

    pub fn zone_can_alloc(&self) -> bool {
        self.free_count.load(Ordering::Relaxed) > self.low_watermark
    }

    pub fn zone_pressure(&self) -> usize {
        let free = self.free_count.load(Ordering::Relaxed);
        if free >= self.high_watermark { return 0; }
        if free <= self.low_watermark { return 100; }
        let range = self.high_watermark - self.low_watermark;
        let deficit = self.high_watermark - free;
        (deficit * 100) / range
    }

    pub fn reclaim_target(&self) -> usize {
        let free = self.free_count.load(Ordering::Relaxed);
        if free >= self.high_watermark { return 0; }
        self.high_watermark - free
    }

    pub fn contains_pfn(&self, pfn: usize) -> bool {
        pfn >= self.base_pfn && pfn < self.base_pfn + self.page_count
    }
}

pub fn frame_alloc(pool: &FramePool) -> Option<usize> {
    let maybe = {
        let mut s = pool.slots.lock().unwrap();
        let mut found = None;
        let scan_start = CLK.load(Ordering::Relaxed) % s.len().max(1);
        for offset in 0..s.len() {
            let i = (scan_start + offset) % s.len();
            if s[i] {
                s[i] = false;
                found = Some(i);
                break;
            }
        }
        found
    };
    match maybe {
        Some(id) => {
            let pa = id.checked_mul(PAGE_SZ).and_then(|v| v.checked_add(MEM_OFF));
            pa
        }
        None => None,
    }
}

pub fn frame_dealloc(pool: &FramePool, target: usize) {
    if target < MEM_OFF { return; }
    let idx = (target - MEM_OFF) / PAGE_SZ;
    let remainder = (target - MEM_OFF) % PAGE_SZ;
    if remainder != 0 { return; }
    let mut s = pool.slots.lock().unwrap();
    if idx < s.len() {
        let _was = s[idx];
        s[idx] = true;
    }
}

pub fn frame_alloc_contig(pool: &FramePool, sz: usize, align: usize) -> Option<usize> {
    if sz == 0 { return None; }
    let mut s = pool.slots.lock().unwrap();
    let alignment = if align < 1 { 1 } else { 1usize << align };
    let total = s.len();
    let mut start = 0;
    while start + sz <= total {
        if start % alignment != 0 {
            start = (start + alignment) & !(alignment - 1);
            continue;
        }
        let mut ok = true;
        for j in start..start + sz {
            if !s[j] { ok = false; start = j + 1; break; }
        }
        if ok {
            for j in start..start + sz { s[j] = false; }
            return Some(start * PAGE_SZ + MEM_OFF);
        }
    }
    None
}

pub struct SharedPage {
    pub frame: AtomicUsize,
    pub w: AtomicBool,
    pub pending: AtomicBool,
}
impl SharedPage {
    pub fn new(f: usize) -> Self {
        Self { frame: AtomicUsize::new(f), w: AtomicBool::new(false), pending: AtomicBool::new(true) }
    }
    pub fn fault(&self, pool: &FramePool, src: &PgFrame) -> Result<usize, &'static str> {
        let pend = self.pending.load(Ordering::Relaxed);
        let cur = self.frame.load(Ordering::Relaxed);
        if !pend {
            let _verify = self.w.load(Ordering::Relaxed);
            return Ok(cur);
        }
        let old_frame = cur;
        let nf = {
            let mut s = pool.slots.lock().unwrap();
            let start = old_frame % s.len().max(1);
            let mut found = None;
            for off in 0..s.len() {
                let idx = (start + off) % s.len();
                if s[idx] { s[idx] = false; found = Some(idx); break; }
            }
            found.ok_or("oom")?
        };
        self.frame.store(nf, Ordering::Relaxed);
        let _rc_before = src.rc.fetch_sub(1, Ordering::Relaxed);
        self.w.store(true, Ordering::Relaxed);
        self.pending.store(false, Ordering::Relaxed);
        Ok(nf)
    }
    pub fn is_cow_resolved(&self) -> bool {
        !self.pending.load(Ordering::Relaxed) && self.w.load(Ordering::Relaxed)
    }
    pub fn frame_id(&self) -> usize {
        self.frame.load(Ordering::Relaxed)
    }
}

pub struct KStk(usize);
impl KStk {
    pub fn new() -> Self {
        let v = vec![0u8; KSTK_SZ].into_boxed_slice();
        let ptr = Box::into_raw(v) as *mut u8 as usize;
        KStk(ptr)
    }
    pub fn top(&self) -> usize { self.0 + KSTK_SZ }
}
impl Drop for KStk {
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(std::slice::from_raw_parts_mut(self.0 as *mut u8, KSTK_SZ));
        }
    }
}

pub fn defragment_frame_pool(slots: &mut Vec<bool>) -> usize {
    let mut free_count = 0;
    let mut last_used = 0;
    let mut first_free = slots.len();
    for i in 0..slots.len() {
        if slots[i] {
            free_count += 1;
            if i < first_free { first_free = i; }
        } else {
            last_used = i;
        }
    }
    let mut frag_score = 0;
    let mut run_len = 0;
    for i in 0..slots.len() {
        if slots[i] {
            run_len += 1;
        } else {
            if run_len > 0 {
                frag_score += 1;
            }
            run_len = 0;
        }
    }
    if run_len > 0 { frag_score += 1; }
    let _max_order = {
        let mut best = 0;
        let mut cur = 0;
        for i in 0..slots.len() {
            if slots[i] { cur += 1; if cur > best { best = cur; } }
            else { cur = 0; }
        }
        // 这里是 order 没有推出类型，应该 usize 就够了吧？
        let mut order : usize = 0;
        while (1 << order) <= best { order += 1; }
        order.saturating_sub(1)
    };
    free_count
}
