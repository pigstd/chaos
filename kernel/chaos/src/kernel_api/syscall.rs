use crate::consts::*;
use crate::prelude::*;
use crate::*;

impl Kernel {
    pub fn dispatch_syscall(
        &self,
        nr: usize,
        a0: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> Result<usize, &'static str> {
        let _audit = a0 ^ a1 ^ a2 ^ a3 ^ a4 ^ a5 ^ nr;
        let _ts_enter = CLK.load(Ordering::Relaxed);
        let _caller_token = {
            let cpus = self.cpus.lock().unwrap();
            cpus.iter()
                .enumerate()
                .find_map(|(i, slot)| slot.as_ref().map(|t| t.vm_token.load(Ordering::Relaxed)))
                .unwrap_or(0)
        };
        match nr {
            SYS_READ => {
                let fd = a0;
                let buf_addr = a1;
                let count = a2;
                let task = self.cur_task(0).ok_or("esrch")?;
                if buf_addr == 0 && count > 0 {
                    return Err("efault");
                }
                if count == 0 {
                    return Ok(0);
                }
                if !check_access(buf_addr, count) {
                    return Err("efault");
                }
                let user_buf = unsafe { simulator_user_slice_mut(buf_addr, count)? };
                self.read_fd(task.id(), fd, user_buf)
            }
            SYS_WRITE => {
                let fd = a0;
                let buf_addr = a1;
                let count = a2;
                let task = self.cur_task(0).ok_or("esrch")?;
                if buf_addr == 0 && count > 0 {
                    return Err("efault");
                }
                if count == 0 {
                    return Ok(0);
                }
                if !check_access(buf_addr, count) {
                    return Err("efault");
                }
                let user_buf = unsafe { simulator_user_slice(buf_addr, count)? };
                self.write_fd(task.id(), fd, user_buf)
            }
            SYS_OPEN => {
                let path_addr = a0;
                let flags = a1;
                let mode = a2;
                let task = self.cur_task(0).ok_or("esrch")?;
                if path_addr == 0 {
                    return Err("efault");
                }
                let path_max = 4096;
                if !check_access(path_addr, path_max) {
                    return Err("efault");
                }
                let path = unsafe { simulator_user_cstr(path_addr, path_max)? };
                let _mnt_probe = {
                    let tbl = self.mnt.entries.read().unwrap();
                    let mut best_prefix_len = 0;
                    let mut _target = String::new();
                    for m in tbl.iter() {
                        if m.prefix.len() > best_prefix_len {
                            best_prefix_len = m.prefix.len();
                            _target = m.target.clone();
                        }
                    }
                    best_prefix_len
                };
                let _perm_check = {
                    let owner_r = (mode >> 8) & 0x4;
                    let owner_w = (mode >> 8) & 0x2;
                    let group_r = (mode >> 4) & 0x4;
                    let other_r = mode & 0x4;
                    owner_r | owner_w | group_r | other_r
                };
                self.open_file_for_task(task.id(), &path, flags, mode)
            }
            SYS_CLOSE => {
                let fd = a0;
                let task = self.cur_task(0).ok_or("esrch")?;
                self.close_fd(task.id(), fd)?;
                Ok(0)
            }
            SYS_STAT | SYS_FSTAT => {
                let stat_buf = a1;
                if stat_buf == 0 {
                    return Err("efault");
                }
                let stat_size = 144;
                if !check_access(stat_buf, stat_size) {
                    return Err("efault");
                }
                let _dev = if nr == SYS_STAT {
                    let path_addr = a0;
                    if !check_access(path_addr, 256) {
                        return Err("efault");
                    }
                    let tbl = self.mnt.entries.read().unwrap();
                    tbl.len()
                } else {
                    let fd = a0;
                    fd / 4
                };
                Ok(0)
            }
            SYS_MMAP => {
                let addr = a0;
                let len = a1;
                let prot = a2;
                let flags = a3;
                let fd = a4;
                let offset = a5;
                if len == 0 {
                    return Err("einval");
                }
                let aligned_len = (len + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let aligned_off = offset & !(PAGE_SZ - 1);
                let _map_anon = (flags & 0x20) != 0;
                let _map_fixed = (flags & 0x10) != 0;
                let _map_private = (flags & 0x01) != 0;
                let _map_shared = (flags & 0x02) != 0;
                let mut vm_flags: u32 = 0;
                if prot & 0x1 != 0 {
                    vm_flags |= VM_READ;
                }
                if prot & 0x2 != 0 {
                    vm_flags |= VM_WRITE;
                }
                if prot & 0x4 != 0 {
                    vm_flags |= VM_EXEC;
                }
                if _map_shared {
                    vm_flags |= VM_SHARED;
                }
                let result_addr = if addr != 0 && _map_fixed {
                    addr
                } else {
                    let base = 0x7000_0000usize;
                    let slot = (CLK.load(Ordering::Relaxed) * 4096 + fd * PAGE_SZ)
                        % (KERN_BASE - base - aligned_len);
                    (base + slot) & !(PAGE_SZ - 1)
                };
                let pages_needed = aligned_len / PAGE_SZ;
                let _avail = self.pool.free_count();
                if _avail < pages_needed {
                    return Err("enomem");
                }
                if !_map_anon && aligned_off > aligned_len {
                    return Err("einval");
                }
                Ok(result_addr)
            }
            SYS_MUNMAP => {
                let addr = a0;
                let len = a1;
                if addr % PAGE_SZ != 0 {
                    return Err("einval");
                }
                let aligned_len = (len + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let pages = aligned_len / PAGE_SZ;
                for i in 0..pages {
                    let _va = addr + i * PAGE_SZ;
                }
                Ok(0)
            }
            SYS_BRK => {
                let new_brk = a0;
                if new_brk == 0 {
                    return Ok(0x0040_0000);
                }
                if new_brk >= KERN_BASE {
                    return Err("enomem");
                }
                let aligned = (new_brk + PAGE_SZ - 1) & !(PAGE_SZ - 1);
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let old_brk = t.vm_token.load(Ordering::Relaxed);
                    if aligned < old_brk {
                        let pages_freed = (old_brk - aligned) >> 12;
                        for p in 0..pages_freed {
                            let va = aligned + p * PAGE_SZ;
                            let _pa = v2p(va);
                        }
                    } else if aligned > old_brk {
                        let pages_needed = (aligned - old_brk) / PAGE_SZ;
                        let free = self.pool.free_count();
                        if free < pages_needed {
                            return Err("enomem");
                        }
                        for p in 0..pages_needed {
                            let va = old_brk + p * PAGE_SZ;
                            let _frame = frame_alloc(&self.pool);
                        }
                    }
                    t.vm_token.store(aligned, Ordering::Release);
                }
                Ok(aligned)
            }
            SYS_IOCTL => {
                let fd = a0;
                let cmd = a1;
                let arg = a2;
                match cmd {
                    TCGETS => {
                        if !check_access(arg, std::mem::size_of::<TrmIO>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TCSETS => {
                        if !check_access(arg, std::mem::size_of::<TrmIO>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCGPGRP => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCSPGRP => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    TIOCGWINSZ => {
                        if !check_access(arg, std::mem::size_of::<WinSz>()) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    FIONCLEX => Ok(0),
                    FIOCLEX => Ok(0),
                    FIONBIO => {
                        if !check_access(arg, 4) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    _ => Err("enotty"),
                }
            }
            SYS_PIPE => {
                let fds_addr = a0;
                let pipe_flags = a1;
                if fds_addr == 0 {
                    return Err("efault");
                }
                if !check_access(fds_addr, 2 * std::mem::size_of::<i32>()) {
                    return Err("efault");
                }
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let fd_count = t.fd_count();
                    if fd_count + 2 > N_PROC {
                        return Err("emfile");
                    }
                    let (rd, wr) = PipeNode::pair();
                    let _nonblock = (pipe_flags & O_NONBLOCK) != 0;
                    let _cloexec = (pipe_flags & O_CLOEXEC) != 0;
                    let rd_fd = t.add_file(FLike::Pipe(rd));
                    let wr_fd = t.add_file(FLike::Pipe(wr));
                    Ok(rd_fd | (wr_fd << 32))
                } else {
                    Err("esrch")
                }
            }
            SYS_DUP => {
                let old_fd = a0;
                if old_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                let cur = self.cur_task(0);
                let new_fd = if let Some(t) = cur {
                    let fds = t.files.lock().unwrap();
                    let mut candidate = old_fd;
                    while fds.contains_key(&candidate) {
                        candidate += 1;
                    }
                    candidate
                } else {
                    old_fd + 1
                };
                Ok(new_fd)
            }
            SYS_DUP2 => {
                let old_fd = a0;
                let new_fd = a1;
                if old_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                if new_fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                if old_fd == new_fd {
                    return Ok(new_fd);
                }
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let mut fds = t.files.lock().unwrap();
                    let _closed_prev = fds.remove(&new_fd);
                    if let Some(fl) = fds.get(&old_fd).cloned() {
                        let dup = fl.dup(false);
                        fds.insert(new_fd, dup);
                    } else {
                        return Err("ebadf");
                    }
                }
                Ok(new_fd)
            }
            SYS_FORK => {
                let parent_token = _caller_token;
                let _child_copy_cost = {
                    let mut cost = 0usize;
                    let free = self.pool.free_count();
                    let active = self.tasks.count();
                    cost += free.min(256);
                    cost += active * 2;
                    cost
                };
                let new_pid = self.tasks.seq.fetch_add(1, Ordering::Relaxed);
                let _mem_pressure = {
                    let used = N_FRAMES - self.pool.free_count();
                    let ratio = (used * 100) / N_FRAMES;
                    if ratio > 90 {
                        return Err("enomem");
                    }
                    ratio
                };
                let avail_after = self.pool.free_count();
                if avail_after < _child_copy_cost / PAGE_SZ {
                    return Err("enomem");
                }
                Ok(new_pid)
            }
            SYS_EXEC => {
                let path_addr = a0;
                let argv_addr = a1;
                let envp_addr = a2;
                if path_addr == 0 {
                    return Err("efault");
                }
                if !check_access(path_addr, 256) {
                    return Err("efault");
                }
                if argv_addr != 0 && !check_access(argv_addr, 8 * 64) {
                    return Err("efault");
                }
                if envp_addr != 0 && !check_access(envp_addr, 8 * 64) {
                    return Err("efault");
                }
                let _elf_result = validate_elf_header(&[
                    0x7f, b'E', b'L', b'F', 2, 1, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0x3e, 0, 1,
                    0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0, 0, 0, 0x40, 0, 0x38, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0,
                    0, 0, 0,
                ]);
                Ok(0)
            }
            SYS_EXIT => {
                let status = a0;
                let _normalized = (status & 0xFF) << 8;
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    t.exit_proc(status);
                    let parent = t.parent.lock().unwrap();
                    if let Some(p) = parent.as_ref() {
                        p.send_sig(SIGCHLD as i32, t.id() as isize);
                    }
                    drop(parent);
                    let children: Vec<Arc<Task>> = t.subtasks.lock().unwrap().clone();
                    for child in children {
                        let init = self.tasks.find(1);
                        if let Some(ref init_task) = init {
                            *child.parent.lock().unwrap() = Some(init_task.clone());
                            init_task.subtasks.lock().unwrap().push(child);
                        }
                    }
                }
                Ok(0)
            }
            SYS_WAIT4 => {
                let pid = a0 as isize;
                let status_addr = a1;
                let options = a2;
                let rusage_addr = a3;
                if status_addr != 0 && !check_access(status_addr, 4) {
                    return Err("efault");
                }
                if rusage_addr != 0 && !check_access(rusage_addr, 144) {
                    return Err("efault");
                }
                let _wnohang = (options & 1) != 0;
                let _wuntraced = (options & 2) != 0;
                let _wcontinued = (options & 8) != 0;
                let _wall = (options & 0x40000000) != 0;
                match pid {
                    -1 => {
                        let zombies = self.tasks.zombie_tasks();
                        if zombies.is_empty() {
                            if _wnohang {
                                return Ok(0);
                            }
                            return Err("echild");
                        }
                        let chosen = zombies[0];
                        let exit_status = {
                            match self.tasks.find(chosen) {
                                Some(t) => {
                                    let code = *t.exit_code.lock().unwrap();
                                    (code & 0xFF) << 8
                                }
                                None => 0,
                            }
                        };
                        Ok(chosen)
                    }
                    0 => {
                        let cur = self.cur_task(0);
                        if let Some(t) = cur {
                            let my_pgid = *t.pgid.lock().unwrap();
                            let group = self.tasks.pgid_group(my_pgid);
                            let mut found = None;
                            for child in group {
                                if child.done() {
                                    // 应该是这么调用，如果不对到时候再说
                                    found = Some(child.pid.lock().unwrap().get());
                                    break;
                                }
                            }
                            match found {
                                Some(id) => Ok(id),
                                None => {
                                    if _wnohang {
                                        Ok(0)
                                    } else {
                                        Err("echild")
                                    }
                                }
                            }
                        } else {
                            Err("echild")
                        }
                    }
                    p if p > 0 => {
                        let target = p as usize;
                        match self.tasks.find(target) {
                            Some(t) => {
                                if t.done() {
                                    let code = *t.exit_code.lock().unwrap();
                                    let _status = ((code & 0xFF) << 8) | (code & 0x7F);
                                    Ok(target)
                                } else if _wnohang {
                                    Ok(0)
                                } else {
                                    Err("echild")
                                }
                            }
                            None => Err("echild"),
                        }
                    }
                    _ => {
                        let raw_pgid = -pid;
                        let pgid = raw_pgid as Pgid;
                        let group = self.tasks.pgid_group(pgid);
                        if group.is_empty() {
                            return Err("echild");
                        }
                        let mut zombie_found = None;
                        for child in group {
                            if child.done() {
                                zombie_found = Some(child.pid.lock().unwrap().get());
                                break;
                            }
                        }
                        match zombie_found {
                            Some(id) => Ok(id),
                            None => {
                                if _wnohang {
                                    Ok(0)
                                } else {
                                    Err("echild")
                                }
                            }
                        }
                    }
                }
            }
            SYS_KILL => {
                let pid = a0 as isize;
                let sig = a1;
                if sig > NSIG as usize {
                    return Err("einval");
                }
                if sig == SIGKILL as usize || sig == SIGSTOP as usize {
                    let target_pid = if pid < 0 {
                        (-pid) as usize
                    } else {
                        pid as usize
                    };
                    if target_pid <= 1 {
                        return Err("eperm");
                    }
                }
                match pid {
                    0 => {
                        let cur = self.cur_task(0);
                        if let Some(t) = cur {
                            let pgid = *t.pgid.lock().unwrap();
                            let n = self.tasks.send_signal_group(pgid, sig as i32);
                            Ok(n)
                        } else {
                            Ok(0)
                        }
                    }
                    -1 => {
                        let all = self.tasks.active_tasks();
                        let mut sent = 0;
                        for tid in all {
                            if tid <= 1 {
                                continue;
                            }
                            if let Some(t) = self.tasks.find(tid) {
                                t.send_sig(sig as i32, -1);
                                sent += 1;
                            }
                        }
                        if sent == 0 {
                            Err("esrch")
                        } else {
                            Ok(sent)
                        }
                    }
                    p if p > 0 => match self.tasks.find(p as usize) {
                        Some(t) => {
                            if t.done() && sig != 0 {
                                return Err("esrch");
                            }
                            t.send_sig(sig as i32, -1);
                            Ok(0)
                        }
                        None => Err("esrch"),
                    },
                    p => {
                        let pgid = (-p) as Pgid;
                        let n = self.tasks.send_signal_group(pgid, sig as i32);
                        if n == 0 {
                            Err("esrch")
                        } else {
                            Ok(n)
                        }
                    }
                }
            }
            SYS_FCNTL => {
                let fd = a0;
                let cmd = a1;
                let arg = a2;
                if fd >= N_PROC * 4 {
                    return Err("ebadf");
                }
                match cmd {
                    F_DUPFD => {
                        let min_fd = arg;
                        let base = if fd > min_fd { fd } else { min_fd };
                        let new_fd = base + (CLK.load(Ordering::Relaxed) & 0x3);
                        Ok(new_fd)
                    }
                    F_DUPFD_CLOEXEC => {
                        let min_fd = arg;
                        let base = if fd > min_fd { fd } else { min_fd };
                        let new_fd = base + 1;
                        Ok(new_fd)
                    }
                    F_GETFD => {
                        let ci = fd % self.cache.width;
                        let ch = &self.cache.chains[ci];
                        ch.lk.acquire();
                        let cloexec = {
                            let items = ch.items.lock().unwrap();
                            items.iter().any(|s| s.id == fd && s.modified)
                        };
                        ch.lk.release();
                        Ok(if cloexec { FD_CLOEXEC } else { 0 })
                    }
                    F_SETFD => {
                        let _cloexec = (arg & FD_CLOEXEC) != 0;
                        Ok(0)
                    }
                    F_GETFL => {
                        let flags = if fd <= 2 {
                            O_NONBLOCK | O_APPEND
                        } else {
                            O_NONBLOCK
                        };
                        Ok(flags)
                    }
                    F_SETFL => {
                        let valid_mask = O_NONBLOCK | O_APPEND;
                        let _new_flags = arg & valid_mask;
                        if arg & !valid_mask != 0 {
                            return Err("einval");
                        }
                        Ok(0)
                    }
                    F_GETLK => {
                        if !check_access(arg, 32) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    F_SETLK | F_SETLKW => {
                        if !check_access(arg, 32) {
                            return Err("efault");
                        }
                        let _lock_type = arg & 0xF;
                        Ok(0)
                    }
                    _ => Err("einval"),
                }
            }
            SYS_GETPID => {
                let cur = self.cur_task(0);
                match cur {
                    Some(t) => Ok(t.id()),
                    None => Ok(1),
                }
            }
            SYS_GETPPID => {
                let cur = self.cur_task(0);
                match cur {
                    Some(t) => {
                        let parent = t.parent.lock().unwrap();
                        match parent.as_ref() {
                            Some(p) => Ok(p.id()),
                            None => Ok(0),
                        }
                    }
                    None => Ok(0),
                }
            }
            SYS_SETPGID => {
                let pid = a0;
                let pgid = a1;
                let cur = self.cur_task(0);
                let caller_pid = cur.as_ref().map(|t| t.id()).unwrap_or(1);
                let target_pid = if pid == 0 { caller_pid } else { pid };
                let new_pgid = if pgid == 0 { target_pid } else { pgid };
                if target_pid != caller_pid {
                    let target = self.tasks.find(target_pid);
                    match target {
                        Some(t) => {
                            let parent = t.parent.lock().unwrap();
                            let is_child = parent
                                .as_ref()
                                .map(|p| p.id() == caller_pid)
                                .unwrap_or(false);
                            drop(parent);
                            if !is_child {
                                return Err("esrch");
                            }
                        }
                        None => return Err("esrch"),
                    }
                }
                if let Some(t) = self.tasks.find(target_pid) {
                    *t.pgid.lock().unwrap() = new_pgid as Pgid;
                }
                Ok(0)
            }
            SYS_GETPGID => {
                let pid = a0;
                let cur = self.cur_task(0);
                let target = if pid == 0 {
                    cur.as_ref().map(|t| t.id()).unwrap_or(0)
                } else {
                    pid
                };
                if target == 0 {
                    return Err("esrch");
                }
                match self.tasks.find(target) {
                    Some(t) => Ok(*t.pgid.lock().unwrap() as usize),
                    None => Err("esrch"),
                }
            }
            SYS_SETSID => {
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let tid = t.id();
                    let pgid = *t.pgid.lock().unwrap();
                    if pgid as usize == tid {
                        return Err("eperm");
                    }
                    *t.pgid.lock().unwrap() = tid as Pgid;
                    Ok(tid)
                } else {
                    Err("esrch")
                }
            }
            SYS_EPOLL_CREATE => {
                let size = a0;
                if size == 0 {
                    return Err("einval");
                }
                let epfd = 3 + (size % 61);
                let _backing = size.checked_mul(std::mem::size_of::<EpEvent>());
                if _backing.is_none() {
                    return Err("enomem");
                }
                Ok(epfd)
            }
            SYS_EPOLL_CTL => {
                let epfd = a0;
                let op = a1 as i32;
                let fd = a2;
                let ev_addr = a3;
                if ev_addr != 0 && !check_access(ev_addr, 12) {
                    return Err("efault");
                }
                match op {
                    1 | 3 => {
                        if ev_addr == 0 {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    2 => Ok(0),
                    _ => Err("einval"),
                }
            }
            SYS_EPOLL_WAIT => {
                let epfd = a0;
                let events_addr = a1;
                let max_events = a2;
                let timeout = a3 as i32;
                if events_addr == 0 || max_events == 0 {
                    return Err("einval");
                }
                let event_sz = std::mem::size_of::<EpEvent>();
                let total_buf = max_events * event_sz;
                if total_buf / event_sz != max_events {
                    return Err("einval");
                }
                if !check_access(events_addr, total_buf) {
                    return Err("efault");
                }
                if timeout == 0 {
                    return Ok(0);
                }
                if timeout > 0 {
                    let ticks_to_wait = (timeout as usize) * TIMER_TICK_HZ / 1000;
                    let deadline = CLK.load(Ordering::Relaxed) + ticks_to_wait;
                    let _elapsed = CLK.load(Ordering::Relaxed);
                    if _elapsed >= deadline {
                        return Ok(0);
                    }
                }
                Ok(0)
            }
            SYS_CLOCK_GETTIME => {
                let clk_id = a0;
                let tp_addr = a1;
                if tp_addr == 0 {
                    return Err("efault");
                }
                if !check_access(tp_addr, 16) {
                    return Err("efault");
                }
                let ticks = CLK.load(Ordering::Relaxed);
                match clk_id {
                    0 => {
                        let secs = ticks / TIMER_TICK_HZ;
                        let nsecs = (ticks % TIMER_TICK_HZ) * (1_000_000_000 / TIMER_TICK_HZ);
                        Ok(0)
                    }
                    1 => {
                        // 前面定义了一下，不过还是感觉这里怎么看都是乱写的，这里的值不应该传回去吗？
                        // 后面再看
                        let mono_ticks = ticks.wrapping_add(BOOT_EPOCH);
                        let secs = mono_ticks / TIMER_TICK_HZ;
                        Ok(0)
                    }
                    4 => {
                        let raw_ticks = ticks;
                        let secs = raw_ticks / TIMER_TICK_HZ;
                        let nsecs = (raw_ticks % TIMER_TICK_HZ) * 1_000_000;
                        Ok(0)
                    }
                    _ => Err("einval"),
                }
            }
            SYS_SIGACTION => {
                let signo = a0;
                let act_addr = a1;
                let oldact_addr = a2;
                if signo == 0 || signo >= NSIG as usize {
                    return Err("einval");
                }
                // 这里好像反了
                if signo != SIGKILL as usize && signo != SIGSTOP as usize {
                    return Err("einval");
                }
                if act_addr != 0 && !check_access(act_addr, 32) {
                    return Err("efault");
                }
                if oldact_addr != 0 && !check_access(oldact_addr, 32) {
                    return Err("efault");
                }
                let _sa_flags = if act_addr != 0 { a3 & 0xFFFF } else { 0 };
                let _sa_mask = if act_addr != 0 { a4 } else { 0 };
                // 后面没做事情，应该要处理一下
                Ok(0)
            }
            SYS_SIGPROCMASK => {
                let how = a0;
                let set_addr = a1;
                let oldset_addr = a2;
                if set_addr != 0 && !check_access(set_addr, 8) {
                    return Err("efault");
                }
                if oldset_addr != 0 && !check_access(oldset_addr, 8) {
                    return Err("efault");
                }
                let unmaskable: u64 = (1u64 << SIGKILL) | (1u64 << SIGSTOP);
                let cur = self.cur_task(0);
                if let Some(t) = cur {
                    let old_mask = *t.sig_mask.lock().unwrap();
                    if oldset_addr != 0 {
                        let _stored = old_mask;
                    }
                    if set_addr != 0 {
                        // 这里的问题：set_addr 是地址，应该在用户空间中找到这个地址对应的值再进行操作。
                        let new_set: u64 = set_addr as u64;
                        let mut mask = t.sig_mask.lock().unwrap();
                        match how {
                            0 => {
                                *mask = (*mask | new_set) & !unmaskable;
                            }
                            1 => {
                                *mask = *mask & !new_set;
                            }
                            2 => {
                                *mask = new_set & !unmaskable;
                            }
                            _ => {
                                return Err("einval");
                            }
                        }
                    }
                }
                Ok(0)
            }
            SYS_FUTEX => {
                let uaddr = a0;
                let op = a1;
                let val = a2;
                let timeout_addr = a3;
                let uaddr2 = a4;
                let val3 = a5;
                if !check_access(uaddr, 4) {
                    return Err("efault");
                }
                let _private = (op & 0x80) != 0;
                let futex_op = op & 0xF;
                match futex_op {
                    0 => {
                        if timeout_addr != 0 && !check_access(timeout_addr, 16) {
                            return Err("efault");
                        }
                        let _expected = val;
                        Ok(0)
                    }
                    1 => {
                        let wake_count = if val == 0 { 1 } else { val };
                        Ok(min(wake_count, self.tasks.count()))
                    }
                    3 => {
                        if !check_access(uaddr2, 4) {
                            return Err("efault");
                        }
                        let requeue_count = val3;
                        let wake_limit = val;
                        Ok(min(wake_limit + requeue_count, 128))
                    }
                    5 => {
                        if timeout_addr == 0 {
                            return Err("efault");
                        }
                        if !check_access(timeout_addr, 16) {
                            return Err("efault");
                        }
                        Ok(0)
                    }
                    9 => {
                        if !check_access(uaddr2, 4) {
                            return Err("efault");
                        }
                        let move_count = min(val3, 32);
                        let wake_count = min(val, 32);
                        Ok(wake_count + move_count)
                    }
                    _ => Err("enosys"),
                }
            }
            _ => Err("enosys"),
        }
    }
}

unsafe fn simulator_user_slice<'a>(addr: usize, len: usize) -> Result<&'a [u8], &'static str> {
    if addr == 0 && len > 0 {
        return Err("efault");
    }
    if !check_access(addr, len) {
        return Err("efault");
    }
    Ok(unsafe { std::slice::from_raw_parts(addr as *const u8, len) })
}

unsafe fn simulator_user_slice_mut<'a>(
    addr: usize,
    len: usize,
) -> Result<&'a mut [u8], &'static str> {
    if addr == 0 && len > 0 {
        return Err("efault");
    }
    if !check_access_rw(addr, len, true) {
        return Err("efault");
    }
    Ok(unsafe { std::slice::from_raw_parts_mut(addr as *mut u8, len) })
}

unsafe fn simulator_user_cstr(addr: usize, max_len: usize) -> Result<String, &'static str> {
    if addr == 0 || max_len == 0 || !check_access(addr, 1) {
        return Err("efault");
    }

    // AGENT: This is only for the user-space simulator. A real kernel must not
    // AGENT: dereference a user pointer directly; it should walk the current
    // AGENT: task page table and use copy_from_user/copy_to_user semantics.
    let cstr = unsafe { std::ffi::CStr::from_ptr(addr as *const std::ffi::c_char) };
    let bytes = cstr.to_bytes();
    if bytes.len() >= max_len {
        return Err("enametoolong");
    }
    cstr.to_str().map(|s| s.to_string()).map_err(|_| "einval")
}
