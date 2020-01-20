use crate::collector::*;
use crate::mem::*;
use crate::trace::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Rooted<T: Trace + ?Sized> {
    pub(crate) inner: *mut RootedInner<T>,
}

impl<T: Trace + ?Sized> Drop for Rooted<T> {
    fn drop(&mut self) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.rooted.store(false, Ordering::Relaxed);
        }
    }
}
/*
impl<T: Trace + Sized + 'static> Traceable for Heap<T> {
    fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        f(self);
    }
}

impl<T: Trace> Finalizer for Heap<T> {
    fn finalize(&mut self) {
        self.get_mut().finalize();
    }
}*/

impl<T: Trace + ?Sized> Rooted<T> {
    /// Returns reference to rooted value
    pub fn get(&self) -> &T {
        unsafe {
            let inner = &*self.inner;
            let inner = &*inner.inner;
            &inner.value
        }
    }
    /// Returns mutable reference to rooted value
    ///
    /// # Safety
    /// Rust semantics doesn't allow two mutable references at the same time and this function is safe as long as you have only one mutable reference.
    ///
    /// If you want to be 100% sure that you don't have two or more mutable references at the same time please use `Rooted<RefCell<T>>`
    ///
    ///
    pub fn get_mut(&mut self) -> &mut T {
        unsafe {
            let inner = &*self.inner;
            let inner = &mut *inner.inner;
            &mut inner.value
        }
    }
    /// Get `Heap<T>` from `Rooted<T>`
    pub fn to_heap(&self) -> Heap<T> {
        unsafe {
            Heap {
                inner: (*self.inner).inner,
            }
        }
    }
}

pub(crate) struct RootedInner<T: Trace + ?Sized> {
    pub(crate) rooted: AtomicBool,
    pub(crate) inner: *mut InnerPtr<T>,
}

impl<T: Trace + Sized + 'static> HeapTrait for RootedInner<T> {
    fn mark(&mut self) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.mark.store(true, Ordering::Relaxed);
            inner.value.mark();
        }
    }
    fn addr(&self) -> Address {
        Address::from_ptr(self.inner as *const u8)
    }

    fn copy_to(&self, addr: Address) {
        debug_assert!(addr.is_non_null() && !self.inner.is_null());
        unsafe {
            std::ptr::copy(
                self.inner as *const u8,
                addr.to_mut_ptr(),
                std::mem::size_of_val(&*self.inner),
            )
        }
    }

    fn unmark(&mut self) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.mark.store(false, Ordering::Relaxed);
            inner.value.unmark();
        }
    }

    fn slot(&mut self) -> Address {
        debug_assert!(!self.inner.is_null());
        let slot = &mut self.inner;
        Address::from_ptr(slot)
    }

    fn set_fwd(&self, addr: Address) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.fwd = addr;
        }
    }

    fn get_fwd(&self) -> Address {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.fwd
        }
    }

    fn size(&self) -> usize {
        debug_assert!(!self.inner.is_null());
        unsafe { std::mem::size_of_val(&*self.inner) }
    }

    fn inner(&self) -> *mut InnerPtr<dyn Trace> {
        self.inner
    }
}

pub trait RootedTrait
where
    Self: HeapTrait,
{
    fn is_rooted(&self) -> bool;
    fn fields(&self) -> Vec<&mut dyn HeapTrait>;
}

impl<T: Trace + Sized + 'static> RootedTrait for RootedInner<T> {
    fn is_rooted(&self) -> bool {
        self.rooted.load(Ordering::Relaxed)
    }
    fn fields(&self) -> Vec<&mut dyn HeapTrait> {
        unsafe {
            let inner = &mut *self.inner;
            inner.value.fields()
        }
    }
}
/// Wraps GC heap pointer.
///
/// GC thing pointers on the heap must be wrapped in a `Heap<T>`
pub struct Heap<T: Trace + ?Sized> {
    inner: *mut InnerPtr<T>,
}

impl<T: Trace + ?Sized> From<Rooted<T>> for Heap<T> {
    fn from(x: Rooted<T>) -> Self {
        unsafe {
            Self {
                inner: (*x.inner).inner,
            }
        }
    }
}

