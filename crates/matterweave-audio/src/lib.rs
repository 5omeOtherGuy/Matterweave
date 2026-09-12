//! Bounded PCM sound-effect service for Matterweave.
//!
//! The crate provides a game-facing API ([`AudioService`]) for registering short PCM
//! clips and playing them through a small, deterministic mixer, plus a native Android
//! output backend built on the pinned `ndk` AAudio bindings.
//!
//! # Contracts
//!
//! ## Limits (enforced, never violated by stealing or overwriting)
//!
//! | Resource | Limit |
//! | --- | --- |
//! | Registered clips | 32 |
//! | Active voices | 8 |
//! | Retained PCM payload | 4 MiB |
//! | Pending control commands | 64 |
//!
//! Operations that would exceed a limit fail explicitly (see [`AudioServiceError`]);
//! voices are never stolen and queued commands are never overwritten.
//!
//! ## Input
//!
//! Mono or stereo PCM f32 at 48 kHz. Clips with invalid channel counts, incomplete
//! frames, nonfinite samples, or wrong sample rates are rejected atomically.
//!
//! ## Output
//!
//! Mixed samples are finite and within `[-1.0, 1.0]`. Mono clips are upmixed to both
//! output channels without attenuation; stereo clips pass through; a mono output
//! device receives the mean of both contributions. The output stream is opened with
//! 48 kHz, f32, two channels requested; a stream that negotiates anything else fails
//! explicitly instead of misinterpreting bytes.
//!
//! ## Real-time safety
//!
//! The mixer render pass (one AAudio data-callback invocation) performs no heap
//! allocation or deallocation, no file or network I/O, no logging, no blocking locks,
//! and no stream shutdown. Control commands travel through a bounded SPSC queue
//! (`ringbuf`); a full queue rejects new commands with backpressure.
//!
//! ## Ownership and lifetime
//!
//! * Clip payloads live inside one preallocated pool owned by the service for its
//!   whole lifetime, so the render path can never touch freed memory.
//! * Handles ([`ClipHandle`], [`VoiceHandle`]) are generational: unregistering a clip
//!   or reusing a voice slot invalidates all older handles to it.
//! * Unregistering a clip silences (does not merely detach) all voices still playing
//!   it, and the freed PCM range is reused only after the audio thread has
//!   acknowledged the Unload command's FIFO sequence after completing PCM reads.
//! * While the output is suspended the mixer clock freezes mid-sample: nothing is
//!   dropped, nothing restarts, and commands queued during suspension are applied in
//!   FIFO order on resume. [`AudioService::health`] reports suspension in `suspended`
//!   as soon as [`AudioService::suspend`] returns, without waiting for a render
//!   invocation; `rt_suspended` is the render thread's own view, which cannot advance
//!   while a paused backend delivers no callbacks (AAudio).
//! * Device loss is reported by the AAudio error callback into shared atomics;
//!   [`AudioService::poll_device`] then closes and reopens the stream on the control
//!   thread with the same mixer core, so voices continue where they stopped.

#![warn(missing_docs)]
#![forbid(unsafe_op_in_unsafe_fn)]

pub mod backend;
pub mod clip;
pub mod command;
pub mod config;
pub mod error;
pub mod handle;
pub(crate) mod mixer;
pub mod service;

pub use clip::ClipSpec;
pub use config::{AudioLimits, MAX_CLIPS, MAX_COMMANDS, MAX_PCM_BYTES, MAX_VOICES};
pub use error::{AudioServiceError, DeviceError, InvalidClipReason};
pub use handle::{ClipHandle, VoiceHandle};
pub use service::{AudioService, HealthSnapshot, PlayOptions, StreamProperties};
