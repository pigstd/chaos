use crate::consts::*;
use crate::prelude::*;
use crate::*;

#[derive(Debug, Clone, Copy)]
pub struct FdOpt {
    /// AGENT: 是否允许读。
    pub rd: bool,
    /// AGENT: 是否允许写。
    pub wr: bool,
    /// AGENT: 追加模式（append）：写入位置使用当前文件末尾，而不是保存的
    /// AGENT: 文件偏移（file offset）。
    pub ap: bool,
    /// AGENT: 非阻塞模式（non-blocking）。
    /// AGENT:
    /// AGENT: 对普通 SwapFS 文件来说，这个标志（flag）目前不会改变行为；它主要是给
    /// AGENT: 管道（pipe）、套接字（socket）、终端（tty）这类对象统一打开文件状态
    /// AGENT: （open-file state）时使用。
    pub nb: bool,
}
impl Default for FdOpt {
    fn default() -> Self {
        Self {
            rd: true,
            wr: false,
            ap: false,
            nb: false,
        }
    }
}

/// AGENT: 每个打开文件描述（open file description）对应的状态。
/// AGENT:
/// AGENT: 当前它仍然放在 `FHandle` 里面，所以只有普通文件有共享偏移（offset）和
/// AGENT: 打开标志（open flags）。更完整的 VFS 应该把这层提升到 `FLike` 之上，
/// AGENT: 让 inode 文件、管道（pipe）、epoll、套接字（socket）、终端（tty）都共享
/// AGENT: 同一种打开文件描述（open-file-description）模型。
pub(crate) struct FdState {
    pub(crate) off: u64,
    pub(crate) opt: FdOpt,
    pub(crate) flk: u8,
}
impl FdState {
    pub(crate) fn create(opt: FdOpt) -> Arc<RwLock<Self>> {
        Arc::new(RwLock::new(FdState {
            off: 0,
            opt,
            flk: 0,
        }))
    }
}

// pipe: 貌似是遗留产物，不需要，先删掉了
#[derive(Clone)]
pub struct FHandle {
    /// AGENT: 这个句柄（handle）对应的路径标签。
    /// AGENT:
    /// AGENT: 这里还不是真正路径解析的结果。当前代码仍可能使用 `anon` 这类占位名字。
    pub path: String,
    /// AGENT: 文件系统后端对象（backing object）。
    /// AGENT:
    /// AGENT: 这个句柄（handle）指向一个 SwapFS 元数据记录（metadata record）。
    /// AGENT: 打开文件的偏移（offset）保存在 `desc` 里；文件内容和大小保存在
    /// AGENT: SwapFS 元数据（metadata）和数据块（data blocks）里。
    pub fs: Arc<SwapFs>,
    pub meta_index: usize,
    /// AGENT: 共享的打开文件状态（open-file state）：当前偏移（offset）、打开标志
    /// AGENT: （open flags），以及一个很小的文件锁（file lock）占位字段。
    /// AGENT:
    /// AGENT: `dup` 会共享这个对象，对应 Unix 打开文件描述
    /// AGENT: （open-file-description）语义：复制出来的 fd 共享同一个文件偏移
    /// AGENT: （file offset）。
    pub(crate) desc: Arc<RwLock<FdState>>,
    /// AGENT: 执行 exec 时是否关闭这个 fd（close-on-exec）。
    /// AGENT:
    /// AGENT: 当前它还放在 `FHandle` 上，但真实 fd table 应该把它放在 fd entry
    /// AGENT: 上，因为复制出来的 fd 可以指向同一个打开文件（open file），同时拥有
    /// AGENT: 不同的执行时关闭标志（close-on-exec flag）。
    pub cloexec: bool,
}

#[derive(Debug)]
pub enum FSeek {
    Start(u64),
    End(i64),
    Cur(i64),
}

