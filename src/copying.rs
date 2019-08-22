pub trait Collectable {
    /// Mark all GC values
    fn mark(&self) {}
    fn destructor(&mut self) {

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
    collected: bool,
    marked: bool,
    ptr: RefCell<T>,   
}

unsafe impl<T: Collectable + ?Sized + Send> Send for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for InGC<T> {}
unsafe impl<T: Collectable + ?Sized + Send> Send for GCValue<T> {}
unsafe impl<T: Collectable + ?Sized + Sync> Sync for GCValue<T> {}

impl<T: Collectable + ?Sized> InGC<T> {
    fn copy_to(&mut self, dest: Address, _size: usize) {
        unsafe {
            let size = std::mem::size_of_val(&self);
            std::ptr::copy_nonoverlapping(
                self as *const Self as *const u8,
                dest.to_mut_ptr::<u8>(),
                size,
            );
            //std::ptr::copy_nonoverlapping(self as *const Self  as *const *const Self as *mut *const u8, dest.to_mut_ptr::<u8>() as *mut *const u8, 8);
        }
    }
}

pub struct GCValue<T: Collectable + ?Sized> {
    ptr: *mut InGC<T>,  
}

impl<T: Collectable + ?Sized> GCValue<T> {
    fn fwd(&self) -> Address {
        unsafe { (*self.ptr).fwd }
    }
    #[inline]
    unsafe fn get_ptr(&self) -> *mut InGC<T> {
        self.ptr
    }
    pub fn collected(&self) -> bool {
        unsafe {
            (*self.ptr).collected
        }
    }

    /// Compare `self` pointer and `other` pointer
    /// ```rust
    /// use cgc::{gc_allocate,gc_collect_not_par};
    /// let a = gc_allocate(0);
    /// let b = a.clone();
    /// assert!(a.ref_equal(&b));
    /// gc_collect_not_par();
    /// ```
    #[inline]
    pub fn ref_equal(&self,other: &GCValue<T>) -> bool {
        unsafe {self.get_ptr() as *const u8 == other.get_ptr() as *const u8}
    }
    /// Borrow value as immutable reference.
    /// Function will panic if current value borrowed as mutable somewhere.
    /// ```rust
    /// use cgc::{gc_allocate,gc_collect_not_par};
    /// let val = gc_allocate(42);
    /// assert_eq!(*val.borrow(),42);
    /// gc_collect_not_par();
    /// ```
    #[inline]
    pub fn borrow(&self) -> Ref<'_, T> {
        unsafe { (*self.ptr).ptr.borrow() }
    }
    /// Borrow value as mutable,will panic if value already borrowed as mutable
    /// ```rust
    /// use cgc::{gc_allocate,gc_collect_not_par};
    /// let a = gc_allocate(0);
    /// *a.borrow_mut() = 42;
    /// assert_eq!(*a.borrow(),42);
    ///
    /// ```
    pub fn borrow_mut(&self) -> RefMut<'_,T> {
        unsafe {
            (*self.ptr).ptr.borrow_mut()
        }
    }

    pub fn set_marked(&self) {
        unsafe {
            (*self.ptr).marked = true;
        }
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
        for i in 0..self.roots.len() {
            let root = self.roots[i];
            let root_ptr: *mut InGC<dyn Collectable> = root.ptr;
            let ptr = unsafe { std::mem::transmute_copy::<*mut InGC<dyn Collectable>,Address>(&root_ptr) };
            if from_space.contains(ptr) {
                
                unsafe {
                    (*root_ptr).marked = true;
                    (*root_ptr).ptr.borrow().mark();
                    //*root.ptr = std::mem::transmute_copy(&self.copy(ptr2, &mut top));
                    let new_pointer = self.copy(root_ptr,&mut top);
                    self.roots[i].ptr = std::mem::transmute_copy::<Address,*mut InGC<dyn Collectable>>(&new_pointer);
                    /*for child in (*root_ptr).ptr.borrow().child().iter() {
                        (*child.ptr).marked = true;
                    }*/
                }
            }
        }
        for i in 0..self.allocated.len() {
            unsafe {
                let object_ = self.allocated[i];
                let object = object_.ptr;
                if (*object).marked {
                    //*let new_addr = */std::mem::transmute_copy(&self.copy(object,&mut top));
                    let new_pointer = self.copy(object,&mut top);
                    self.allocated[i].ptr = std::mem::transmute_copy::<_,*mut InGC<dyn Collectable>>(&new_pointer);
                    //self.allocated[i].ptr = new_addr;
                }
                let object_ = self.allocated[i];
                let object = object_.ptr;
                let real_size = std::mem::size_of_val(&*object);
                

                scan = scan.offset(real_size);
            }
        }
        let mut retained_pointers = vec![];
        self.allocated.retain(|x| {
            
            unsafe {
                if (*x.ptr).marked == false {
                    //(*x.ptr).collected = true;
                    x.borrow_mut().destructor();
                    retained_pointers.push(*x);
                }
            }
            unsafe {(*x.ptr).marked}
        });
        for p in retained_pointers.iter() {
            self.remove_root(*p);
        }
        for i in 0..self.allocated.len() {
            unsafe {
                (&mut(*self.allocated[i].ptr).fwd as *mut Address).write(Address::null());
                (&mut(*self.allocated[i].ptr).marked as *mut bool).write( false);
                
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
           // println!("Objects count before collection: {},after: {}",prev_count,self.allocated.len());
        }
    }

    fn copy(&self, obj: *mut InGC<dyn Collectable>, top: &mut Address) -> Address {
        let obj: *mut InGC<dyn Collectable> = obj;
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
            // set forward address if we will visit this object again
            (&mut(*obj).fwd as *mut Address).write(addr);

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
                (&mut(*val_.ptr).fwd as *mut Address).write(Address::null());
                (&mut(*val_.ptr).ptr as *mut RefCell<T>).write(RefCell::new(val));
                (&mut(*val_.ptr).marked as *mut bool).write( false);
                (&mut(*val_.ptr).collected as *mut bool).write(false);
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
            (&mut(*val_.ptr).fwd as *mut Address).write(Address::null());
            (&mut(*val_.ptr).ptr as *mut RefCell<T>).write(RefCell::new(val));
            (&mut(*val_.ptr).marked as *mut bool).write( false);
            (&mut(*val_.ptr).collected as *mut bool).write(false);
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



macro_rules! collectable_for_simple_types {
    ($($t: tt),*) => {
      $(  impl Collectable for $t {
            
        }
      )*
    };
}

collectable_for_simple_types! {
    u8,u16,u32,u64,u128,
    i8,i16,i32,i128,i64,
    bool,String
}

impl<T: Collectable> Collectable for Vec<T> {
    fn mark(&self) {
        
        for x in self.iter() {
            x.mark();
        }
    }
    fn destructor(&mut self) {
        println!("vec dtor");
    }
    
}

impl<T: Collectable> Collectable for GCValue<T> {
    fn mark(&self) {
        self.set_marked();
        self.borrow().mark();
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

use std::hash::{Hash,Hasher};
impl<T: Hash + Collectable> Hash for GCValue<T> {
    fn hash<H: Hasher>(&self,h: &mut H) {
        self.borrow().hash(h);
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
