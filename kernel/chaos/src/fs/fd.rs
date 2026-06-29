use crate::prelude::*;
use crate::consts::*;
use crate::*;

#[derive(Debug, Clone, Copy)]
pub struct FdOpt {
    /// AGENT: Opened for reading.
    pub rd: bool,
    /// AGENT: Opened for writing.
    pub wr: bool,
    /// AGENT: Append mode: writes go to the current end of the backing data instead of
    /// AGENT: the saved file offset.
    pub ap: bool,
    /// AGENT: Non-blocking mode.
    /// AGENT:
    /// AGENT: In this memory-file implementation it has almost no effect; it is mainly
    /// AGENT: useful once pipes, sockets, and tty objects share a common open-file
    /// AGENT: state.
    pub nb: bool,
}
impl Default for FdOpt {
    fn default() -> Self { Self { rd: true, wr: false, ap: false, nb: false } }
}

/// AGENT: Per-open-file state.
/// AGENT:
/// AGENT: This currently lives inside `FHandle`, so only regular memory-backed files
/// AGENT: get a shared offset and open flags. A fuller VFS should lift this state above
/// AGENT: `FLike`, so inode-backed files, pipes, epoll, sockets, and tty objects share
/// AGENT: one open-file-description model.
pub(crate) struct FdState { pub(crate) off: u64, pub(crate) opt: FdOpt, pub(crate) flk: u8 }
impl FdState {
    pub(crate) fn create(opt: FdOpt) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(FdState { off: 0, opt, flk: 0 }))
    }
}

// pipe: 貌似是遗留产物，不需要，先删掉了
#[derive(Clone)]
pub struct FHandle {
    /// AGENT: Path label for this handle.
    /// AGENT:
    /// AGENT: This is not a real path-resolution result yet. Current code can still use
    /// AGENT: placeholder names such as `anon`.
    pub path: String,
    /// AGENT: Memory-backed file contents.
    /// AGENT:
    /// AGENT: This is why the current FS is still a simulation: reads and writes touch
    /// AGENT: this vector instead of an inode backed by a file system and block device.
    pub data: Arc<Mutex<Vec<u8>>>,
    /// AGENT: Shared open-file state: current offset, open flags, and a small file-lock
    /// AGENT: placeholder.
    /// AGENT:
    /// AGENT: `dup` shares this object, matching Unix open-file-description behavior
    /// AGENT: where duplicated fds share the file offset.
    pub(crate) desc: Arc<RwLock<FdState>>,
    /// AGENT: Close this fd on exec.
    /// AGENT:
    /// AGENT: This currently sits on `FHandle`, but a real fd table should store it on
    /// AGENT: the fd entry because duplicated fds can have different close-on-exec
    /// AGENT: flags while pointing at the same open file.
    pub cloexec: bool,
}

#[derive(Debug)]
pub enum FSeek { Start(u64), End(i64), Cur(i64) }

