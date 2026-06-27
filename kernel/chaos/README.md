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

`kernel/src/kernel.rs` is only a compatibility entry that includes this crate root.

## Source Layout

All source files are included at crate root from `src/lib.rs`. This is intentional for
the first refactor step: the old single file had many private cross-subsystem field
accesses. Keeping one module namespace preserves behavior while making the source
physically navigable. A later refactor can turn these files into real Rust modules by
adding `pub(crate)` boundaries and explicit imports.

| File | Contents |
| --- | --- |
| `src/lib.rs` | Crate root, shared lint settings, and ordered `include!` list. |
| `src/prelude.rs` | Shared `std` imports used by the simulation. This also makes the user-space dependency explicit. |
| `src/consts.rs` | Kernel-wide constants: page size, address layout, syscall numbers, flags, signal numbers, scheduler constants, socket constants, and I/O queue limits. |
| `src/memory.rs` | Virtual memory regions/maps, address translation helpers, access checks, frame refcounts, frame pools, copy-on-write helpers, kernel stack simulation, slab allocator entries, address spaces, and buddy allocator. |
| `src/sync.rs` | Global kernel lock, spin lock, event flags/bus, wait queues, condition-style sync queue, semaphore primitive, and futex tables. |
| `src/ipc.rs` | Circular byte buffer, pipe endpoints, byte channel, System V style semaphore arrays/contexts, and shared-memory context/tag helpers. |
| `src/fs.rs` | File descriptor state, file handles, file-like enum, pseudo nodes, epoll structures, terminal structs, page cache, kernel object registry, block cache, mount table, I/O queue, and disk simulation. |
| `src/net.rs` | Socket state enum and IPv4/TCP checksum/header helpers. |
| `src/elf.rs` | ELF header validation used by simulated exec/task creation paths. |
| `src/capability.rs` | Capability set representation and capability inheritance/check helpers. |
| `src/signal.rs` | Signal action and signal set state, pending/block masks, and delivery helper logic. |
| `src/time.rs` | Timer entries, timer wheel, global simulated clocks, tick helpers, uptime helper, and serial newline normalization. |
| `src/trap.rs` | Saved CPU context representation and trap/interrupt controller simulation. |
| `src/sched.rs` | Load balancing helper, schedule policy, run queue, vruntime updates, and preemption counters. |
| `src/process.rs` | Process initialization stack layout, `Task`, `TaskTable`, process group logic, and resource limits. |
| `src/kernel_api.rs` | Top-level `Kernel` facade, CPU/current-task state, syscall dispatcher, memory/cache/page helpers, fork/exec/pipe/wait operations, and high-level workload helpers. |
| `src/util.rs` | Generic utility algorithms: pattern scanning, CRC32, varint encode/decode, bit manipulation, alignment, hashing, and power/log helpers. |

## Relationship To A Future Kernel-Mode Port

This crate is still a user-space simulation. It uses `std`, host threads, host mutexes,
and host timing APIs. The split makes the next step clearer, but it does not make the
code `no_std` yet.

A future kernel-mode port should first introduce runtime traits for the pieces that are
currently host-backed: locking, sleeping/wakeup, time, allocation, block devices, and
task execution. The current crate can then keep a `std` backend for demos while a real
kernel backend replaces those host services.
