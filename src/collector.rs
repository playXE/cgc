use crate::mem::*;
use crate::rooting::*;
use crate::trace::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub struct InnerPtr<T: Trace + ?Sized> {
    pub(crate) fwdptr: AtomicUsize,
    pub(crate) value: T,
}

const MARK_BITS: usize = 2;
const MARK_MASK: usize = (2 << MARK_BITS) - 1;
const FWD_MASK: usize = !0 & !MARK_MASK;

impl<T: Trace + ?Sized> InnerPtr<T> {
    #[inline(always)]
    pub fn fwdptr_non_atomic(&self) -> Address {
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);
        (fwdptr & FWD_MASK).into()
    }

    #[inline(always)]
    pub fn set_fwdptr_non_atomic(&mut self, addr: Address) {
        debug_assert!((addr.to_usize() & MARK_MASK) == 0);
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);
        self.fwdptr
            .store(addr.to_usize() | (fwdptr & MARK_MASK), Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn mark_non_atomic(&mut self) {
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);
        self.fwdptr.store(fwdptr | 1, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn unmark_non_atomic(&mut self) {
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);
        self.fwdptr.store(fwdptr & FWD_MASK, Ordering::Relaxed);
    }

    #[inline(always)]
    pub fn is_marked_non_atomic(&self) -> bool {
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);
        (fwdptr & MARK_MASK) != 0
    }

    #[inline(always)]
    pub fn try_mark_non_atomic(&self) -> bool {
        let fwdptr = self.fwdptr.load(Ordering::Relaxed);

        if (fwdptr & MARK_MASK) != 0 {
            return false;
        }

        self.fwdptr.store(fwdptr | 1, Ordering::Relaxed);
        true
    }

    #[inline(always)]
    pub fn try_mark(&self) -> bool {
        let old = self.fwdptr.load(Ordering::Relaxed);
        self.fwdptr
            .compare_exchange(old, old | 1, Ordering::SeqCst, Ordering::Relaxed)
            .is_ok()
    }
}

pub struct GcHandle<T: Trace + ?Sized>(*mut InnerPtr<T>);
pub struct RootHandle(*mut dyn RootedTrait);

pub struct GlobalCollector {
    heap: Vec<GcHandle<dyn Trace>>,
    roots: Vec<RootHandle>,
    memory_heap: Region,
    alloc: crate::bump::BumpAllocator,
    sweep_alloc: SweepAllocator,
    //stats: parking_lot::Mutex<CollectionStats>,
}

impl GlobalCollector {
    pub fn new(heap_size: usize) -> Self {
        let heap_start = commit(heap_size, false);
        if heap_start.is_null() {
            panic!("GC: could not allocate heap of size {} bytes", heap_size);
        }
        let heap_end = heap_start.offset(heap_size);
        let heap = Region::new(heap_start, heap_end);
        Self {
            heap: vec![],
            roots: vec![],
            memory_heap: heap,
            alloc: crate::bump::BumpAllocator::new(heap.start, heap.end),
            sweep_alloc: SweepAllocator::new(heap),
            //stats: parking_lot::Mutex::new(CollectionStats::new()),
        }
    }

    pub fn alloc<T: Trace + Sized + 'static>(&mut self, x: T) -> Rooted<T> {
        //let mut timer = Timer::new(true);
        let ptr = self
            .sweep_alloc
            .allocate(std::mem::size_of::<InnerPtr<T>>())
            .to_mut_ptr::<InnerPtr<T>>();
        unsafe {
            if !ptr.is_null() {
                ptr.write(InnerPtr {
                    fwdptr: AtomicUsize::new(0),
                    value: x,
                });
                self.heap.push(GcHandle(ptr));

                let rooted = Box::into_raw(Box::new(RootedInner {
                    inner: ptr,
                    rooted: AtomicBool::new(true),
                }));

                let root = Rooted { inner: rooted };

                self.roots.push(RootHandle(rooted));

                return root;
            }

            self.compact();
            let ptr = self
                .sweep_alloc
                .allocate(std::mem::size_of::<InnerPtr<T>>())
                .to_mut_ptr::<InnerPtr<T>>();
            ptr.write(InnerPtr {
                fwdptr: AtomicUsize::new(0),
                value: x,
            });
            self.heap.push(GcHandle(ptr));

            let rooted = Box::into_raw(Box::new(RootedInner {
                inner: ptr,
                rooted: AtomicBool::new(true),
            }));

            let root = Rooted { inner: rooted };

            self.roots.push(RootHandle(rooted));
            //let stop = timer.stop();
            //let mut stats = self.stats.lock();
            //stats.add_alloc(stop);
            return root;
        }
    }

    pub fn compact(&mut self) {
        //let mut timer = Timer::new(true);
        let mut mc = MarkSweepCompact {
            heap: self.memory_heap,
            heap_objects: &self.heap,
            rootset: &self.roots,
            top: self.memory_heap.start,
            sweepalloc: &mut self.sweep_alloc,
        };
        let (rootset, heap) = mc.collect();
        self.alloc.reset(mc.top, self.memory_heap.end);

        /*self.heap.retain(|x| unsafe {
            let inner: *mut InnerPtr<dyn Trace> = x.0;
            if !inner.is_null() {
                match (*inner).mark.compare_exchange(
                    true,
                    false,
                    Ordering::Acquire,
                    Ordering::Relaxed,
                ) {
                    Ok(true) => {
                        (*inner).value.unmark();
                        true
                    }
                    Err(false) => {
                        (*inner).value.finalize();
                        false
                    }
                    _ => unreachable!(),
                }
            } else {
                false
            }
        });*/
        //let duration = timer.stop();
        //let mut stats = self.stats.lock();
        //stats.add(duration);
        self.roots = rootset;
        self.heap = heap;
    }

    /*pub fn summary(&self) {
        let mut timer = TIMER.write();
        let runtime = timer.stop();
        let stats = self.stats.lock();

        //let (mutator, gc) = stats.percentage(runtime);
        eprintln!("GC stats: total={:.1}", runtime);
        eprintln!("GC stats: mutator={:.1}", stats.mutator(runtime));
        eprintln!("GC stats: collection={:.1}", stats.pause());
        eprintln!("GC stats: allocations={:.1}", stats.allocation_pause());
        eprintln!("GC stats: allocations-count={}", stats.allocations());
        eprintln!("GC stats: collection-count={}", stats.collections());
        eprintln!("GC stats: collection-pauses={}", stats.pauses());
        /*eprintln!("GC stats: threshold={}", self.threshold);
        eprintln!("GC stats: total allocated={}", self.total_allocated);
        */
        eprintln!(
            "GC summary: {:.1} ms allocation, {:.1}ms collection ({}), {:.1}ms mutator, {:.1}ms total",
            stats.allocation_pause(),
            stats.pause(),
            stats.collections(),
            stats.mutator(runtime),
            runtime,
            /*mutator,
            gc,*/
        );
    }*/
}

