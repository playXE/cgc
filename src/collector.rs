use crate::mem::*;
use crate::rooting::*;
use crate::trace::*;

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
pub struct InnerPtr<T: Trace + ?Sized> {
    pub(crate) fwdptr: AtomicUsize,
    pub(crate) value: T,
}

unsafe impl<T: Trace + ?Sized> Send for InnerPtr<T> {}
unsafe impl<T: Trace + ?Sized> Sync for InnerPtr<T> {}

const MARK_BITS: usize = 2;
const MARK_MASK: usize = (2 << MARK_BITS) - 1;
const COLOR_MASK: usize = 7;
const GC_GRAY: usize = 0;
const GC_WHITE_A: usize = 1;
const GC_WHITE_B: usize = 1 << 1;
const GC_WHITES: usize = GC_WHITE_A | GC_WHITE_B;

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
pub struct RootHandle(pub(crate) *mut dyn RootedTrait);

pub struct GlobalCollector {
    heap: Vec<GcHandle<dyn Trace>>,
    roots: Vec<RootHandle>,
    memory_heap: Region,

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
            sweep_alloc: SweepAllocator::new(heap),
        }
    }

    pub fn alloc<T: Trace + Sized + 'static>(&mut self, x: T) -> Rooted<T> {
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

            self.collect();
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
            return root;
        }
    }

    pub fn fragmentation(&self) -> f32 {
        self.sweep_alloc.free_list.fragmentation()
    }

    pub fn collect(&mut self) {
        self.heap.sort_unstable_by(|x, y| {
            Address::from_ptr(x.0 as *const u8).cmp(&Address::from_ptr(y.0 as *const u8))
        });

        let mut mc = MarkCompact {
            heap: self.memory_heap,
            heap_objects: &self.heap,
            rootset: &self.roots,
            top: self.memory_heap.start,
            freelist: FreeList::new(),
        };
        let (rootset, heap, compacted) = mc.collect(self.sweep_alloc.free_list.fragmentation());

        if compacted {
            self.sweep_alloc.top = mc.top;
            self.sweep_alloc.limit = self.memory_heap.end;
            self.sweep_alloc.free_list = FreeList::new(); // reset free list since we compacted the heap
        }
        self.sweep_alloc.free_list = mc.freelist;
        self.roots = rootset;
        self.heap = heap;
        trace!("Mark-Compact GC: Stop");
    }

    pub fn force_compact(&mut self) {
        self.heap.sort_unstable_by(|x, y| {
            Address::from_ptr(x.0 as *const u8).cmp(&Address::from_ptr(y.0 as *const u8))
        });

        let mut mc = MarkCompact {
            heap: self.memory_heap,
            heap_objects: &self.heap,
            rootset: &self.roots,
            top: self.memory_heap.start,
            freelist: FreeList::new(),
        };
        let (rootset, heap, compacted) = mc.collect(1.0);
        if compacted {
            self.sweep_alloc.top = mc.top;
            self.sweep_alloc.limit = self.memory_heap.end;
            self.sweep_alloc.free_list = FreeList::new(); // reset free list since we compacted the heap
        }
        self.sweep_alloc.free_list = mc.freelist;
        self.roots = rootset;
        self.heap = heap;
        trace!("Mark-Compact GC: Stop");
    }
}

impl Drop for GlobalCollector {
    fn drop(&mut self) {
        trace!("Mark-Compact GC: GC droped.Start collection");
        self.collect();
        uncommit(self.memory_heap.start, self.memory_heap.size());
    }
}

const GC_STEP_SIZE: usize = 1024;
const GC_STEP_RATION: usize = 200;

#[derive(PartialEq, Eq)]
enum IncrementalState {
    Done,
    Marking,
    Sweeping,
}

pub struct IncrementalCollection<'a> {
    state: IncrementalState,
    rootset: &'a [RootHandle],
}

impl<'a> IncrementalCollection<'a> {
    pub fn step(&mut self) {
        let (limit, mut result) = ((GC_STEP_SIZE / 100) * GC_STEP_RATION, 0);

        while result < limit {
            if self.state == IncrementalState::Done {
                break;
            }
        }
    }
}

