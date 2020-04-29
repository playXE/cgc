use parking_lot::{Condvar, Mutex};
use std::sync::{atomic::AtomicUsize, atomic::Ordering, Arc};
pub struct Barrier {
    active: Mutex<usize>,
    done: Condvar,
}

impl Barrier {
    pub fn new() -> Barrier {
        Barrier {
            active: Mutex::new(0),
            done: Condvar::new(),
        }
    }

    pub fn guard(&self, safepoint_id: usize) {
        let mut active = self.active.lock();
        assert_eq!(*active, 0);
        assert_ne!(safepoint_id, 0);
        *active = safepoint_id;
    }

    pub fn resume(&self, safepoint_id: usize) {
        let mut active = self.active.lock();
        assert_eq!(*active, safepoint_id);
        assert_ne!(safepoint_id, 0);
        *active = 0;
        self.done.notify_all();
    }

    pub fn wait(&self, safepoint_id: usize) {
        let mut active = self.active.lock();
        assert_ne!(safepoint_id, 0);

        while *active == safepoint_id {
            self.done.wait(&mut active);
        }
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

pub struct MutatorThread {
    pub state: StateManager,
    pub rootset: std::cell::RefCell<Vec<*mut dyn super::api::RootedTrait>>,
}

impl MutatorThread {
    pub fn new() -> Self {
        Self {
            state: StateManager::new(),
            rootset: std::cell::RefCell::new(vec![]),
        }
    }
    pub fn state(&self) -> ThreadState {
        self.state.state()
    }

    pub fn park(&self) {
        self.state.park();
    }

    pub fn unpark(&self) {
        if super::heap::HEAP.threads.safepoint_id() != 0 {
            crate::safepoint::block(self);
        }

        self.state.unpark();
    }

    pub fn block(&self, safepoint_id: usize) {
        self.state.block(safepoint_id);
    }

    pub fn unblock(&self) {
        self.state.unblock();
    }

    pub fn in_safepoint(&self, safepoint_id: usize) -> bool {
        self.state.in_safepoint(safepoint_id)
    }
}

unsafe impl Send for MutatorThread {}
unsafe impl Sync for MutatorThread {}
pub struct Threads {
    pub threads: Mutex<Vec<Arc<MutatorThread>>>,
    pub cond_join: Condvar,

    pub next_id: AtomicUsize,
    pub safepoint: Mutex<(usize, usize)>,

    pub barrier: Barrier,
}

impl Threads {
    pub fn new() -> Threads {
        Threads {
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

thread_local! {
    pub static THREAD: std::cell::RefCell<Arc<MutatorThread>> = std::cell::RefCell::new(Arc::new(MutatorThread::new()));
}
