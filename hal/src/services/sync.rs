//! Small synchronization primitives for statically allocated drivers.

use core::{
    cell::UnsafeCell,
    mem::MaybeUninit,
    sync::atomic::{AtomicU8, Ordering},
};

/// A value initialized once and then shared for the remainder of the boot.
pub struct StaticCell<T> {
    state: AtomicU8,
    value: UnsafeCell<MaybeUninit<T>>,
}

unsafe impl<T: Sync> Sync for StaticCell<T> {}

impl<T> StaticCell<T> {
    pub const fn uninit() -> Self {
        Self {
            state: AtomicU8::new(0),
            value: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    pub fn get_or_init(&'static self, init: impl FnOnce() -> T) -> &'static T {
        match self
            .state
            .compare_exchange(0, 1, Ordering::Acquire, Ordering::Acquire)
        {
            Ok(_) => {
                // SAFETY: this thread exclusively transitioned the state from
                // uninitialized to initializing, so it is the sole writer.
                unsafe { (*self.value.get()).write(init()) };
                self.state.store(2, Ordering::Release);
            }
            Err(1) => {
                while self.state.load(Ordering::Acquire) == 1 {
                    core::hint::spin_loop();
                }
            }
            Err(2) => {}
            Err(_) => unreachable!(),
        }

        // SAFETY: state 2 is stored after the value is written. Acquire loads
        // above synchronize competing initializers before this reference forms.
        unsafe { (&*self.value.get()).assume_init_ref() }
    }

    pub fn get(&'static self) -> Option<&'static T> {
        if self.state.load(Ordering::Acquire) == 2 {
            // SAFETY: state 2 is published only after the value is initialized.
            Some(unsafe { (&*self.value.get()).assume_init_ref() })
        } else {
            None
        }
    }
}

/// Minimal spinlock
///
/// This is only intended for multi-votr use rn. interrupt support hasn't
/// been added yet, trying an interrupt in this couls lead to undefined behaviour
pub struct SpinLock<T> {
    locked: AtomicU8,
    value: UnsafeCell<T>,
}

unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(value: T) -> Self {
        Self {
            locked: AtomicU8::new(0),
            value: UnsafeCell::new(value),
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        while self
            .locked
            .compare_exchange_weak(0, 1, Ordering::Acquire, Ordering::Relaxed)
            .is_err() {
            while self.locked.load(Ordering::Relaxed) != 0 {
                core::hint::spin_loop();
            }
        }
        SpinLockGuard { lock:self }
    }
}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
}

impl<'a, T> core::ops::Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        // SAFETY: holding the gaurd means we hold
        // the lock, so we get access to the value
        unsafe { &*self.lock.value.get() }
    }
}

impl<'a, T> core::ops::DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.value.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(0, Ordering::Release);
    }
}