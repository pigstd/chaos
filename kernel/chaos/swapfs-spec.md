# SwapFS v1 设计说明

## 目标

SwapFS v1 是 `kernel/chaos` 的第一版真实文件系统实验。它不追求完整 VFS，也不实现 inode tree、目录树、权限、日志或复杂块分配。目标是把原来的 `FHandle -> Vec<u8>` 内存文件模型，替换成一个能通过磁盘块寻址读写的简化文件系统。

核心模型：

```text
fd table
  -> FLike::File(FHandle)
      -> SwapFs open file state
          -> metadata index
              -> metadata table record
                  -> continuous data blocks on BlockDevice
```

第一版只支持 flat namespace：

```text
/a
/b
/hello.txt
```

不支持：

```text
/usr/bin/sh
目录文件
hard link
symlink
rename
权限模型
```

## 磁盘布局

磁盘先被看成一个大数组，按固定 block size 切分：

```text
block 0:
  superblock

block bitmap_start_block..bitmap_start_block + bitmap_block_count:
  block bitmap

block meta_start_block..meta_start_block + meta_block_count:
  metadata table

block data_start_block..total_blocks:
  file data blocks
```

建议第一版使用：

```text
SWAPFS_BLOCK_SIZE = 512
SWAPFS_NAME_LEN = 64
SWAPFS_MAGIC = 0x53574150  // "SWAP"
SWAPFS_VERSION = 1
```

磁盘上的结构必须是固定 byte layout，不允许直接写 Rust `String`、`Vec`、`usize` 或指针。

推荐磁盘结构：

```rust
#[repr(C)]
pub struct SwapFsSuperBlockDisk {
    pub magic: u32,
    pub version: u32,
    pub block_size: u32,
    pub total_blocks: u64,
    pub bitmap_start_block: u64,
    pub bitmap_block_count: u64,
    pub meta_start_block: u64,
    pub meta_block_count: u64,
    pub data_start_block: u64,
    pub max_files: u32,
}

#[repr(C)]
pub struct SwapFsMetaDisk {
    pub used: u8,
    pub reserved0: [u8; 7],
    pub name: [u8; SWAPFS_NAME_LEN],
    pub start_block: u64,
    pub block_count: u64,
    pub size: u64,
    pub reserved1: [u8; 32],
}
```

`SwapFsSuperBlockDisk` 当前固定为 64 bytes：

```text
offset 0    magic: u32
offset 4    version: u32
offset 8    block_size: u32
offset 12   total_blocks: u64
offset 20   bitmap_start_block: u64
offset 28   bitmap_block_count: u64
offset 36   meta_start_block: u64
offset 44   meta_block_count: u64
offset 52   data_start_block: u64
offset 60   max_files: u32
offset 64   end
```

`SwapFsMetaDisk` 固定为 128 bytes，不使用 89-byte 紧凑布局。原因是：

```text
512 / 128 = 4 metadata records per block
```

并且 `used` 后面留 7 bytes padding，让后面的字段按 8-byte 边界排列：

```text
offset 0    used: u8
offset 1    reserved0: [u8; 7]
offset 8    name: [u8; 64]
offset 72   start_block: u64
offset 80   block_count: u64
offset 88   size: u64
offset 96   reserved1: [u8; 32]
offset 128  end
```

字段含义：

- `used`: 这个 metadata slot 是否有效。
- `name`: 文件名，固定长度字节数组，第一版只保存不含 `/` 的文件名。
- `start_block`: 文件数据起始 block。
- `block_count`: 文件已经分配的连续 block 数量。
- `size`: 文件真实长度，读操作不能超过它。
- `bitmap_start_block`: block bitmap 的起始 block。当前布局固定为 1。
- `bitmap_block_count`: block bitmap 占用的 block 数量。一个 512-byte bitmap block 可以记录 4096 个 block。
- `meta_start_block`: metadata table 的起始 block，等于 `bitmap_start_block + bitmap_block_count`。
- `meta_block_count`: metadata table 占用的 block 数量。
- `data_start_block`: 文件数据区起始 block，等于 `meta_start_block + meta_block_count`。
- `max_files`: metadata table 允许容纳的最大文件数量。

## 新增模块

新增目录：

```text
kernel/chaos/src/fs/swapfs/
```

新增模块建议：

```text
fs/swapfs/mod.rs
fs/swapfs/layout.rs
fs/swapfs/fs.rs
fs/swapfs/bitmap.rs
fs/swapfs/metadata.rs
fs/swapfs/data.rs
```

职责：

