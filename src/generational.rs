pub trait Collectable {
    /// Get all children GC objects from `self`
    fn visit(&self,_: &mut GenerationalGC) {}
}

use std::cell::{Ref, RefCell, RefMut};

use super::{mmap,ProtType,Address,Region,FormattedSize,formatted_size,M,K,bump};
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

impl<T: Collectable> Collectable for GCValue<T> {
    fn visit(&self,gc: &mut GenerationalGC ) {
        self.borrow().visit(gc);
    }
}

struct InGC<T: Collectable + ?Sized> {
    mark: u8,
    marked: bool,
    gen: u8,
    fwd: Address,
    ptr: RefCell<T>,
}

unsafe impl<T: Collectable + ?Sized + Send> Send for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Send> Send for GCValue<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for GCValue<T> {}

impl<T: Collectable + ?Sized> InGC<T> {
    #[inline]
    fn is_marked(&self) -> bool {
        self.marked & (self.mark & 0x80 != 0)
    }
    #[inline]
    fn get_mark(&self) -> Address {
        self.fwd
    }

    #[inline]
    fn is_soft_marked(&self) -> bool {
        self.marked & (self.mark & 0x40 != 0)
    }

    #[inline]
    fn set_soft_mark(&mut self) {
        self.marked = true;
        self.mark |= 0x40;
    }
    #[inline]
    fn set_mark(&mut self,addr: Address) {
        self.fwd = addr;
        self.marked = true;
        self.mark |= 0x80;
    }

    fn reset_soft_mark(&mut self) {
        if self.is_soft_marked() {
            self.mark ^= 0x40;
        }
    }
    #[inline]
    fn increment_gen(&mut self) {
        if self.gen < 5 {
            self.gen += 1;
        }
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
    fn get_ptr(&self) -> *mut InGC<T> {
        self.ptr
    }

    pub fn borrow(&self) -> Ref<'_, T> {
        unsafe { (*self.ptr).ptr.borrow() }
    }

    pub fn borrow_mut(&self) -> RefMut<'_, T> {
        unsafe { (*self.ptr).ptr.borrow_mut() }
    }

    pub fn generation(&self) -> u8 {
        unsafe {
            (*self.ptr).gen
        }
    }

}

impl<T: Collectable + ?Sized> Clone for GCValue<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: Collectable + ?Sized> Copy for GCValue<T> {}

use super::bump::BumpAllocator;


#[derive(PartialOrd, PartialEq,Ord,Eq,Debug,Copy, Clone)]
enum GCType {
    OldSpace,
    NewSpace
}


pub struct GenerationalGC {
    total: Region,
    separator: Address,
    gc_type: GCType,
    alloc: BumpAllocator,
    roots: Vec<GCValue<dyn Collectable>>,
    allocated: Vec<GCValue<dyn Collectable>>,
    grey: Vec<GCValue<dyn Collectable>>,
    black: Vec<GCValue<dyn Collectable>>,
    tmp_space: Address,
    pub stats: bool,
}

