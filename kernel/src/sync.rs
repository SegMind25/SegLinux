//! Locks.
//!
//! [`SpinLock`] disables interrupts while held, so code that runs in an
//! interrupt handler (the timer calls the scheduler, which allocates) can
//! never spin on a lock its own CPU already holds. [`Mutex`] puts the
//! waiting thread to sleep instead of spinning; it needs the scheduler.

use crate::task::wait::WaitQueue;
use core::cell::UnsafeCell;
use core::mem::ManuallyDrop;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::instructions::interrupts;

pub struct SpinLock<T> {
    inner: spin::Mutex<T>,
}

pub struct SpinGuard<'a, T> {
    guard: ManuallyDrop<spin::MutexGuard<'a, T>>,
    irq_enabled: bool,
}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            inner: spin::Mutex::new(value),
        }
    }

    pub fn lock(&self) -> SpinGuard<'_, T> {
        let irq_enabled = interrupts::are_enabled();
        interrupts::disable();
        SpinGuard {
            guard: ManuallyDrop::new(self.inner.lock()),
            irq_enabled,
        }
    }
}

impl<T> Drop for SpinGuard<'_, T> {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.guard) };
        if self.irq_enabled {
            interrupts::enable();
        }
    }
}

impl<T> Deref for SpinGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.guard
    }
}

impl<T> DerefMut for SpinGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.guard
    }
}

/// A sleeping lock for kernel threads.
pub struct Mutex<T> {
    locked: AtomicBool,
    waiters: WaitQueue,
    data: UnsafeCell<T>,
}

unsafe impl<T: Send> Send for Mutex<T> {}
unsafe impl<T: Send> Sync for Mutex<T> {}

pub struct MutexGuard<'a, T> {
    mutex: &'a Mutex<T>,
}

impl<T> Mutex<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
            data: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, T> {
        loop {
            if self
                .locked
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
                .is_ok()
            {
                return MutexGuard { mutex: self };
            }
            self.waiters
                .wait_until(|| !self.locked.load(Ordering::Relaxed));
        }
    }
}

impl<T> Drop for MutexGuard<'_, T> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        self.mutex.waiters.wake_one();
    }
}

impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }
    }
}

impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }
    }
}
