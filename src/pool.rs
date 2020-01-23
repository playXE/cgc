use std::{io, thread};

use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicUsize, Ordering};
use core::{fmt, ptr, time};

#[derive(Debug)]
///Describes possible reasons for join to fail
pub enum JoinError {
    ///Job wasn't finished and aborted.
    Aborted,
    ///Timeout expired, job continues.
    Timeout,
}

impl Into<JoinError> for crossbeam_channel::RecvTimeoutError {
    fn into(self) -> JoinError {
        match self {
            crossbeam_channel::RecvTimeoutError::Timeout => JoinError::Timeout,
            crossbeam_channel::RecvTimeoutError::Disconnected => JoinError::Aborted,
        }
    }
}

///Handle to the job, allowing to await for it to finish
pub struct JobHandle<T> {
    inner: crossbeam_channel::Receiver<T>,
}

impl<T> fmt::Debug for JobHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "JobHandle")
    }
}

impl<T> JobHandle<T> {
    #[inline]
    ///Awaits for job to finish indefinitely.
    pub fn wait(self) -> Result<T, JoinError> {
        self.inner.recv().map_err(|_| JoinError::Aborted)
    }

    #[inline]
    ///Awaits for job to finish for limited time.
    pub fn wait_timeout(&self, timeout: time::Duration) -> Result<T, JoinError> {
        self.inner.recv_timeout(timeout).map_err(|err| err.into())
    }
}

enum Message {
    Execute(Box<dyn FnOnce() + Send + 'static>),
    Shutdown,
}

struct State {
    send: crossbeam_channel::Sender<Message>,
    recv: crossbeam_channel::Receiver<Message>,
    //Use lock to serialize changes to threads
    thread_num: parking_lot::RwLock<u16>,
}

///Thread pool that allows to change number of threads at runtime.
///
///On `Drop` it instructs threads to shutdown, but doesn't await for them to finish
///
///# Note
///
///The pool doesn't implement any sort of flow control.
///If workers are busy, message will remain in queue until any other thread can take it.
///
///# Clone
///
///Thread pool intentionally doesn't implement `Clone`
///If you want to share it, then share it by using global variable.
///
///# Panic
///
///Each thread wraps execution of job into `catch_unwind` to ensure that thread is not aborted
///on panic
pub struct ThreadPool {
    stack_size: AtomicUsize,
    name: &'static str,
    init_lock: parking_lot::Once,
    state: MaybeUninit<State>,
}

impl ThreadPool {
    ///Creates new thread pool with default params
    pub const fn new() -> Self {
        Self::with_defaults("", 0)
    }

    ///Creates new instance by specifying all params
    pub const fn with_defaults(name: &'static str, stack_size: usize) -> Self {
        Self {
            stack_size: AtomicUsize::new(stack_size),
            name,
            init_lock: parking_lot::Once::new(),
            state: MaybeUninit::uninit(),
        }
    }

    fn get_state(&self) -> &State {
        self.init_lock.call_once(|| {
            let (send, recv) = crossbeam_channel::unbounded();
            unsafe {
                ptr::write(
                    self.state.as_ptr() as *mut State,
                    State {
                        send,
                        recv,
                        thread_num: parking_lot::RwLock::new(0),
                    },
                );
            }
        });

        unsafe { &*self.state.as_ptr() }
    }

    #[inline]
    ///Sets stack size to use.
    ///
    ///By default it uses default value, used by Rust's stdlib.
    ///But setting this variable overrides it, allowing to customize it.
    ///
    ///This setting takes effect only when creating new threads
    pub fn set_stack_size(&self, stack_size: usize) -> usize {
        self.stack_size.swap(stack_size, Ordering::AcqRel)
    }

    ///Sets worker number, starting new threads if it is greater than previous
    ///
    ///In case if it is less, extra threads are shut down.
    ///Returns previous number of threads.
    ///
    ///By default when pool is created no threads are started.
    ///
    ///If any thread fails to start, function returns immediately with error.
    ///
    ///# Note
    ///
    ///Any calls to this method are serialized, which means under hood it locks out
    ///any attempt to change number of threads, until it is done
    pub fn set_threads(&self, thread_num: u16) -> io::Result<u16> {
        let state = self.get_state();

        let mut state_thread_num = state.thread_num.write();
        let old_thread_num = *state_thread_num;
        *state_thread_num = thread_num;

        if old_thread_num > thread_num {
            let shutdown_num = old_thread_num - thread_num;
            for _ in 0..shutdown_num {
                if state.send.send(Message::Shutdown).is_err() {
                    break;
                }
            }
        } else if thread_num > old_thread_num {
            let create_num = thread_num - old_thread_num;
            let stack_size = self.stack_size.load(Ordering::Acquire);
            let state = self.get_state();

            for num in 0..create_num {
                let recv = state.recv.clone();

                let builder = match self.name {
                    "" => thread::Builder::new(),
                    name => thread::Builder::new().name(name.to_owned()),
                };

                let builder = match stack_size {
                    0 => builder,
                    stack_size => builder.stack_size(stack_size),
                };

                let result = builder.spawn(move || loop {
                    match recv.recv() {
                        Ok(Message::Execute(job)) => {
                            //TODO: for some reason closures has no impl, wonder why?
                            let job = std::panic::AssertUnwindSafe(job);
                            let _ = std::panic::catch_unwind(|| (job.0)());
                        }
                        Ok(Message::Shutdown) | Err(_) => break,
                    }
                });

                match result {
                    Ok(_) => (),
                    Err(error) => {
                        *state_thread_num = old_thread_num + num;
                        return Err(error);
                    }
                }
            }
        }

        Ok(old_thread_num)
    }

    ///Schedules new execution, sending it over to one of the workers.
    pub fn spawn<F: FnOnce() + Send + 'static>(&self, job: F) {
        let state = self.get_state();
        let _ = state.send.send(Message::Execute(Box::new(job)));
    }

    ///Schedules execution, that allows to await and receive it's result.
    pub fn spawn_handle<R: Send + 'static, F: FnOnce() -> R + Send + 'static>(
        &self,
        job: F,
    ) -> JobHandle<R> {
        let (send, recv) = crossbeam_channel::bounded(1);
        let state = self.get_state();
        let job = move || {
            let _ = send.send(job());
        };
        let _ = state.send.send(Message::Execute(Box::new(job)));

        JobHandle { inner: recv }
    }
}

impl fmt::Debug for ThreadPool {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "ThreadPool {{ threads: {} }}",
            self.get_state().thread_num.read()
        )
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        unsafe {
            ptr::drop_in_place(self.state.as_mut_ptr());
        }
    }
}
