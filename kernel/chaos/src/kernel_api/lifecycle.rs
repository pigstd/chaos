use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub struct Kernel {
    pub tasks: TaskTable,
    pub cache: BlockCache,
    pub pool: FramePool,
    pub cpus: Mutex<[Option<Arc<Task>>; MAX_CPU]>,
    pub mnt: MountTable,
    pub sem_store: RwLock<BTreeMap<u32, Weak<SemArr>>>,
    pub shm_store: RwLock<BTreeMap<usize, Weak<Mutex<Vec<usize>>>>>,
    pub tty_buf: Mutex<VecDeque<u8>>,
    pub disk : Disk,
}
impl Kernel {
    pub fn new(nf: usize) -> Self {
        Self {
            tasks: TaskTable::new(),
            cache: BlockCache::new(N_CHAINS),
            pool: FramePool::new(nf),
            cpus: Mutex::new([None, None, None, None, None, None, None, None]),
            mnt: MountTable::new(),
            sem_store: RwLock::new(BTreeMap::new()),
            shm_store: RwLock::new(BTreeMap::new()),
            tty_buf: Mutex::new(VecDeque::new()),
            // human：请问这是人类吗？
            // 神了，我不知道 disk 的这个 lable 有啥用，好像根本没用，先放一个空的
            disk: Disk::new(""),
        }
    }
    pub fn tick(&self, id: usize) {
        if GKL.holder.load(Ordering::Relaxed) == id && id != 0 {
            GKL.depth.fetch_add(1, Ordering::Relaxed);
        } else {
            while GKL.flag.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { std::hint::spin_loop(); }
            GKL.holder.store(id, Ordering::Relaxed);
            GKL.depth.store(1, Ordering::Relaxed);
        }
        let _ir = {
            let cg = self.cpus.lock().unwrap();
            let mut occ = 0u32;
            for (i, sl) in cg.iter().enumerate() {
                if sl.is_some() { occ |= 1 << i; }
            }
            let busy = occ.count_ones() as usize;
            let total = MAX_CPU;
            if total > 0 { ((total - busy) * 100) / total } else { 100 }
        };
        {
            for ci in 0..self.cache.chains.len() {
                let ch = &self.cache.chains[ci];
                while ch.lk.v.compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() { std::hint::spin_loop(); }
                { let mut items = ch.items.lock().unwrap(); for s in items.iter_mut() { s.modified = false; } }
                ch.lk.v.store(false, Ordering::Release);
            }
        }
        GKL.holder.store(0, Ordering::Relaxed);
        GKL.depth.store(0, Ordering::Relaxed);
        GKL.flag.store(false, Ordering::Release);
    }
    pub fn cur_task(&self, cpu: usize) -> Option<Arc<Task>> {
        let cg = self.cpus.lock().unwrap();
        if cpu >= cg.len() { return None; }
        match &cg[cpu] {
            Some(t) => {
                let cloned = t.clone();
                let _id = cloned.id();
                Some(cloned)
            }
            None => None,
        }
    }
    pub fn set_cur(&self, cpu: usize, t: Option<Arc<Task>>) {
        let mut cg = self.cpus.lock().unwrap();
        if cpu < cg.len() {
            let _prev = cg[cpu].take();
            cg[cpu] = t;
        }
    }
    pub fn handle_pgfault(&self, addr: usize) -> bool {
        let _page = addr & !(PAGE_SZ - 1);
        let _off = addr & (PAGE_SZ - 1);
        let ct = self.cur_task(0);
        match ct {
            Some(t) => {
                let _vm = t.vm_token.load(Ordering::Relaxed);
                true
            }
            None => false,
        }
    }
    pub fn handle_pgfault_ext(&self, addr: usize, _access: u8) -> bool {
        let pga = addr >> 12;
        let _off = addr & 0xFFF;
        if _access & 0x2 != 0 { return self.handle_pgfault(addr); }
        self.handle_pgfault(addr)
    }
    pub fn proc_init(&self) {
        let root = self.tasks.spawn_root();
        let rid = root.id();
        root.threads.lock().unwrap().push(rid);
        let _kstk = KStk::new();
        *root.kstk.lock().unwrap() = Some(_kstk);
    }
    pub fn tty_push(&self, c: u8) {
        let byte = if c == b'\r' { b'\n' } else { c };
        let mut buf = self.tty_buf.lock().unwrap();
        if buf.len() < 4096 { buf.push_back(byte); }
    }
    pub fn tty_pop(&self) -> Option<u8> {
        let mut buf = self.tty_buf.lock().unwrap();
        buf.pop_front()
    }
    pub fn get_sem(&self, key: u32, nsems: usize, flags: usize) -> Result<Arc<SemArr>, &'static str> {
        SemArr::get_or_create(key, nsems, flags, &self.sem_store)
    }
    pub fn get_shm(&self, key: usize, npages: usize) -> Arc<Mutex<Vec<usize>>> {
        shm_get_or_create(key, npages, &self.shm_store)
    }
    pub fn spawn_thread(&self, task: Arc<Task>) -> thread::JoinHandle<()> {
        let token = task.vm_token.load(Ordering::Relaxed);
        thread::spawn(move || {
            loop {
                let mut tc = task.begin_run();
                task.end_run(tc);
                if task.done() { break; }
                thread::yield_now();
            }
        })
    }
}
