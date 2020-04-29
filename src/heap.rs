use crate::api::*;
use crate::mem::*;

#[cfg(not(feature = "trace-gc-timings"))]
const TRACE_GC_TIMINGS: bool = false;
#[cfg(feature = "trace-gc-timings")]
const TRACE_GC_TIMINGS: bool = true;

#[cfg(not(feature = "trace-gc"))]
const TRACE_GC: bool = false;
#[cfg(feature = "trace-gc")]
const TRACE_GC: bool = true;
pub struct HeapInner<T: Trace + ?Sized> {
    forward: TaggedPointer<u8>,
    soft_mark: bool,
    gen: u8,
    pub(crate) value: T,
}

impl<T: Trace + ?Sized> HeapInner<T> {
    pub fn is_marked(&self) -> bool {
        self.forward.bit_is_set(0)
    }
    pub fn set_soft_mark(&mut self) {
        self.soft_mark = true;
    }

    pub fn is_soft_marked(&self) -> bool {
        self.soft_mark
    }

    pub fn reset_soft_mark(&mut self) {
        self.soft_mark = false;
    }
    pub fn generation(&self) -> u8 {
        self.gen
    }

    pub fn inc_gen(&mut self) {
        if self.gen < 5 {
            self.gen += 1;
        }
    }
    pub(crate) fn mark(&mut self, b: bool) {
        if b {
            self.forward.set_bit(0);
        } else {
            self.forward.unset_bit(0);
        }
    }

    pub fn fwdptr(&self) -> Address {
        Address::from(self.forward.untagged() as usize)
    }

    pub fn set_fwdptr(&mut self, addr: Address) {
        let new_x = TaggedPointer::new(addr.to_mut_ptr::<u8>());
        self.forward = new_x;
    }
}

pub struct GcHandle<T: Trace + ?Sized>(pub(crate) *mut HeapInner<T>);
pub struct RootHandle(*mut dyn RootedTrait);

use super::space::*;

struct GcValue {
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
        if !self.value().is_marked() {
            self.value().set_fwdptr(addr);
            self.value().mark(true);
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum GCType {
    None,
    NewSpace,
    OldSpace,
}

pub struct Heap {
    new_space: Space,
    old_space: Space,
    needs_gc: GCType,
    heap: Vec<*mut HeapInner<dyn Trace>>,
    rootset: Vec<RootHandle>,
}

impl Heap {
    pub fn new(new_page_size: usize, old_page_size: usize) -> Heap {
        Self {
            new_space: Space::new(page_align(new_page_size)),
            old_space: Space::new(page_align(old_page_size)),
            needs_gc: GCType::None,
            heap: vec![],
            rootset: vec![],
        }
    }
    pub fn allocate<T: Trace + 'static>(&mut self, value: T) -> Rooted<T> {
        let mut needs_gc = false;
        let memory = self
            .new_space
            .allocate(std::mem::size_of::<HeapInner<T>>(), &mut needs_gc);
        self.needs_gc = GCType::NewSpace;

        let inner = memory.to_mut_ptr::<HeapInner<T>>();

        unsafe {
            inner.write(HeapInner {
                forward: TaggedPointer::null(),
                soft_mark: false,
                gen: 0,
                value,
            });
        }

        let root = Box::into_raw(Box::new(RootedInner {
            rooted: true,
            inner: inner,
        }));
        self.rootset.push(RootHandle(root));
        self.heap.push(inner);
        Rooted { inner: root }
    }

    pub fn needs_gc(&self) -> bool {
        self.needs_gc != GCType::None
    }

    pub fn safepoint(&mut self) {
        if self.needs_gc() {
            self.collect();
        }
    }

    pub fn collect(&mut self) {
        let time = std::time::Instant::now();
        let mut collection = Collection {
            gc_type: self.needs_gc,
            tmp_space: Space::new(self.new_space.page_size),
            heap: self,
            grey_set: LinkedList::new(),
            black_set: vec![],
            new_heap: vec![],
        };
        collection.collect(false);
        if TRACE_GC_TIMINGS {
            log::trace!("GC finished in {}ns", time.elapsed().as_nanos());
        }
    }
}

use std::collections::LinkedList;
impl std::hash::Hash for RootHandle {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (self.0 as *const u8 as usize).hash(state);
    }
}
struct Collection<'a> {
    heap: &'a mut Heap,
    tmp_space: Space,
    black_set: Vec<GcValue>,
    grey_set: LinkedList<GcValue>,
    gc_type: GCType,
    new_heap: Vec<*mut HeapInner<dyn Trace>>,
}

