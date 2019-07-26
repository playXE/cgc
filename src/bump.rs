use super::Address;
use std::sync::atomic::{AtomicUsize, Ordering};

pub(crate) struct BumpAllocator {
    top: AtomicUsize,
    limit: AtomicUsize,
}

impl BumpAllocator {
    pub(crate) fn new(top: Address, limit: Address) -> BumpAllocator {
        BumpAllocator {
            top: AtomicUsize::new(top.to_usize()),
            limit: AtomicUsize::new(limit.to_usize()),
        }
    }

    pub(crate) fn reset(&self, top: Address, limit: Address) {
        self.top.store(top.to_usize(), Ordering::Relaxed);
        self.limit.store(limit.to_usize(), Ordering::Relaxed);
    }

    pub(crate) fn reset_limit(&self, limit: Address) {
        self.limit.store(limit.to_usize(), Ordering::Relaxed);
    }

    pub(crate) fn top(&self) -> Address {
        self.top.load(Ordering::Relaxed).into()
    }

    pub(crate) fn limit(&self) -> Address {
        self.limit.load(Ordering::Relaxed).into()
    }

    pub(crate) fn bump_alloc(&self, size: usize) -> Address {
        let mut old = self.top.load(Ordering::Relaxed);
        let mut new;

        loop {
            new = old + size;

            if new > self.limit.load(Ordering::Relaxed) {
                return Address::null();
            }

            let res = self
                .top
                .compare_exchange_weak(old, new, Ordering::SeqCst, Ordering::Relaxed);

            match res {
                Ok(_) => break,
                Err(x) => old = x,
            }
        }

        old.into()
    }
}
