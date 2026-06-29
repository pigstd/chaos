# SwapFS v1 设计说明

## 目标

SwapFS v1 是 `kernel/chaos` 的第一版真实文件系统实验。它不追求完整 VFS，也不实现 inode tree、目录树、权限、日志或复杂块分配。目标是把当前 `FHandle -> Vec<u8>` 的内存文件，替换成一个能通过磁盘块寻址读写的简化文件系统。

核心模型：

```text
fd table
  -> FLike::File(FHandle)
      -> SwapFs open file state
          -> metadata index
              -> metadata table record
                  -> continuous data blocks on Disk
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

block 1..meta_block_count:
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
    pub meta_start_block: u64,
    pub meta_block_count: u64,
    pub data_start_block: u64,
    pub next_free_block: u64,
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
- `next_free_block`: 顺序分配指针。第一版删除文件后不回收空间，所以它只递增。

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
fs/swapfs/metadata.rs
fs/swapfs/data.rs
```

职责：

| 模块 | 职责 |
| --- | --- |
| `layout.rs` | 定义磁盘格式常量、superblock、metadata record，以及 encode/decode 辅助函数。 |
| `fs.rs` | 定义 `SwapFs` 和 `SwapFsAlloc`，负责 format/mount、superblock 同步、顺序 data block 分配。 |
| `metadata.rs` | 负责按 metadata index 定位/读写 record，以及文件名查找、metadata slot 分配、create/open。 |
| `data.rs` | 负责按 metadata index 执行 read_at/write_at/set_len，以及跨 block 读写、扩容搬家、补零。 |
| `mod.rs` | 声明 SwapFS 子模块，并对外 re-export `SwapFs` 和 layout 常量。 |

`fs/mod.rs` 需要新增：

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

`Disk` 应该直接改成真正的块设备抽象。第一版不用关心真实硬件驱动，内部先用一个数组伪装磁盘；未来替换成真实磁盘驱动时，保持 `read_block/write_block` 语义不变即可。

`Disk` 对外暴露的核心接口：

```rust
impl Disk {
    pub fn new(label: &str, blocks: usize, block_size: usize) -> Self;
    pub fn block_size(&self) -> usize;
    pub fn block_count(&self) -> usize;
    pub fn read_block(&self, block_id: usize, out: &mut [u8]) -> Result<(), &'static str>;
    pub fn write_block(&self, block_id: usize, data: &[u8]) -> Result<(), &'static str>;
    pub fn flush(&self) -> Result<(), &'static str>;
}
```

内部结构建议：

```rust
pub struct Disk {
    pub ops: AtomicUsize,
    pub label: String,
    block_size: usize,
    blocks: usize,
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
stdout.write    -> 写到当前模拟环境的 console sink，或者第一版只记录/丢弃并返回 buf.len()
stderr.read     -> Err("ebadf")
stderr.write    -> 同 stdout
poll_status     -> stdin 按无输入处理；stdout/stderr 永远 writable
```

如果当前用户态模拟内核暂时没有 console sink，`stdout/stderr.write` 可以先返回 `Ok(buf.len())`，但要在注释里说明这是占位行为。这样 syscall 和 fd 层可以先跑通，后面接真实串口、SBI console 或宿主输出时不用再动 `FHandle`。

`FLike` 需要新增：

```rust
pub enum FLike {
    File(FHandle),
    Tty(TtyHandle),
    Pipe(PipeNode),
    Ep(EpInst),
}
```