| 模块 | 职责 |
| --- | --- |
| `layout.rs` | 定义磁盘格式常量、superblock、metadata record，以及 encode/decode 辅助函数。 |
| `fs.rs` | 定义 `SwapFs`，负责 format/mount、superblock 校验、bitmap 初始化和分配并发保护。 |
| `bitmap.rs` | 负责从磁盘 bitmap block 读写 bit，扫描连续空闲 data blocks，并标记 alloc/free。 |
| `metadata.rs` | 负责按 metadata index 定位/读写 record，以及文件名查找、metadata slot 分配、create/open。 |
| `data.rs` | 负责按 metadata index 执行 read_at/write_at/set_len，以及跨 block 读写、扩容搬家、补零。 |
| `mod.rs` | 声明 SwapFS 子模块，并对外 re-export `SwapFs` 和 layout 常量。 |

`fs/mod.rs` 当前已经导出：

```rust
pub mod swapfs;
pub use swapfs::*;
```

同时新增一个很小的终端对象模块：

```text
fs/tty.rs
```

`tty.rs` 不属于 SwapFS。它只负责 stdin/stdout/stderr 这类终端 fd，让标准 fd 不再伪装成普通 `FHandle` 文件。

`TTY` 是 Unix 里的传统名字，来自 teletype。历史上终端设备就是电传打字机；现在在内核语境里通常泛指 terminal/console 这类字符设备。所以这里叫 `TtyHandle`，意思是“终端/控制台句柄”，不是磁盘文件句柄。

## 需要修改的现有模块

### `fs/disk.rs`

`Disk` 当前是用户态模拟用的内存块设备：内部用一个数组伪装磁盘。SwapFS 不直接依赖具体的 `Disk` 类型，而是依赖 `BlockDevice` trait；未来替换成真实磁盘驱动时，只要实现同样的 `read_block/write_block` 语义即可。

`fs/block_device.rs` 暴露块设备 trait：

```rust
pub trait BlockDevice: Send + Sync + std::any::Any {
    fn block_size(&self) -> usize;
    fn block_count(&self) -> u64;
    fn read_block(&self, block_id: u64, out: &mut [u8]) -> Result<(), &'static str>;
    fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), &'static str>;
    fn flush(&self) -> Result<(), &'static str>;
}
```

`Disk` 对外暴露的核心接口：

```rust
impl Disk {
    pub fn new(label: &str, blocks: u64, block_size: usize) -> Self;
    pub fn block_size(&self) -> usize;
    pub fn block_count(&self) -> u64;
    pub fn read_block(&self, block_id: u64, out: &mut [u8]) -> Result<(), &'static str>;
    pub fn write_block(&self, block_id: u64, data: &[u8]) -> Result<(), &'static str>;
    pub fn flush(&self) -> Result<(), &'static str>;
}
```

内部结构建议：

```rust
pub struct Disk {
    pub ops: AtomicUsize,
    pub label: String,
    block_size: usize,
    blocks: u64,
    storage: Mutex<Vec<u8>>,
}
```

行为要求：

- `Disk::new(label, blocks, block_size)` 创建 `blocks * block_size` 字节的零初始化数组。
- `read_block(block_id, out)` 从内部数组读取一个 block。
- `write_block(block_id, data)` 把一个 block 写入内部数组。
- `out.len()` 和 `data.len()` 必须等于 `block_size`，否则返回 `Err("einval")`。
- `block_id >= blocks` 返回 `Err("einval")`。
- `flush()` 第一版只记录一次操作并返回 `Ok(())`。
- `ops` 用于测试和观测读写次数。

旧的磁盘 mock 行为不再保留。新的正确语义是：写入某个 block 后，再读同一个 block 必须读回同样的数据。

### `fs/tty.rs`

在改 `FHandle` 之前，先把标准输入输出从普通文件里拆出来：

```rust
pub enum TtyKind {
    Stdin,
    Stdout,
    Stderr,
}

#[derive(Clone)]
pub struct TtyHandle {
    pub kind: TtyKind,
    pub(crate) desc: Arc<RwLock<FdState>>,
    pub cloexec: bool,
}
```

第一版行为：

```text
stdin.read      -> 暂时返回 Ok(0) 表示 EOF，后续再接键盘/console input
stdin.write     -> Err("ebadf")
stdout.read     -> Err("ebadf")
stdout.write    -> 写到当前用户态模拟环境的 stdout，并返回 buf.len()
stderr.read     -> Err("ebadf")
stderr.write    -> 写到当前用户态模拟环境的 stderr，并返回 buf.len()
poll_status     -> stdin 按无输入处理；stdout/stderr 永远 writable
```

这是用户态模拟内核的占位 console sink。真实内核移植时应把这层替换成串口、SBI console 或其他 console driver，而不是改回 `FHandle`。

`FLike` 当前已经有 `Tty` 分支：

```rust
pub enum FLike {
    File(FHandle),
    Tty(TtyHandle),
    Pipe(PipeNode),
    Ep(EpInst),
}
```

`FLike::{read,write,poll,io_ctl,dup}` 里都已经分发到 `Tty`。重点是：`TtyHandle` 和 `FHandle` 同属于 fd table 里的 file-like object，但它们不是同一种底层对象。

### `fs/fd.rs`

当前 `FHandle` 已经保存：