impl<T: Trace + ?Sized> From<&Rooted<T>> for Heap<T> {
    fn from(x: &Rooted<T>) -> Self {
        unsafe {
            Self {
                inner: (*x.inner).inner,
            }
        }
    }
}

impl<T: Trace + ?Sized> Heap<T> {
    pub fn get(&self) -> &T {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &*self.inner;
            &inner.value
        }
    }

    /// Returns mutable reference to rooted value
    ///
    /// # Safety
    /// Rust semantics doesn't allow two mutable references at the same time and this function is safe as long as you have only one mutable reference.
    ///
    /// If you want to be 100% sure that you don't have two or more mutable references at the same time please use `Rooted<RefCell<T>>`
    ///
    ///
    pub fn get_mut(&mut self) -> &mut T {
        unsafe {
            let inner = &mut *self.inner;

            &mut inner.value
        }
    }
}

impl<T: Trace + Sized + 'static> HeapTrait for Heap<T> {
    fn addr(&self) -> Address {
        Address::from_ptr(self.inner as *const u8)
    }
    fn copy_to(&self, addr: Address) {
        debug_assert!(addr.is_non_null() && !self.inner.is_null());
        unsafe {
            std::ptr::copy(
                self.inner as *const u8,
                addr.to_mut_ptr(),
                std::mem::size_of_val(&*self.inner),
            )
        }
    }
    fn mark(&mut self) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.value.mark();
        }
    }

    fn unmark(&mut self) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.value.unmark();
        }
    }

    fn slot(&mut self) -> Address {
        debug_assert!(!self.inner.is_null());
        let slot = &mut self.inner;
        Address::from_ptr(slot)
    }

    fn set_fwd(&self, addr: Address) {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.fwd = addr;
        }
    }

    fn get_fwd(&self) -> Address {
        unsafe {
            debug_assert!(!self.inner.is_null());
            let inner = &mut *self.inner;
            inner.fwd
        }
    }

    fn size(&self) -> usize {
        debug_assert!(!self.inner.is_null());
        unsafe { std::mem::size_of_val(&*self.inner) }
    }

    fn inner(&self) -> *mut InnerPtr<dyn Trace> {
        self.inner
    }
}

impl<T: Trace> Copy for Heap<T> {}
impl<T: Trace> Clone for Heap<T> {
    fn clone(&self) -> Self {
        *self
    }
}

use std::cmp;

impl<T: Trace + PartialOrd> PartialOrd for Heap<T> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.get().partial_cmp(other.get())
    }
}

impl<T: Trace + Ord> Ord for Heap<T> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.get().cmp(other.get())
    }
}

impl<T: Trace + PartialEq> PartialEq for Heap<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get().eq(other.get())
    }
}

impl<T: Trace + Eq> Eq for Heap<T> {}

use std::hash::{Hash, Hasher};

impl<T: Trace + Hash> Hash for Heap<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

use std::fmt;

default impl<T: Trace + fmt::Display> fmt::Display for Heap<T> {
    default fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

default impl<T: Trace + fmt::Debug> fmt::Debug for Heap<T> {
    default fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.get())
    }
}

impl<T: Trace + PartialOrd> PartialOrd for Rooted<T> {
    fn partial_cmp(&self, other: &Self) -> Option<cmp::Ordering> {
        self.get().partial_cmp(other.get())
    }
}

impl<T: Trace + Ord> Ord for Rooted<T> {
    fn cmp(&self, other: &Self) -> cmp::Ordering {
        self.get().cmp(other.get())
    }
}

impl<T: Trace + PartialEq> PartialEq for Rooted<T> {
    fn eq(&self, other: &Self) -> bool {
        self.get().eq(other.get())
    }
}

impl<T: Trace + Eq> Eq for Rooted<T> {}

impl<T: Trace + Hash> Hash for Rooted<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get().hash(state);
    }
}

default impl<T: Trace + fmt::Display> fmt::Display for Rooted<T> {
    default fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

default impl<T: Trace + fmt::Debug> fmt::Debug for Rooted<T> {
    default fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self.get())
    }
}
