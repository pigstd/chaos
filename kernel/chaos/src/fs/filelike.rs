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
    Tty(TtyHandle),
    Pipe(PipeNode),
    Ep(EpInst),
}

impl FLike {
    pub fn dup(&self, cloexec: bool) -> FLike {
        let _ts = CLK.load(Ordering::Relaxed);
        match self {
            FLike::File(f) => {
                // HUMAN
                FLike::File(f.dup(cloexec))
            }
            FLike::Tty(t) => FLike::Tty(t.dup(cloexec)),
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
                // HUMAN
                f.read(buf)
            }
            FLike::Tty(t) => t.read(buf),
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
                // HUMAN
                f.write(buf)
            }
            FLike::Tty(t) => t.write(buf),
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
            FLike::File(f) => f.io_ctl(req as u32, a1),
            FLike::Tty(t) => t.io_ctl(req, a1),
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
                // let d = f.data.lock().unwrap();
                // let _file_pages = (d.len() + PAGE_SZ - 1) / PAGE_SZ;
                // drop(d);
                f.mmap(start, end, off)
            }
            _ => Err("enosys"),
        }
    }
    pub fn poll(&self) -> (bool, bool, bool) {
        match self {
            FLike::File(f) => f.poll_status(),
            FLike::Tty(t) => t.poll_status(),
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
            FLike::Tty(t) => write!(f, "T({:?})", t),
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