```rust
path: String
fs: Arc<SwapFs>
meta_index: usize
desc: Arc<RwLock<FdState>>
cloexec: bool
```

`FHandle` 是 SwapFS 普通文件句柄，不再保留 memory-backed 普通文件设计。当前结构：

```rust
pub struct FHandle {
    pub path: String,
    pub fs: Arc<SwapFs>,
    pub meta_index: usize,
    pub(crate) desc: Arc<RwLock<FdState>>,
    pub cloexec: bool,
}
```

含义：

```text
path       = 打开时使用的路径标签
fs         = 所属 SwapFS 实例
meta_index = 指向 SwapFS metadata table 的 index
desc       = open-file state，保存 offset 和 open flags
cloexec    = exec 时是否关闭
```

`FHandle::read_at/write_at/metadata_sz/set_len/fallocate` 已经改为调用 SwapFS：

```text
read_at      -> self.fs.read_at(self.meta_index, off, buf)
write_at     -> self.fs.write_at(self.meta_index, off, buf)
metadata_sz  -> self.fs.metadata_len(self.meta_index)
set_len      -> self.fs.set_len(self.meta_index, len)
fallocate    -> 如果 offset + len 超过当前 size，则 self.fs.set_len(...)
```

`FHandle::new` 当前是创建 SwapFS 普通文件句柄的构造函数：

```rust
pub fn new(path: &str, fs: Arc<SwapFs>, meta_index: usize, opt: FdOpt, cloexec: bool) -> Self;
```

`FHandle::with_data` 和 `data: Arc<Mutex<Vec<u8>>>` 已删除。`inode_ref()` 暂时返回 `(Arc<SwapFs>, meta_index)`，只是兼容旧名字；后续有 VFS/inode 时应改成真正 inode 引用。

`FHandle` 当前核心行为：

| 方法 | 当前行为 |
| --- | --- |
| `new` | 保存 `path/fs/meta_index/desc/cloexec` |
| `dup` | 共享同一个 `desc`，所以共享 offset；但允许新的 `cloexec` |
| `read` | 检查读权限，按 `desc.off` 调 `read_at`，成功后推进 offset |
| `read_at` | 检查读权限后调 `SwapFs::read_at(meta_index, off, buf)` |
| `write` | 检查写权限；append 模式用文件当前 size 作为 offset；成功后推进 offset |
| `write_at` | 检查写权限后调 `SwapFs::write_at(meta_index, off, buf)` |
| `seek` | 只修改 `desc.off`；`End` 需要读取 SwapFS metadata size |
| `metadata_sz` | 返回 SwapFS metadata size |
| `set_len` | 调 `SwapFs::set_len`，用于 open truncate 和后续 truncate syscall |

其他方法当前多为兼容旧接口的 stub，但已经不再访问旧的 `data` 字段：

| 方法 | 当前处理 |
| --- | --- |
| `sync_all/sync_data` | 返回 `Ok(())`，注释标记为 stub；当前没有 page cache/writeback 路径 |
| `lookup` | 返回 `Ok(())` 的占位函数；目录语义后续应挪到 inode/VFS 层 |
| `read_entry` | 返回 `entry_N` 这类合成名字，只是兼容旧接口 |
| `poll_status` | 普通文件按 open flags 认为 ready；metadata 读失败时报告 error |
| `io_ctl` | 当前总是返回 `Ok(0)`；真实 `ioctl` 后续应由 tty/device/socket/inode 自己实现 |
| `mmap` | 当前只校验/占位并返回 `Ok(())`；等 VM/page cache 后再做 |
| `inode_ref` | 暂时返回 `(Arc<SwapFs>, meta_index)`；后续 VFS 化时改名或删除 |
| `advise_readahead` | 只计算请求范围，不提交真实 I/O，返回 `Ok(())` |
| `fallocate` | v1 暂时通过 `SwapFs::set_len` 实现，会改变可见 size |
| `splice_to` | 通过 `read_at` + `dst.write` 实现；不直接访问任何内部 backing |

旧的 memory-backed `data` 字段已经从普通文件路径中移除。当前迁移结果：

| 方法 | 当前行为 |
| --- | --- |
| `dup` | clone `fs`、`meta_index` 和共享的 `desc` |
| `read` | 用 `desc.off` 调 `read_at`，底层走 SwapFS |
| `write` | append 时用 `metadata_len()`，底层走 SwapFS |
| `seek(End)` | 用 SwapFS metadata size |
| `set_len` | 调 `SwapFs::set_len`，必要时扩容分配块 |
| `metadata_sz` | 返回 metadata 里的 `size` |
| `poll_status` | 按 open flags 返回 ready，metadata 读失败时报 error |
| `fallocate` | 当前调 `SwapFs::set_len`，后续再拆成只保留 capacity 的语义 |
| `splice_to` | 通过 `read_at` + `dst.write` 做，不直接看内部字段 |