`FLike::{read,write,poll,io_ctl,dup}` 里新增 `Tty` 分支。重点是：`TtyHandle` 和 `FHandle` 同属于 fd table 里的 file-like object，但它们不是同一种底层对象。

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
fallocate    -> self.fs.set_len(self.meta_index, offset + len)   // v1 暂时会改变可见 size
```

`FHandle::new` 应该变成创建 SwapFS 句柄的构造函数，例如：

```rust
pub fn new(path: &str, fs: Arc<SwapFs>, meta_index: usize, opt: FdOpt, cloexec: bool) -> Self;
```

`FHandle::with_data` 和 `data: Arc<Mutex<Vec<u8>>>` 已删除。`inode_ref()` 暂时返回 `(Arc<SwapFs>, meta_index)`，只是兼容旧名字；后续有 VFS/inode 时应改成真正 inode 引用。

`FHandle` 第一阶段先只完成普通文件读写必须依赖的基础方法：

| 方法 | v1 要求 |
| --- | --- |
| `new` | 保存 `path/fs/meta_index/desc/cloexec` |
| `dup` | 共享同一个 `desc`，所以共享 offset；但允许新的 `cloexec` |
| `read` | 检查读权限，按 `desc.off` 调 `read_at`，成功后推进 offset |
| `read_at` | 调 `SwapFs::read_at(meta_index, off, buf)` |
| `write` | 检查写权限；append 模式用文件当前 size 作为 offset；成功后推进 offset |
| `write_at` | 调 `SwapFs::write_at(meta_index, off, buf)` |
| `seek` | 只修改 `desc.off`；`End` 需要读取 SwapFS metadata size |
| `metadata_sz` | 返回 SwapFS metadata size |
| `set_len` | 调 `SwapFs::set_len`，用于 open truncate 和后续 truncate syscall |

其他方法第一版可以先 stub，但必须明确语义，不要继续访问旧的 `data` 字段：

| 方法 | v1 处理 |
| --- | --- |
| `sync_all/sync_data` | 可以先调用 `SwapFs::sync_meta` 和 `Disk::flush`，或者保守返回 `Ok(())` 并注释为 stub |
| `lookup/read_entry` | SwapFS v1 没有目录，先返回 `Err("enosys")` 或保留只用于旧测试的 stub |
| `poll_status` | 普通文件按 open flags 认为 ready；metadata 读失败时报告 error |
| `io_ctl` | 普通文件返回 `Err("enotty")` 更合理；tty/device 后续自己实现 ioctl |
| `mmap` | 返回 `Err("enosys")`，等 VM/page cache 后再做 |
| `inode_ref` | 暂时返回 `(Arc<SwapFs>, meta_index)`；后续 VFS 化时改名或删除 |
| `advise_readahead` | 返回 `Ok(())` 或 `Err("enosys")`，不做真实预读 |
| `fallocate` | v1 暂时通过 `SwapFs::set_len` 实现，会改变可见 size |
| `splice_to` | 可以用 `read` + `dst.write` 实现；不直接访问任何内部 backing |

迁移时需要逐个处理当前依赖 `data` 的方法：

| 方法 | 当前行为 | SwapFS 后行为 |
| --- | --- | --- |
| `dup` | clone `data` 和 `desc` | clone `fs`、`meta_index` 和 `desc` |
| `read` | 用 `desc.off` 调 `read_at` | 保持 offset 逻辑，底层 `read_at` 走 SwapFS |
| `write` | append 时用 `data.len()` | append 时用 `metadata_sz()` 或 `SwapFs::metadata_len` |
| `seek(End)` | 用 `data.len()` | 用 SwapFS metadata size |
| `set_len` | resize `Vec<u8>` | 调 `SwapFs::set_len`，必要时扩容分配块 |
| `metadata_sz` | 返回 `Vec<u8>.len()` | 返回 metadata 里的 `size` |
| `poll_status` | 检查 `data` 推断 error | 按 open flags 返回 ready，metadata 读失败时报 error |
| `sync_all/sync_data` | 空实现 | 至少调用 `SwapFs::sync_meta` 和 `Disk::flush` |
| `fallocate` | resize `Vec<u8>` | 当前调 `SwapFs::set_len`，后续再拆成只保留 capacity 的语义 |
| `splice_to` | 直接从 `data` 复制 | 通过 `read` + `dst.write` 做，不直接看内部字段 |

`lookup`、`read_entry` 暂时不应该继续挂在普通文件 `FHandle` 上实现目录语义。SwapFS v1 没有目录，可以先保留为 stub；后续有目录时应该挪到目录文件或 VFS/inode 层。

`/dev/tty` 这类对象不是 SwapFS 普通文件。当前代码暂时把它建成 `FLike::File(FHandle)`，接入 SwapFS 时需要改掉。第一版可以选择：

```text
方案 A: 新增 FLike::Tty，占位实现 stdin/stdout/stderr。
方案 B: 暂时不在 new_user_task() 中自动创建 /dev/tty fd，相关测试需要避开它。
```

推荐方案 A，因为它能保持 fd0/fd1/fd2 的概念，同时避免把设备文件塞进 SwapFS 普通文件。

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

当前 `SYS_OPEN/SYS_READ/SYS_WRITE` 主要是 mock。

当前 `SYS_OPEN` 已经会创建 SwapFS-backed `FHandle`，但还没有从用户地址空间读取真实路径字符串；它临时把 `path_addr` 编成 `anon_<addr>` 作为 flat 文件名。下一步应该先做内核内部 fd API，再把真实 syscall 的用户内存路径读取接进去。

```text
SYS_OPEN:
  1. 校验 path_addr。
  2. 暂时用 anon_<path_addr> 作为路径名。
  3. 不经过 `MountTable`，直接把 path normalize 成 SwapFS v1 支持的根目录文件名。
  4. 调 Kernel.swapfs.open_or_create(name, true, initial_blocks) 得到 meta_index。
  5. 创建 FHandle::new(path, fs, meta_index, opt, cloexec)。
  6. 插入当前 Task.files，返回 fd。

