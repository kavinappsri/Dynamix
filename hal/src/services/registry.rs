//! A list of things needed for rest of kernel lifetime
//!
//! it's currently a place for device drivers and such, since there is no unloading of drivers yet.

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};
use crate::services::device::Device;

struct Node<T: 'static> {
    value: T,
    next: AtomicPtr<Node<T>>,
}

pub struct AppendList<T: 'static> {
    head: AtomicPtr<Node<T>>,
}

impl<T: 'static> AppendList<T> {
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Adds `value` to the end of the [`AppendList`], and return a reference
    pub fn push(&self, value: T) -> &'static T {
        let node = Box::into_raw(Box::new(Node {value, next: AtomicPtr::new(ptr::null_mut())} ));

        let mut link = &self.head;
        loop {
            // SAFETY: node was leaked above, and will never be freed.
            match link.compare_exchange(ptr::null_mut(), node, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return unsafe { &(*node).value },
                Err(existing) => link = unsafe { &(*existing).next },
            }
        }
    }

    pub fn iter(&self) -> Iter<T> {
        Iter { next: self.head.load(Ordering::Acquire) }
    }

    pub fn first(&self) -> Option<&'static T> {
        self.iter().next()
    }

    pub fn len(&self) -> usize {
        self.iter().count()
    }

    pub fn is_empty(&self) -> bool {
        self.head.load(Ordering::Acquire).is_null()
    }
}

impl<T: 'static> Default for AppendList<T> {
    fn default() -> Self {
        Self::new()
    }
}

pub struct Iter<T: 'static> {
    next: *mut Node<T>,
}

impl<T: 'static> Iterator for Iter<T> {
    type Item = &'static T;

    fn next(&mut self) -> Option<&'static T> {
        if self.next.is_null() {
            return None;
        }

        // SAFETY: next is not null, leaked value
        let node = unsafe { &(*self.next) };
        self.next = node.next.load(Ordering::Acquire);
        Some(&node.value)
    }
}

/// One instance a driver created for a device.
pub struct Entry<T: ?Sized + 'static> {
    pub device: &'static Device,
    pub handle: &'static T,
}

/// Every instance of one kind of driver
pub struct Class<T: ?Sized + 'static> {
    list: AppendList<Entry<T>>,
}

impl<T: ?Sized + 'static> Class<T> {
    pub const fn new() -> Self {
        Self { list: AppendList::new() }
    }

    pub fn register(&self, device: &'static Device, handle: &'static T) {
        self.list.push(Entry { device, handle });
    }

    pub fn iter(&self) -> Iter<Entry<T>> {
        self.list.iter()
    }

    /// The first instance that was bound: what the kernel uses unless it has
    /// a reason to pick another.
    pub fn primary(&self) -> Option<&'static T> {
        self.list.first().map(|entry| entry.handle)
    }
}

impl<T: ?Sized + 'static> Default for Class<T> {
    fn default() -> Self {
        Self::new()
    }
}
