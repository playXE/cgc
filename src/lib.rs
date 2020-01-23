#![feature(specialization)]
#[macro_use]
extern crate log;

pub mod bump;
pub mod collector;
pub mod freelist;
pub mod mem;
pub mod rooting;
pub mod trace;

lazy_static::lazy_static! {
    static ref COLLECTOR: parking_lot::RwLock<mem::Ptr<collector::GlobalCollector>> = parking_lot::RwLock::new(mem::Ptr::new(collector::GlobalCollector::new(1024 * 1024 * 100)));
}

pub use collector::GlobalCollector;
pub use rooting::{Heap, Rooted};
pub use trace::*;

pub fn write_barrier(parent: &dyn HeapTrait,child: &dyn HeapTrait) -> bool {
	let should_emit_barrier = parent.color() == collector::GcColor::Black && child.color() ==  collector::GcColor::White;
	if !should_emit_barrier {
		return false;
	}
	let gc = COLLECTOR.read();
	parent.set_color(collector::GcColor::Grey);
	gc.grey_list.lock().push_back(collector::GcHandle(parent.inner()));
	return true;
}

pub fn gc_collect() {
    COLLECTOR.write().get().major();
}

pub fn gc_alloc<T: Trace + Sized + 'static>(value: T) -> Rooted<T> {
    COLLECTOR.write().get().alloc(value)
}

pub fn gc_get_fragmentation() -> f32 {
    COLLECTOR.read().get().fragmentation()
}
