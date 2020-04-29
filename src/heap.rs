use crate::api::*;
use crate::mem::*;
use crate::space::*;
use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
pub const GC_WHITE: u8 = 0;
pub const GC_GREY: u8 = 1;
pub const GC_BLACK: u8 = 2;

pub struct HeapInner<T: super::api::Trace + ?Sized> {
    /// Foward address, initially points to `self` for read barriers.
    pub(crate) forward: AtomicUsize,
    pub(crate) color: AtomicU8,
    pub(crate) value: T,
}
impl<T: super::api::Trace + ?Sized> HeapInner<T> {
    pub fn mark(&self, _x: bool) {}
    pub fn fwdptr(&self) -> Address {
        Address::from_ptr(self.forward.load(Ordering::Acquire) as *const u8)
    }
    pub fn set_fwdptr(&self, fwdptr: Address) {
        self.forward.store(fwdptr.to_usize(), Ordering::Release);
    }
    pub fn is_marked(&self) -> bool {
        false
    }
}

pub(crate) unsafe fn read_barrier_impl<T: Trace>(src: *mut *mut HeapInner<T>) -> *mut HeapInner<T> {
    let src = &**src;
    // src.forward points to fromspace or to tospace.
    src.forward.load(Ordering::Acquire) as *mut _
}

pub(crate) unsafe fn write_barrier_impl<T: Trace + 'static>(src: *mut *mut HeapInner<T>) {
    let cell = &mut **src;
    if HEAP.state.load(Ordering::Acquire) != GC_COPYING {
        return;
    }
    if cell.color.load(Ordering::Acquire) == GC_GREY {
        // Object is in worklist,return.
        return;
    }

    // Push object to worklist so GC will scan object for new objects written to our object.
    cell.color.store(GC_GREY, Ordering::Release);
    HEAP.worklist.push(GcValue {
        slot: Address::from_ptr(src),
        value: *src,
    });
}

pub struct GcValue {
    slot: Address,
    value: *mut HeapInner<dyn Trace>,
}

impl GcValue {
    fn value(&self) -> &mut HeapInner<dyn Trace> {
        unsafe { &mut *self.value }
    }
    fn relocate(&self, addr: Address) {
        if self.slot.is_non_null() {
            unsafe {
                let slot = self.slot.to_mut_ptr::<*mut *mut u8>();
                *slot = addr.to_mut_ptr();
            }
        }
        if !self.value().color.load(Ordering::Acquire) != GC_BLACK {
            self.value().set_fwdptr(addr);
            self.value().color.store(GC_BLACK, Ordering::Release);
        }
    }
}

unsafe impl Send for GcValue {}

pub const GC_NONE: u8 = 0;
pub const GC_COPYING: u8 = 2;
pub const GC_INIT: u8 = 1;

pub struct GlobalHeap {
    worklist: SegQueue<GcValue>,
    state: AtomicU8,
    fence_mutator: AtomicBool,
    weak_handles: parking_lot::Mutex<Vec<*mut HeapInner<dyn Trace>>>,
    from_space: parking_lot::Mutex<Space>,
    to_space: parking_lot::Mutex<Space>,
    pub(crate) threads: crate::threads::Threads,
}

unsafe impl Send for GlobalHeap {}
unsafe impl Sync for GlobalHeap {}

impl GlobalHeap {
    pub fn new() -> Self {
        Self {
            to_space: parking_lot::Mutex::new(Space::new(32 * 1024)),
            from_space: parking_lot::Mutex::new(Space::new(32 * 1024)),
            worklist: SegQueue::new(),
            state: AtomicU8::new(0),
            fence_mutator: AtomicBool::new(false),
            weak_handles: parking_lot::Mutex::new(vec![]),
            threads: crate::threads::Threads::new(),
        }
    }
    fn flip() {
        let mut x = HEAP.to_space.lock();
        x.reset_pages();
        let mut y = HEAP.from_space.lock();
        std::mem::swap(&mut *x, &mut *y);
    }
    fn visit(value: &mut HeapInner<dyn Trace>) {
        value.value.references().iter().for_each(|item| unsafe {
            HEAP.worklist.push(GcValue {
                slot: (&**item).slot(),
                value: (&**item).inner(),
            })
        });
    }
    fn collect_impl() {
        crate::safepoint::stop_the_world(|mutators| {
            HEAP.state.store(GC_COPYING, Ordering::Relaxed);
            for thread in mutators.iter() {
                for root in thread.rootset.borrow().iter() {
                    unsafe {
                        let value = GcValue {
                            slot: (&**root).slot(),
                            value: (&**root).inner(),
                        };
                        HEAP.worklist.push(value);
                    }
                }
            }
            // flip tospace and fromspace
            Self::flip();
        });
        // copy objects
        Self::process_grey();
        // disable write barriers
        HEAP.state.store(GC_NONE, Ordering::Release);
        // sweep objects that needs sweeping.
        HEAP.weak_handles
            .lock()
            .retain(|item| unsafe { (&**item).color.load(Ordering::Relaxed) == GC_WHITE })
    }

    fn process_grey() {
        while HEAP.worklist.is_empty() == false {
            let value: GcValue = loop {
                match HEAP.worklist.pop() {
                    Ok(val) => break val,
                    Err(x) => panic!("{}", x),
                }
            };

            if value.value().color.load(Ordering::Relaxed) == GC_WHITE {
                let hvalue = HEAP
                    .to_space
                    .lock()
                    .allocate(std::mem::size_of_val(value.value()), &mut false);
                value
                    .value()
                    .forward
                    .store(hvalue.to_usize(), Ordering::Release);
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        value.value() as *mut _ as *const u8,
                        hvalue.to_mut_ptr::<u8>(),
                        std::mem::size_of_val(value.value()),
                    );
                }
                Self::visit(value.value());
                value.relocate(hvalue);
            } else {
                value.relocate(value.value().fwdptr());
            }
        }
    }
}

pub struct Heap {}

lazy_static::lazy_static! {
    pub static ref HEAP: GlobalHeap = GlobalHeap::new();
}

thread_local! {}