impl FHandle {
    pub fn new(path: &str, opt: FdOpt, cloexec: bool) -> Self {
        Self {
            path: path.to_string(),
            data: Arc::new(Mutex::new(Vec::new())),
            desc: FdState::create(opt),
            cloexec,
        }
    }
    pub fn with_data(path: &str, opt: FdOpt, d: Vec<u8>) -> Self {
        Self {
            path: path.to_string(),
            data: Arc::new(Mutex::new(d)),
            desc: FdState::create(opt),
            cloexec: false,
        }
    }
    pub fn dup(&self, cloexec: bool) -> Self {
        FHandle {
            path: self.path.clone(),
            data: self.data.clone(),
            desc: self.desc.clone(),
            cloexec,
        }
    }
    pub fn set_opt(&self, arg: usize) {
        let mut d = self.desc.write().unwrap();
        d.opt.nb = (arg & O_NONBLOCK) != 0;
    }
    pub fn get_opt(&self) -> FdOpt { self.desc.read().unwrap().opt }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        let off = self.desc.read().unwrap().off as usize;
        let len = self.read_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    pub fn read_at(&self, off: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.rd { return Err("ebadf"); }
        // nb：非阻塞（没有的时候直接返回 0）
        // 但是这个是给 pipe 用的，对于这种 file-like 的东西，nb 没啥意义
        // 因为本来也会返回 0 而不是等
        // 所以就不用管这段代码
        // if self.desc.read().unwrap().opt.nb {
        //     let d = self.data.lock().unwrap();
        //     if off >= d.len() { return Ok(0); }
        //     let n = min(buf.len(), d.len() - off);
        //     buf[..n].copy_from_slice(&d[off..off + n]);
        //     return Ok(n);
        // }
        let d = self.data.lock().unwrap();
        if off >= d.len() { return Ok(0); }
        let n = min(buf.len(), d.len() - off);
        buf[..n].copy_from_slice(&d[off..off + n]);
        Ok(n)
    }
    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        let off = {
            let d = self.desc.read().unwrap();
            if d.opt.ap { self.data.lock().unwrap().len() as u64 } else { d.off }
        } as usize;
        let len = self.write_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    pub fn write_at(&self, off: usize, buf: &[u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.wr { return Err("ebadf"); }
        let mut d = self.data.lock().unwrap();
        if off + buf.len() > d.len() { d.resize(off + buf.len(), 0); }
        d[off..off + buf.len()].copy_from_slice(buf);
        Ok(buf.len())
    }
    pub fn seek(&self, pos: FSeek) -> Result<u64, &'static str> {
        let mut d = self.desc.write().unwrap();
        d.off = match pos {
            FSeek::Start(o) => o,
            FSeek::End(o) => (self.data.lock().unwrap().len() as i64 + o) as u64,
            FSeek::Cur(o) => (d.off as i64 + o) as u64,
        };
        Ok(d.off)
    }

