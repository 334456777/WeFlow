//! Counts what the program allocates (#513), for the benchmark (#259): with the `alloc-stats` feature the global
//! allocator is mimalloc behind counters, and when `WEFLOW_ALLOC_STATS` names a file the counts are written there
//! as JSON when `main` returns. The counters cost a few atomic operations per call, so timings come from a build
//! without the feature; this one only gives the counts.

use std::alloc::{GlobalAlloc, Layout};
use std::sync::atomic::{AtomicU64, Ordering};

/// The environment variable naming the file the counts go to.
const REPORT_ENV: &str = "WEFLOW_ALLOC_STATS";

static ALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static REALLOCATIONS: AtomicU64 = AtomicU64::new(0);
static DEALLOCATIONS: AtomicU64 = AtomicU64::new(0);
/// Bytes asked for: each allocation's size, and what a reallocation adds when it grows.
static ALLOCATED_BYTES: AtomicU64 = AtomicU64::new(0);
static LIVE_BYTES: AtomicU64 = AtomicU64::new(0);
static PEAK_LIVE_BYTES: AtomicU64 = AtomicU64::new(0);

/// mimalloc, counting each call.
#[derive(Debug)]
pub struct Counting;

fn grew(by: u64) {
    ALLOCATED_BYTES.fetch_add(by, Ordering::Relaxed);
    let live = LIVE_BYTES.fetch_add(by, Ordering::Relaxed) + by;
    PEAK_LIVE_BYTES.fetch_max(live, Ordering::Relaxed);
}

fn shrank(by: u64) {
    LIVE_BYTES.fetch_sub(by, Ordering::Relaxed);
}

// SAFETY: every call is passed on unchanged to mimalloc, a sound global allocator; the counters only observe it.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which mimalloc's needs.
        let ptr = unsafe { mimalloc::MiMalloc.alloc(layout) };
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            grew(layout.size() as u64);
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: as in `alloc`.
        let ptr = unsafe { mimalloc::MiMalloc.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            grew(layout.size() as u64);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller passes a block this allocator gave out, with its layout.
        unsafe { mimalloc::MiMalloc.dealloc(ptr, layout) };
        DEALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        shrank(layout.size() as u64);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract, which mimalloc's needs.
        let new = unsafe { mimalloc::MiMalloc.realloc(ptr, layout, new_size) };
        if !new.is_null() {
            REALLOCATIONS.fetch_add(1, Ordering::Relaxed);
            let (old, new_size) = (layout.size() as u64, new_size as u64);
            if new_size >= old {
                grew(new_size - old);
            } else {
                shrank(old - new_size);
            }
        }
        new
    }
}

/// Writes the counts to the file `WEFLOW_ALLOC_STATS` names, if any, when dropped at the end of `main`. A process
/// that ends another way (`std::process::exit`, a kill) writes nothing.
#[derive(Debug)]
pub struct Report;

impl Drop for Report {
    fn drop(&mut self) {
        let Some(path) = std::env::var_os(REPORT_ENV) else {
            return;
        };
        // read before formatting the report allocates
        let counts = [
            ("allocations", ALLOCATIONS.load(Ordering::Relaxed)),
            ("reallocations", REALLOCATIONS.load(Ordering::Relaxed)),
            ("deallocations", DEALLOCATIONS.load(Ordering::Relaxed)),
            ("allocatedBytes", ALLOCATED_BYTES.load(Ordering::Relaxed)),
            ("peakLiveBytes", PEAK_LIVE_BYTES.load(Ordering::Relaxed)),
            ("liveBytesAtExit", LIVE_BYTES.load(Ordering::Relaxed)),
        ];
        let doc: serde_json::Map<String, serde_json::Value> = counts
            .into_iter()
            .map(|(name, n)| (name.to_string(), n.into()))
            .collect();
        // best effort: a benchmark that finds no file reports the counts as missing
        let _ = std::fs::write(path, serde_json::Value::Object(doc).to_string());
    }
}