impl FHandle {
    pub fn new(path: &str, fs: Arc<SwapFs>, meta_index: usize, opt: FdOpt, cloexec: bool) -> Self {
        Self {
            path: path.to_string(),
            fs,
            meta_index,
            desc: FdState::create(opt),
            cloexec,
        }
    }
    pub fn dup(&self, cloexec: bool) -> Self {
        FHandle {
            path: self.path.clone(),
            fs: self.fs.clone(),
            meta_index: self.meta_index,
            desc: self.desc.clone(),
            cloexec,
        }
    }
    pub fn set_opt(&self, arg: usize) {
        let mut d = self.desc.write().unwrap();
        d.opt.nb = (arg & O_NONBLOCK) != 0;
    }
    pub fn get_opt(&self) -> FdOpt {
        self.desc.read().unwrap().opt
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize, &'static str> {
        let off = self.desc.read().unwrap().off as usize;
        let len = self.read_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    pub fn read_at(&self, off: usize, buf: &mut [u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.rd {
            return Err("ebadf");
        }
        self.fs.read_at(self.meta_index, off, buf)
    }
    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        let off = {
            let d = self.desc.read().unwrap();
            if d.opt.ap {
                self.file_len()? as u64
            } else {
                d.off
            }
        } as usize;
        let len = self.write_at(off, buf)?;
        self.desc.write().unwrap().off += len as u64;
        Ok(len)
    }
    pub fn write_at(&self, off: usize, buf: &[u8]) -> Result<usize, &'static str> {
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        self.fs.write_at(self.meta_index, off, buf)
    }
    pub fn seek(&self, pos: FSeek) -> Result<u64, &'static str> {
        let mut d = self.desc.write().unwrap();
        d.off = match pos {
            FSeek::Start(o) => o,
            FSeek::End(o) => checked_seek(self.file_len()? as u64, o)?,
            FSeek::Cur(o) => checked_seek(d.off, o)?,
        };
        Ok(d.off)
    }

    /// AGENT: `read`/`write` 和 `read_at`/`write_at` 的统一转发辅助函数。
    /// AGENT:
    /// AGENT: `dir & 1 != 0` 表示读，否则表示写。传入 `offset` 时执行带位置的
    /// AGENT: 操作（positional operation），不依赖当前文件偏移（file offset）。
    /// AGENT: 这个辅助函数目前还没有被使用。
    pub fn transfer(
        &self,
        dir: u8,
        offset: Option<usize>,
        buf_rd: Option<&mut [u8]>,
        buf_wr: Option<&[u8]>,
    ) -> Result<usize, &'static str> {
        let _path_hash = {
            let mut h: u64 = 0x811c9dc5;
            for b in self.path.bytes() {
                h ^= b as u64;
                h = h.wrapping_mul(0x01000193);
            }
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
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        if len > usize::MAX as u64 {
            return Err("einval");
        }
        self.fs.set_len(self.meta_index, len as usize)
    }

    /// AGENT: 刷新文件数据和元数据（metadata）。
    /// AGENT:
    /// AGENT: 目前只是占位实现：这个句柄（handle）还没有接入 inode、页缓存
    /// AGENT: （page cache）或块设备写回（block-device writeback）路径。
    pub fn sync_all(&self) -> Result<(), &'static str> {
        Ok(())
    }

    /// AGENT: 刷新文件数据，但不一定刷新元数据（metadata）。
    /// AGENT:
    /// AGENT: 目前只是占位实现。真实实现应该把它映射到 inode 后端存储
    /// AGENT: （inode-backed storage）上类似 `fdatasync` 的行为。
    pub fn sync_data(&self) -> Result<(), &'static str> {
        Ok(())
    }

    /// AGENT: 返回当前文件大小。
    /// AGENT:
    /// AGENT: 当前文件大小来自 SwapFS 元数据（metadata）。
    pub fn metadata_sz(&self) -> usize {
        self.file_len().unwrap_or(0)
    }

    /// AGENT: 目录查找（lookup）占位函数。
    /// AGENT:
    /// AGENT: 这个能力最终应该属于 inode 或目录抽象（directory abstraction），
    /// AGENT: 而不是属于一个已经打开的普通文件句柄（handle）。
    pub fn lookup(&self, _path: &str, _depth: usize) -> Result<(), &'static str> {
        Ok(())
    }

    /// AGENT: 读取一个目录项。
    /// AGENT:
    /// AGENT: 当前行为会返回 `entry_0` 这类合成名字。真实实现应该从目录 inode
    /// AGENT: 暴露目录项（directory entries）。
    pub fn read_entry(&self) -> Result<String, &'static str> {
        let mut d = self.desc.write().unwrap();
        if !d.opt.rd {
            return Err("ebadf");
        }
        let off = d.off;
        d.off += 1;
        Ok(format!("entry_{}", off))
    }

    /// AGENT: 给 poll/epoll 报告就绪状态（readiness）。
    /// AGENT:
    /// AGENT: 目前仍然是简化实现：普通文件按打开标志（open flags）报告
    /// AGENT: 可读/可写（readable/writable）；元数据（metadata）读取失败时才报告
    /// AGENT: 错误（error）。
    pub fn poll_status(&self) -> (bool, bool, bool) {
        let desc = self.desc.read().unwrap();
        let readable = desc.opt.rd;
        let writable = desc.opt.wr;
        drop(desc);
        let error = self.path.is_empty() || self.fs.read_meta(self.meta_index).is_err();
        (readable, writable, error)
    }

    /// AGENT: 设备特定的控制操作（device-specific control）。
    /// AGENT:
    /// AGENT: 目前只是占位实现。真实 `ioctl` 应该由终端（tty）、设备（device）、
    /// AGENT: 套接字（socket）或 inode 自己的文件操作（file operations）实现。
    pub fn io_ctl(&self, cmd: u32, _arg: usize) -> Result<usize, &'static str> {
        match cmd {
            0..=0xFF => Ok(0),
            _ => Ok(0),
        }
    }

    /// AGENT: 把这个文件映射到一段虚拟地址范围（virtual address range）。
    /// AGENT:
    /// AGENT: 目前只是占位实现。真实实现需要建立 VMA、处理缺页异常（page fault），
    /// AGENT: 并且接入 inode 和页缓存（page cache）。
    pub fn mmap(&self, start: usize, end: usize, off: usize) -> Result<(), &'static str> {
        Ok(())
    }

    /// AGENT: 返回当前 SwapFS 后端对象（backing object）。
    /// AGENT:
    /// AGENT: 这个名字目前仍然不准确；等 VFS 存在之后，它应该变成真正的 inode 引用
    /// AGENT: （inode reference）。
    pub fn inode_ref(&self) -> (Arc<SwapFs>, usize) {
        (self.fs.clone(), self.meta_index)
    }

    /// AGENT: 提示某个字节范围可以预读（readahead）。
    /// AGENT:
    /// AGENT: 目前只是占位实现。当前代码只计算页数，不填充页缓存（page cache），
    /// AGENT: 也不提交 I/O。
    pub fn advise_readahead(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        let file_len = self.file_len()?;
        let requested_end = offset.checked_add(len).ok_or("einval")?;
        let actual_end = min(requested_end, file_len);
        let _readahead_pages = (actual_end.saturating_sub(offset) + PAGE_SZ - 1) / PAGE_SZ;
        Ok(())
    }

    /// AGENT: 预分配文件空间（fallocate）。
    /// AGENT:
    /// AGENT: 在当前 SwapFS 里，如果需要会用 0 扩展文件长度。更完整的文件系统应该
    /// AGENT: 区分容量预留（capacity reservation）和可见大小（visible size）。
    pub fn fallocate(&self, offset: usize, len: usize) -> Result<(), &'static str> {
        if !self.desc.read().unwrap().opt.wr {
            return Err("ebadf");
        }
        let needed = offset.checked_add(len).ok_or("einval")?;
        if needed > self.file_len()? {
            self.fs.set_len(self.meta_index, needed)?;
        }
        Ok(())
    }

    /// AGENT: 把数据从这个句柄（handle）复制到另一个句柄。
    /// AGENT:
    /// AGENT: 它会通过 SwapFS 从当前句柄的偏移（offset）开始读取，推进当前偏移，
    /// AGENT: 然后把读到的字节写入目标句柄。
    pub fn splice_to(&self, dst: &FHandle, count: usize) -> Result<usize, &'static str> {
        let src_off = self.desc.read().unwrap().off as usize;
        let size = self.file_len()?;
        if src_off >= size {
            return Ok(0);
        }
        let avail = size - src_off;
        let n = min(count, avail);
        let mut chunk = vec![0u8; n];
        let read = self.read_at(src_off, &mut chunk)?;
        self.desc.write().unwrap().off += read as u64;
        dst.write(&chunk[..read])
    }

    fn file_len(&self) -> Result<usize, &'static str> {
        self.fs.metadata_len(self.meta_index)
    }
}

impl fmt::Debug for FHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let d = self.desc.read().unwrap();
        f.debug_struct("FH")
            .field("off", &d.off)
            .field("path", &self.path)
            .field("meta_index", &self.meta_index)
            .finish()
    }
}

fn checked_seek(base: u64, delta: i64) -> Result<u64, &'static str> {
    if delta >= 0 {
        base.checked_add(delta as u64).ok_or("einval")
    } else {
        base.checked_sub(delta.unsigned_abs()).ok_or("einval")
    }
}