SYS_READ:
  1. 找当前 task。
  2. 通过 fd 找 FLike。
  3. 临时测试路径可先不做真实 user buffer copy，而是增加内核内部 read API。
  4. 最终应该调用 FLike::read。

SYS_WRITE:
  1. 找当前 task。
  2. 通过 fd 找 FLike。
  3. 最终调用 FLike::write。

SYS_CLOSE:
  1. 从 Task.files 移除 fd。
  2. 不需要删除文件数据。
```

由于当前内核还没有真实用户地址空间拷贝，建议第一阶段先实现内核内部 API：

```rust
Kernel::open_file_for_task(task_id, path, flags, mode) -> Result<usize, &'static str>
Kernel::read_fd(task_id, fd, buf: &mut [u8]) -> Result<usize, &'static str>
Kernel::write_fd(task_id, fd, buf: &[u8]) -> Result<usize, &'static str>
```

确认 SwapFS 行为后，再改 `dispatch_syscall`。

### `process/task.rs`

`Task.files: BTreeMap<usize, FLike>` 可以暂时保留。

需要重点确认：

- `Task::add_file` 仍然只负责分配 fd。
- `Task::dup_fd` 需要共享 `FHandle.desc`，这样 dup 后共享 offset。
- `cloexec` 长期应该移到 fd entry，但 v1 可以继续放在 `FHandle`。
- `new_user_task()` 当前用 `FHandle::new("/dev/tty", ...)` 创建标准输入输出。接入 SwapFS 后不能继续这样做，因为 `FHandle::new` 会需要真实 `SwapFs + meta_index`。这里应该迁移到 `FLike::Tty` 或暂时不初始化标准 fd。

## SwapFS 内部 API

推荐最小 API：

```rust
pub struct SwapFs {
    disk: Arc<Disk>,
    sb: RwLock<SwapFsSuperBlock>,
    alloc: Mutex<SwapFsAlloc>,
}

