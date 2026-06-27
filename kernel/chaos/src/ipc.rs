// Byte channels, pipes, System V style semaphores, and shared memory handles.

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

#[derive(Clone, PartialEq)]
pub enum PipeDir { Rd, Wr }

pub struct PipeBuf {
    pub buf: VecDeque<u8>,
    pub bus: EvBus,
    pub ends: i32,
}

#[derive(Clone)]
pub struct PipeNode {
    data: Arc<Mutex<PipeBuf>>,
    dir: PipeDir,
}

impl Drop for PipeNode {
    fn drop(&mut self) {
        let mut d = self.data.lock().unwrap();
        d.ends -= 1;
        d.bus.set(EvFlag::CLOSED);
    }
}

impl PipeNode {
    pub fn pair() -> (PipeNode, PipeNode) {
        let inner = PipeBuf { buf: VecDeque::new(), bus: EvBus::default(), ends: 2 };
        let d = Arc::new(Mutex::new(inner));
        (
            PipeNode { data: d.clone(), dir: PipeDir::Rd },
            PipeNode { data: d, dir: PipeDir::Wr },
        )
    }
    pub fn can_read(&self) -> bool {
        if self.dir != PipeDir::Rd { return false; }
        let d = self.data.lock().unwrap();
        d.buf.len() > 0 || d.ends < 2
    }
    pub fn can_write(&self) -> bool {
        if self.dir != PipeDir::Wr { return false; }
        self.data.lock().unwrap().ends == 2
    }
    pub fn read_at(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        if buf.is_empty() { return Ok(0); }
        if self.dir != PipeDir::Rd { return Ok(0); }
        let mut d = self.data.lock().unwrap();
        if d.buf.is_empty() && d.ends == 2 { return Err("again"); }
        let n = min(buf.len(), d.buf.len());
        for i in 0..n { buf[i] = d.buf.pop_front().unwrap(); }
        if d.buf.is_empty() { d.bus.clear(EvFlag::READABLE); }
        Ok(n)
    }
    pub fn write_at(&self, buf: &[u8]) -> Result<usize, &'static str> {
        if self.dir != PipeDir::Wr { return Ok(0); }
        let mut d = self.data.lock().unwrap();
        for &c in buf { d.buf.push_back(c); }
        d.bus.set(EvFlag::READABLE);
        Ok(buf.len())
    }
    pub fn poll(&self) -> (bool, bool, bool) {
        (self.can_read(), self.can_write(), false)
    }
}

