// AGENT

use crate::consts::*;
use crate::prelude::*;
use crate::*;
use std::io::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl TtyHandle {
    pub fn new(kind: TtyKind, cloexec: bool) -> Self {
        let opt = match kind {
            TtyKind::Stdin => FdOpt {
                rd: true,
                wr: false,
                ap: false,
                nb: false,
            },
            TtyKind::Stdout | TtyKind::Stderr => FdOpt {
                rd: false,
                wr: true,
                ap: false,
                nb: false,
            },
        };
        Self {
            kind,
            desc: FdState::create(opt),
            cloexec,
        }
    }

    pub fn stdin() -> Self {
        Self::new(TtyKind::Stdin, false)
    }
    pub fn stdout() -> Self {
        Self::new(TtyKind::Stdout, false)
    }
    pub fn stderr() -> Self {
        Self::new(TtyKind::Stderr, false)
    }

    pub fn dup(&self, cloexec: bool) -> Self {
        Self {
            kind: self.kind,
            desc: self.desc.clone(),
            cloexec,
        }
    }

    pub fn read(&self, _buf: &mut [u8]) -> Result<usize, &'static str> {
        match self.kind {
            TtyKind::Stdin => Ok(0),
            TtyKind::Stdout | TtyKind::Stderr => Err("ebadf"),
        }
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize, &'static str> {
        match self.kind {
            TtyKind::Stdin => Err("ebadf"),
            TtyKind::Stdout | TtyKind::Stderr => {
                // AGENT: User-space simulation console sink.
                // AGENT: A real kernel port should route this to a console/serial driver.
                let result = match self.kind {
                    TtyKind::Stdout => std::io::stdout()
                        .write_all(buf)
                        .and_then(|_| std::io::stdout().flush()),
                    TtyKind::Stderr => std::io::stderr()
                        .write_all(buf)
                        .and_then(|_| std::io::stderr().flush()),
                    TtyKind::Stdin => unreachable!(),
                };
                result.map(|_| buf.len()).map_err(|_| "eio")
            }
        }
    }

    pub fn io_ctl(&self, cmd: usize, _arg: usize) -> Result<usize, &'static str> {
        match cmd {
            TCGETS | TCSETS | TIOCGPGRP | TIOCSPGRP | TIOCGWINSZ | FIONCLEX | FIOCLEX => Ok(0),
            FIONBIO => Ok(0),
            _ => Err("enotty"),
        }
    }

    pub fn poll_status(&self) -> (bool, bool, bool) {
        match self.kind {
            TtyKind::Stdin => (false, false, false),
            TtyKind::Stdout | TtyKind::Stderr => (false, true, false),
        }
    }
}

impl fmt::Debug for TtyHandle {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("TTY")
            .field("kind", &self.kind)
            .field("cloexec", &self.cloexec)
            .finish()
    }
}
