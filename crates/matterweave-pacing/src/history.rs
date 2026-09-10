//! Fixed-capacity ring buffer. Allocation-free after construction; overflow
//! overwrites the oldest sample, so no input sequence can grow memory.

/// Capacity is defined by [`HISTORY_CAPACITY`](crate::HISTORY_CAPACITY); this
/// module stays generic over the stored values only.
use crate::HISTORY_CAPACITY;

/// Ring of at most [`HISTORY_CAPACITY`] `u64` samples stored inline.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ring {
    buf: [u64; HISTORY_CAPACITY],
    len: usize,
    head: usize,
}

impl Ring {
    pub(crate) const fn new() -> Self {
        Self {
            buf: [0; HISTORY_CAPACITY],
            len: 0,
            head: 0,
        }
    }

    /// Records a sample, overwriting the oldest once full.
    pub(crate) fn push(&mut self, value: u64) {
        self.buf[self.head] = value;
        self.head = (self.head + 1) % HISTORY_CAPACITY;
        if self.len < HISTORY_CAPACITY {
            self.len += 1;
        }
    }

    /// Copies samples in chronological order (oldest first) into `out` and
    /// returns the count (at most [`HISTORY_CAPACITY`] and `out.len()`).
    pub(crate) fn copy_ordered(&self, out: &mut [u64; HISTORY_CAPACITY]) -> usize {
        let n = self.len.min(out.len());
        for (slot, i) in out.iter_mut().zip(0..n) {
            *slot = self.buf[(self.start() + i) % HISTORY_CAPACITY];
        }
        n
    }

    /// Copies the newest `n` samples in chronological order into `out` and
    /// returns the count actually copied.
    pub(crate) fn last_n(&self, n: usize, out: &mut [u64; HISTORY_CAPACITY]) -> usize {
        let count = n.min(self.len).min(out.len());
        let skip = self.len - count;
        for (slot, i) in out.iter_mut().zip(0..count) {
            *slot = self.buf[(self.start() + skip + i) % HISTORY_CAPACITY];
        }
        count
    }

    /// Index of the oldest sample.
    fn start(&self) -> usize {
        if self.len < HISTORY_CAPACITY {
            0
        } else {
            self.head
        }
    }
}