pub struct MarkCompact<'a> {
    heap: Region,
    top: Address,
    rootset: &'a [RootHandle],
    heap_objects: &'a [GcHandle<dyn Trace>],
    freelist: FreeList,
}

impl<'a> MarkCompact<'a> {
    pub fn collect(
        &mut self,
        fragmentation: f32,
    ) -> (Vec<RootHandle>, Vec<GcHandle<dyn Trace>>, bool) {
        trace!("Mark-Compact GC: Phase 1 (marking)");
        self.mark_live();
        trace!("Mark-Compact GC: Phase 2 (sweep)");
        let new_heap = self.sweep(fragmentation >= 0.50);

        if fragmentation >= 0.50 {
            trace!("Mark-Compact GC: Phase 3 (compaction)");
            self.compute_forward();

            (self.relocate(), new_heap, true)
        } else {
            let mut rootset = vec![];
            for root in self.rootset.iter() {
                if Ptr(root.0).get().is_rooted() {
                    Ptr(root.0).get().unmark();
                    rootset.push(RootHandle(root.0));
                }
            }
            (rootset, new_heap, false)
        }
    }

    pub fn mark_live(&mut self) -> usize {
        let mut result = 0;
        for root in self.rootset.iter() {
            let root: Ptr<dyn RootedTrait> = Ptr(root.0);
            if root.get().is_rooted() {
                unsafe {
                    if !(*root.get().inner()).is_marked_non_atomic() {
                        (*root.get().inner()).mark_non_atomic();
                        let mut fields = root.get().fields();

                        for field in fields.iter_mut() {
                            if (*field.inner()).is_marked_non_atomic() == false {
                                (*field.inner()).mark_non_atomic();
                                result += 1;
                            }
                        }
                        result += 1;
                    }
                }
            }
        }
        result
    }

    pub fn sweep(&mut self, fragmented: bool) -> Vec<GcHandle<dyn Trace>> {
        let mut new_heap = vec![];
        let mut garbage_start = Address::null();
        trace!(
            "Mark-Compact GC: Sweep heap with {} object(s)",
            self.heap_objects.len()
        );
        for value in self.heap_objects.iter() {
            let value: *mut InnerPtr<dyn Trace> = value.0;
            unsafe {
                if (*value).is_marked_non_atomic() {
                    if !fragmented {
                        self.add_freelist(garbage_start, Address::from_ptr(value as *const u8));
                    }
                    garbage_start = Address::null();
                    new_heap.push(GcHandle(value));
                } else if garbage_start.is_non_null() {
                    trace!("Mark-Compact GC: Sweep 0x{:x}", value as *const u8 as usize);
                    (*value).value.finalize();
                } else {
                    trace!("Mark-Compact GC: Sweep 0x{:x}", value as *const u8 as usize);
                    (*value).value.finalize();
                    garbage_start = Address::from_ptr(value as *const u8);
                }
            }
        }
        if !fragmented {
            self.add_freelist(garbage_start, self.heap.end);
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
                root.get().unmark();
                for field in root.get().fields() {
                    let field: &mut dyn HeapTrait = field;

                    let slot = field.slot().to_mut_ptr::<*mut u8>();
                    let fwd = field.get_fwd();
                    if field.addr() != fwd {
                        trace!(
                            "relocate field 0x{:x}->0x{:x}",
                            field.addr().to_usize(),
                            fwd.to_usize()
                        );
                        field.copy_to(fwd);
                    }
                    unsafe {
                        *slot = fwd.to_mut_ptr::<u8>();
                    }
                }

                let slot = root.get().slot().to_mut_ptr::<*mut u8>();
                let fwd = root.get().get_fwd();
                if Address::from_ptr(root.0 as *const u8) != fwd {
                    trace!(
                        "relocate root 0x{:x}->0x{:x}",
                        root.0 as *const u8 as usize,
                        fwd.to_usize()
                    );
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

    fn add_freelist(&mut self, start: Address, end: Address) {
        if start.is_null() {
            return;
        }

        let size = end.offset_from(start);
        self.freelist.add(start, size);
    }
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
            if new_free_size != 0 {
                self.free_list.add(free_start, new_free_size);
            }
            return object;
        }

        Address::null()
    }
}