impl GenerationalGC {
    pub fn new(heap_size: Option<usize>) -> GenerationalGC {
        let alignment = 2 * super::page_size() as usize;
        let heap_size = align_usize(heap_size.unwrap_or(M * 128), alignment);
        let ptr = super::mmap(heap_size, ProtType::Writable);
        let heap_start = Address::from_ptr(ptr);
        let heap = heap_start.region_start(heap_size);

        let semi_size = heap_size / 2;
        let separator = heap_start.offset(semi_size);

        GenerationalGC {
            total: heap,
            separator,
            roots: vec![],
            stats: false,
            allocated: vec![],
            alloc: BumpAllocator::new(heap_start, separator),
            black: vec![],
            grey: vec![],
            tmp_space: Address::null(),
            gc_type: GCType::OldSpace
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

    pub fn push_grey(&mut self,x: GCValue<dyn Collectable>) {
        self.grey.push(x);
    }

    fn in_current_space(&self,value: &InGC<dyn Collectable>) -> bool {
        return (self.gc_type == GCType::OldSpace && value.gen > 5)
        || (self.gc_type == GCType::NewSpace && value.gen < 5)
    }

    /// Get space from where we copy objects
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

    pub fn collect(&mut self) {
        let start_time = time::PreciseTime::now();
        let to_space = self.to_space();
        let from_space = self.from_space();
        let old_size = self.alloc.top().offset_from(from_space.start);
        let top = to_space.start;
        let scan = top;
        self.tmp_space = top;
            for i in 0..self.roots.len() {
            let root = self.roots[i];
            self.grey.push(root);
            self.process_grey();
        }
        let top = self.tmp_space;
        self.tmp_space = scan;
        let mut i = 0;
        while scan < top {
            let value = self.allocated[i];
            let object = value.ptr;
            unsafe {
                self.grey.push(value);
                self.process_grey();
                let real_size = std::mem::size_of_val(&*object);
                self.tmp_space = self.tmp_space.offset(real_size);
            }


            i = i + 1;
        }
        for item in self.black.iter() {
            let value = item;
            unsafe {
                (*value.ptr).reset_soft_mark();
            }
        }
        self.black.clear();

        self.tmp_space = Address::null();
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
                ((*val_.ptr).mark) = 0;
                ((*val_.ptr).gen) = 0;
                ((*val_.ptr).marked) = false;
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
            ((*val_.ptr).fwd) = Address::null();
            ((*val_.ptr).mark) = 0;
            ((*val_.ptr).gen) = 0;
            ((*val_.ptr).marked) = false;
        }
        self.allocated.push(val_);
        return val_;
    }


    fn process_grey(&mut self) {
        while !self.grey.is_empty() {
            let value = self.grey.remove(0);
            let inner: &mut InGC<dyn Collectable> = unsafe {&mut *value.ptr};
            if !inner.marked {
                if !self.in_current_space(inner) {
                    if !inner.is_soft_marked() {
                        inner.set_soft_mark();
                        self.black.push(value);
                        unsafe {
                            inner.ptr.borrow().visit(self);
                        }

                        continue;
                    }
                }

                assert!(!inner.is_soft_marked());
                let new_addr = self.tmp_space;
                let mut space = self.tmp_space;
                self.copy(inner as *mut _,&mut space);
                self.tmp_space = space;
                inner.fwd = new_addr;
                if inner.is_marked() {
                    inner.set_mark(new_addr);
                }
            } else {
                inner.fwd = inner.get_mark();
                inner.set_mark(inner.fwd);
            }
        }
    }

    fn copy(&self, obj: *mut InGC<dyn Collectable>, top: &mut Address) -> Address {
        let obj: *mut InGC<dyn Collectable> = obj;
        unsafe {
            let size = std::mem::size_of_val(&*obj);
            // copy object to new space
            let addr = *top;
            (*obj).copy_to(addr, size);
            (*obj).increment_gen();
            // move pointer
            *top = top.offset(size);

            addr
        }
    }
}

use parking_lot::Mutex;

lazy_static::lazy_static!(
    pub static ref GC: Mutex<GenerationalGC> = Mutex::new(GenerationalGC::new(Option::None));
);

unsafe impl Send for GenerationalGC {}
unsafe impl Sync for GenerationalGC {}

pub fn gc_collect() {
    std::thread::spawn(|| {
        GC.lock().collect();
    });
}

pub fn gc_collect_not_par() {
    GC.lock().collect();
}

pub fn gc_allocate_sync<T: Collectable + Sized + 'static + Send>(val: T) -> GCValue<T> {
    std::thread::spawn(move || GC.lock().allocate(val))
        .join()
        .unwrap()
}

pub fn gc_allocate<T: Collectable + Sized + 'static>(val: T) -> GCValue<T> {
    //GC.with(|x| {
    GC.lock().allocate(val)
}

pub fn gc_rmroot(val: GCValue<dyn Collectable>) {
    GC.lock().remove_root(val);
}

pub fn gc_enable_stats() {
    //GC.with(|x| {
    let mut lock = GC.lock();
    lock.stats = !lock.stats;
    drop(lock);
    //})
}



pub fn gc_add_root(obj: GCValue<dyn Collectable>) {
    GC.lock().add_root(obj);
}




macro_rules! collectable_for_simple_types {
    ($($t: tt),*) => {
      $(  impl Collectable for $t {
            fn visit(&self,_: &mut GenerationalGC) {

            }

        }
      )*
    };
}

collectable_for_simple_types! {
    u8,u16,u32,u64,u128,
    i8,i16,i32,i64,i128,
    bool,String
}

impl<T: Collectable> Collectable for Vec<T> {
    fn visit(&self,gc: &mut GenerationalGC)  {
       for x in self.iter() {
            x.visit(gc);
        }
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