`lookup`、`read_entry` 暂时不应该继续挂在普通文件 `FHandle` 上实现目录语义。SwapFS v1 没有目录，可以先保留为 stub；后续有目录时应该挪到目录文件或 VFS/inode 层。

`/dev/tty` 这类对象不是 SwapFS 普通文件。当前代码已经采用 `FLike::Tty` 表示 fd0/fd1/fd2，避免把设备文件塞进 SwapFS metadata table。

### `fs/filelike.rs`

`FLike::File` 的分发思路可以保留：

```rust
FLike::File(f) => f.read(...)
FLike::File(f) => f.write(...)
```

`FLike::File` 应调用 `FHandle` 方法，而不是在 `FLike` 里展开普通文件细节：

```rust
FLike::File(f) => f.read(buf)
FLike::File(f) => f.write(buf)
```

这样 `FLike` 不需要知道 `FHandle` 如何通过 SwapFS metadata index 定位磁盘块。

### `kernel_api/lifecycle.rs`

`Kernel` 已经持有一个默认 SwapFS：

```rust
pub disk: Arc<Disk>,
pub swapfs: Arc<SwapFs>,
```

当前初始化：

```text
Kernel::new(nf)
  -> Arc::new(Disk::new("", 1024, SWAPFS_BLOCK_SIZE))
  -> SwapFs::mount_or_format(disk, 1024, 128)
```

### `kernel_api/syscall.rs`

当前 `SYS_OPEN/SYS_READ/SYS_WRITE/SYS_CLOSE` 已经接到 SwapFS-backed fd 路径。

用户态模拟版本没有真实页表，所以 syscall 层暂时把传入的 `path_addr/buf_addr`
当成当前 Rust 进程里的真实指针（raw pointer）使用。

```text
SYS_OPEN:
  1. 校验 path_addr。
  2. 通过用户态模拟 raw pointer 读取 NUL 结尾路径字符串。
  3. 不经过完整 `MountTable` 挂载语义，直接把路径交给 SwapFS v1。
  4. 调 `Kernel::open_file_for_task(task_id, path, flags, mode)`。
  5. 创建 `FHandle::new(path, fs, meta_index, opt, cloexec)`。
  6. 插入当前 `Task.files`，返回 fd。

SYS_READ:
  1. 找当前 task。
  2. 检查用户输出 buffer 地址。
  3. 把 `buf_addr` 转成 `&mut [u8]`。
  4. 调 `Kernel::read_fd`，直接写入这个用户态模拟 buffer。

SYS_WRITE:
  1. 找当前 task。
  2. 检查用户输入 buffer 地址。
  3. 把 `buf_addr` 转成 `&[u8]`。
  4. 调 `Kernel::write_fd` 写入 `FLike`。

SYS_CLOSE:
  1. 找当前 task。
  2. 调 `Kernel::close_fd` 从 `Task.files` 移除 fd。
  3. 不需要删除文件数据。
```

当前已经实现内核内部 API：

```rust
Kernel::open_file_for_task(task_id, path, flags, mode) -> Result<usize, &'static str>
Kernel::read_fd(task_id, fd, buf: &mut [u8]) -> Result<usize, &'static str>
Kernel::write_fd(task_id, fd, buf: &[u8]) -> Result<usize, &'static str>
Kernel::close_fd(task_id, fd) -> Result<(), &'static str>
```

syscall 层现在只负责参数转换和用户态模拟 raw pointer 转换，文件读写路径是：

```text
dispatch_syscall -> simulator raw pointer -> Kernel fd API -> Task.files -> FLike -> FHandle -> SwapFS -> Disk
```

这里的 raw pointer 访问只是用户态模拟内核里的演示替代物，不是真实页表。
真实内核移植时应该把 `simulator_user_slice/simulator_user_slice_mut/simulator_user_cstr`
换成页表权限检查和 `copy_from_user/copy_to_user`。

### `process/task.rs`

`Task.files: BTreeMap<usize, FLike>` 可以暂时保留。

需要重点确认：

- `Task::add_file` 仍然只负责分配 fd。
- `Task::dup_fd` 需要共享 `FHandle.desc`，这样 dup 后共享 offset。
- `cloexec` 长期应该移到 fd entry，但 v1 可以继续放在 `FHandle`。
- `new_user_task()` 当前已经用 `FLike::Tty(TtyHandle::stdin/stdout/stderr)` 初始化 fd0/fd1/fd2，不再通过 SwapFS 普通文件伪装 `/dev/tty`。

## SwapFS 内部 API

推荐最小 API：

```rust
pub struct SwapFs {
    disk: Arc<dyn BlockDevice>,
    sb: RwLock<SwapFsSuperBlockDisk>,
    op_lock: crate::sync::rwlock::RwLock,
    alloc: Mutex<()>,
    bitmap: Bitmap,
}
```

`op_lock` 是 SwapFS v1 的粗粒度文件系统操作锁。当前规则：

