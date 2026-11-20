//! Memory accounting for the validation tools.
//!
//! # Two independent measurements
//!
//! The tools report **two** memory figures, because neither alone tells the
//! truth about a process:
//!
//! * [`MemorySample::heap_peak_bytes`] — the bytes the process's own
//!   allocator had outstanding at the busiest moment. A tracking global
//!   allocator counts every allocation and deallocation, so this is the
//!   library's own working set, with no operating-system overhead in it.
//! * [`MemorySample::peak_working_set_bytes`] — the operating system's
//!   resident-set high-water mark for the process. It includes the binary
//!   itself, the stacks, and the allocator's own bookkeeping, and it never
//!   goes down within the life of a process.
//!
//! A report that quotes only one of them can mislead: the heap figure says
//! nothing about the process's overhead, and the working set says nothing
//! about which structure held the memory. Every report in this crate states
//! both, and states what each one includes and excludes.
//!
//! # The unsafe code
//!
//! The Windows measurement calls `K32GetProcessMemoryInfo` from
//! `kernel32.dll`. That is the only unsafe code in the crate, and it is
//! confined to this module.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

/// A global allocator that counts the bytes currently allocated.
pub struct Tracking;

static CURRENT_BYTES: AtomicIsize = AtomicIsize::new(0);
static PEAK_BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOC_COUNT: AtomicUsize = AtomicUsize::new(0);

/// Add (or subtract) bytes from the current total, never going below zero.
fn add_bytes(delta: isize) -> usize {
    let mut current = CURRENT_BYTES.load(Ordering::Relaxed);
    loop {
        let next = (current + delta).max(0) as usize;
        match CURRENT_BYTES.compare_exchange_weak(
            current,
            next as isize,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return next,
            Err(actual) => current = actual,
        }
    }
}

/// Record a new high-water mark for the heap total.
fn update_peak(current: usize) {
    let mut peak = PEAK_BYTES.load(Ordering::Relaxed);
    while current > peak {
        match PEAK_BYTES.compare_exchange_weak(peak, current, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(actual) => peak = actual,
        }
    }
}

unsafe impl GlobalAlloc for Tracking {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            let current = add_bytes(layout.size() as isize);
            ALLOC_COUNT.fetch_add(1, Ordering::Relaxed);
            update_peak(current);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add_bytes(-(layout.size() as isize));
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            let current = add_bytes(new_size as isize - layout.size() as isize);
            update_peak(current);
        }
        new_ptr
    }
}

/// The bytes currently allocated on the heap.
pub fn current_heap_bytes() -> usize {
    CURRENT_BYTES.load(Ordering::Relaxed).max(0) as usize
}

/// The highest number of bytes the heap ever held.
pub fn peak_heap_bytes() -> usize {
    PEAK_BYTES.load(Ordering::Relaxed)
}

/// The number of allocations the process has made.
pub fn allocation_count() -> usize {
    ALLOC_COUNT.load(Ordering::Relaxed)
}

/// Forget the recorded high-water marks, keeping the current total.
///
/// The current total is the real one; only the peaks are reset, so a tool
/// can measure the peak of one phase after the peak of another.
pub fn reset_peaks() {
    PEAK_BYTES.store(current_heap_bytes(), Ordering::Relaxed);
}

/// A memory measurement.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemorySample {
    /// The bytes the allocator had outstanding at the moment of the sample.
    pub heap_current_bytes: usize,
    /// The highest number of outstanding bytes the allocator has seen.
    pub heap_peak_bytes: usize,
    /// The process's resident set size at the moment of the sample.
    pub working_set_bytes: u64,
    /// The process's resident-set high-water mark.
    pub peak_working_set_bytes: u64,
    /// The process's private commit charge at the moment of the sample.
    pub private_bytes: u64,
    /// The process's peak pagefile usage.
    pub peak_pagefile_bytes: u64,
}

/// Take a memory sample.
pub fn sample() -> MemorySample {
    let (working_set, peak_working_set, private_bytes, peak_pagefile_bytes) = platform::counters();
    MemorySample {
        heap_current_bytes: current_heap_bytes(),
        heap_peak_bytes: peak_heap_bytes(),
        working_set_bytes: working_set,
        peak_working_set_bytes: peak_working_set,
        private_bytes,
        peak_pagefile_bytes,
    }
}

/// The Windows implementation of the process counters.
#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    /// `PROCESS_MEMORY_COUNTERS_EX` from `psapi.h`.
    #[repr(C)]
    #[derive(Default)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        peak_pagefile_usage: usize,
        pagefile_usage: usize,
        peak_paged_pool_usage: usize,
        paged_pool_usage: usize,
        peak_non_paged_pool_usage: usize,
        non_paged_pool_usage: usize,
        private_usage: usize,
    }

    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(
            process: *mut c_void,
            counters: *mut ProcessMemoryCountersEx,
            cb: u32,
        ) -> i32;
    }

    /// The pseudo-handle for the current process.
    const CURRENT_PROCESS: *mut c_void = -1isize as *mut c_void;
    pub(super) fn counters() -> (u64, u64, u64, u64) {
        let mut counters = ProcessMemoryCountersEx {
            cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
            ..Default::default()
        };
        // SAFETY: `counters` is a live, correctly sized structure, and the
        // pseudo-handle for the current process is always valid.
        let ok = unsafe { K32GetProcessMemoryInfo(CURRENT_PROCESS, &mut counters, counters.cb) };
        if ok == 0 {
            return (0, 0, 0, 0);
        }
        (
            counters.working_set_size as u64,
            counters.peak_working_set_size as u64,
            counters.private_usage as u64,
            counters.peak_pagefile_usage as u64,
        )
    }
}

/// The Linux implementation of the process counters.
#[cfg(target_os = "linux")]
mod platform {
    pub(super) fn counters() -> (u64, u64, u64, u64) {
        let mut current = 0u64;
        let mut peak = 0u64;
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("VmHWM:") {
                    peak = rest
                        .trim()
                        .trim_end_matches(" kB")
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0)
                        * 1024;
                } else if let Some(rest) = line.strip_prefix("VmRSS:") {
                    current = rest
                        .trim()
                        .trim_end_matches(" kB")
                        .trim()
                        .parse::<u64>()
                        .unwrap_or(0)
                        * 1024;
                }
            }
        }
        (current, peak, 0, 0)
    }
}

/// A fallback for the platforms that are neither Windows nor Linux.
#[cfg(not(any(windows, target_os = "linux")))]
mod platform {
    pub(super) fn counters() -> (u64, u64, u64, u64) {
        (0, 0, 0, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tracking allocator, for this test binary.
    ///
    /// The two binaries that report memory install it themselves. The
    /// library's own test binary needs its own, or the counter would read
    /// zero and the test would measure nothing. A crate may declare only
    /// one global allocator, but a library compiled as a dependency has
    /// `cfg(test)` off, so this declaration never reaches the binaries.
    #[global_allocator]
    static TEST_ALLOCATOR: Tracking = Tracking;

    #[test]
    fn a_sample_has_the_heap_figures() {
        let sample = sample();
        assert!(sample.heap_peak_bytes >= sample.heap_current_bytes);
    }
}