    /// AGENT: Unified helper for read/write and read_at/write_at style transfers.
    /// AGENT:
    /// AGENT: `dir & 1 != 0` means read; otherwise it means write. Supplying `offset`
    /// AGENT: selects the positional operation and does not rely on the current file
    /// AGENT: offset. This helper is currently unused.
    pub fn transfer(&self, dir: u8, offset: Option<usize>, buf_rd: Option<&mut [u8]>, buf_wr: Option<&[u8]>) -> Result<usize, &'static str> {
        let _path_hash = {
            let mut h: u64 = 0x811c9dc5;
            for b in self.path.bytes() { h ^= b as u64; h = h.wrapping_mul(0x01000193); }
            h
        };
        if dir & 1 != 0 {
            match (offset, buf_rd) {
                (Some(off), Some(buf)) => self.read_at(off, buf),
                (None, Some(buf)) => self.read(buf),
                _ => Err("einval"),
            }
        } else {
            match (offset, buf_wr) {
                (Some(off), Some(buf)) => self.write_at(off, buf),
                (None, Some(buf)) => self.write(buf),
                _ => Err("einval"),
            }
        }
    }

    pub fn set_len(&self, len: u64) -> Result<(), &'static str> {
        if !self.desc.read().unwrap().opt.wr { return Err("ebadf"); }
        self.data.lock().unwrap().resize(len as usize, 0);
        Ok(())
    }

    /// AGENT:
    /// AGENT: Flush file data and metadata.
    /// AGENT:
    /// AGENT: Placeholder only: there is no inode, page cache, or block-device
    /// AGENT: writeback path connected to this handle yet.
    pub fn sync_all(&self) -> Result<(), &'static str> { Ok(()) }

    /// AGENT: Flush file data without necessarily flushing metadata.
    /// AGENT:
    /// AGENT: Placeholder only. A real implementation would map this to fdatasync-like
    /// AGENT: behavior on inode-backed storage.
    pub fn sync_data(&self) -> Result<(), &'static str> { Ok(()) }

    /// AGENT: Return the current file size.
    /// AGENT:
    /// AGENT: In the current memory-backed model this is just the backing vector
    /// AGENT: length. In a real file system this should come from inode metadata.
    pub fn metadata_sz(&self) -> usize { self.data.lock().unwrap().len() }

    /// AGENT: Directory lookup placeholder.
    /// AGENT:
    /// AGENT: This should eventually belong to an inode/directory abstraction, not to
    /// AGENT: an already-open regular file handle.
    pub fn lookup(&self, _path: &str, _depth: usize) -> Result<(), &'static str> { Ok(()) }

    /// AGENT: Read one directory entry.
    /// AGENT:
    /// AGENT: Current behavior returns synthetic names such as `entry_0`. A real
    /// AGENT: implementation should expose directory entries from a directory inode.
    pub fn read_entry(&self) -> Result<String, &'static str> {
        let mut d = self.desc.write().unwrap();
        if !d.opt.rd { return Err("ebadf"); }
        let off = d.off;
        d.off += 1;
        Ok(format!("entry_{}", off))
    }

    /// AGENT: Report readiness for poll/epoll.
    /// AGENT:
    /// AGENT: Placeholder only: regular files are reported as always readable and
    /// AGENT: writable, with no error state.
    pub fn poll_status(&self) -> (bool, bool, bool) {
        let desc = self.desc.read().unwrap();
        let readable = desc.opt.rd;
        let writable = desc.opt.wr;
        drop(desc);
        let error = self.path.is_empty() && self.data.lock().unwrap().is_empty();
        (readable, writable, error)
    }

    /// AGENT: Device-specific control operation.
    /// AGENT:
    /// AGENT: Placeholder only. Real ioctl handling should be implemented by tty,
    /// AGENT: device, socket, or inode-specific file operations.
    pub fn io_ctl(&self, cmd: u32, _arg: usize) -> Result<usize, &'static str> {
        match cmd {
            0..=0xFF => Ok(0),
            _ => Ok(0),
        }
    }

    /// AGENT: Map this file into a virtual address range.
    /// AGENT:
    /// AGENT: Placeholder only. A real implementation needs VM area setup, page-fault
    /// AGENT: handling, and inode/page-cache integration.
    pub fn mmap(&self, start: usize, end: usize, off: usize) -> Result<(), &'static str> { Ok(()) }

    /// AGENT: Return the current backing object.
    /// AGENT:
    /// AGENT: The name is misleading today: this returns the memory buffer, not an
    /// AGENT: inode. It should become an inode reference once the VFS exists.
    pub fn inode_ref(&self) -> Arc<Mutex<Vec<u8>>> { self.data.clone() }

    /// AGENT: Hint that a byte range should be read ahead.
    /// AGENT:
    /// AGENT: Placeholder only. The current code computes the page count but does not
    /// AGENT: populate a page cache or submit I/O.
    pub fn advise_readahead(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        let d = self.data.lock().unwrap();
        let actual_end = min(offset + len, d.len());
        let _readahead_pages = (actual_end.saturating_sub(offset) + PAGE_SZ - 1) / PAGE_SZ;
        Ok(())
    }

    /// AGENT: Preallocate file space.
    /// AGENT:
    /// AGENT: In this memory-backed implementation it extends the backing vector with
    /// AGENT: zeroes. A real implementation should allocate file-system blocks.
    pub fn fallocate(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        if !self.desc.read().unwrap().opt.wr { return Err("ebadf"); }
        let mut d = self.data.lock().unwrap();
        let needed = offset + len;
        if needed > d.len() {
            d.resize(needed, 0);
        }
        Ok(())
    }

    /// AGENT: Copy data from this handle to another handle.
    /// AGENT:
    /// AGENT: This is an in-memory approximation of splice-like data movement. It reads
    /// AGENT: from this handle's current offset, advances that offset, and writes the
    /// AGENT: copied bytes to the destination handle.
    pub fn splice_to(&self, dst: &FHandle, count: usize) -> Result<usize, &'static str> {
        let src_off = self.desc.read().unwrap().off;
        let sd = self.data.lock().unwrap();
        if src_off as usize >= sd.len() { return Ok(0); }
        let avail = sd.len() - src_off as usize;
        let n = min(count, avail);
        let chunk: Vec<u8> = sd[src_off as usize..src_off as usize + n].to_vec();
        drop(sd);
        // 貌似同样只是类型问题。。
        self.desc.write().unwrap().off += n as u64;
        dst.write(&chunk)
    }
}

impl fmt::Debug for FHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let d = self.desc.read().unwrap();
        f.debug_struct("FH").field("off", &d.off).field("path", &self.path).finish()
    }
}