```text
read guard:
  read_meta / find_meta_by_name / find_free_meta / open / metadata_len / read_at

write guard:
  write_meta / alloc_blocks / create / open_or_create(create=true) / write_at / set_len
```

实现上采用 public API 加锁、内部 `_locked` helper 不重复加锁的结构。原因是当前 `sync::rwlock::RwLock` 不是可重入锁；如果 `write_at()` 拿了 write lock 后又调用会拿 read lock 的 public `read_meta()`，会自旋等待自己释放锁，形成死锁。

`alloc: Mutex<()>` 不是分配状态。它只是在当前进程里串行化分配/释放操作，避免两个线程同时扫描并修改 bitmap。空闲块状态的真实来源在磁盘 bitmap blocks 里。

SwapFS v1 不在 `mount()` 时把完整 metadata table 加载成 `Vec<SwapFsMeta>`。metadata 的真实位置是磁盘上的 metadata blocks；运行时按 `metadata index` 读写对应 record：

```text
meta_block  = meta_start_block + index / SWAPFS_META_PER_BLOCK
meta_offset = (index % SWAPFS_META_PER_BLOCK) * SWAPFS_META_DISK_SIZE
```

这样 `FHandle` 只需要保存 `meta_index`，后续要查 size/start_block 时通过 `SwapFs::read_meta(index)` 从磁盘读取。metadata cache 可以后续再加，不作为 v1 的必需结构。

block bitmap 的真实位置也在磁盘上：

```text
bitmap_start_block = 1
bitmap_block_count = ceil(total_blocks / (SWAPFS_BLOCK_SIZE * 8))
```

`Bitmap` 不把完整 bitmap 常驻内存。它保存 bitmap 区域的位置和允许分配的 data block 范围；`alloc_blocks/set_use/set_free/is_free` 执行时按需读写对应 bitmap block。

方法：

```rust
impl SwapFs {
    pub fn format(disk: Arc<dyn BlockDevice>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
    pub fn mount(disk: Arc<dyn BlockDevice>) -> Result<Arc<Self>, &'static str>;
    pub fn mount_or_format(disk: Arc<dyn BlockDevice>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
    pub fn super_block(&self) -> SwapFsSuperBlockDisk;
    pub fn max_files(&self) -> usize;
    pub fn read_meta(&self, meta_index: usize) -> Result<SwapFsMetaDisk, &'static str>;
    pub fn write_meta(&self, meta_index: usize, meta: &SwapFsMetaDisk) -> Result<(), &'static str>;
    pub fn find_meta_by_name(&self, name: &str) -> Result<usize, &'static str>;
    pub fn find_free_meta(&self) -> Result<usize, &'static str>;
    pub fn alloc_blocks(&self, block_count: u64) -> Result<u64, &'static str>;
    pub fn open(&self, name: &str) -> Result<usize, &'static str>;
    pub fn create(&self, name: &str, initial_blocks: u64) -> Result<usize, &'static str>;
    pub fn open_or_create(&self, name: &str, create: bool, initial_blocks: u64) -> Result<usize, &'static str>;
    pub fn read_at(&self, meta_index: usize, off: usize, buf: &mut [u8]) -> Result<usize, &'static str>;
    pub fn write_at(&self, meta_index: usize, off: usize, buf: &[u8]) -> Result<usize, &'static str>;
    pub fn set_len(&self, meta_index: usize, len: usize) -> Result<(), &'static str>;
    pub fn metadata_len(&self, meta_index: usize) -> Result<usize, &'static str>;
}
```

当前代码还没有实现 `unlink`。metadata slot 的复用来自 `find_free_meta()` 扫描 `used == 0` 的 record；普通文件扩容搬家时，旧 data block 区间会通过 bitmap 释放。后续实现删除文件时，应该同时清 metadata record 并释放对应 data blocks。

错误约定第一版可以继续使用 `&'static str`：

```text
enoent   找不到文件
eexist   O_CREAT | O_EXCL 时文件已存在
enospc   没有 metadata slot 或没有连续数据空间
einval   文件名非法、offset 溢出、block 越界
eio      磁盘读写失败
ebadf    fd 或打开权限错误
```

## 挂载模型

这里的“挂载”第一版只表示：从一个 `BlockDevice` 上读取 superblock，把它变成内存里的 `Arc<SwapFs>`。metadata table 不会整体读入内存，后续通过 `read_meta(index)` 按需读取。它不是完整的 Linux mount namespace，也没有多个挂载点、路径覆盖、bind mount 或 `/proc`、`/dev` 这类特殊文件系统。

当前代码已有 `MountTable` / `Kernel.mnt`，但 SwapFS v1 暂时不接它。`MountTable` 现在只是路径前缀到字符串 target 的重写表，例如把 `/mnt/a` 解析成 `dev0:/a`；它还没有把 `dev0` 映射到真实 `Disk`、`SwapFs` 或 VFS 对象。为了先跑通真实文件读写，v1 的 `open/read/write` 主线不要经过 `MountTable`。

