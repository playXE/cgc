pub unsafe trait Trace: Finalizer {
    fn mark(&mut self);
    fn unmark(&mut self);
    fn fields(&mut self) -> Vec<&mut dyn HeapTrait>;
}

/// The Finalizer trait. Can be specialized for a specific type to define finalization logic for that type.
pub trait Finalizer {
    fn finalize(&mut self) {
        // noop
    }
}

use crate::mem::Address;

pub trait HeapTrait {
    fn mark(&mut self);
    fn unmark(&mut self);
    fn slot(&mut self) -> Address;
    fn size(&self) -> usize;
    fn get_fwd(&self) -> Address;
    fn set_fwd(&self, _: Address);
    fn copy_to(&self, _: Address);
    fn addr(&self) -> Address;
    fn inner(&self) -> *mut crate::collector::InnerPtr<dyn Trace> {
        unimplemented!()
    }

    fn color(&self) -> crate::collector::GcColor;
    fn set_color(&self, _: crate::collector::GcColor);
}
/// The Traceable trait, which needs to be implemented on garbage-collected objects.
pub trait Traceable
where
    Self: Finalizer,
{
    fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

unsafe impl<T: Traceable> Trace for T {
    fn mark(&mut self) {
        self.trace_with(|field| field.mark());
    }

    fn unmark(&mut self) {
        self.trace_with(|field| field.unmark())
    }

    fn fields(&mut self) -> Vec<&mut dyn HeapTrait> {
        let mut ret = vec![];
        self.trace_with(|field| {
            ret.push(field);
        });
        ret
    }
}

macro_rules! simple {
    ($($t: ty)*) => {
        $(
            impl Traceable for $t {}
            impl Finalizer for $t {}
        )*
    };
}

simple!(
    i8
    i16
    i32
    i64
    i128
    u8
    u16
    u32
    u64
    u128
    f64
    f32
    bool
    String
    isize
    usize
    std::fs::File
    std::fs::FileType
    std::fs::Metadata
    std::fs::OpenOptions
    std::io::Stdin
    std::io::Stdout
    std::io::Stderr
    std::io::Error
    std::net::TcpStream
    std::net::TcpListener
    std::net::UdpSocket
    std::net::Ipv4Addr
    std::net::Ipv6Addr
    std::net::SocketAddrV4
    std::net::SocketAddrV6
    std::path::Path
    std::path::PathBuf
    std::process::Command
    std::process::Child
    std::process::ChildStdout
    std::process::ChildStdin
    std::process::ChildStderr
    std::process::Output
    std::process::ExitStatus
    std::process::Stdio
    std::sync::Barrier
    std::sync::Condvar
    std::sync::Once
    std::ffi::CStr
    std::ffi::CString
    &'static str
);

impl<T: Trace> Traceable for Option<T>
where
    T: HeapTrait,
{
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        match self {
            Some(value) => f(value),
            _ => (),
        }
    }
}

impl<T: Trace> Traceable for Option<T> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace + Sized> Finalizer for Option<T> {
    fn finalize(&mut self) {
        match self {
            Some(value) => value.finalize(),
            _ => (),
        }
    }
}

impl<T: Trace + HeapTrait> Traceable for Vec<T> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for x in self.iter_mut() {
            f(x);
        }
    }
}

impl<T> Traceable for Vec<T>
where
    T: Trace,
{
    default fn trace_with<'a>(&mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace> Finalizer for Vec<T> {
    fn finalize(&mut self) {
        for x in self.iter_mut() {
            x.finalize();
        }
        unsafe {
            std::alloc::dealloc(
                self.as_mut_ptr() as *mut u8,
                std::alloc::Layout::for_value(self),
            );
        }
    }
}

use std::collections::*;

impl<T: Trace, U: Trace> Traceable for HashMap<T, U> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace, U: Trace + HeapTrait> Traceable for HashMap<T, U> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for (_, val) in self.iter_mut() {
            f(val);
        }
    }
}

impl<T: Trace, U: Trace> Finalizer for HashMap<T, U> {
    fn finalize(&mut self) {
        for (_key, val) in self.iter_mut() {
            //key.finalize();
            val.finalize();
        }
    }
}

impl<T: Trace> Finalizer for VecDeque<T> {
    fn finalize(&mut self) {
        for x in self.iter_mut() {
            x.finalize();
        }
    }
}

impl<T: Trace> Traceable for VecDeque<T> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace + HeapTrait> Traceable for VecDeque<T> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for x in self.iter_mut() {
            f(x);
        }
    }
}

impl<T: Trace> Finalizer for LinkedList<T> {
    fn finalize(&mut self) {
        for x in self.iter_mut() {
            x.finalize();
        }
    }
}

impl<T: Trace> Traceable for LinkedList<T> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace + HeapTrait> Traceable for LinkedList<T> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for x in self.iter_mut() {
            f(x);
        }
    }
}

impl<T: Trace> Finalizer for HashSet<T> {
    fn finalize(&mut self) {
        for x in self.iter() {
            let x = unsafe { &mut *(x as *const T as *mut T) };
            x.finalize();
        }
    }
}

impl<T: Trace> Traceable for HashSet<T> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace + HeapTrait> Traceable for HashSet<T> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for x in self.iter() {
            unsafe { f(&mut *(x as *const T as *mut T)) };
        }
    }
}

impl<T: Trace> Finalizer for BTreeSet<T> {
    fn finalize(&mut self) {
        for x in self.iter() {
            let x = unsafe { &mut *(x as *const T as *mut T) };
            x.finalize();
        }
    }
}

impl<T: Trace> Traceable for BTreeSet<T> {
    default fn trace_with<'a>(&'a mut self, _: impl FnMut(&'a mut dyn HeapTrait)) {}
}

impl<T: Trace + HeapTrait> Traceable for BTreeSet<T> {
    default fn trace_with<'a>(&'a mut self, mut f: impl FnMut(&'a mut dyn HeapTrait)) {
        for x in self.iter() {
            unsafe { f(&mut *(x as *const T as *mut T)) };
        }
    }
}
