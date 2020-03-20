use crate::mem::Address;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
pub struct InnerGc<T: ?Sized> {
    pub color: AtomicU8,
    pub fwdptr: AtomicUsize,
    pub value: T,
}

#[derive(Eq, PartialEq, Copy, Clone, Debug)]
#[repr(u8)]
pub enum GcColor {
    WhiteA,
    WhiteB,
    Grey,
    Black,
}

const MARK_BITS: usize = 2;
const MARK_MASK: usize = (2 << MARK_BITS) - 1;
const FWD_MASK: usize = !0 & !MARK_MASK;

impl<T: ?Sized> InnerGc<T> {
    pub fn block_header<'a>(&self) -> &'a mut crate::block::Block {
        let addr = (self as *const Self as *const u8 as isize
            & crate::block::BLOCK_BYTEMAP_MASK as isize) as usize;
        unsafe {
            let ptr = addr as *mut crate::block::Block;
            &mut *ptr
        }
    }

    #[inline(always)]
    pub fn color(&self) -> GcColor {
        unsafe { std::mem::transmute(self.color.load(Ordering::Relaxed)) }
    }
    #[inline(always)]
    pub fn set_color(&self, color: GcColor) {
        self.color.store(color as u8, Ordering::Relaxed);
    }

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
pub unsafe trait Trace {
    fn mark(&mut self);
    fn unmark(&mut self);
    fn fields(&mut self) -> Tracer;
}

pub trait HeapTrait {
    fn mark(&self);
    fn unmark(&self);
    fn slot(&self) -> Address;
    fn size(&self) -> usize;
    fn get_fwd(&self) -> Address;
    fn set_fwd(&self, _: Address);
    fn copy_to(&self, _: Address);
    fn addr(&self) -> Address;
    fn inner(&self) -> *mut crate::gc::InnerGc<dyn Trace> {
        unimplemented!()
    }

    fn color(&self) -> crate::gc::GcColor;
    fn set_color(&self, _: crate::gc::GcColor);
}
/// The Traceable trait, which needs to be implemented on garbage-collected objects.
pub trait Traceable {
    fn trace_with<'a>(&'a mut self, _: &mut Tracer<'a>) {}
}

unsafe impl<T: Traceable> Trace for T {
    fn mark(&mut self) {
        let mut tracer = Default::default();
        self.trace_with(&mut tracer);
        tracer.mark();
    }

    fn unmark(&mut self) {
        let mut tracer = Default::default();
        self.trace_with(&mut tracer);
        tracer.unmark();
    }

    fn fields(&mut self) -> Tracer {
        let mut tracer = Default::default();
        self.trace_with(&mut tracer);
        tracer
    }
}

macro_rules! simple {
    ($($t: ty)*) => {
        $(
            impl Traceable for $t {}
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

#[derive(Default)]
pub struct Tracer<'a> {
    stack: Vec<&'a mut dyn HeapTrait>,
}

impl<'a> Tracer<'a> {
    pub fn trace(&mut self, elem: &'a mut dyn HeapTrait) {
        self.stack.push(elem);
    }

    pub fn mark(&self) {
        self.stack.iter().for_each(|item| item.mark());
    }

    pub fn unmark(&self) {
        self.stack.iter().for_each(|item| item.unmark());
    }
    pub fn stack(&self) -> &[&'a mut dyn HeapTrait] {
        &self.stack
    }
}
