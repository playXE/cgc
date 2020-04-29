use super::heap::*;
use super::threads::*;
use std::sync::Arc;
pub fn block(thread: &MutatorThread) {
    let safepoint_id = HEAP.threads.safepoint_id();
    assert_ne!(safepoint_id, 0);
    let state = thread.state();

    match state {
        ThreadState::Running | ThreadState::Parked => {
            thread.block(safepoint_id);
        }
        ThreadState::Blocked => {
            panic!("illegal thread state: {:?}", state);
        }
    };

    let _mtx = HEAP.threads.barrier.wait(safepoint_id);
    thread.unblock();
}

fn resume_threads(_threads: &[Arc<MutatorThread>], safepoint_id: usize) {
    HEAP.threads.barrier.resume(safepoint_id);
    HEAP.threads.clear_safepoint_request();
}

fn all_threads_blocked(
    thread_self: &Arc<MutatorThread>,
    threads: &[Arc<MutatorThread>],
    safepoint_id: usize,
) -> bool {
    let mut all_blocked = true;

    for thread in threads {
        if Arc::ptr_eq(thread, thread_self) {
            assert!(thread.state().is_parked());
            continue;
        }

        if !thread.in_safepoint(safepoint_id) {
            all_blocked = false;
        }
    }

    all_blocked
}

fn stop_threads(threads: &[Arc<MutatorThread>]) -> usize {
    let thread_self = THREAD.with(|thread| thread.borrow().clone());
    let safepoint_id = HEAP.threads.request_safepoint();

    HEAP.threads.barrier.guard(safepoint_id);

    while !all_threads_blocked(&thread_self, threads, safepoint_id) {
        std::thread::yield_now();
    }

    safepoint_id
}
pub fn stop_the_world<F, R>(f: F) -> R
where
    F: FnOnce(&[Arc<MutatorThread>]) -> R,
{
    THREAD.with(|thread| thread.borrow().park());

    let threads = HEAP.threads.threads.lock();
    if threads.len() == 1 {
        let ret = f(&*threads);
        THREAD.with(|thread| thread.borrow().unpark());
        return ret;
    }

    let safepoint_id = stop_threads(&*threads);
    let ret = f(&*threads);
    resume_threads(&*threads, safepoint_id);
    THREAD.with(|thread| thread.borrow().unpark());
    ret
}

pub extern "C" fn gc_guard() {
    let thread = THREAD.with(|thread| thread.borrow().clone());
    block(&thread);
}

#[macro_export]
macro_rules! safepoint {
    () => {
        $crate::safepoint::gc_guard();
    };
}
