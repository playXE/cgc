use crate::block::*;
use crate::mem::Address;
use parking_lot::{Condvar, Mutex};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
pub struct MutatorThread {
    pub blocks: RefCell<Vec<Block>>,
    state: StateManager,
    rooted: RefCell<()>,
}

unsafe impl Sync for MutatorThread {}
unsafe impl Send for MutatorThread {}

impl MutatorThread {
    pub fn state(&self) -> ThreadState {
        self.state.state()
    }
    pub fn in_safepoint(&self, id: usize) -> bool {
        self.state.in_safepoint(id)
    }
    pub fn block(&self, safepoint_id: usize) {
        self.state.block(safepoint_id);
    }

    pub fn unblock(&self) {
        self.state.unblock();
    }
    pub fn park(&self) {
        self.state.park()
    }

    pub fn unpark(&self) {
        if crate::heap::HEAP.mutators.safepoint_id() != 0 {}

        self.state.unpark()
    }

    fn allocate_mem(&mut self, size: usize) -> Option<Address> {
        crate::safepoint::safepoint();
        let result = if let Some(mem) = self.try_allocate_from_blocks(size) {
            mem
        } else {
            // slow case
            // TODO: Request block from global state.
            unimplemented!()
        };

        Some(result)
    }

    fn try_allocate_from_blocks(&mut self, size: usize) -> Option<Address> {
        self.blocks
            .borrow_mut()
            .iter_mut()
            .find_map(|block| match block.state {
                BlockState::Free | BlockState::Usable => block.allocate(size),
                _ => None,
            })
    }
}

pub struct StateManager {
    mtx: Mutex<(ThreadState, usize)>,
}

impl StateManager {
    fn new() -> StateManager {
        StateManager {
            mtx: Mutex::new((ThreadState::Running, 0)),
        }
    }

    fn state(&self) -> ThreadState {
        let mtx = self.mtx.lock();
        mtx.0
    }

    fn park(&self) {
        let mut mtx = self.mtx.lock();
        assert!(mtx.0.is_running());
        mtx.0 = ThreadState::Parked;
    }

    fn unpark(&self) {
        let mut mtx = self.mtx.lock();
        assert!(mtx.0.is_parked());
        mtx.0 = ThreadState::Running;
    }

    fn block(&self, safepoint_id: usize) {
        let mut mtx = self.mtx.lock();
        assert!(mtx.0.is_running());
        mtx.0 = ThreadState::Blocked;
        mtx.1 = safepoint_id;
    }

    fn unblock(&self) {
        let mut mtx = self.mtx.lock();
        assert!(mtx.0.is_blocked());
        mtx.0 = ThreadState::Running;
        mtx.1 = 0;
    }

    fn in_safepoint(&self, safepoint_id: usize) -> bool {
        assert_ne!(safepoint_id, 0);
        let mtx = self.mtx.lock();

        match mtx.0 {
            ThreadState::Running => false,
            ThreadState::Blocked => mtx.1 == safepoint_id,
            ThreadState::Parked => true,
        }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum ThreadState {
    Running = 0,
    Parked = 1,
    Blocked = 2,
}

impl From<usize> for ThreadState {
    fn from(value: usize) -> ThreadState {
        match value {
            0 => ThreadState::Running,
            1 => ThreadState::Parked,
            2 => ThreadState::Blocked,
            _ => unreachable!(),
        }
    }
}

impl ThreadState {
    pub fn is_running(&self) -> bool {
        match *self {
            ThreadState::Running => true,
            _ => false,
        }
    }

    pub fn is_parked(&self) -> bool {
        match *self {
            ThreadState::Parked => true,
            _ => false,
        }
    }

    pub fn is_blocked(&self) -> bool {
        match *self {
            ThreadState::Blocked => true,
            _ => false,
        }
    }

    pub fn to_usize(&self) -> usize {
        *self as usize
    }
}

impl Default for ThreadState {
    fn default() -> ThreadState {
        ThreadState::Running
    }
}
use crate::barriers::Barrier;
thread_local! {
    pub static THREAD: RefCell<Arc<MutatorThread>> = RefCell::new(Arc::new(MutatorThread {
        blocks: RefCell::new(vec![]),
        state: StateManager::new(),
        rooted: RefCell::new(Default::default())
    }));
}

pub struct Mutators {
    pub barrier: Barrier,
    pub next_id: AtomicUsize,
    pub safepoint: Mutex<(usize, usize)>,
    pub threads: Mutex<Vec<Arc<MutatorThread>>>,
    pub cond_join: Condvar,
}

impl Mutators {
    pub fn new() -> Self {
        Self {
            threads: Mutex::new(Vec::new()),
            cond_join: Condvar::new(),
            next_id: AtomicUsize::new(1),
            safepoint: Mutex::new((0, 1)),
            barrier: Barrier::new(),
        }
    }
    pub fn attach_current_thread(&self) {
        THREAD.with(|thread| {
            let mut threads = self.threads.lock();
            threads.push(thread.borrow().clone());
        });
    }

    pub fn attach_thread(&self, thread: Arc<MutatorThread>) {
        let mut threads = self.threads.lock();
        threads.push(thread);
    }
    pub fn next_id(&self) -> usize {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn safepoint_id(&self) -> usize {
        let safepoint = self.safepoint.lock();
        safepoint.0
    }

    pub fn safepoint_requested(&self) -> bool {
        let safepoint = self.safepoint.lock();
        safepoint.0 != 0
    }

    pub fn request_safepoint(&self) -> usize {
        let mut safepoint = self.safepoint.lock();
        assert_eq!(safepoint.0, 0);
        safepoint.0 = safepoint.1;
        safepoint.1 += 1;

        safepoint.0
    }

    pub fn clear_safepoint_request(&self) {
        let mut safepoint = self.safepoint.lock();
        assert_ne!(safepoint.0, 0);
        safepoint.0 = 0;
    }

    pub fn detach_current_thread(&self) {
        THREAD.with(|thread| {
            thread.borrow().park();
            let mut threads = self.threads.lock();
            threads.retain(|elem| !Arc::ptr_eq(elem, &*thread.borrow()));
            self.cond_join.notify_all();
        });
    }
    pub fn join_all(&self) {
        let mut threads = self.threads.lock();

        while threads.len() > 0 {
            self.cond_join.wait(&mut threads);
        }
    }

    pub fn each<F>(&self, mut f: F)
    where
        F: FnMut(&Arc<MutatorThread>),
    {
        let threads = self.threads.lock();

        for thread in threads.iter() {
            f(thread)
        }
    }
}
