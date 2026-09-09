//! Error types for the audio service.
//!
//! All errors are descriptive plain data: no backend (NDK) types leak through the
//! public API.

use std::fmt;

/// Why a clip registration was rejected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidClipReason {
    /// Sample rate is not the required 48 kHz.
    SampleRate,
    /// Channel count is neither mono nor stereo.
    Channels,
    /// Sample count is not a whole number of frames.
    IncompleteFrame,
    /// The clip contains no samples.
    Empty,
    /// At least one sample (or the gain) is NaN or infinite.
    NonfiniteSample,
}

impl fmt::Display for InvalidClipReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::SampleRate => "sample rate must be 48000 Hz",
            Self::Channels => "channel count must be 1 (mono) or 2 (stereo)",
            Self::IncompleteFrame => "sample count must be a whole number of frames",
            Self::Empty => "clip must contain at least one frame",
            Self::NonfiniteSample => "samples must be finite",
        };
        f.write_str(text)
    }
}

/// An error reported by an output device (backend-specific detail is an integer
/// code, not a backend type).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceError {
    /// Backend error code (0 is never an error).
    pub code: i32,
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "device error code {}", self.code)
    }
}

/// Everything that can go wrong through the [`crate::AudioService`] API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioServiceError {
    /// Registration refused: all 32 clip slots are in use.
    ClipLimit,
    /// Playback refused: all 8 voice slots are active (voices are never stolen).
    VoiceLimit,
    /// Registration refused: no free PCM range of the requested size exists.
    PcmBudgetExceeded {
        /// Samples required by the rejected clip.
        required_samples: usize,
        /// Largest contiguous free range currently available.
        largest_free_range_samples: usize,
    },
    /// The command queue is full (64 pending). Nothing was enqueued; retry later.
    CommandQueueFull,
    /// A freed PCM range exists but the audio thread has not yet acknowledged the
    /// unload (no render callback ran since). Retry after the next callback or after
    /// [`crate::AudioService::resume`].
    RangeNotYetAcknowledged,
    /// Clip registration data is invalid. State was not changed.
    InvalidClip(InvalidClipReason),
    /// Gain is nonfinite or exceeds the supported magnitude.
    InvalidGain,
    /// The clip handle is stale (unregistered or superseded by a newer registration
    /// in the same slot).
    StaleClipHandle,
    /// The voice handle is stale (slot reused by a newer voice).
    StaleVoiceHandle,
    /// The voice exists but already finished or was stopped; only `stop_voice` is a
    /// successful no-op in that case.
    VoiceNotActive,
    /// The output device reported an error or disconnection and the stream is not
    /// open. Call [`crate::AudioService::poll_device`] to attempt recreation.
    DeviceLost(DeviceError),
    /// Opening the output stream failed.
    StreamOpenFailed {
        /// Backend error code, if the backend reported one.
        code: i32,
    },
    /// The opened stream cannot carry the required format (48 kHz f32, mono/stereo).
    /// It was closed again instead of misinterpreting samples.
    FormatRejected {
        /// Negotiated sample rate.
        sample_rate: u32,
        /// Negotiated channel count.
        channels: u8,
        /// Negotiated format is f32 iff true.
        float_format: bool,
    },
    /// Starting the stream failed.
    StreamStartFailed {
        /// Backend error code, if the backend reported one.
        code: i32,
    },
    /// Suspending the stream failed.
    SuspendFailed {
        /// Backend error code, if the backend reported one.
        code: i32,
    },
    /// No output backend is compiled in for this target.
    NoAudioBackend,
}

impl fmt::Display for AudioServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipLimit => write!(f, "clip limit reached ({})", crate::MAX_CLIPS),
            Self::VoiceLimit => write!(f, "voice limit reached ({})", crate::MAX_VOICES),
            Self::PcmBudgetExceeded {
                required_samples,
                largest_free_range_samples,
            } => write!(
                f,
                "PCM budget exceeded: need {required_samples} samples, largest free range {largest_free_range_samples}"
            ),
            Self::CommandQueueFull => write!(
                f,
                "command queue full ({} pending); retry later",
                crate::MAX_COMMANDS
            ),
            Self::RangeNotYetAcknowledged => write!(
                f,
                "freed PCM range not acknowledged by audio thread yet; retry after the next callback or resume"
            ),
            Self::InvalidClip(reason) => write!(f, "invalid clip: {reason}"),
            Self::InvalidGain => write!(
                f,
                "gain must be finite and within ±{}",
                crate::config::MAX_GAIN
            ),
            Self::StaleClipHandle => write!(f, "stale clip handle"),
            Self::StaleVoiceHandle => write!(f, "stale voice handle"),
            Self::VoiceNotActive => write!(f, "voice is not active"),
            Self::DeviceLost(e) => write!(f, "audio device lost: {e}"),
            Self::StreamOpenFailed { code } => write!(f, "stream open failed (code {code})"),
            Self::FormatRejected {
                sample_rate,
                channels,
                float_format,
            } => write!(
                f,
                "stream format rejected: rate {sample_rate}, channels {channels}, f32 {float_format}; requires 48000 Hz f32 mono/stereo"
            ),
            Self::StreamStartFailed { code } => write!(f, "stream start failed (code {code})"),
            Self::SuspendFailed { code } => write!(f, "stream suspend failed (code {code})"),
            Self::NoAudioBackend => write!(f, "no audio backend compiled for this target"),
        }
    }
}

impl std::error::Error for AudioServiceError {}
