#![allow(unused, dead_code, non_upper_case_globals, non_camel_case_types, unused_assignments, unused_mut)]

//! User-space Chaos kernel simulation.
//!
//! The implementation is organized as real Rust modules under subsystem directories.
//! Public root re-exports preserve the old `use chaos_tests::*` API.

mod prelude;
pub mod consts;
pub mod memory;
pub mod sync;
pub mod ipc;
pub mod fs;
pub mod net;
pub mod elf;
pub mod capability;
pub mod signal;
pub mod time;
pub mod trap;
pub mod sched;
pub mod process;
pub mod kernel_api;
pub mod util;

pub use capability::*;
pub use consts::*;
pub use elf::*;
pub use fs::*;
pub use ipc::*;
pub use kernel_api::*;
pub use memory::*;
pub use net::*;
pub use process::*;
pub use sched::*;
pub use signal::*;
pub use sync::*;
pub use time::*;
pub use trap::*;
pub use util::*;