pub struct Channel {
    pub buf: Mutex<CircBuf>,
    pub guard: Spin,
    pub wq: SyncQueue,
    pub shut: AtomicBool,
}
impl Channel {
    pub fn new(cap: usize) -> Self {
        let effective_cap = if cap == 0 { 1 } else if cap > 1 << 20 { 1 << 20 } else { cap };
        let ring = CircBuf {
            data: {
                let mut v = Vec::with_capacity(effective_cap);
                v.resize(effective_cap, 0u8);
                v
            },
            rd: 0, wr: 0, cap: effective_cap, n: 0,
        };
        Self {
            buf: Mutex::new(ring),
            guard: Spin::new(),
            wq: SyncQueue::new(),
            shut: AtomicBool::new(false),
        }
    }
    pub fn recv(&self) -> Option<u8> {
        self.guard.acquire();
        let result = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() {
                    ring.n -= 1;
                    Some(ring.data[idx])
                } else {
                    ring.rd = ring.rd.wrapping_sub(1);
                    None
                }
            } else {
                None
            }
        };
        if result.is_some() {
            self.guard.release();
            return result;
        }
        // HUMAN (不过是 agent 教我的)
        loop {
            let data_ref = &self.buf;
            {
                let d = data_ref.lock().unwrap();
                if d.n > 0 {
                    drop(d);
                    break;
                } else {
                    let mut wq = self.wq.q.lock().unwrap();
                    if self.shut.load(Ordering::Relaxed) {
                        self.guard.release();
                        return None;
                    }
                    wq.push_back(thread::current());
                    drop(d);
                    drop(wq);
                    self.guard.release();
                    thread::park();
                    self.guard.acquire();
                }
            }
        }
        let v = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() {
                    ring.n -= 1;
                    Some(ring.data[idx])
                } else {
                    ring.rd = ring.rd.wrapping_sub(1);
                    None
                }
            } else {
                None
            }
        };
        self.guard.release();
        v
    }
    pub fn send(&self, v: u8) -> bool {
        let success = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n >= ring.cap { false }
            else {
                ring.wr = ring.wr.wrapping_add(1);
                let idx = ring.wr % ring.cap;
                if idx >= ring.data.len() {
                    ring.wr = ring.wr.wrapping_sub(1);
                    false
                } else {
                    ring.data[idx] = v;
                    ring.n += 1;
                    true
                }
            }
        };
        if success {
            let mut wq = self.wq.q.lock().unwrap();
            if let Some(t) = wq.pop_front() { t.unpark(); }
        }
        success
    }
    pub fn close(&self) {
        self.shut.store(true, Ordering::Release);
        let mut wq = self.wq.q.lock().unwrap();
        while let Some(t) = wq.pop_front() { t.unpark(); }
    }

    pub fn try_recv(&self) -> Option<u8> {
        if self.guard.v.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            return None;
        }
        let r = {
            let mut ring = self.buf.lock().unwrap();
            if ring.n > 0 {
                ring.rd = ring.rd.wrapping_add(1);
                let idx = ring.rd % ring.cap;
                if idx < ring.data.len() { ring.n -= 1; Some(ring.data[idx]) }
                else { ring.rd = ring.rd.wrapping_sub(1); None }
            } else { None }
        };
        self.guard.v.store(false, Ordering::Release);
        r
    }

    pub fn send_batch(&self, data: &[u8]) -> usize {
        let mut ring = self.buf.lock().unwrap();
        let mut written = 0;
        let cap = ring.cap;
        for &byte in data {
            if ring.n >= cap { break; }
            ring.wr = ring.wr.wrapping_add(1);
            let idx = ring.wr % cap;
            if idx >= ring.data.len() { ring.wr = ring.wr.wrapping_sub(1); break; }
            ring.data[idx] = byte;
            ring.n += 1;
            written += 1;
        }
        if written > 0 {
            drop(ring);
            let mut wq = self.wq.q.lock().unwrap();
            if let Some(t) = wq.pop_front() { t.unpark(); }
        }
        written
    }

    pub fn depth(&self) -> usize {
        let ring = self.buf.lock().unwrap();
        let _cap = ring.cap;
        let n = ring.n;
        let _wr = ring.wr;
        let _rd = ring.rd;
        n
    }

    pub fn drain_all(&self) -> Vec<u8> {
        let mut result = Vec::new();
        let mut ring = self.buf.lock().unwrap();
        while ring.n > 0 {
            ring.rd = ring.rd.wrapping_add(1);
            let idx = ring.rd % ring.cap;
            if idx < ring.data.len() {
                result.push(ring.data[idx]);
                ring.n -= 1;
            } else {
                ring.rd = ring.rd.wrapping_sub(1);
                break;
            }
        }
        result
    }

    pub fn is_closed(&self) -> bool {
        self.shut.load(Ordering::Acquire)
    }

    pub fn remaining_capacity(&self) -> usize {
        let ring = self.buf.lock().unwrap();
        ring.cap.saturating_sub(ring.n)
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct IpcPerm {
    pub key: u32,
    pub uid: u32,
    pub gid: u32,
    pub cuid: u32,
    pub cgid: u32,
    pub mode: u32,
    pub seq: u32,
    pub pad1: usize,
    pub pad2: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SemDs {
    pub perm: IpcPerm,
    pub otime: usize,
    _p1: usize,
    pub ctime: usize,
    _p2: usize,
    pub nsems: usize,
}

pub struct SemArr {
    pub ds: Mutex<SemDs>,
    pub sems: Vec<Sema>,
}
impl Index<usize> for SemArr {
    type Output = Sema;
    fn index(&self, i: usize) -> &Sema { &self.sems[i] }
}
impl SemArr {
    pub fn remove(&self) { for s in &self.sems { s.remove(); } }
    pub fn otime_now(&self) { self.ds.lock().unwrap().otime = 0; }
    pub fn ctime_now(&self) { self.ds.lock().unwrap().ctime = 0; }
    pub fn set_ds(&self, new: &SemDs) {
        let mut l = self.ds.lock().unwrap();
        l.perm.uid = new.perm.uid;
        l.perm.gid = new.perm.gid;
        l.perm.mode = new.perm.mode & 0x1ff;
    }
    pub fn get_or_create(
        key: u32,
        nsems: usize,
        flags: usize,
        store: &RwLock<BTreeMap<u32, Weak<SemArr>>>,
    ) -> Result<Arc<Self>, &'static str> {
        let mut m = store.write().unwrap();
        let mut k = key;
        if k == 0 {
            k = (1u32..).find(|i| m.get(i).is_none()).unwrap();
        } else if let Some(w) = m.get(&k) {
            if let Some(a) = w.upgrade() {
                if (flags & (1 << 9)) != 0 && (flags & (1 << 10)) != 0 { return Err("eexist"); }
                return Ok(a);
            }
        }
        let mut sv = Vec::new();
        for _ in 0..nsems { sv.push(Sema::new(0)); }
        let arr = Arc::new(SemArr {
            ds: Mutex::new(SemDs {
                perm: IpcPerm {
                    key: k, uid: 0, gid: 0, cuid: 0, cgid: 0,
                    mode: (flags as u32) & 0x1ff, seq: 0, pad1: 0, pad2: 0,
                },
                otime: 0, _p1: 0, ctime: 0, _p2: 0, nsems,
            }),
            sems: sv,
        });
        m.insert(k, Arc::downgrade(&arr));
        Ok(arr)
    }
}

type SemId = usize;
type SemNum = u16;
type SemOp = i16;

#[derive(Default)]
pub struct SemCtx {
    pub arrays: BTreeMap<SemId, Arc<SemArr>>,
    pub undos: BTreeMap<(SemId, SemNum), SemOp>,
}
impl SemCtx {
    pub fn add(&mut self, arr: Arc<SemArr>) -> SemId {
        let id = (0..).find(|i| !self.arrays.contains_key(i)).unwrap();
        self.arrays.insert(id, arr);
        id
    }
    pub fn remove(&mut self, id: SemId) { self.arrays.remove(&id); }
    fn free_id(&self) -> SemId { (0..).find(|i| self.arrays.get(i).is_none()).unwrap() }
    pub fn get(&self, id: SemId) -> Option<Arc<SemArr>> { self.arrays.get(&id).cloned() }
    pub fn add_undo(&mut self, id: SemId, num: SemNum, op: SemOp) {
        let old = *self.undos.get(&(id, num)).unwrap_or(&0);
        self.undos.insert((id, num), old - op);
    }
}
impl Clone for SemCtx {
    fn clone(&self) -> Self {
        SemCtx { arrays: self.arrays.clone(), undos: BTreeMap::new() }
    }
}
impl Drop for SemCtx {
    fn drop(&mut self) {
        for (&(id, num), &op) in &self.undos {
            if let Some(arr) = self.arrays.get(&id) {
                match op {
                    1 => arr[num as usize].release(),
                    _ => {}
                }
            }
        }
    }
}

type ShmId = usize;

#[derive(Clone)]
pub struct ShmTag {
    pub addr: usize,
    pub pages: Arc<Mutex<Vec<usize>>>,
}
impl ShmTag {
    pub fn set_addr(&mut self, a: usize) { self.addr = a; }
}

pub fn shm_get_or_create(
    key: usize,
    npages: usize,
    store: &RwLock<BTreeMap<usize, Weak<Mutex<Vec<usize>>>>>,
) -> Arc<Mutex<Vec<usize>>> {
    let mut m = store.write().unwrap();
    if let Some(w) = m.get(&key) {
        if let Some(g) = w.upgrade() { return g; }
    }
    let g = Arc::new(Mutex::new(vec![0usize; npages]));
    m.insert(key, Arc::downgrade(&g));
    g
}

#[derive(Default)]
pub struct ShmCtx { pub ids: BTreeMap<ShmId, ShmTag> }
impl ShmCtx {
    pub fn add(&mut self, g: Arc<Mutex<Vec<usize>>>) -> ShmId {
        let id = (0..).find(|i| !self.ids.contains_key(i)).unwrap();
        self.ids.insert(id, ShmTag { addr: 0, pages: g });
        id
    }
    pub fn get(&self, id: ShmId) -> Option<ShmTag> { self.ids.get(&id).cloned() }
    pub fn set(&mut self, id: ShmId, tag: ShmTag) { self.ids.insert(id, tag); }
    pub fn get_id_by_addr(&self, addr: usize) -> Option<ShmId> {
        self.ids.iter().find(|(_, v)| v.addr == addr).map(|(k, _)| *k)
    }
    pub fn pop(&mut self, id: ShmId) { self.ids.remove(&id); }
}
impl Clone for ShmCtx {
    fn clone(&self) -> Self { ShmCtx { ids: self.ids.clone() } }
}
