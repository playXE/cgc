use crate::heap::*;
use crate::mutator::*;
use std::sync::Arc;
pub fn block(thread: &MutatorThread) {
    let safepoint_id = HEAP.mutators.safepoint_id();
    assert_ne!(safepoint_id, 0);
    let state = thread.state();
    match state {
        ThreadState::Running | ThreadState::Parked => {
            thread.block(safepoint_id);
        }
        ThreadState::Blocked => panic!(
            "illegal thread state: thread #{:?} {:?}",
            std::thread::current().id(),
            state
        ),
    }

    let _mtx = HEAP.mutators.barrier.wait(safepoint_id);
    thread.unblock()
}

pub extern "C" fn safepoint() {
    let thread = THREAD.with(|th| th.borrow().clone());
    block(&thread);
}

fn resume_threads(_threads: &[Arc<MutatorThread>], safepoint_id: usize) {
    HEAP.mutators.barrier.resume(safepoint_id);
    HEAP.mutators.clear_safepoint_request();
}

fn stop_threads(threads: &[Arc<MutatorThread>]) -> usize {
    let thread_self = THREAD.with(|th| th.borrow().clone());
    let safepoint_id = HEAP.mutators.request_safepoint();
    HEAP.mutators.barrier.guard(safepoint_id);
    while !all_threads_blocked(&thread_self, threads, safepoint_id) {
        std::thread::yield_now();
    }
    safepoint_id
}

fn all_threads_blocked(
    thread_self: &Arc<MutatorThread>,
    threads: &[Arc<MutatorThread>],
    safepoint_id: usize,
) -> bool {
    let mut all_blocked = true;
    for thread in threads {
        if Arc::ptr_eq(thread, thread_self) {
            continue;
        }
        if !thread.in_safepoint(safepoint_id) {
            all_blocked = false;
        }
    }

    all_blocked
}

pub fn stop_the_world<F, R>(f: F) -> R
where
    F: FnOnce(&[Arc<MutatorThread>]) -> R,
{
    THREAD.with(|th| th.borrow().park());
    let threads = HEAP.mutators.threads.lock();
    if threads.len() == 1 {
        let ret = f(&*threads);
        THREAD.with(|th| th.borrow().unpark());
        return ret;
    }

    let safepoint_id = stop_threads(&*threads);
    let ret = f(&*threads);
    resume_threads(&*threads, safepoint_id);
    THREAD.with(|th| th.borrow().unpark());
    ret
}
