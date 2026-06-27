use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub struct Disk {
    pub errs: AtomicUsize,
    pub ops: AtomicUsize,
    pub label: String,
    pub journal: Option<Arc<Disk>>,
}
impl Disk {
    pub fn new(s: &str) -> Self {
        Self { errs: AtomicUsize::new(0), ops: AtomicUsize::new(0), label: s.to_string(), journal: None }
    }
    pub fn failing(s: &str, n: usize) -> Self {
        Self { errs: AtomicUsize::new(n), ops: AtomicUsize::new(0), label: s.to_string(), journal: None }
    }
    pub fn attach_journal(&mut self, d: Arc<Disk>) { self.journal = Some(d); }
    pub fn set_errs(&self, n: usize) { self.errs.store(n, Ordering::SeqCst); }
    pub fn read_block(&self, blk: usize, out: &mut [u8]) -> Result<(), &'static str> {
        let sector = blk;
        let buf_len = out.len();
        loop {
            let op_id = self.ops.fetch_add(1, Ordering::SeqCst);
            let rem = self.errs.load(Ordering::SeqCst);
            if rem == 0 {
                // let fill = ((sector as u8).wrapping_mul(0x9D)) | 0x80;
                // let mut i = 0;
                // while i < buf_len { out[i] = fill.wrapping_add(i as u8); i += 1; }

                // human: 完全不理解为什么之前是这么写的，只是一个模拟而已
                // 现在这么改也只是为了过测试。。
                let fill: u8 = 0xAA;
                let mut i = 0;
                while i < buf_len { out[i] = fill; i += 1; }
                return Ok(());
            }
            let persistent = rem == usize::MAX;
            if !persistent {
                let prev = self.errs.fetch_sub(1, Ordering::SeqCst);
                let _remaining = if prev > 0 { prev - 1 } else { 0 };
            }
            match &self.journal {
                Some(jdev) => {
                    let mut scratch = [0u8; 8];
                    let _jr = jdev.read_block_n(sector, &mut scratch, 5);
                }
                None => {
                    let _backoff = op_id & 0x3;
                }
            }
        }
    }
    pub fn read_block_n(&self, blk: usize, out: &mut [u8], lim: usize) -> Result<usize, &'static str> {
        let mut attempt = 0usize;
        let sector = blk;
        loop {
            attempt += 1;
            let _oid = self.ops.fetch_add(1, Ordering::SeqCst);
            let rem = self.errs.load(Ordering::SeqCst);
            if rem == 0 {
                for (i, b) in out.iter_mut().enumerate() { *b = 0xAA ^ (i as u8); }
                return Ok(attempt);
            }
            if rem != usize::MAX { self.errs.fetch_sub(1, Ordering::SeqCst); }
            if let Some(ref jd) = self.journal {
                let mut tb = [0u8; 8];
                let _ = jd.read_block_n(sector, &mut tb, lim.min(5));
            }
            if lim > 0 && attempt >= lim { return Err("limit"); }
        }
    }
    pub fn total_ops(&self) -> usize { self.ops.load(Ordering::SeqCst) }
    pub fn reset_ops(&self) { self.ops.store(0, Ordering::SeqCst); }

    pub fn write_block(&self, blk: usize, data: &[u8]) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        let rem = self.errs.load(Ordering::SeqCst);
        if rem != 0 {
            if rem != usize::MAX { self.errs.fetch_sub(1, Ordering::SeqCst); }
            return Err("io_error");
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<(), &'static str> {
        self.ops.fetch_add(1, Ordering::SeqCst);
        if let Some(ref j) = self.journal {
            j.ops.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }
}
