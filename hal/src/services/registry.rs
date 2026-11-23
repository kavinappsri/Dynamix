//! Append-only lists of things that live for the rest of the boot.
//!
//! Devices and the instances drivers create are never removed (there is no
//! unbinding yet), so a list is just nodes that are allocated once, leaked, and
//! linked in at the tail. Readers walk it without locks and without
//! allocating, which is what the panic and fault paths need from the console.
//! Adding needs the heap, so drivers can only register once it is up.

use alloc::boxed::Box;
use core::ptr;
use core::sync::atomic::{AtomicPtr, Ordering};

use crate::services::device::Device;

struct Node<T: 'static> {
    value: T,
    next: AtomicPtr<Node<T>>,
}

/// A list that only ever grows. Iteration order is insertion order.
pub struct AppendList<T: 'static> {
    head: AtomicPtr<Node<T>>,
}

impl<T: 'static> AppendList<T> {
    pub const fn new() -> Self {
        Self { head: AtomicPtr::new(ptr::null_mut()) }
    }

    /// Adds `value` at the end and returns a reference that stays valid forever.
    pub fn push(&self, value: T) -> &'static T {
        let node = Box::into_raw(Box::new(Node { value, next: AtomicPtr::new(ptr::null_mut()) }));

        let mut link = &self.head;
        loop {
            match link.compare_exchange(ptr::null_mut(), node, Ordering::AcqRel, Ordering::Acquire) {
                // SAFETY: `node` was leaked just above and is never freed.
                Ok(_) => return unsafe { &(*node).value },
                // SAFETY: a non-null link points at a leaked node.
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
        // SAFETY: non-null links point at leaked, fully initialized nodes.
        let node = unsafe { &*self.next };
        self.next = node.next.load(Ordering::Acquire);
        Some(&node.value)
    }
}

/// One instance a driver created for a device.
pub struct Entry<T: ?Sized + 'static> {
    pub device: &'static Device,
    pub handle: &'static T,
}

/// Every instance of one kind of driver (all the serial ports, all the
/// displays, ...), in the order they were bound.
pub struct Class<T: ?Sized + 'static> {
    list: AppendList<Entry<T>>,
    /// The instance firmware says to use, if it said.
    preferred: AtomicPtr<Entry<T>>,
}

impl<T: ?Sized + 'static> Class<T> {
    pub const fn new() -> Self {
        Self { list: AppendList::new(), preferred: AtomicPtr::new(ptr::null_mut()) }
    }

    pub fn register(&self, device: &'static Device, handle: &'static T) {
        self.list.push(Entry { device, handle });
    }

    pub fn iter(&self) -> Iter<Entry<T>> {
        self.list.iter()
    }

    /// The instance the kernel uses: the one firmware [preferred](Self::prefer)
    /// if it named one, else the first that was bound.
    pub fn primary(&self) -> Option<&'static T> {
        let preferred = self.preferred.load(Ordering::Acquire);
        if !preferred.is_null() {
            // SAFETY: only `prefer` stores here, and only a reference to a
            // list entry, which is never freed.
            return Some(unsafe { (*preferred).handle });
        }
        self.list.first().map(|entry| entry.handle)
    }

    /// Makes `entry`, which must come from this class, the primary instance.
    pub fn prefer(&self, entry: &'static Entry<T>) {
        self.preferred.store(ptr::from_ref(entry).cast_mut(), Ordering::Release);
    }
}

impl<T: ?Sized + 'static> Default for Class<T> {
    fn default() -> Self {
        Self::new()
    }
}
