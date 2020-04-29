use crate::mem::*;
use std::sync::atomic::AtomicUsize;
pub struct HeapInner<T: super::api::Trace + ?Sized> {
    pub(crate) value: T,
}
impl<T: super::api::Trace + ?Sized> HeapInner<T> {
    pub fn mark(&self, _x: bool) {}
    pub fn fwdptr(&self) -> Address {
        Address::null()
    }
    pub fn set_fwdptr(&self, fwdptr: Address) {}
    pub fn is_marked(&self) -> bool {
        false
    }
}
