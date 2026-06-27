# Chaos Kernel Simulation

`kernel/chaos` is the standalone user-space crate for the Chaos kernel simulation.
It was split out from the old monolithic `kernel/src/kernel.rs`.

The original rCore-style tree under `kernel/src/` is legacy code in this repository.
It is not the code path used by `chaos-tests`, and it currently does not build as a
complete kernel tree because some original rCore dependencies and modules are absent.

## Build And Test Path

`chaos-tests` now depends on this crate normally:

```toml
chaos-kernel = { path = "../kernel/chaos" }
```

`chaos-tests/src/lib.rs` re-exports this crate, so existing tests can keep using:

```rust
use chaos_tests::*;
```

`kernel/src/kernel.rs` is only a compatibility entry that points at the same module
files with `#[path]` declarations.

## Source Layout

`src/lib.rs` now declares real Rust modules and re-exports their public APIs at crate
root. This preserves the old `use chaos_tests::*` test surface while allowing each
subsystem to be maintained through folders and `mod.rs` files. Cross-subsystem internals
that were formerly private in the monolithic file are exposed only as `pub(crate)`.

| File | Contents |
| --- | --- |
| `src/lib.rs` | Crate root, module declarations, and public re-exports for compatibility. |
| `src/prelude.rs` | Shared `std` imports used by the simulation. This also makes the user-space dependency explicit. |
| `src/consts.rs` | Kernel-wide constants: page size, address layout, syscall numbers, flags, signal numbers, scheduler constants, socket constants, and I/O queue limits. |
| `src/memory/` | Virtual memory maps, address helpers, frame pools, copy-on-write pages, kernel stack simulation, slab entries, address spaces, and buddy allocation. |
| `src/sync/` | Global kernel lock, spin lock, event bus, condition-style wait queues, semaphores, futexes, and generic wait queues. |
| `src/ipc/` | Circular buffers, pipe endpoints, byte channels, System V semaphore arrays/contexts, and shared-memory context/tag helpers. |
| `src/fs/` | File descriptors, file handles, file-like objects, epoll, terminal structs, page cache, kernel object registry, block cache, mounts, I/O queue, and disk simulation. |
| `src/net.rs` | Socket state enum and IPv4/TCP checksum/header helpers. |
| `src/elf.rs` | ELF header validation used by simulated exec/task creation paths. |
| `src/capability.rs` | Capability set representation and capability inheritance/check helpers. |
| `src/signal.rs` | Signal action and signal set state, pending/block masks, and delivery helper logic. |
| `src/time.rs` | Timer entries, timer wheel, global simulated clocks, tick helpers, uptime helper, and serial newline normalization. |
| `src/trap.rs` | Saved CPU context representation and trap/interrupt controller simulation. |
| `src/sched.rs` | Load balancing helper, schedule policy, run queue, vruntime updates, and preemption counters. |
| `src/process/` | Process initialization stack layout, `Task`, `TaskTable`, process group logic, and resource limits. |
| `src/kernel_api/` | Top-level `Kernel` facade, CPU/current-task state, syscall dispatcher, memory/cache/page helpers, fork/exec/pipe/wait operations, and high-level workload helpers. |
| `src/util.rs` | Generic utility algorithms: pattern scanning, CRC32, varint encode/decode, bit manipulation, alignment, hashing, and power/log helpers. |

## Large Subsystem Folders

- `memory/`: `vm`, `frame`, `address`, `slab`, `addr_space`, and `buddy`.
- `sync/`: `lock`, `event`, `queue`, `sema`, `futex`, and `wait`.
- `ipc/`: `ring`, `pipe`, `channel`, and `sysv`.
- `fs/`: `fd`, `filelike`, `epoll`, `terminal`, `page_cache`, `kobj`, `block_cache`, `mount`, `io`, and `disk`.
- `process/`: `init`, `pid`, `task`, `table`, `group`, and `limits`.
- `kernel_api/`: `lifecycle`, `syscall`, and `ops`.

## Relationship To A Future Kernel-Mode Port

This crate is still a user-space simulation. It uses `std`, host threads, host mutexes,
and host timing APIs. The split makes the next step clearer, but it does not make the
code `no_std` yet.

A future kernel-mode port should first introduce runtime traits for the pieces that are
currently host-backed: locking, sleeping/wakeup, time, allocation, block devices, and
task execution. The current crate can then keep a `std` backend for demos while a real
kernel backend replaces those host services.
