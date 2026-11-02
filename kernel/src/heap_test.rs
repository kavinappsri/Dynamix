//! Boot-time exercise of the kernel heap.

use alloc::alloc::{Layout, alloc, dealloc};
use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use hal::services::heap::KernelHeap;
use hal::services::pmm;

pub fn run(heap: &KernelHeap) {
    // Start from a trimmed heap so the leak check at the end compares like with like.
    heap.trim();
    let frames_before = pmm::stats().expect("PMM not ready").0;
    let stats_before = heap.stats();

    // A single small object.
    {
        let boxed = Box::new(0xDEAD_BEEF_u64);
        assert_eq!(*boxed, 0xDEAD_BEEF, "heap: Box round trip");
    }

    // A Vec that grows from the smallest size class through the large tier.
    {
        let mut values: Vec<u32> = Vec::new();
        for i in 0..100_000u32 {
            values.push(i.wrapping_mul(2_654_435_761));
        }
        for (i, value) in values.iter().enumerate() {
            assert_eq!(*value, (i as u32).wrapping_mul(2_654_435_761), "heap: Vec contents corrupted at {i}");
        }
    }

    // A block bigger than the PMM's old 4 MiB limit, freed again straight away.
    {
        let big = alloc::vec![0xA5u8; 5 << 20];
        assert!(big.iter().all(|&b| b == 0xA5), "heap: 5 MiB block contents");
    }

    // Strings and formatting.
    {
        let mut text = String::new();
        for i in 0..500 {
            text.push_str(&format!("{i},"));
        }
        assert!(text.starts_with("0,1,2,") && text.ends_with("499,"), "heap: String contents");
    }

    // Many small nodes, partly freed again.
    {
        let mut map = BTreeMap::new();
        for i in 0..2000u32 {
            map.insert(i, format!("value {i}"));
        }
        for i in (0..2000u32).step_by(2) {
            map.remove(&i);
        }
        assert_eq!(map.len(), 1000, "heap: BTreeMap length");
        assert_eq!(map.get(&1999).map(String::as_str), Some("value 1999"), "heap: BTreeMap lookup");
    }

    // Over-aligned requests, small and page sized.
    for align in [16usize, 64, 256, 2048, 4096] {
        let layout = Layout::from_size_align(100, align).unwrap();
        // SAFETY: the layout has a non-zero size and the block is freed with it.
        unsafe {
            let ptr = alloc(layout);
            assert!(!ptr.is_null(), "heap: alloc with align {align} failed");
            assert_eq!(ptr as usize % align, 0, "heap: misaligned block for align {align}");
            dealloc(ptr, layout);
        }
    }

    // Everything was freed, so after a trim the PMM must have all its frames back.
    heap.trim();
    let stats_after = heap.stats();
    assert_eq!(stats_after.small_live_bytes, stats_before.small_live_bytes, "heap: small blocks leaked");
    assert_eq!(stats_after.large_frames, stats_before.large_frames, "heap: large blocks leaked");
    assert_eq!(pmm::stats().expect("PMM not ready").0, frames_before, "heap: frames leaked");
}
