use crate::gc::InnerGc;
use std::sync::atomic::{AtomicBool, Ordering};
pub(crate) struct RootedInner<T: ?Sized> {
    pub rooted: AtomicBool,
    pub inner: *mut InnerGc<T>,
}
