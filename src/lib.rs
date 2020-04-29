pub mod api;
pub mod heap;
pub mod mem;
pub mod safepoint;
pub mod space;
pub mod threads;

/// Write barrier *must* be executed before store to some heap object happens.
///
///
/// ## Where and when to use?
/// You should place write barrier before store and write barrier is needed only when you store other GC value into GC value.
pub fn write_barrier<T: api::HeapTrait + ?Sized>(src: &T) {
    unsafe {
        heap::write_barrier_impl(src.inner());
    }
}
