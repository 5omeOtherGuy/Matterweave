//! Clip registration data.

use crate::config::SAMPLE_RATE;
use crate::error::{AudioServiceError, InvalidClipReason};

/// A clip to register: interleaved mono or stereo f32 samples at 48 kHz.
///
/// For stereo, samples alternate left, right. The sample count must be a whole
/// number of frames (`len % channels == 0`). All samples must be finite.
///
/// Registration validates the whole clip before any state changes; a rejected clip
/// leaves the service untouched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipSpec<'a> {
    /// Interleaved f32 samples.
    pub samples: &'a [f32],
    /// 1 (mono) or 2 (stereo).
    pub channels: u8,
}

impl<'a> ClipSpec<'a> {
    /// Validate without touching any service state.
    pub(crate) fn validate(&self) -> Result<ValidatedClip, AudioServiceError> {
        if self.channels != 1 && self.channels != 2 {
            return Err(AudioServiceError::InvalidClip(InvalidClipReason::Channels));
        }
        if self.samples.is_empty() {
            return Err(AudioServiceError::InvalidClip(InvalidClipReason::Empty));
        }
        if !self.samples.len().is_multiple_of(self.channels as usize) {
            return Err(AudioServiceError::InvalidClip(
                InvalidClipReason::IncompleteFrame,
            ));
        }
        // Validate every sample before mutating anything (atomic rejection).
        for &s in self.samples {
            if !s.is_finite() {
                return Err(AudioServiceError::InvalidClip(
                    InvalidClipReason::NonfiniteSample,
                ));
            }
        }
        Ok(ValidatedClip {
            frames: self.samples.len() / self.channels as usize,
            channels: self.channels,
        })
    }

    /// Mono stereo helper: one channel, `samples` frames.
    pub fn mono(samples: &'a [f32]) -> Self {
        Self {
            samples,
            channels: 1,
        }
    }

    /// Stereo helper: interleaved L/R, `samples.len() / 2` frames.
    pub fn stereo(samples: &'a [f32]) -> Self {
        Self {
            samples,
            channels: 2,
        }
    }

    /// The required sample rate, for API clarity.
    pub const SAMPLE_RATE: u32 = SAMPLE_RATE;
}

/// Validation results needed by the service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ValidatedClip {
    /// Whole frames.
    pub frames: usize,
    /// 1 or 2.
    pub channels: u8,
}
