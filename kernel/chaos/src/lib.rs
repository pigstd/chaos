#![allow(unused, dead_code, non_upper_case_globals, non_camel_case_types, unused_assignments, unused_mut)]

//! User-space Chaos kernel simulation.
//!
//! The files below are included at crate root on purpose. The original implementation
//! was a single file with many private cross-subsystem field accesses; keeping one
//! module namespace preserves behavior while making the source physically navigable.

include!("prelude.rs");
include!("consts.rs");
include!("memory.rs");
include!("sync.rs");
include!("ipc.rs");
include!("fs.rs");
include!("net.rs");
include!("elf.rs");
include!("capability.rs");
include!("signal.rs");
include!("time.rs");
include!("trap.rs");
include!("sched.rs");
include!("process.rs");
include!("kernel_api.rs");
include!("util.rs");
