#![allow(dead_code)]
#![feature(coerce_unsized)]
#![feature(unsize)]

pub(crate) mod bump;
use bump::BumpAllocator;
pub(crate) mod internal;
pub(crate) use internal::*;
pub mod impl_;

/// Trait that you must need to implement for objects that you want GC.
pub trait Trace {
    /// Trace all GC references in current object.
    /// ```rust
    /// impl Trace for MyObject {
    ///     fn trace(&self) {
    ///         let item1: &GC<dyn Trace> = &self.field;
    ///         item1.mark();
    ///     }
    /// }
    /// ```
    fn trace(&self) {}
}

use std::marker::Unsize;
use std::ops::CoerceUnsized;

impl<T: Trace + ?Sized + Unsize<U>, U: Trace + ?Sized> CoerceUnsized<GC<U>> for GC<T> {}

use std::cell::RefCell;

struct InGC<T: Trace + ?Sized> {
    fwd: Address,
    mark: bool,
    ptr: RefCell<T>,
}

impl<T: Trace + ?Sized> InGC<T> {
    fn copy_to(&mut self, to: Address) {
        unsafe {
            std::ptr::copy_nonoverlapping(
                self as *const _ as *const u8,
                to.to_mut_ptr(),
                std::mem::size_of_val(self),
            );
        }
    }
}

pub struct GC<T: Trace + ?Sized> {
    ptr: RefCell<*mut InGC<T>>,
}

impl<T: Trace + ?Sized> GC<T> {
    /// Get shared reference to object
    ///
    /// Function will panic if object already mutable borrowed
    pub fn borrow(&self) -> std::cell::Ref<'_, T> {
        unsafe { (*(*self.ptr.borrow())).ptr.borrow() }
    }

    /// Get mutable reference to object
    ///
    /// Function will panic if object already mutable borrowed
    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, T> {
        unsafe { (*(*self.ptr.borrow())).ptr.borrow_mut() }
    }
    /// Compare two pointers
    pub fn ref_eq(&self, other: &GC<T>) -> bool {
        *self.ptr.borrow() == *other.ptr.borrow()
    }
    /// Mark current object
    pub fn mark(&self) {
        let mut ptr = unsafe { &mut **self.ptr.borrow() };
        ptr.mark = true;
        self.borrow().trace();
    }
}
impl<T: Trace + ?Sized> Clone for GC<T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr.clone(),
        }
    }
}

pub const K: usize = 1024;
pub const M: usize = K * K;

pub struct GarbageCollector {
    total: Region,
    separator: Address,
    pub verbose: bool,
    alloc: BumpAllocator,
    allocated: Vec<GC<dyn Trace>>,
    roots: Vec<GC<dyn Trace>>,
}

fn align_usize(value: usize, align: usize) -> usize {
    if align == 0 {
        return value;
    }

    ((value + align - 1) / align) * align
}

