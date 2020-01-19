use crate::mem::*;
use crate::rooting::*;
use crate::trace::*;

use std::sync::atomic::{AtomicBool, Ordering};
pub struct InnerPtr<T: Trace + ?Sized> {
    pub(crate) mark: AtomicBool,
    pub(crate) fwd: Address,
    pub(crate) value: T,
}

pub struct GcHandle<T: Trace + ?Sized>(*mut InnerPtr<T>);
pub struct RootHandle(*mut dyn RootedTrait);

pub struct GlobalCollector {
    heap: Vec<GcHandle<dyn Trace>>,
    roots: Vec<RootHandle>,
    memory_heap: Region,
    alloc: crate::bump::BumpAllocator,
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
        }
    }

    pub fn alloc<T: Trace + Sized + 'static>(&mut self, x: T) -> Rooted<T> {
        let ptr = self
            .alloc
            .bump_alloc(std::mem::size_of::<InnerPtr<T>>())
            .to_mut_ptr::<InnerPtr<T>>();
        unsafe {
            if !ptr.is_null() {
                ptr.write(InnerPtr {
                    mark: AtomicBool::new(false),
                    fwd: Address::null(),
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
                .alloc
                .bump_alloc(std::mem::size_of::<InnerPtr<T>>())
                .to_mut_ptr::<InnerPtr<T>>();
            ptr.write(InnerPtr {
                mark: AtomicBool::new(false),
                fwd: Address::null(),
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

    pub fn collect(&mut self) {
        let mut mc = MarkCompact {
            heap: self.memory_heap,
            heap_objects: &self.heap,
            rootset: &self.roots,
            top: self.memory_heap.start,
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
        self.roots = rootset;
        self.heap = heap;
    }
}

impl Drop for GlobalCollector {
    fn drop(&mut self) {
        uncommit(self.memory_heap.start, self.memory_heap.size());
    }
}

pub struct MarkCompact<'a> {
    heap: Region,
    top: Address,
    rootset: &'a [RootHandle],
    heap_objects: &'a [GcHandle<dyn Trace>],
}

impl<'a> MarkCompact<'a> {
    pub fn collect(&mut self) -> (Vec<RootHandle>, Vec<GcHandle<dyn Trace>>) {
        self.mark_live();
        self.compute_forward();
        self.relocate()
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

    pub fn compute_forward(&mut self) {
        for value in self.heap_objects.iter() {
            let value: *mut InnerPtr<dyn Trace> = value.0;

            unsafe {
                if (*value).mark.load(Ordering::Relaxed) {
                    let fwd = self.allocate(std::mem::size_of_val(&*value));
                    (*value).fwd = fwd;
                } else {
                    (*value).value.finalize();
                }
            }
        }
    }

    pub fn relocate(&mut self) -> (Vec<RootHandle>, Vec<GcHandle<dyn Trace>>) {
        let mut new_rootset = vec![];
        let mut new_heap = vec![];

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
                    new_heap.push(GcHandle(field.inner()));
                }

                let slot = root.get().slot().to_mut_ptr::<*mut u8>();
                let fwd = root.get().get_fwd();
                if Address::from_ptr(root.0 as *const u8) != fwd {
                    root.get().copy_to(fwd);
                }
                unsafe {
                    *slot = fwd.to_mut_ptr::<u8>();
                };
                new_heap.push(GcHandle(root.get().inner()));
                new_rootset.push(RootHandle(root.0));
            }
        }

        (new_rootset, new_heap)
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
