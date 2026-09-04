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
