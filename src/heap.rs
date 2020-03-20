use crate::barriers::Barrier;
use crate::block::*;
use crate::mutator::*;
use parking_lot::{Condvar, Mutex};
pub struct Heap {
    pub blocks: Mutex<Vec<Block>>,
    pub mutators: Mutators,
}

lazy_static::lazy_static! {
    pub static ref HEAP: Heap = Heap {
        blocks: Mutex::new(vec![]),
        mutators: Mutators::new()
    };
}

impl Heap {}