v1 的结构是：

```text
Kernel
  disk: Arc<Disk>              // 当前默认使用用户态内存 Disk
  swapfs: Arc<SwapFs>

路径 "/a" 或 "a"
  -> normalize 成 "a"
  -> 在 Kernel.swapfs 里查 metadata
```

也就是说，第一版默认所有普通文件都属于根 SwapFS：

```text
/a
/b
/hello.txt
```

都直接走：

```text
Kernel.swapfs.open_or_create(...)
```

不处理：

```text
/mnt/disk1/a
/dev/tty
/proc/1/stat
```

这些路径需要 VFS/mount/devfs/procfs 之后再做。已有 `MountTable` 代码先保留，但不作为 SwapFS v1 的依赖。

三个函数的职责应该分开：

```text
SwapFs::format(disk, total_blocks, max_files)
  破坏性初始化文件系统。
  写入 superblock。
  计算 bitmap_block_count。
  清空 bitmap blocks 和 metadata table。
  设置 meta_start_block = 1 + bitmap_block_count。
  设置 data_start_block = meta_start_block + meta_block_count。
  根据 superblock 创建 Bitmap 对象。
  返回新建好的 SwapFs。

SwapFs::mount(disk)
  非破坏性加载已有文件系统。
  读取 block 0。
  校验 magic/version/block_size/total_blocks/bitmap/meta/data 布局。
  不加载完整 metadata table。
  根据 superblock 创建 Bitmap 对象。
  返回 SwapFs。

SwapFs::mount_or_format(disk, total_blocks, max_files)
  测试和用户态演示用便利函数。
  如果 block 0 是合法 SwapFS，就 mount。
  如果不是合法 SwapFS，就 format。
```

长期看，真实内核一般不会无条件 `mount_or_format`，因为磁盘格式不认识时直接格式化会丢数据。第一版为了演示和测试可以这样做，但代码注释里应该说明它是开发期便利接口。

`/dev/tty` 不通过这个挂载模型解决。它应该由 `FLike::Tty` 或未来的 devfs 提供，而不是写进 SwapFS 的 metadata table。

## 分配与删除策略

第一版分两类分配：

```text
metadata slot 分配：
  扫描 metadata table。
  找到第一个 used == 0 的 record。
  把新文件 metadata 写入这个 slot。

data block 分配：
  通过磁盘上的 block bitmap 分配。
  当前 metadata 只记录 start_block + block_count，所以必须找到连续空闲 blocks。
  alloc_blocks(0) 返回 Ok(0)，用于空文件或还没有分配数据块的文件。
```

metadata slot 查找：

```text
find_meta_by_name(name):
  for index in 0..max_files:
    meta = read_meta(index)
    if meta.used && meta.name == name:
      return index
  return enoent

find_free_meta():
  for index in 0..max_files:
    meta = read_meta(index)
    if !meta.used:
      return index
  return enospc
```

创建文件第一版流程：

```text
create:
  1. normalize/validate name。
  2. find_meta_by_name(name)，如果已存在则按 flags 返回已有 index 或 eexist。
  3. find_free_meta() 得到 metadata index。
  4. start_block = alloc_blocks(initial_blocks)。
  5. block_count = initial_blocks。
  6. write_meta(index, new_meta)。
  7. alloc_blocks 内部通过 bitmap.set_use() 把 data blocks 标记为已占用。
```

删除当前还没有实现 `unlink` API。后续实现时应做两件事：

```text
unlink:
  1. metadata.used = false，写回 metadata table。
  2. 如果 block_count > 0，通过 bitmap.set_free(start_block..start_block + block_count - 1) 释放 data blocks。
```

当前版本中，文件扩容搬家成功后会释放旧 data block 区间，所以被 `bitmap.set_free()` 释放过的 data blocks 可以复用。单独把 metadata record 写成 unused 不会自动释放旧 blocks，因为当前还没有 `unlink`。metadata slot 仍然只由 `SwapFsMetaDisk.used` 表示，没有单独 metadata bitmap。

## 文件内容读写

当前 `read_at/write_at/set_len` 直接根据 metadata 里的 `start_block + block_count + size` 访问 data blocks：

```text
read_at(index, off, buf):
  1. read_meta(index)。
  2. 检查 metadata.used、size <= block_count * block_size、data block 范围合法。
  3. 如果 off >= size，返回 0。
  4. 最多读取 min(buf.len, size - off) 字节。
  5. 读操作可以跨 block，但不会读出 metadata.size 之外的数据。

write_at(index, off, buf):
  1. read_meta(index)。
  2. 计算 end = off + buf.len，检查 offset 溢出。
  3. 如果 end 超过当前容量，则先按 capacity 策略扩容搬家。
  4. 如果 off > old_size，中间空洞写 0。
  5. 把 buf 写入对应 data blocks。
  6. 如果 end > old_size，更新 metadata.size。

set_len(index, len):
  1. 如果 len 超过当前容量，则先按 capacity 策略扩容搬家。
  2. 如果 len > old_size，新增可见区间写 0。
  3. 如果 len < old_size，只缩小 metadata.size，不回收 data blocks。
```

