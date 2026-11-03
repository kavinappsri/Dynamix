//! Global variables

use hal::services::heap::KernelHeap;

#[global_allocator]
pub static HEAP: KernelHeap = KernelHeap::new();