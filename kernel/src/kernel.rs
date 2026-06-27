#![allow(unused, dead_code, non_upper_case_globals, non_camel_case_types, unused_assignments, unused_mut)]

// Compatibility entry for the old assignment path.
//
// The Chaos user-space kernel simulation now lives in `kernel/chaos`.
// These path modules keep direct users of `kernel/src/kernel.rs` on the
// same real module tree as the standalone crate.
#[path = "../chaos/src/prelude.rs"]
mod prelude;
#[path = "../chaos/src/consts.rs"]
pub mod consts;
#[path = "../chaos/src/memory/mod.rs"]
pub mod memory;
#[path = "../chaos/src/sync/mod.rs"]
pub mod sync;
#[path = "../chaos/src/ipc/mod.rs"]
pub mod ipc;
#[path = "../chaos/src/fs/mod.rs"]
pub mod fs;
#[path = "../chaos/src/net.rs"]
pub mod net;
#[path = "../chaos/src/elf.rs"]
pub mod elf;
#[path = "../chaos/src/capability.rs"]
pub mod capability;
#[path = "../chaos/src/signal.rs"]
pub mod signal;
#[path = "../chaos/src/time.rs"]
pub mod time;
#[path = "../chaos/src/trap.rs"]
pub mod trap;
#[path = "../chaos/src/sched.rs"]
pub mod sched;
#[path = "../chaos/src/process/mod.rs"]
pub mod process;
#[path = "../chaos/src/kernel_api/mod.rs"]
pub mod kernel_api;
#[path = "../chaos/src/util.rs"]
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