真实文件系统通常会维护 free-space metadata，例如：

```text
inode/meta bitmap   管 metadata slot 是否空闲
block bitmap        管 data block 是否空闲
free list/tree      管空闲区间
```

SwapFS 当前已经把 data block 分配切到 bitmap：

```text
block 0        superblock
block 1..B     block bitmap
block B+1..M   metadata table
block data...  file data
```

第一版 bitmap 只记录 data block 是否空闲，不单独引入 metadata bitmap。metadata slot 是否空闲仍由 `SwapFsMetaDisk.used` 表示。

如果仍保持 `start_block + block_count` 的连续文件模型，bitmap 分配时需要找连续空闲区间；如果允许非连续 blocks，则 metadata 需要升级为 extent list 或 block list。

扩容：

当前实现采用简单搬家：

```text
write 超过当前 block_count:
  1. 计算 required_blocks = ceil(desired_size / block_size)。
  2. 计算 grown_blocks = max(required_blocks, max(1, old_block_count * 2))。
  3. 先检查原文件后面的 blocks 是否足够空闲；如果足够，直接用 bitmap 标记为已使用并扩大 block_count。
  4. 如果不能原地扩展，则通过 bitmap 分配一个新的连续区间。
  5. 如果 grown_blocks 分配失败，则 fallback 到 required_blocks 再试一次。
  6. 把旧文件的可见内容复制到新位置。
  7. 更新 metadata.start_block/block_count；调用者随后写回 metadata.size。
  8. 新区间分配和复制成功后，通过 bitmap 释放旧 block 区间。
```

## 路径规则

第一版只支持根目录下文件名：

```text
"a"
"/a"
"/hello.txt"
```

统一 normalize 成不带前导 `/` 的 name：

```text
"/hello.txt" -> "hello.txt"
"hello.txt"  -> "hello.txt"
```

非法路径：

```text
""
"/"
"/a/b"
"a/b"
超过 SWAPFS_NAME_LEN 的名字
包含 NUL byte 的名字
```

## 测试计划

新增文件系统重构测试统一放到 `fs-refactor` integration test：

```text
chaos-tests/tests/fs_refactor/main.rs
chaos-tests/tests/fs_refactor/disk.rs
chaos-tests/tests/fs_refactor/tty.rs
chaos-tests/tests/fs_refactor/swapfs_layout.rs
chaos-tests/tests/fs_refactor/swapfs_format.rs
chaos-tests/tests/fs_refactor/swapfs_metadata.rs
chaos-tests/tests/fs_refactor/swapfs_data.rs
chaos-tests/tests/fs_refactor/fhandle.rs
chaos-tests/tests/fs_refactor/kernel_fd.rs
chaos-tests/tests/fs_refactor/syscall_fd.rs
chaos-tests/tests/fs_refactor/syscall_e2e.rs
```

测试统一放在 `chaos-tests/tests/fs_refactor/` 下。当前已经覆盖 Disk、Tty、SwapFS layout、format/mount、metadata record 读写、metadata create/open、SwapFS data read/write/set_len/growth、FHandle、Kernel fd API、syscall fd 和复杂 syscall end-to-end。

最小测试：

1. format/mount：
   - `Disk::new(label, blocks, block_size)`
   - `SwapFs::format(...)`
   - superblock magic/version 正确。
   - `SwapFs::mount(...)` 能从同一个 disk 重新加载已有 superblock 和 metadata。
   - `SwapFs::mount_or_format(...)` 遇到合法 SwapFS 时必须 mount，不能重新 format。

2. create/open：
   - 创建 `/a`。
   - 再 open `/a` 能拿到同一个 metadata index。
   - 重复 create with exclusive 返回 `eexist`。
   - `open_or_create` 打开已有文件时不重新分配 data blocks。
   - `alloc_blocks(0)` 允许创建空文件，后续写入时再由 bitmap 分配 data blocks。
   - 直接把 metadata slot 写成 unused 后，slot 可以复用，但旧 data blocks 不会自动释放。
   - 重新 mount 后仍能通过 metadata 找到已创建的文件。

3. read/write：
   - 写入 `hello`。
   - read_at 能读回 `hello`。
   - read 不能超过 `size`。
   - 跨 block 写入后能完整读回。
   - 稀疏写入时，中间空洞读出来是 0。
   - set_len 扩大文件时新增区域是 0，缩小时只改变可见 size。

4. fd offset：
   - 同一个 fd 连续读会推进 offset。
   - 两次独立 open offset 不共享。
   - dup 后 offset 共享。

5. metadata persistence：
   - 写文件后重新 mount 同一个 memory disk。
   - 文件 metadata 和内容仍可读取。

