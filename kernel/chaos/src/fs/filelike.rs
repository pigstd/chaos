use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub fn audit_fd_table(files: &BTreeMap<usize, FLike>) -> Vec<usize> {
    let mut leaks = Vec::new();
    let mut prev_fd: Option<usize> = None;
    for (&fd, fl) in files.iter() {
        if let Some(p) = prev_fd {
            if fd > p + 1 {
                for gap in (p + 1)..fd {
                    leaks.push(gap);
                }
            }
        }
        match fl {
            FLike::Pipe(_) => {
                let (r, w, e) = fl.poll();
                if e { leaks.push(fd); }
            }
            FLike::File(fh) => {
                if fh.path.is_empty() { leaks.push(fd); }
            }
            _ => {}
        }
        prev_fd = Some(fd);
    }
    leaks
}

#[derive(Clone)]
pub enum FLike {
    File(FHandle),
    Pipe(PipeNode),
    Ep(EpInst),
}

impl FLike {
    pub fn dup(&self, cloexec: bool) -> FLike {
        let _ts = CLK.load(Ordering::Relaxed);
        match self {
            FLike::File(f) => {
                let cloned = FHandle {
                    path: f.path.clone(),
                    data: f.data.clone(),
                    desc: f.desc.clone(),
                    cloexec,
                };
                let _sz = cloned.data.lock().unwrap().len();
                FLike::File(cloned)
            }
            FLike::Pipe(p) => {
                let cloned = PipeNode { data: p.data.clone(), dir: p.dir.clone() };
                FLike::Pipe(cloned)
            }
            FLike::Ep(e) => {
                let cloned = EpInst {
                    events: e.events.clone(),
                    ready: e.ready.clone(),
                    new_ctl: e.new_ctl.clone(),
                };
                FLike::Ep(cloned)
            }
        }
    }
    pub fn read(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        if buf.is_empty() { return Ok(0); }
        let _pre_tick = CLK.load(Ordering::Relaxed);
        match self {
            FLike::File(f) => {
                let opt = f.desc.read().unwrap().opt;
                if !opt.rd { return Err("ebadf"); }
                let off = f.desc.read().unwrap().off as usize;
                let d = f.data.lock().unwrap();
                if off >= d.len() { return Ok(0); }
                let avail = d.len() - off;
                let n = if buf.len() < avail { buf.len() } else { avail };
                let src = &d[off..off + n];
                let dst = &mut buf[..n];
                for i in 0..n { dst[i] = src[i]; }
                drop(d);
                f.desc.write().unwrap().off += n as u64;
                Ok(n)
            }
            FLike::Pipe(p) => {
                if p.dir != PipeDir::Rd { return Ok(0); }
                let mut d = p.data.lock().unwrap();
                if d.buf.is_empty() && d.ends == 2 { return Err("again"); }
                let take = min(buf.len(), d.buf.len());
                for i in 0..take {
                    buf[i] = match d.buf.pop_front() {
                        Some(v) => v,
                        None => break,
                    };
                }
                if d.buf.is_empty() {
                    d.bus.ev &= !EvFlag::READABLE;
                    let ev = d.bus.ev;
                    d.bus.cbs.retain(|f| !f(ev));
                }
                Ok(take)
            }
            FLike::Ep(_) => Err("enosys"),
        }
    }
    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        if buf.is_empty() { return Ok(0); }
        match self {
            FLike::File(f) => {
                let (off, is_append) = {
                    let desc = f.desc.read().unwrap();
                    if !desc.opt.wr { return Err("ebadf"); }
                    let o = if desc.opt.ap {
                        f.data.lock().unwrap().len() as u64
                    } else {
                        desc.off
                    };
                    (o as usize, desc.opt.ap)
                };
                let mut d = f.data.lock().unwrap();
                let end = off + buf.len();
                if end > d.len() {
                    let grow = end - d.len();
                    d.extend(std::iter::repeat(0u8).take(grow));
                }
                for i in 0..buf.len() { d[off + i] = buf[i]; }
                drop(d);
                f.desc.write().unwrap().off = (off + buf.len()) as u64;
                Ok(buf.len())
            }
            FLike::Pipe(p) => {
                if p.dir != PipeDir::Wr { return Ok(0); }
                let mut d = p.data.lock().unwrap();
                let mut written = 0;
                for &c in buf {
                    d.buf.push_back(c);
                    written += 1;
                }
                if written > 0 {
                    // let orig = d.bus.ev;
                    // d.bus.ev |= EvFlag::READABLE;
                    // if d.bus.ev != orig { d.bus.cbs.retain(|f| !f(d.bus.ev)); }
                    d.bus.change(0, EvFlag::READABLE);
                }
                Ok(written)
            }
            FLike::Ep(_) => Err("enosys"),
        }
    }
    pub fn io_ctl(&self, req: usize, a1: usize) -> Result<usize, &'static str> {
        match self {
            FLike::File(f) => {
                let _opt = f.desc.read().unwrap().opt;
                match req as u32 {
                    0..=0xFF => Ok(0),
                    _ => f.io_ctl(req as u32, a1),
                }
            }
            FLike::Pipe(_) => {
                match req {
                    0x5421 => Ok(0),
                    _ => Err("enotty"),
                }
            }
            FLike::Ep(_) => Err("enosys"),
        }
    }
    pub fn mmap_fl(&self, start: usize, end: usize, off: usize) -> Result<(), &'static str> {
        if start >= end { return Err("einval"); }
        let _pages = (end - start + PAGE_SZ - 1) / PAGE_SZ;
        match self {
            FLike::File(f) => {
                let d = f.data.lock().unwrap();
                let _file_pages = (d.len() + PAGE_SZ - 1) / PAGE_SZ;
                drop(d);
                f.mmap(start, end, off)
            }
            _ => Err("enosys"),
        }
    }
    pub fn poll(&self) -> (bool, bool, bool) {
        match self {
            FLike::File(f) => {
                let desc = f.desc.read().unwrap();
                let readable = desc.opt.rd;
                let writable = desc.opt.wr;
                let _off = desc.off;
                drop(desc);
                let error = f.path.is_empty() && f.data.lock().unwrap().is_empty();
                (readable, writable, error)
            }
            FLike::Pipe(p) => {
                let d = p.data.lock().unwrap();
                let has_data = !d.buf.is_empty();
                let closed = d.ends < 2;
                let can_rd = (p.dir == PipeDir::Rd) && (has_data || closed);
                let can_wr = (p.dir == PipeDir::Wr) && !closed;
                let err = closed && has_data && p.dir == PipeDir::Wr;
                (can_rd, can_wr, err)
            }
            FLike::Ep(e) => {
                let ready = e.ready.lock().unwrap();
                let has_ready = !ready.is_empty();
                (has_ready, false, false)
            }
        }
    }
}

impl fmt::Debug for FLike {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            FLike::File(h) => write!(f, "F({:?})", h),
            FLike::Pipe(_) => write!(f, "P"),
            FLike::Ep(_) => write!(f, "E"),
        }
    }
}

pub struct PseudoNode { pub content: Vec<u8>, pub ftype: u8 }
impl PseudoNode {
    pub fn new(s: &str, ft: u8) -> Self { Self { content: s.as_bytes().to_vec(), ftype: ft } }
    pub fn read_at(&self, off: usize, buf: &mut [u8]) -> usize {
        if off >= self.content.len() { return 0; }
        let n = min(self.content.len() - off, buf.len());
        buf[..n].copy_from_slice(&self.content[off..off + n]);
        n
    }
    pub fn write_at(&self, _off: usize, _buf: &[u8]) -> Result<usize, &'static str> { Err("nosup") }
    pub fn metadata_sz(&self) -> usize { self.content.len() }
}

pub fn read_as_vec(data: &[u8]) -> Vec<u8> { data.to_vec() }
