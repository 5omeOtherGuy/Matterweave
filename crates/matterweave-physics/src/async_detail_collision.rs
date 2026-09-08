//! Bounded background preparation of detail collision shapes.
//!
//! STUB (RED): types and signatures only. Behaviour is implemented in the
//! GREEN checkpoint. See the module rewrite for the real controller.

use crate::PreparedDetailCollision;
use matterweave_detail::DetailScene;

/// Live counts for debugging and integration; not a stable metrics contract.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AsyncDetailStats {
    /// Pending source snapshots awaiting the worker: at most one.
    pub queued: usize,
    /// Jobs currently executing on the worker: at most one.
    pub inflight: usize,
    /// Prepared results awaiting `poll`: at most one.
    pub results: usize,
    /// Results dropped by saturation, supersession, cancellation or reset.
    pub discarded: u64,
    pub generation: u64,
}

/// Single-worker asynchronous detail-collision preparation controller.
pub struct AsyncDetailCollision;

impl Default for AsyncDetailCollision {
    fn default() -> Self {
        Self::new()
    }
}

impl AsyncDetailCollision {
    pub fn new() -> Self {
        Self
    }

    pub fn request(&mut self, _scene: &DetailScene) -> bool {
        false
    }

    pub fn poll(
        &mut self,
        _current: &DetailScene,
    ) -> Option<Result<PreparedDetailCollision, String>> {
        None
    }

    pub fn reset(&mut self) {}

    pub fn available(&self) -> bool {
        false
    }

    pub fn stats(&self) -> AsyncDetailStats {
        AsyncDetailStats::default()
    }
}
