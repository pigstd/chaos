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
    pub name: [u8; SWAPFS_NAME_LEN],
    pub start_block: u64,
    pub block_count: u64,
    pub size: u64,
}
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
fs/swapfs/file.rs
```

职责：

| 模块 | 职责 |
| --- | --- |
| `layout.rs` | 定义磁盘格式常量、superblock、metadata record，以及 encode/decode 辅助函数。 |
| `fs.rs` | 定义 `SwapFs`，负责 format/mount、metadata table 加载、文件查找、创建、扩容、删除标记。 |
| `file.rs` | 定义 `SwapFile` 或 `SwapOpenFile`，负责按 metadata index 执行 read/write/seek/set_len。 |
| `mod.rs` | 对外 re-export `SwapFs`、`SwapFile`、layout 常量和错误类型。 |

`fs/mod.rs` 需要新增：

```rust
pub mod swapfs;
pub use swapfs::*;
```

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

旧的 `read_block()` 填充 `0xAA`、`write_block()` 不保存数据、`Disk::failing()` 错误注入逻辑都可以删除或改测试。新的正确语义是：写入某个 block 后，再读同一个 block 必须读回同样的数据。

### `fs/fd.rs`

当前 `FHandle` 保存：

```rust
path: String
data: Arc<Mutex<Vec<u8>>>
desc: Arc<RwLock<FdState>>
cloexec: bool
```

SwapFS 接入后，`FHandle` 不应该再只表示内存文件。建议第一版改成枚举 backing：

```rust
pub enum FileBacking {
    Memory(Arc<Mutex<Vec<u8>>>),
    Swap(Arc<SwapFs>, usize), // usize 是 metadata index
}

pub struct FHandle {
    pub path: String,
    pub backing: FileBacking,
    pub(crate) desc: Arc<RwLock<FdState>>,
    pub cloexec: bool,
}
```

这样可以保留旧内存文件行为，同时逐步把 `SYS_OPEN` 接到 SwapFS。

`FHandle::read_at/write_at/metadata_sz/set_len/fallocate` 需要改为按 `backing` 分发：

```text
Memory -> 当前 Vec<u8> 逻辑
Swap   -> SwapFs::read_at/write_at/metadata_len/set_len
```

`inode_ref()` 当前返回 `Arc<Mutex<Vec<u8>>>`，接入 SwapFS 后语义不再成立。第一版可以先保留但只支持 `Memory`，遇到 `Swap` 返回一个错误更合理；如果不想改签名，先不要在 SwapFS 路径调用它。

### `fs/filelike.rs`

`FLike::File` 的分发思路可以保留：

```rust
FLike::File(f) => f.read(...)
FLike::File(f) => f.write(...)
```

但当前实现直接访问 `f.data` 和 `f.desc`。接入 SwapFS 后，应改为调用 `FHandle` 方法，而不是在 `FLike` 里展开普通文件细节：

```rust
FLike::File(f) => f.read(buf)
FLike::File(f) => f.write(buf)
```

这样 `FLike` 不需要知道文件来自 Memory 还是 SwapFS。

### `kernel_api/lifecycle.rs`

`Kernel` 需要持有一个默认 SwapFS：

```rust
pub swapfs: Arc<SwapFs>,
```

初始化建议：

```text
Kernel::new(nf)
  -> Disk::new("swapfs0", blocks, 512)
  -> SwapFs::mount_or_format(disk)
```

为了不一次性破坏现有测试，也可以先新增：

```rust
pub fn with_swapfs(nf: usize, blocks: usize) -> Self
```

然后让新测试使用 `Kernel::with_swapfs(...)`。

### `kernel_api/syscall.rs`

当前 `SYS_OPEN/SYS_READ/SYS_WRITE` 主要是 mock。

第一版需要把文件路径接到 fd table：

```text
SYS_OPEN:
  1. 校验 path_addr。
  2. 第一版如果还没有用户内存字符串读取，可以临时用测试辅助路径，或新增内核内部 open API。
  3. 调 SwapFs::open_or_create(name, flags) 得到 meta_index。
  4. 创建 FHandle::swap(path, fs, meta_index, opt, cloexec)。
  5. 插入当前 Task.files，返回 fd。

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

## SwapFS 内部 API

推荐最小 API：

```rust
pub struct SwapFs {
    disk: Arc<Disk>,
    sb: RwLock<SwapFsSuperBlock>,
    metas: RwLock<Vec<SwapFsMeta>>,
    alloc: Mutex<SwapFsAlloc>,
}

pub struct SwapFsMeta {
    used: bool,
    name: String,
    start_block: u64,
    block_count: u64,
    size: u64,
}

pub struct SwapFsAlloc {
    next_free_block: u64,
}
```

方法：

```rust
impl SwapFs {
    pub fn format(disk: Arc<Disk>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
    pub fn mount_or_format(disk: Arc<Disk>, total_blocks: u64, max_files: usize) -> Result<Arc<Self>, &'static str>;
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

## 分配与删除策略

第一版采用顺序分配：

```text
create:
  start_block = next_free_block
  block_count = initial_blocks
  next_free_block += block_count
```

删除：

```text
unlink:
  metadata.used = false
  不回收 data blocks
  next_free_block 不回退
```

扩容：

第一版推荐先实现简单搬家：

```text
write 超过当前 block_count:
  1. 计算需要的新 block_count。
  2. 从 next_free_block 分配一段新的连续空间。
  3. 把旧文件内容复制到新位置。
  4. 更新 metadata.start_block/block_count/size。
  5. 旧空间不回收。
```

如果暂时不想实现搬家，也可以 v1a 直接返回 `enospc`，但文档和测试要明确。

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

新增测试建议放到：

```text
chaos-tests/tests/basic/group_swapfs.rs
```

最小测试：

1. format/mount：
   - `Disk::new(label, blocks, block_size)`
   - `SwapFs::format(...)`
   - superblock magic/version 正确。

2. create/open：
   - 创建 `/a`。
   - 再 open `/a` 能拿到同一个 metadata index。
   - 重复 create with exclusive 返回 `eexist`。

3. read/write：
   - 写入 `hello`。
   - seek/read 能读回 `hello`。
   - read 不能超过 `size`。

4. fd offset：
   - 同一个 fd 连续读会推进 offset。
   - 两次独立 open offset 不共享。
   - dup 后 offset 共享。

5. metadata persistence：
   - 写文件后重新 mount 同一个 memory disk。
   - 文件 metadata 和内容仍可读取。

6. expand：
   - 写入超过初始 block capacity。
   - 如果实现搬家，确认内容完整且 metadata 更新。
   - 如果 v1a 不支持扩容，确认返回 `enospc`。

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

## 实现顺序

建议按这个顺序落地：

1. 重写 `Disk::new/read_block/write_block`，让 Disk 直接作为大数组块设备读写，并更新旧的磁盘测试。
2. 增加 `fs/swapfs/layout.rs`，实现 superblock/meta record 的 encode/decode。
3. 增加 `SwapFs::format/mount_or_format`，能读写 superblock 和 metadata table。
4. 实现 `SwapFs::create/open/read_at/write_at/metadata_len`。
5. 改 `FHandle`，增加 `FileBacking::Memory | Swap`，并让 `read_at/write_at/metadata_sz/set_len` 分发。
6. 改 `FLike::File`，让它调用 `FHandle::read/write`，不要直接访问 `FHandle` 内部字段。
7. 增加 `Kernel` 内部 fd API：`open_file_for_task/read_fd/write_fd`。
8. 最后再改 `dispatch_syscall` 的 `SYS_OPEN/SYS_READ/SYS_WRITE`。

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