impl<'a> Collection<'a> {
    fn collect(&mut self, _: bool) {
        let time = std::time::Instant::now();
        let mut grey = LinkedList::new();
        if self.gc_type == GCType::None {
            self.gc_type = GCType::NewSpace;
            self.heap.needs_gc = GCType::NewSpace;
        }
        self.tmp_space = Space::new(self.heap.new_space.page_size);
        if TRACE_GC {
            log::trace!("GC Type: {:?}", self.gc_type);
        }

        if TRACE_GC {
            log::trace!("GC Phase 1: Mark roots");
        }
        self.heap.rootset.retain(|root| unsafe {
            let retain;
            if (&*root.0).is_rooted() {
                if TRACE_GC {
                    log::trace!(
                        "Root {:p} identified at {:p}",
                        (&*root.0).inner(),
                        (&*root.0).slot().to_ptr::<u8>()
                    );
                }
                let value = GcValue {
                    slot: (*root.0).slot(),
                    value: (&*root.0).inner(),
                };
                //value.value().mark(true);
                grey.push_back(value);
                retain = true;
            } else {
                let _ = Box::from_raw(root.0);
                retain = false;
            }
            retain
        });
        self.grey_set = grey;
        if TRACE_GC {
            log::trace!("GC Phase 2: Copy objects");
        }
        self.process_grey();

        let space = if self.gc_type == GCType::NewSpace {
            &mut self.heap.new_space
        } else {
            &mut self.heap.old_space
        };
        space.swap(&mut self.tmp_space);
        if TRACE_GC {
            log::trace!("GC Phase 3: Finalization");
        }
        while let Some(item) = self.heap.heap.pop() {
            if unsafe { !(&*item).is_marked() } && unsafe { !(&*item).is_soft_marked() } {
                unsafe {
                    if TRACE_GC {
                        log::trace!("Finalize {:p}", item);
                    }
                    (&mut *item).value.finalize();
                    std::ptr::drop_in_place(item);
                }
            }
        }
        while let Some(item) = self.black_set.pop() {
            item.value().reset_soft_mark();
        }
        for root in self.heap.rootset.iter() {
            unsafe {
                self.new_heap.push((&*root.0).inner());
                for r in (&*root.0).references() {
                    self.new_heap.push((&*r).inner());
                }
            }
        }
        std::mem::swap(&mut self.heap.heap, &mut self.new_heap);
        if self.gc_type != GCType::NewSpace || self.heap.needs_gc == GCType::NewSpace {
            // Reset GC flag
            self.heap.needs_gc = GCType::None;
            if TRACE_GC_TIMINGS {
                log::trace!(
                    "GC with type {:?} finished in {}ns",
                    self.gc_type,
                    time.elapsed().as_nanos()
                );
            }
        } else {
            if TRACE_GC {
                log::trace!("GC Phase 4: Restart GC for old space");
            }
            // Or call gc for old space.
            self.collect(true);
        }
    }

    fn visit(&mut self, value: &mut HeapInner<dyn Trace>) {
        value.value.references().iter().for_each(|item| unsafe {
            self.grey_set.push_back(GcValue {
                slot: (&**item).slot(),
                value: (&**item).inner(),
            })
        });
    }
    fn copy_to(
        value: &mut HeapInner<dyn Trace>,
        old_space: &mut Space,
        new_space: &mut Space,
        needs_gc: &mut GCType,
    ) -> Address {
        let bytes = std::mem::size_of_val(value);
        value.inc_gen();
        let result;
        if value.generation() >= 5 {
            let mut gc = false;
            result = old_space.allocate(bytes, &mut gc);
            if gc {
                *needs_gc = GCType::OldSpace;
            }
        } else {
            let mut gc = false;
            result = new_space.allocate(bytes, &mut gc);
            if gc {
                *needs_gc = GCType::NewSpace;
            }
        }
        unsafe {
            std::ptr::copy_nonoverlapping(
                value as *mut HeapInner<dyn Trace> as *mut u8,
                result.to_mut_ptr::<u8>(),
                bytes,
            );
        }
        result
    }
    fn process_grey(&mut self) {
        while let Some(value) = self.grey_set.pop_front() {
            if !value.value().is_marked() {
                if !self.is_in_current_space(value.value()) {
                    if !value.value().is_soft_marked() {
                        value.value().set_soft_mark();
                        self.visit(value.value());
                        self.new_heap.push(value.value());
                        self.black_set.push(value);
                    }
                    continue;
                }
                let hvalue;
                if self.gc_type == GCType::NewSpace {
                    let mut g = GCType::None;
                    hvalue = Self::copy_to(
                        value.value(),
                        &mut self.heap.old_space,
                        &mut self.tmp_space,
                        &mut g,
                    );
                    if let GCType::OldSpace = g {
                        self.heap.needs_gc = GCType::OldSpace;
                    }
                } else {
                    hvalue = Self::copy_to(
                        value.value(),
                        &mut self.tmp_space,
                        &mut self.heap.new_space,
                        &mut GCType::None,
                    );
                }
                if TRACE_GC {
                    log::trace!("Copy {:p}->{:p}", value.value(), hvalue.to_mut_ptr::<u8>());
                }

                value.relocate(hvalue);
                self.visit(value.value());
            } else {
                value.relocate(value.value().fwdptr());
            }
        }
    }

    fn is_in_current_space(&self, val: &mut HeapInner<dyn Trace>) -> bool {
        (self.gc_type == GCType::OldSpace && val.generation() >= 5)
            || (self.gc_type == GCType::NewSpace && val.generation() < 5)
    }
}
