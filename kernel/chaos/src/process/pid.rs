use crate::prelude::*;
use crate::consts::*;
use crate::*;

pub type Tid = usize;
pub type Pgid = i32;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Pid(pub usize);
impl Pid {
    pub const INIT: usize = 1;
    pub fn new() -> Self { Pid(0) }
    pub fn get(&self) -> usize { self.0 }
    pub fn is_init(&self) -> bool { self.0 == Self::INIT }
}
impl fmt::Display for Pid {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result { write!(f, "{}", self.0) }
}
