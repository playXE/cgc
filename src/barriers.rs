use parking_lot::{Condvar, Mutex};

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