pub struct SwapFsAlloc {
    next_free_block: u64,
}
```

`SwapFsAlloc` 不是新的磁盘真相。它只是 `superblock.next_free_block` 的运行时副本，用来减少每次分配前都读 superblock 的麻烦。每次分配 data blocks 并推进 `next_free_block` 后，必须更新内存里的 `alloc.next_free_block`，并通过 `sync_super()` 把新的 `next_free_block` 写回 block 0。否则重新 mount 后会重复分配已经用过的 block。

SwapFS v1 不在 `mount()` 时把完整 metadata table 加载成 `Vec<SwapFsMeta>`。metadata 的真实位置是磁盘上的 metadata blocks；运行时按 `metadata index` 读写对应 record：

```text
meta_block  = meta_start_block + index / SWAPFS_META_PER_BLOCK
meta_offset = (index % SWAPFS_META_PER_BLOCK) * SWAPFS_META_DISK_SIZE
```

这样 `FHandle` 只需要保存 `meta_index`，后续要查 size/start_block 时通过 `SwapFs::read_meta(index)` 从磁盘读取。metadata cache 可以后续再加，不作为 v1 的必需结构。

方法：

```rust
impl SwapFs {
    pub fn format(disk: Arc<Disk>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
    pub fn mount(disk: Arc<Disk>) -> Result<Arc<Self>, &'static str>;
    pub fn mount_or_format(disk: Arc<Disk>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
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
    pub fn unlink(&self, name: &str) -> Result<(), &'static str>;
    pub fn sync_meta(&self, meta_index: usize) -> Result<(), &'static str>;
    pub fn sync_super(&self) -> Result<(), &'static str>;
}
```

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

这里的“挂载”第一版只表示：从一个 `Disk` 上读取 superblock，把它变成内存里的 `Arc<SwapFs>`。metadata table 不会整体读入内存，后续通过 `read_meta(index)` 按需读取。它不是完整的 Linux mount namespace，也没有多个挂载点、路径覆盖、bind mount 或 `/proc`、`/dev` 这类特殊文件系统。

当前代码已有 `MountTable` / `Kernel.mnt`，但 SwapFS v1 暂时不接它。`MountTable` 现在只是路径前缀到字符串 target 的重写表，例如把 `/mnt/a` 解析成 `dev0:/a`；它还没有把 `dev0` 映射到真实 `Disk`、`SwapFs` 或 VFS 对象。为了先跑通真实文件读写，v1 的 `open/read/write` 主线不要经过 `MountTable`。

v1 的结构是：

```text
Kernel
  disk: Arc<Disk>
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
  清空 metadata table。
  设置 next_free_block = data_start_block。
  返回新建好的 SwapFs。

SwapFs::mount(disk)
  非破坏性加载已有文件系统。
  读取 block 0。
  校验 magic/version/block_size/total_blocks。
  不加载完整 metadata table。
  恢复 alloc.next_free_block。
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
  使用 superblock.next_free_block / SwapFsAlloc.next_free_block 顺序分配。
  删除文件后暂时不回收 data blocks。
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
  7. alloc_blocks 内部已经 sync_super() 写回新的 next_free_block。
```

删除：

```text
unlink:
  metadata.used = false
  write_meta(index, unused_meta)
  不回收 data blocks
  next_free_block 不回退
```

这意味着 v1 中 metadata slot 可以复用，但 data blocks 暂时不会复用。后续加入 bitmap 后，再同时管理 metadata bitmap 和 data block bitmap。

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

SwapFS v1 暂时不做 bitmap，是为了先跑通：

```text
create -> write -> read -> remount -> read
```

后续可以升级为：

```text
block 0        superblock
block 1        metadata bitmap
block 2        data block bitmap
block 3..M     metadata table
block data...  file data
```

如果仍保持 `start_block + block_count` 的连续文件模型，bitmap 分配时需要找连续空闲区间；如果允许非连续 blocks，则 metadata 需要升级为 extent list 或 block list。

扩容：

当前实现采用简单搬家：

```text
write 超过当前 block_count:
  1. 计算 required_blocks = ceil(desired_size / block_size)。
  2. 计算 grown_blocks = max(required_blocks, max(1, old_block_count * 2))。
  3. 优先从 next_free_block 分配 grown_blocks，类似 Vec 扩容，减少频繁搬家。
  4. 如果 grown_blocks 因空间不足失败，并且 grown_blocks > required_blocks，则 fallback 到 required_blocks。
  5. 把旧文件内容复制到新位置。
  6. 更新 metadata.start_block/block_count/size。
  7. 旧空间不回收。
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
chaos-tests/tests/fs_refactor/swapfs.rs
chaos-tests/tests/fs_refactor/fd.rs
```

测试统一放在 `chaos-tests/tests/fs_refactor/` 下。当前已经覆盖 Disk、Tty、SwapFS layout、format/mount、metadata record 读写，以及 SwapFS metadata create/open；后续 FHandle/fd、syscall 接入测试继续在同一个 `fs-refactor` 目标下新增模块。

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
   - metadata slot 可以复用，但 data blocks 不回收，`next_free_block` 继续向前。
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
   - 空间不足时返回 `enospc`，metadata size 和 `next_free_block` 不应被错误推进。

7. unlink：
   - unlink 后 open 返回 `enoent`。
   - 空间不回收，`next_free_block` 不减少。

8. syscall/Kernel API：
   - `Kernel::open_file_for_task` 返回 fd。
   - `Kernel::write_fd/read_fd` 通过 fd 读写 SwapFS 文件。
   - 后续再把 `SYS_OPEN/READ/WRITE` 接到这条路径。

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

建议按这个顺序落地：

1. 重写 `Disk::new/read_block/write_block`，让 Disk 直接作为大数组块设备读写，并更新旧的磁盘测试。
2. 增加 `fs/tty.rs` 和 `FLike::Tty`，把 `new_user_task()` 的 fd0/fd1/fd2 从 `FHandle("/dev/tty")` 迁移到 `TtyHandle`。
3. 增加 `fs/swapfs/layout.rs`，实现 superblock/meta record 的 encode/decode。
4. 增加 `SwapFs::format/mount/mount_or_format`，能读写 superblock 和 metadata table。
5. 实现 `SwapFs::create/open/open_or_create/metadata_len`，先跑通“文件名 -> metadata index -> data block 分配”。
6. 实现 `SwapFs::read_at/write_at/set_len`，让 metadata 指向的连续 data blocks 能读写文件内容。
7. 改 `FHandle`，删除 `data: Arc<Mutex<Vec<u8>>>`，改成 `fs: Arc<SwapFs>` 和 `meta_index: usize`。
8. 让 `FHandle::read_at/write_at/metadata_sz/set_len/fallocate` 直接调用 SwapFS。
9. 改 `FLike::File`，让它只调用 `FHandle` 方法，不直接访问 `FHandle` 内部字段。
10. 增加 `Kernel` 内部 fd API：`open_file_for_task/read_fd/write_fd`。
11. 最后再改 `dispatch_syscall` 的 `SYS_OPEN/SYS_READ/SYS_WRITE`。

## 暂时不做

- 嵌套目录。
- 真正 inode 编号和 inode cache。
- dentry cache。
- 权限、owner、mode、mtime/ctime/atime。
- hard link/symlink。
- block free list。
- journaling。
- page cache 和 mmap 文件页。
- 异步 I/O 调度。

这些后续可以在 SwapFS v2 或 VFS 重构时补。