impl Drop for GlobalCollector {
    fn drop(&mut self) {
        uncommit(self.memory_heap.start, self.memory_heap.size());
    }
}

pub struct MarkSweepCompact<'a> {
    heap: Region,
    top: Address,
    rootset: &'a [RootHandle],
    heap_objects: &'a [GcHandle<dyn Trace>],
    sweepalloc: &'a mut SweepAllocator,
}

impl<'a> MarkSweepCompact<'a> {
    pub fn collect(&mut self) -> (Vec<RootHandle>, Vec<GcHandle<dyn Trace>>) {
        self.mark_live();
        let new_heap = self.sweep();
        if self.sweepalloc.free_list.fragmentation() >= 0.70 {
            self.compute_forward();
            println!("comapcting");
            self.sweepalloc
                .free_list
                .add(self.top, self.heap.end.to_usize() - self.top.to_usize());
            (self.relocate(), new_heap)
        } else {
            let mut rootset = vec![];
            for root in self.rootset.iter() {
                if unsafe { (*root.0).is_rooted() } {
                    rootset.push(RootHandle(root.0))
                }
            }

            (rootset, new_heap)
        }
    }

    pub fn mark_live(&mut self) {
        for root in self.rootset.iter() {
            let root: Ptr<dyn RootedTrait> = Ptr(root.0);
            if root.get().is_rooted() {
                root.get().mark();
                let mut fields = root.get().fields();

                for field in fields.iter_mut() {
                    field.mark();
                }
            }
        }
    }

    pub fn sweep(&mut self) -> Vec<GcHandle<dyn Trace>> {
        let mut new_heap = vec![];
        for value in self.heap_objects.iter() {
            let value: *mut InnerPtr<dyn Trace> = value.0;
            unsafe {
                if !(*value).is_marked_non_atomic() {
                    (*value).value.finalize();
                    let start = Address::from_ptr(value as *const u8);

                    self.sweepalloc
                        .free_list
                        .add(start, std::mem::size_of_val(&*value));
                } else {
                    new_heap.push(GcHandle(value));
                }
            }
        }

        new_heap
    }

    pub fn compute_forward(&mut self) {
        for value in self.heap_objects.iter() {
            let value: *mut InnerPtr<dyn Trace> = value.0;

            unsafe {
                if (*value).is_marked_non_atomic() {
                    let fwd = self.allocate(std::mem::size_of_val(&*value));
                    (*value).set_fwdptr_non_atomic(fwd);
                    //(*value).fwd = fwd;
                }
            }
        }
    }

    pub fn relocate(&mut self) -> Vec<RootHandle> {
        let mut new_rootset = vec![];

        for root in self.rootset.iter() {
            let root: Ptr<dyn RootedTrait> = Ptr(root.0);
            if root.get().is_rooted() {
                for field in root.get().fields() {
                    let field: &mut dyn HeapTrait = field;

                    let slot = field.slot().to_mut_ptr::<*mut u8>();
                    let fwd = field.get_fwd();
                    if field.addr() != fwd {
                        field.copy_to(fwd);
                    }
                    unsafe {
                        *slot = fwd.to_mut_ptr::<u8>();
                    }
                }

                let slot = root.get().slot().to_mut_ptr::<*mut u8>();
                let fwd = root.get().get_fwd();
                if Address::from_ptr(root.0 as *const u8) != fwd {
                    root.get().copy_to(fwd);
                }
                unsafe {
                    *slot = fwd.to_mut_ptr::<u8>();
                };
                new_rootset.push(RootHandle(root.0));
            }
        }

        new_rootset
    }

