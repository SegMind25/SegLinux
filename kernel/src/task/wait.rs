//! Wait queues: threads sleep here until another thread wakes them.

use super::{State, Task, sched};
use crate::sync::SpinLock;
use alloc::collections::VecDeque;
use alloc::sync::Arc;

pub struct WaitQueue {
    waiters: SpinLock<VecDeque<Arc<Task>>>,
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl WaitQueue {
    pub const fn new() -> Self {
        Self {
            waiters: SpinLock::new(VecDeque::new()),
        }
    }

    /// Sleeps until `cond` is true. `cond` is checked under the queue lock,
    /// and wakers take the same lock, so a wake-up cannot be lost between
    /// the check and going to sleep.
    pub fn wait_until(&self, mut cond: impl FnMut() -> bool) {
        loop {
            {
                let mut q = self.waiters.lock();
                if cond() {
                    return;
                }
                let me = sched::current();
                me.set_state(State::Blocked);
                q.push_back(me);
            }
            sched::schedule();
        }
    }

    pub fn wake_one(&self) {
        let mut q = self.waiters.lock();
        while let Some(t) = q.pop_front() {
            if wake(t) {
                return;
            }
        }
    }

    pub fn wake_all(&self) {
        let mut q = self.waiters.lock();
        while let Some(t) = q.pop_front() {
            wake(t);
        }
    }
}

fn wake(t: Arc<Task>) -> bool {
    if t.cas_state(State::Blocked, State::Ready) {
        sched::make_ready(t);
        true
    } else {
        false
    }
}
