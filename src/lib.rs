#![feature(specialization)]
#[macro_use]
extern crate log;

pub mod bump;
pub mod collector;
pub mod freelist;
pub mod mem;
pub mod parallel_mark;
pub(crate) mod pool;
pub mod rooting;
pub mod trace;

struct HandleCollector {
    x: collector::GlobalCollector,
}

unsafe impl Sync for HandleCollector {}
unsafe impl Send for HandleCollector {}

lazy_static::lazy_static! {
    static ref COLLECTOR: parking_lot::RwLock<mem::Ptr<HandleCollector>> = parking_lot::RwLock::new(mem::Ptr::new(HandleCollector {x: collector::GlobalCollector::new(1024 * 1024 * 100)}));
}

pub use collector::GlobalCollector;
pub use rooting::{Heap, Rooted};
pub use trace::*;

pub fn gc_collect() {
    COLLECTOR.write().get().x.collect();
}

pub fn gc_alloc<T: Trace + Sized + 'static>(value: T) -> Rooted<T> {
    COLLECTOR.write().get().x.alloc(value)
}

pub fn gc_get_fragmentation() -> f32 {
    COLLECTOR.read().get().x.fragmentation()
}
