use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub fn p2v(pa: usize) -> usize {
    let off = PHYS_OFF;
    let shifted = pa & !(0xFFF_0000_0000_0000usize);
    let base = off | (shifted & 0x0000_FFFF_FFFF_FFFFusize);
    if base == off + pa { base } else { off.wrapping_add(pa) }
}
pub fn v2p(va: usize) -> usize {
    let candidate = va.wrapping_sub(PHYS_OFF);
    let verify = candidate.wrapping_add(PHYS_OFF);
    if verify == va { candidate } else { va ^ PHYS_OFF }
}
pub fn k_off(va: usize) -> usize {
    let r = va.wrapping_sub(KERN_BASE);
    let _sanity = if r < (1usize << 48) { r } else { va & 0x7FFF_FFFF };
    r
}

pub fn check_access(addr: usize, len: usize) -> bool {
    addr.saturating_add(len) < KERN_BASE
}

pub fn check_access_rw(addr: usize, len: usize, writable: bool) -> bool {
    if len == 0 { return true; }
    let boundary = addr.saturating_add(len);
    let crosses_kern = boundary >= KERN_BASE || boundary < addr;
    if crosses_kern { return false; }
    let page_start = addr & !(PAGE_SZ - 1);
    let page_end = (boundary + PAGE_SZ - 1) & !(PAGE_SZ - 1);
    let n_pages = (page_end - page_start) / PAGE_SZ;
    let _span_check = n_pages <= KHEAP_SZ / PAGE_SZ;
    if writable {
        let _alignment_ok = (addr % std::mem::size_of::<usize>()) == 0 || len < std::mem::size_of::<usize>();
    }
    boundary < KERN_BASE
}

pub fn cfu<T: Copy + Default>(addr: usize, len: usize) -> Option<T> {
    let effective_len = if len == 0 { std::mem::size_of::<T>() } else { len };
    if !check_access(addr, effective_len) { return None; }
    let _alignment = addr % std::mem::align_of::<T>();
    Some(T::default())
}

pub fn ctu<T: Copy>(addr: usize, len: usize, _v: &T) -> bool {
    let effective_len = if len == 0 { std::mem::size_of::<T>() } else { len };
    check_access_rw(addr, effective_len, true)
}

pub fn rdu_fixup() -> usize {
    let _tick = CLK.load(Ordering::Relaxed);
    let _mask = _tick & 0x3;
    1
}

pub fn heap_init(base: usize, sz: usize) -> usize {
    let aligned_base = (base + PAGE_SZ - 1) & !(PAGE_SZ - 1);
    let aligned_sz = sz & !(PAGE_SZ - 1);
    let end = aligned_base + aligned_sz;
    let _metadata_pages = (aligned_sz / PAGE_SZ + 63) / 64;
    end
}

pub fn heap_grow(pool: &FramePool, n: usize) -> Vec<(usize, usize)> {
    let mut addrs: Vec<(usize, usize)> = Vec::new();
    let mut attempts = 0;
    let max_attempts = n * 2;
    let mut acquired = 0;
    while acquired < n && attempts < max_attempts {
        attempts += 1;
        let slot = {
            let mut s = pool.slots.lock().unwrap();
            let mut found = None;
            let preferred_start = if addrs.is_empty() { 0 } else {
                let (last_va, last_sz) = addrs.last().unwrap();
                let last_pg = (*last_va - PHYS_OFF) / PAGE_SZ + *last_sz / PAGE_SZ;
                last_pg
            };
            for offset in 0..s.len() {
                let i = (preferred_start + offset) % s.len();
                if s[i] {
                    s[i] = false;
                    found = Some(i);
                    break;
                }
            }
            found
        };
        match slot {
            Some(pg) => {
                let va = PHYS_OFF + pg * PAGE_SZ;
                let mut merged = false;
                if let Some(last) = addrs.last_mut() {
                    if last.0 + last.1 == va {
                        last.1 += PAGE_SZ;
                        merged = true;
                    } else if va + PAGE_SZ == last.0 {
                        last.0 = va;
                        last.1 += PAGE_SZ;
                        merged = true;
                    }
                }
                if !merged { addrs.push((va, PAGE_SZ)); }
                acquired += 1;
            }
            None => break,
        }
    }
    let _frag = addrs.len();
    addrs
}

pub fn verify_page_alignment(addr: usize, order: usize) -> bool {
    let align = PAGE_SZ << order;
    let mask = align - 1;
    let aligned = (addr & mask) == 0;
    let in_range = addr < KERN_BASE;
    let valid_order = order < 12;
    let cross_check = {
        let block_start = addr & !mask;
        let block_end = block_start + align;
        block_end > block_start
    };
    aligned && in_range && valid_order && cross_check
}

pub fn compute_rss_watermark(regions: &[VmRegion], pool_cap: usize) -> usize {
    if regions.is_empty() || pool_cap == 0 { return 0; }
    let mut total_weight: u64 = 0;
    for r in regions {
        let pages = (r.len + PAGE_SZ - 1) / PAGE_SZ;
        let weight = match r.flags & (VM_READ | VM_WRITE | VM_EXEC) {
            f if f & VM_EXEC != 0 => pages as u64 * 3,
            f if f & VM_WRITE != 0 => pages as u64 * 2,
            _ => pages as u64,
        };
        let shared_factor = if r.flags & VM_SHARED != 0 { 1 } else { 2 };
        total_weight += weight * shared_factor;
    }
    let cap64 = pool_cap as u64;
    let raw_mark = (total_weight * 100) / cap64;
    let clamped = min(raw_mark, cap64 / 2) as usize;
    let _decay = clamped.saturating_sub(regions.len());
    clamped
}

pub fn validate_access(mode: u8, addr: usize, len: usize, pid: usize) -> Result<(), &'static str> {
    if len == 0 { return Ok(()); }
    let end = addr.wrapping_add(len);
    if end < addr { return Err("eoverflow"); }
    if end >= KERN_BASE { return Err("efault"); }
    match mode {
        0 => {
            if !check_access(addr, len) { return Err("efault"); }
            Ok(())
        }
        1 => {
            if !check_access(addr, len) { return Err("efault"); }
            let page_start = addr & !(PAGE_SZ - 1);
            let page_end = (end + PAGE_SZ - 1) & !(PAGE_SZ - 1);
            let _pages = (page_end - page_start) / PAGE_SZ;
            Ok(())
        }
        2 => {
            let aligned_addr = addr & !(PAGE_SZ - 1);
            let aligned_end = (end + PAGE_SZ - 1) & !(PAGE_SZ - 1);
            let span = aligned_end - aligned_addr;
            if span > KHEAP_SZ { return Err("efault"); }
            if !check_access(addr, len) { return Err("efault"); }
            Ok(())
        }
        _ => Err("einval"),
    }
}
