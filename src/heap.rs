use crate::api::*;
use crate::mem::*;
use crate::space::*;
use crossbeam::queue::SegQueue;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering};
pub const GC_WHITE: u8 = 0;
pub const GC_GREY: u8 = 1;
pub const GC_BLACK: u8 = 2;

#[cfg(not(feature = "trace-gc"))]
const TRACE_GC: bool = false;

#[cfg(feature = "trace-gc")]
const TRACE_GC: bool = true;

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

pub(crate) unsafe fn read_barrier_impl<T: Trace>(src_: *mut HeapInner<T>) -> *mut HeapInner<T> {
    let src = &*src_;

    // src.forward points to fromspace or to tospace.
    let r = src.forward.load(Ordering::Acquire) as *mut _;
    log::trace!("Read barrier: From {:p} to {:p}", src_, r);
    r
}

pub(crate) unsafe fn write_barrier_impl(src: *mut HeapInner<dyn Trace>) {
    let cell = &mut *src;
    if HEAP.state.load(Ordering::Acquire) != GC_COPYING {
        return;
    }
    if cell.color.load(Ordering::Acquire) == GC_GREY {
        // Object is in worklist,return.
        return;
    }

    // Push object to worklist so GC will scan object for new objects written to our object.
    cell.color.store(GC_GREY, Ordering::Release);
    HEAP.worklist.push(GcValue { value: src });
}

pub struct GcValue {
    value: *mut HeapInner<dyn Trace>,
}

impl GcValue {
    fn value(&self) -> &mut HeapInner<dyn Trace> {
        unsafe { &mut *self.value }
    }
    fn relocate(&self, addr: Address) {
        /*if self.slot.is_non_null() {
           unsafe {
                let slot = self.slot.to_mut_ptr::<*mut *mut u8>();
                *slot = addr.to_mut_ptr();
            }
        }*/
        if !self.value().color.load(Ordering::Acquire) != HEAP.black.load(Ordering::Relaxed) {
            self.value().set_fwdptr(addr);
            self.value()
                .color
                .store(HEAP.black.load(Ordering::Relaxed), Ordering::Release);
        }
    }
}

unsafe impl Send for GcValue {}

pub const GC_NONE: u8 = 0;
pub const GC_COPYING: u8 = 2;
pub const GC_INIT: u8 = 1;
pub const GC_TERMINATE: u8 = 3;

pub struct GlobalHeap {
    worklist: SegQueue<GcValue>,
    state: AtomicU8,
    fence_mutator: AtomicBool,
    needs_gc: AtomicBool,
    weak_handles: parking_lot::Mutex<Vec<*mut HeapInner<dyn Trace>>>,
    from_space: parking_lot::Mutex<Space>,
    to_space: parking_lot::Mutex<Space>,
    white: AtomicU8,
    black: AtomicU8,
    pub(crate) threads: crate::threads::Threads,
}

unsafe impl Send for GlobalHeap {}
unsafe impl Sync for GlobalHeap {}

impl GlobalHeap {
    pub fn collect(&self) {
        //self.state.store(GC_INIT, Ordering::Release);

        crate::safepoint::stop_the_world(|mutators| {
            log::trace!("Start GC");

            for thread in mutators.iter() {
                thread.rootset.borrow_mut().retain(|root| unsafe {
                    if (&**root).is_rooted() {
                        let value = GcValue {
                            value: (&**root).inner(),
                        };
                        HEAP.worklist.push(value);
                        true
                    } else {
                        let _ = Box::from_raw(*root);
                        false
                    }
                });
            }
            HEAP.state.store(GC_COPYING, Ordering::Relaxed);
        });
        log::trace!("Resume threads");
    }
    pub fn new() -> Self {
        Self {
            white: AtomicU8::new(GC_WHITE),
            black: AtomicU8::new(GC_BLACK),
            needs_gc: AtomicBool::new(false),
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

        let mut y = HEAP.from_space.lock();
        y.reset_pages();
        std::mem::swap(&mut *x, &mut *y);
    }
    fn visit(value: &mut HeapInner<dyn Trace>) {
        value.value.references().iter().for_each(|item| unsafe {
            HEAP.worklist.push(GcValue {
                value: (&**item).inner(),
            })
        });
    }
    fn flip_colours() {
        let white = HEAP.white.load(Ordering::Relaxed);
        let black = HEAP.black.load(Ordering::Relaxed);
        HEAP.white.store(black, Ordering::Relaxed);
        HEAP.black.store(white, Ordering::Relaxed);
    }
    fn collect_impl() {
        // copy objects
        Self::process_grey();
        // disable write barriers
        HEAP.state.store(GC_NONE, Ordering::Release);
        // sweep objects that needs sweeping.
        let mut handles = HEAP.weak_handles.lock();
        for i in (0..handles.len()).rev() {
            if unsafe {
                (&*handles[i]).color.load(Ordering::Relaxed) == HEAP.white.load(Ordering::Relaxed)
            } {
                let item = handles.swap_remove(i);
                unsafe {
                    (&mut *item).value.finalize();
                    std::ptr::drop_in_place(item);
                }
            } else {
                unsafe {
                    (&*handles[i])
                        .color
                        .store(HEAP.white.load(Ordering::Relaxed), Ordering::Relaxed);
                }
            }
        }
        crate::safepoint::stop_the_world(|_| {
            log::trace!("GC Worker: flip");
            Self::flip();
            Self::flip_colours();
        });
        /*HEAP.weak_handles.lock().retain(|item| {
            if unsafe { (&**item).color.load(Ordering::Relaxed) == GC_WHITE } {
                unsafe {
                    (&mut **item).value.finalize();
                    std::ptr::drop_in_place(*item);
                }
                false
            } else {
                true
            }
        })*/
    }

    pub fn allocate<T: Trace + 'static>(&self, value: T, finalize: bool) -> *mut HeapInner<T> {
        let mut space = self.from_space.lock();
        let mut gc = false;
        let memory = space.allocate(std::mem::size_of::<HeapInner<T>>(), &mut gc);
        log::trace!("Allocate {:p}", memory.to_ptr::<u8>());
        if self.state.load(Ordering::Relaxed) != GC_COPYING {
            self.needs_gc.store(gc, Ordering::Relaxed);
        }
        unsafe {
            let raw = memory.to_mut_ptr::<HeapInner<T>>();
            raw.write(HeapInner {
                forward: AtomicUsize::new(raw as usize),
                color: AtomicU8::new(self.white.load(Ordering::Relaxed)),
                value,
            });
            if finalize {
                self.weak_handles.lock().push(raw);
            }

            raw
        }
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
                log::trace!(
                    "GC Worker: Copy {:p}->{:p}",
                    value.value(),
                    hvalue.to_mut_ptr::<u8>()
                );
                Self::visit(value.value());
                value.relocate(hvalue);
            } else {
                //value.relocate(value.value().fwdptr());
            }
        }
    }
}

fn collect_routine() {
    loop {
        let mut attempt = 0;
        while HEAP.state.load(Ordering::Relaxed) == GC_NONE {
            if attempt < 512 {
                std::thread::yield_now();
            } else {
                std::thread::sleep(std::time::Duration::from_nanos(10));
            }
            attempt += 1;
        }
        if HEAP.state.load(Ordering::Relaxed) == GC_TERMINATE {
            return;
        }

        GlobalHeap::collect_impl();
    }
}

lazy_static::lazy_static! {
    pub static ref HEAP: GlobalHeap = {
        std::thread::spawn(|| collect_routine());
        GlobalHeap::new()
    };
}
