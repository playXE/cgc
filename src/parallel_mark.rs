use crate::pool::ThreadPool;

static POOL: ThreadPool = ThreadPool::with_defaults("marking", 0);

use crate::collector::*;
use crate::rooting::*;
use crate::trace::*;

pub fn start(rootset: &[RootHandle]) {
    let mut nrootset = vec![];
    for root in rootset.iter() {
        if unsafe { (*root.0).is_rooted() } {
            unsafe {
                let inner = &mut *(*root.0).inner();
                if !inner.is_marked_non_atomic() {
                    inner.mark_non_atomic();
                    nrootset.push(inner as *mut InnerPtr<dyn Trace>);
                }
            }
        }
    }
    let workers = num_cpus::get();
    POOL.set_threads(workers as _).unwrap();
}