6. expand：
   - 写入超过初始 block capacity。
   - 搬家后确认内容完整且 metadata 更新。
   - 空间不足时返回 `enospc`，metadata size 和 bitmap 不应被错误推进。
   - 原地扩容、搬家扩容、增长容量 fallback 到 required capacity 都有覆盖。

7. unlink：
   - 当前还没有实现；后续应测试 unlink 后 open 返回 `enoent`，并且 data blocks 被 bitmap 释放后可以复用。

8. syscall/Kernel API：
   - `Kernel::open_file_for_task` 返回 fd。
   - `Kernel::write_fd/read_fd` 通过 fd 读写 SwapFS 文件。
   - `Kernel::close_fd` 从对应 task 的 fd table 移除 fd。
   - 覆盖 create/open、append、truncate、权限错误、close 后 ebadf。
   - `SYS_OPEN/READ/WRITE/CLOSE` 通过用户态模拟 raw pointer 接到这条路径。

9. Disk 块设备语义：
   - 新建 `Disk::new("d0", 8, 512)` 后，读取任意 block 返回全 0。
   - 写入 block 3 后，读取 block 3 能得到完全相同的数据。
   - 写入 block 3 不影响 block 2 或 block 4。
   - `out.len() != block_size` 或 `data.len() != block_size` 返回 `einval`。
   - 越界 block 返回 `einval`。

10. Tty 拆分：
   - `new_user_task()` 的 fd0/fd1/fd2 不再创建 `FHandle("/dev/tty")`。
   - fd0 是 `FLike::Tty(Stdin)`。
   - fd1 是 `FLike::Tty(Stdout)`。
   - fd2 是 `FLike::Tty(Stderr)`。
   - stdout/stderr write 返回写入长度。
   - stdin write 返回 `ebadf`。

11. FHandle SwapFS 化：
   - `FHandle::new(path, fs, meta_index, opt, cloexec)` 创建的句柄读写 SwapFS。
   - `FHandle::read/write` 推进 offset。
   - `FHandle::dup` 共享 offset，但可设置新的 `cloexec`。
   - `FLike::File` 只调用 `FHandle` 方法，不直接访问 `desc`、`data` 或 SwapFS 内部字段。
   - 标准 fd 不能再通过 SwapFS 普通文件伪装 `/dev/tty`。

## 实现顺序

当前主线已经按这个顺序落地：

1. 增加 `BlockDevice` trait，并让 `Disk::new/read_block/write_block` 作为内存块设备实现它。
2. 增加 `fs/tty.rs` 和 `FLike::Tty`，把 `new_user_task()` 的 fd0/fd1/fd2 从 `FHandle("/dev/tty")` 迁移到 `TtyHandle`。
3. 增加 `fs/swapfs/layout.rs`，实现 superblock/meta record 的 encode/decode。
4. 增加 `fs/swapfs/bitmap.rs`，用磁盘 bitmap block 记录 data block 是否空闲。
5. 增加 `SwapFs::format/mount/mount_or_format`，能读写 superblock、清空 bitmap 和 metadata table。
6. 实现 `SwapFs::create/open/open_or_create/metadata_len`，跑通“文件名 -> metadata index -> bitmap 分配 data blocks”。
7. 实现 `SwapFs::read_at/write_at/set_len`，让 metadata 指向的连续 data blocks 能读写文件内容，并在扩容搬家后释放旧 block 区间。
8. 改 `FHandle`，删除 `data: Arc<Mutex<Vec<u8>>>`，改成 `fs: Arc<SwapFs>` 和 `meta_index: usize`。
9. 让 `FHandle::read_at/write_at/metadata_sz/set_len/fallocate` 直接调用 SwapFS。
10. 改 `FLike::File`，让它只调用 `FHandle` 方法，不直接访问 `FHandle` 内部字段。
11. 增加 `Kernel` 内部 fd API：`open_file_for_task/read_fd/write_fd/close_fd`。
12. 改 `dispatch_syscall` 的 `SYS_OPEN/SYS_READ/SYS_WRITE/SYS_CLOSE`，通过用户态模拟 raw pointer 访问 buffer。
13. 给 `sync::rwlock::RwLock` 增加 RAII guard，并在 `SwapFs` 上增加全局 `op_lock`，先用粗粒度锁保护 metadata/bitmap/data 的多步操作。

## 暂时不做

- 嵌套目录。
- 真正 inode 编号和 inode cache。
- dentry cache。
- 权限、owner、mode、mtime/ctime/atime。
- hard link/symlink。
- extent list、block list 或更复杂的 free-space tree。
- per-file、per-metadata-block、per-bitmap-block 这类细粒度并发锁。
- journaling。
- 崩溃恢复和跨内核实例共享同一块磁盘。
- page cache 和 mmap 文件页。
- 异步 I/O 调度。

这些后续可以在 SwapFS v2 或 VFS 重构时补。