impl GarbageCollector {
    pub fn new(heap_size: Option<usize>) -> GarbageCollector {
        let alignment = 2 * page_size() as usize;
        let heap_size = align_usize(heap_size.unwrap_or(M * 128), alignment);
        let ptr = mmap(heap_size, ProtType::Writable);
        let heap_start = Address::from_ptr(ptr);
        let heap = heap_start.region_start(heap_size);

        let semi_size = heap_size / 2;
        let separator = heap_start.offset(semi_size);
        GarbageCollector {
            total: heap,
            separator,
            roots: vec![],
            verbose: false,
            allocated: vec![],
            alloc: BumpAllocator::new(heap_start, separator),
        }
    }
    /// Remove object from rootset
    pub fn remove_root(&mut self, val: GC<dyn Trace>) {
        for i in 0..self.roots.len() {
            if self.roots[i].ref_eq(&val) {
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
    pub fn add_root(&mut self, val: GC<dyn Trace>) {
        for root in self.roots.iter() {
            if root.ref_eq(&val) {
                return;
            }
        }

        self.roots.push(val);
    }

    pub(crate) fn from_space(&self) -> Region {
        if self.alloc.limit() == self.separator {
            Region::new(self.total.start, self.separator)
        } else {
            Region::new(self.separator, self.total.end)
        }
    }
    /// Get space where we need copy objects
    pub(crate) fn to_space(&self) -> Region {
        if self.alloc.limit() == self.separator {
            Region::new(self.separator, self.total.end)
        } else {
            Region::new(self.total.start, self.separator)
        }
    }
    /// Allocate value in GC space.
    pub fn allocate<T: Trace + Sized + 'static>(&mut self, val: T) -> GC<T> {
        let mem: *mut InGC<T> =
            unsafe { std::mem::transmute(self.alloc.bump_alloc(std::mem::size_of::<InGC<T>>())) };
        if !mem.is_null() {
            unsafe {
                mem.write(InGC {
                    fwd: Address::null(),
                    ptr: RefCell::new(val),
                    mark: false,
                });
            }

            let cell = GC {
                ptr: RefCell::new(mem),
            };
            self.allocated.push(cell.clone());
            return cell;
        } else {
            self.collect();
            let mem: *mut InGC<T> = unsafe {
                std::mem::transmute(self.alloc.bump_alloc(std::mem::size_of::<InGC<T>>()))
            };
            unsafe {
                mem.write(InGC {
                    fwd: Address::null(),
                    ptr: RefCell::new(val),
                    mark: false,
                });
            }

            let cell = GC {
                ptr: RefCell::new(mem),
            };
            self.allocated.push(cell.clone());
            return cell;
        }
    }
    /// Collect garbage
    pub fn collect(&mut self) {
        let start_time = time::PreciseTime::now();
        let to_space = self.to_space();
        let from_space = self.from_space();
        let mut top = to_space.start;
        let mut dead = 0;
        let old_size = self.alloc.top().offset_from(from_space.start);
        for i in 0..self.roots.len() {
            let root = self.roots[i].clone();
            root.mark();
        }

        self.allocated.retain(|x: &GC<dyn Trace>| {
            let mark = unsafe { (**x.ptr.borrow()).mark };
            if mark == false {
                dead += 1;
            }
            mark
        });

        let live = self.allocated.len();

        for i in 0..self.allocated.len() {
            let x = self.allocated[i].clone();
            unsafe { &mut **x.ptr.borrow_mut() }.mark = false;
            unsafe { &mut **x.ptr.borrow_mut() }.fwd = Address::null();
            let new_addr = self.copy(*x.ptr.borrow(), &mut top);
            *x.ptr.borrow_mut() = unsafe { std::mem::transmute_copy(&new_addr) };
        }

        /*for i in 0..self.grey.len() {
            unsafe {
                let item = self.grey[i].clone();
                let new_addr = self.copy(*item.ptr.borrow(), &mut top);
                *item.ptr.borrow_mut() = std::mem::transmute_copy(&new_addr);
            }
        }*/
        self.alloc.reset(top, to_space.end);

        if self.verbose {
            let end = time::PreciseTime::now();
            let new_size = top.offset_from(to_space.start);
            let garbage = old_size.wrapping_sub(new_size);
            let garbage_ratio = if old_size == 0 {
                0f64
            } else {
                (garbage as f64 / old_size as f64) * 100f64
            };
            println!(
                "GC: {:.1} ms ({:.1} ns ), {}->{} size, {}/{:.0}% garbage ({} dead,{} live)",
                start_time.to(end).num_milliseconds(),
                start_time.to(end).num_nanoseconds().unwrap_or(0),
                formatted_size(old_size),
                formatted_size(new_size),
                formatted_size(garbage),
                garbage_ratio,
                dead,
                live
            );
        }
    }

    fn copy(&mut self, obj: *mut InGC<dyn Trace>, to: &mut Address) -> Address {
        unsafe {
            if (*obj).fwd != Address::null() {
                return (*obj).fwd;
            }
            let addr = *to;
            (*obj).copy_to(addr);

            *to = to.offset(std::mem::size_of_val(&*obj));
            (&mut (*obj).fwd as *mut Address).write(addr);
            return addr;
        };
    }
}

impl Drop for GarbageCollector {
    fn drop(&mut self) {
        munmap(self.total.start.to_ptr(), self.total.size());
    }
}

macro_rules! collectable_for_simple_types {
    ($($t: tt),*) => {
      $(  impl Trace for $t {
            fn trace(&self) {}
        }
      )*
    };
}

collectable_for_simple_types! {
    u8,u16,u32,u64,u128,
    i8,i16,i32,i128,i64,
    bool,String
}

impl<T: Trace> Trace for Vec<T> {
    fn trace(&self) {
        self.iter().for_each(|x| x.trace());
    }
}

impl<T: Trace> Trace for GC<T> {
    fn trace(&self) {
        self.mark();
    }
}

use std::fmt;

impl<T: fmt::Debug + Trace> fmt::Debug for GC<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.borrow())
    }
}

impl<T: Trace + Eq> Eq for GC<T> {}

impl<T: Trace + PartialEq> PartialEq for GC<T> {
    fn eq(&self, other: &Self) -> bool {
        *self.borrow() == *other.borrow()
    }
}

use std::cmp::{Ord, Ordering, PartialOrd};

impl<T: Trace + PartialOrd> PartialOrd for GC<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.borrow().partial_cmp(&other.borrow())
    }
}

impl<T: Trace + Ord + Eq> Ord for GC<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.borrow().cmp(&other.borrow())
    }
}

use std::hash::{Hash, Hasher};
impl<T: Hash + Trace> Hash for GC<T> {
    fn hash<H: Hasher>(&self, h: &mut H) {
        self.borrow().hash(h);
    }
}
