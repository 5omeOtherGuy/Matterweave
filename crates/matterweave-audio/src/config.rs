//! Hard service limits and their acceptance context.
//!
//! These are task-specific acceptance limits for this service, not owner-approved
//! product guarantees.

/// Maximum number of simultaneously registered clips.
pub const MAX_CLIPS: usize = 32;

/// Maximum number of simultaneously active voices.
pub const MAX_VOICES: usize = 8;

/// Maximum retained PCM payload in bytes (f32 samples, 4 bytes each).
///
/// The clip pool is allocated once at service creation with exactly this size and is
/// never resized, which bounds all PCM memory for the whole service lifetime.
pub const MAX_PCM_BYTES: usize = 4 * 1024 * 1024;

/// Maximum number of pending control commands between the game thread and the
/// render thread.
pub const MAX_COMMANDS: usize = 64;

/// Sample rate accepted for clip data and requested from the output stream.
pub const SAMPLE_RATE: u32 = 48_000;

/// Maximum absolute gain accepted for playback and gain changes.
pub const MAX_GAIN: f32 = 8.0;

/// Acceptance limits bundled for documentation and tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioLimits;

impl AudioLimits {
    /// Total f32 sample capacity of the clip pool.
    pub const fn pcm_pool_samples() -> usize {
        MAX_PCM_BYTES / 4
    }
}
