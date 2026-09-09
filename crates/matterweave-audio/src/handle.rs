//! Generational handles handed to the game.
//!
//! Handles are opaque, cheap to copy and compare, and become stale when the slot they
//! named is unregistered or reused. Stale handles never affect a later voice or clip:
//! every operation revalidates the generation against the service's mirror of the
//! mixer state, and the mixer revalidates again when it applies the command.

use core::fmt;

/// Handle to a registered clip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ClipHandle {
    /// Clip slot index (0..32).
    pub(crate) slot: u8,
    /// Generation of the registration in that slot.
    pub(crate) generation: u32,
}

impl ClipHandle {
    /// Opaque slot index; only meaningful for diagnostics.
    pub fn slot(&self) -> u8 {
        self.slot
    }

    /// Opaque generation; only meaningful for diagnostics.
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

impl fmt::Display for ClipHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "clip {}@{}", self.slot, self.generation)
    }
}

/// Handle to an active voice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VoiceHandle {
    /// Voice slot index (0..8).
    pub(crate) slot: u8,
    /// Generation of the voice occupying that slot.
    pub(crate) generation: u32,
}

impl VoiceHandle {
    /// Opaque slot index; only meaningful for diagnostics.
    pub fn slot(&self) -> u8 {
        self.slot
    }

    /// Opaque generation; only meaningful for diagnostics.
    pub fn generation(&self) -> u32 {
        self.generation
    }
}

impl fmt::Display for VoiceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "voice {}@{}", self.slot, self.generation)
    }
}
