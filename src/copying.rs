pub trait Collectable {
    /// Get all children GC objects from `self`
       fn child(&self) -> Vec<GCValue<dyn Collectable>> {
        vec![]
    }
    #[doc(hidden)]
    fn size(&self) -> usize {
        0
    }
}

use std::cell::{Ref, RefCell, RefMut};

use super::*;
use bump::*;

fn align_usize(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }

    ((value + align - 1) / align) * align
}

use std::marker::Unsize;
use std::ops::CoerceUnsized;

impl<T: Collectable + ?Sized + Unsize<U>, U: Collectable + ?Sized> CoerceUnsized<GCValue<U>>
for GCValue<T>
{
}
struct InGC<T: Collectable + ?Sized> {
    fwd: Address,
    ptr: RefCell<T>
}

unsafe impl<T: Collectable + ?Sized + Send> Send for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Send> Send for GCValue<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for GCValue<T> {}

impl<T: Collectable + ?Sized> InGC<T> {
    fn size(&self) -> usize {
        self.ptr.borrow().size()
    }

    fn copy_to(&self, dest: Address, size: usize) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                self as *const Self as *const u8,
                dest.to_mut_ptr::<u8>(),
                size,
            )
        }
    }
}

pub struct GCValue<T: Collectable + ?Sized> {
    ptr: *mut InGC<T>,
}

impl<T: Collectable + ?Sized> GCValue<T> {
    fn size(&self) -> usize {
        unsafe { ((*self.ptr).ptr).borrow().size() }
    }

    fn fwd(&self) -> Address {
        unsafe { (*self.ptr).fwd }
    }

    fn get_ptr(&self) -> *mut InGC<T> {
        self.ptr
    }

    pub fn borrow(&self) -> Ref<'_, T> {
        unsafe { (*self.ptr).ptr.borrow() }
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        unsafe { (*self.ptr).ptr.borrow_mut() }
    }
}

impl<T: Collectable + ?Sized> Clone for GCValue<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Collectable + ?Sized> Copy for GCValue<T> {}

pub struct CopyGC {
    total: Region,
    separator: Address,

    alloc: BumpAllocator,
    roots: Vec<GCValue<dyn Collectable>>,
    allocated: Vec<GCValue<dyn Collectable>>,
    pub stats: bool,
}
extern "C" {
    fn malloc(_: usize) -> *mut u8;
    fn free(_: *mut u8);
    fn memcpy(_: *mut u8, _: *const u8, _: usize);
}

impl CopyGC {
    /// Construct new garbage collector
    pub fn new(heap_size: Option<usize>) -> CopyGC {
        let alignment = 2 * super::page_size() as usize;
        let heap_size = align_usize(heap_size.unwrap_or(M * 128), alignment);
        let ptr = super::mmap(heap_size, ProtType::Writable);
        let heap_start = Address::from_ptr(ptr);
        let heap = heap_start.region_start(heap_size);

        let semi_size = heap_size / 2;
        let separator = heap_start.offset(semi_size);

        CopyGC {
            total: heap,
            separator,
            roots: vec![],
            stats: false,
            allocated: vec![],
            alloc: BumpAllocator::new(heap_start, separator),
        }
    }

    pub fn total_allocated(&self) -> usize {
        let mut s = 0;
        for allocated in self.allocated.iter() {
            s += allocated.size();
        }
        s
    }
    /// Get space from where we copy objects
    pub fn from_space(&self) -> Region {
        if self.alloc.limit() == self.separator {
            Region::new(self.total.start, self.separator)
        } else {
            Region::new(self.separator, self.total.end)
        }
    }
    /// Get space where we need copy objects
    pub fn to_space(&self) -> Region {
        if self.alloc.limit() == self.separator {
            Region::new(self.separator, self.total.end)
        } else {
            Region::new(self.total.start, self.separator)
        }
    }
    /// Remove object from rootset
    pub fn remove_root(&mut self, val: GCValue<dyn Collectable>) {
        for i in 0..self.roots.len() {
            if self.roots[i].ptr == val.ptr {
                self.roots.remove(i);
                break;
            }
        }
    }
    /// Add root object to rootset
    ///
    /// What is a root object?
    /// - Static or global variables
    /// - Some object that may own some other objects
    ///
    pub fn add_root(&mut self, val: GCValue<dyn Collectable>) {
        let mut contains = false;
        for root in self.roots.iter() {
            contains = root.ptr == val.ptr;
            if contains {
                break;
            }
        }

        if !contains {
            self.roots.push(val);
        }
    }
    /// Collect garbage
    pub fn collect(&mut self) {
        let start_time = time::PreciseTime::now();
        let to_space = self.to_space();
        let from_space = self.from_space();
        let old_size = self.alloc.top().offset_from(from_space.start);
        let mut top = to_space.start;
        let mut scan = top;
        // Visit all roots and move them to new space
        for i in 0..self.roots.len() {
            let mut root = self.roots[i];
            let root_ptr = root.ptr;
            let ptr = unsafe { std::mem::transmute_copy(&root_ptr) };
            // if current space contains root move it to new space
            if from_space.contains(ptr) {
                let ptr2 = unsafe { std::mem::transmute_copy(&root_ptr) };
                unsafe {
                    root.ptr = std::mem::transmute_copy(&self.copy(ptr2, &mut top));
                }
            }
        }
        let mut i = 0;
        // Visit all objects in current space then move them to new space if needed
        while scan < top {
            unsafe {
                let object: *mut InGC<dyn Collectable> = self.allocated[i].ptr;
                assert!(!object.is_null());
                for child in (*object).ptr.borrow().child().iter() {
                    let child_ptr: *mut InGC<dyn Collectable> = child.get_ptr();
                    if child_ptr.is_null() {
                        panic!();
                    }
                    // If current space contains object then move it to new space
                    if from_space.contains(std::mem::transmute_copy(&child_ptr)) {
                        *(child_ptr as *mut *mut InGC<dyn Collectable>) = std::mem::transmute_copy(
                            &self.copy(std::mem::transmute_copy(&child_ptr), &mut top),
                        );
                    }
                }
                i = i + 1;
                let real_size = std::mem::size_of_val(&*object);
                scan = scan.offset(real_size);
            }
        }
        self.alloc.reset(top, to_space.end);

        if self.stats {
            let end = time::PreciseTime::now();
            let new_size = top.offset_from(to_space.start);
            let garbage = old_size.wrapping_sub(new_size);
            let garbage_ratio = if old_size == 0 {
                0f64
            } else {
                (garbage as f64 / old_size as f64) * 100f64
            };
            println!(
                "GC: {:.1} ms ({:.1} ns ), {}->{} size, {}/{:.0}% garbage",
                start_time.to(end).num_milliseconds(),
                start_time.to(end).num_nanoseconds().unwrap_or(0),
                formatted_size(old_size),
                formatted_size(new_size),
                formatted_size(garbage),
                garbage_ratio,
            );
        }
    }

