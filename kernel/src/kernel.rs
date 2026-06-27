#![allow(unused, dead_code, non_upper_case_globals, non_camel_case_types, unused_assignments, unused_mut)]

// Compatibility entry for the old assignment path.
//
// The Chaos user-space kernel simulation now lives in `kernel/chaos`.
// Keep this file thin and include the same split source files directly so
// older tooling that opens `kernel/src/kernel.rs` still sees the full API.
include!("../../kernel/chaos/src/prelude.rs");
include!("../../kernel/chaos/src/consts.rs");
include!("../../kernel/chaos/src/memory.rs");
include!("../../kernel/chaos/src/sync.rs");
include!("../../kernel/chaos/src/ipc.rs");
include!("../../kernel/chaos/src/fs.rs");
include!("../../kernel/chaos/src/net.rs");
include!("../../kernel/chaos/src/elf.rs");
include!("../../kernel/chaos/src/capability.rs");
include!("../../kernel/chaos/src/signal.rs");
include!("../../kernel/chaos/src/time.rs");
include!("../../kernel/chaos/src/trap.rs");
include!("../../kernel/chaos/src/sched.rs");
include!("../../kernel/chaos/src/process.rs");
include!("../../kernel/chaos/src/kernel_api.rs");
include!("../../kernel/chaos/src/util.rs");