    fn allocate(&mut self, object_size: usize) -> Address {
        let addr = self.top;
        let next = self.top.offset(object_size);

        if next <= self.heap.end {
            self.top = next;
            return addr;
        }

        panic!("FAIL: Not enough space for objects.");
    }
}

struct CollectionStats {
    collections: usize,
    allocations: usize,
    allocation_pauses: f64,
    total_allocation_pauses: Vec<f64>,
    total_pause: f64,
    pauses: Vec<f64>,
}
use std::fmt;
pub struct AllNumbers(Vec<f64>);

impl fmt::Display for AllNumbers {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        let mut first = true;
        for num in &self.0 {
            if !first {
                write!(f, ",")?;
            }
            write!(f, "{:.1}", num)?;
            first = false;
        }
        write!(f, "]")
    }
}

impl CollectionStats {
    fn new() -> CollectionStats {
        CollectionStats {
            collections: 0,
            total_pause: 0f64,
            pauses: Vec::new(),
            total_allocation_pauses: vec![],
            allocation_pauses: 0f64,
            allocations: 0,
        }
    }

    fn add(&mut self, pause: f64) {
        self.collections += 1;
        self.total_pause += pause;
        self.pauses.push(pause);
    }
    fn add_alloc(&mut self, pause: f64) {
        self.allocations += 1;
        self.allocation_pauses += pause;
        self.total_allocation_pauses.push(pause);
    }

    fn pause(&self) -> f64 {
        self.total_pause
    }

    fn pauses(&self) -> AllNumbers {
        AllNumbers(self.pauses.clone())
    }

    fn mutator(&self, runtime: f64) -> f64 {
        runtime - self.total_pause
    }

    fn collections(&self) -> usize {
        self.collections
    }

    fn allocations(&self) -> usize {
        self.allocations
    }

    fn allocation_pause(&self) -> f64 {
        self.allocation_pauses
    }
    fn allocation_pauses(&self) -> AllNumbers {
        AllNumbers(self.total_allocation_pauses.clone())
    }

    fn percentage(&self, runtime: f64) -> (f64, f64) {
        let gc_percentage = ((self.total_pause / runtime) * 100.0).round();
        let mutator_percentage = 100.0 - gc_percentage;

        (mutator_percentage, gc_percentage)
    }
}

lazy_static::lazy_static!(
    static ref TIMER: parking_lot::RwLock<Timer> = parking_lot::RwLock::new(Timer::new(true));
);

pub struct Timer {
    active: bool,
    timestamp: u64,
}

impl Timer {
    pub fn new(active: bool) -> Timer {
        let ts = if active { timestamp() } else { 0 };

        Timer {
            active: active,
            timestamp: ts,
        }
    }

    pub fn stop(&mut self) -> f64 {
        assert!(self.active);
        let curr = timestamp();
        let last = self.timestamp;
        self.timestamp = curr;

        in_ms(curr - last)
    }

    pub fn stop_with<F>(&self, f: F) -> u64
    where
        F: FnOnce(f64),
    {
        if self.active {
            let ts = timestamp() - self.timestamp;

            f(in_ms(ts));

            ts
        } else {
            0
        }
    }

    pub fn ms<F>(active: bool, f: F) -> f64
    where
        F: FnOnce(),
    {
        if active {
            let ts = timestamp();
            f();
            let diff = timestamp() - ts;
            in_ms(diff)
        } else {
            f();
            0.0f64
        }
    }
}

pub fn in_ms(ns: u64) -> f64 {
    (ns as f64) / 1000.0 / 1000.0
}

pub fn timestamp() -> u64 {
    use core::convert::TryInto;
    (time::PrimitiveDateTime::now() - time::PrimitiveDateTime::unix_epoch())
        .whole_nanoseconds()
        .try_into()
        .expect("You really shouldn't be using this in the year 2554...")
}

use crate::freelist::*;

struct SweepAllocator {
    top: Address,
    limit: Address,
    free_list: FreeList,
}

impl SweepAllocator {
    fn new(heap: Region) -> SweepAllocator {
        SweepAllocator {
            top: heap.start,
            limit: heap.end,
            free_list: FreeList::new(),
        }
    }

    fn allocate(&mut self, size: usize) -> Address {
        let object = self.top;
        let next_top = object.offset(size);

        if next_top <= self.limit {
            self.top = next_top;
            return object;
        }

        let (free_space, size) = self.free_list.alloc(size);

        if free_space.is_non_null() {
            let object = free_space.addr();
            let free_size = size;
            assert!(size <= free_size);

            let free_start = object.offset(size);
            let free_end = object.offset(free_size);
            let new_free_size = free_end.offset_from(free_start);

            self.free_list.add(free_start, new_free_size);
            return object;
        }

        Address::null()
    }
}
