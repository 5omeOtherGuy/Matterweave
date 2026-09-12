//! Fixed PCM storage shared independently of mutable mixer state.
//!
//! `UnsafeCell` permits interior mutation through shared pool owners; it does NOT
//! permit conflicting accesses. The service owns unpublished/free ranges, writes
//! them before FIFO publication, and cannot write retired ranges until an Acquire
//! observes the completed Unload sequence. The mixer reads only published voice
//! ranges and releases that acknowledgment after its last PCM read. Neither side
//! ever creates a reference to an inner f32 or a mutable slice of the allocation.

use std::cell::UnsafeCell;

use crate::config::MAX_PCM_BYTES;

pub(crate) struct PcmPool {
    samples: Box<[UnsafeCell<f32>]>,
}

// SAFETY: safe methods only allocate or inspect immutable allocation metadata.
// All payload access is unsafe and requires the caller to exclude conflicting
// access to the selected cells and establish publication/retirement ordering.
// Shared references to UnsafeCell permit the disjoint writes; no &mut PcmPool or
// &mut inner f32 is created. Allocation is fixed and lives until its last Arc drops.
unsafe impl Sync for PcmPool {}
// Send is derived automatically: UnsafeCell<f32> and Box are Send.

impl PcmPool {
    pub(crate) fn new(samples: usize) -> Self {
        assert!(samples <= MAX_PCM_BYTES / size_of::<f32>());
        Self {
            samples: std::iter::repeat_with(|| UnsafeCell::new(0.0))
                .take(samples)
                .collect(),
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.samples.len()
    }

    /// Write a checked range directly, with no temporary PCM allocation/copy.
    ///
    /// # Safety
    /// Caller must exclusively own the destination cells for this entire call:
    /// no other reader, writer or reference to their inner f32 may exist. Source
    /// must not alias them. Prior readers must have completed (Acquire of the
    /// completed Unload sequence), and future readers require publication after
    /// this call (FIFO release/acquire). Other cells may be accessed concurrently.
    pub(crate) unsafe fn write(&self, start: usize, source: &[f32]) {
        let end = start.checked_add(source.len()).expect("PCM range overflow");
        let destination = &self.samples[start..end]; // shared UnsafeCells, not f32s
        for (cell, &sample) in destination.iter().zip(source) {
            // SAFETY: caller exclusively owns this cell's payload; get() supplies
            // its interior raw pointer without forming an exclusive reference.
            unsafe { cell.get().write(sample) };
        }
    }

    /// Read one checked sample by value, never exposing a reference into the pool.
    ///
    /// # Safety
    /// Caller must hold a published range containing index: its write happens
    /// before this read, and no writer or exclusive inner reference may overlap
    /// the read. Do not acknowledge range retirement before the last read ends.
    pub(crate) unsafe fn read(&self, index: usize) -> f32 {
        // SAFETY: caller establishes initialization publication and excludes
        // conflicting access; bounds are checked, and f32 is copied by value.
        unsafe { self.samples[index].get().read() }
    }
}