    fn copy(&self, obj: *mut InGC<dyn Collectable>, top: &mut Address) -> Address {
        let obj: *mut InGC<dyn Collectable> = obj;
        assert!(!obj.is_null());
        unsafe {
            // if this object already moved to new space return it's address
            if (*obj).fwd.is_non_null() {
                return (*obj).fwd;
            }

            let addr = *top;
            let size = std::mem::size_of_val(&*obj);
            // copy object to new space
            (*obj).copy_to(addr, size);
            // move pointer
            *top = top.offset(size);
            assert!(top.is_non_null());
            // set forward address if we will visit this object again
            (*obj).fwd = addr;
            assert!(addr.is_non_null());

            addr
        }
    }
    /// Allocate new value in GC heap and return `GCValue` instance
    pub fn allocate<T: Collectable + Sized + 'static>(&mut self, val: T) -> GCValue<T> {
        let real_layout = std::alloc::Layout::new::<InGC<T>>();
        let ptr = self.alloc.bump_alloc(real_layout.size());

        if ptr.is_non_null() {
            let val_ = GCValue {
                ptr: ptr.to_mut_ptr(),
            };

            unsafe {
                ((*val_.ptr).fwd) = Address::null();
                ((*val_.ptr).ptr) = RefCell::new(val);
            }
            self.allocated.push(val_);
            return val_;
        }

        self.collect();
        let ptr = self.alloc.bump_alloc(real_layout.size());
        let val_ = GCValue {
            ptr: ptr.to_mut_ptr(),
        };
        unsafe {
            ((*val_.ptr).ptr) = RefCell::new(val);
        }
        self.allocated.push(val_);
        return val_;
    }
}

impl Drop for CopyGC {
    fn drop(&mut self) {
        munmap(self.total.start.to_ptr(), self.total.size());
    }
}

impl Collectable for i64 {
    fn child(&self) -> Vec<GCValue<dyn Collectable>> {
        vec![]
    }

    fn size(&self) -> usize {
        std::mem::size_of::<i64>()
    }
}

macro_rules! collectable_for_simple_types {
    ($($t: tt),*) => {
      $(  impl Collectable for $t {
            fn child(&self) -> Vec<GCValue<dyn Collectable>> {
                vec![]
            }

            fn size(&self) -> usize {
                std::mem::size_of::<$t>()
            }
        }
      )*
    };
}

collectable_for_simple_types! {
    u8,u16,u32,u64,u128,
    i8,i16,i32,i128,
    bool,String
}

impl<T: Collectable> Collectable for Vec<T> {
    fn child(&self) -> Vec<GCValue<dyn Collectable>> {
        let mut child = vec![];
        for x in self.iter() {
            child.extend(x.child().iter().cloned());
        }
        child
    }

    fn size(&self) -> usize {
        std::mem::size_of::<Self>()
    }
}

impl<T: Collectable> Collectable for GCValue<T> {
    fn child(&self) -> Vec<GCValue<dyn Collectable>> {
        self.borrow().child()
    }

    fn size(&self) -> usize {
        self.borrow().size()
    }
}

use std::fmt;

impl<T: fmt::Debug + Collectable> fmt::Debug for GCValue<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.borrow())
    }
}

impl<T: Collectable + Eq> Eq for GCValue<T> {}

impl<T: Collectable + PartialEq> PartialEq for GCValue<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.borrow() == *other.borrow()
    }
}

use std::cmp::{Ord, Ordering, PartialOrd};

impl<T: Collectable + PartialOrd> PartialOrd for GCValue<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.borrow().partial_cmp(&other.borrow())
    }
}

impl<T: Collectable + Ord + Eq> Ord for GCValue<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.borrow().cmp(&other.borrow())
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    #[test]
    fn alloc_int() {
        let val = gc_allocate_sync(42);
        gc_enable_stats();
        assert_eq!(*val.borrow(), 42);
    }

    #[test]
    fn alloc_10000strings() {
        for _ in 0..10000 {
            gc_allocate("Hello,World!".to_owned());
        }
        gc_collect_not_par();
        assert!(true);
    }
}
