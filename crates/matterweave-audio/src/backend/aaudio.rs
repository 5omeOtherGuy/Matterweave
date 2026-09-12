//! Native Android output via the pinned `ndk` AAudio bindings (0.9.0).
//!
//! Lifetime contract (verified against ndk-0.9.0 source):
//!
//! * The data/error callbacks are boxed `FnMut(+Send)` closures stored inside the
//!   `AudioStream` object; `AAudioStream_close` (called by `AudioStream::drop`)
//!   joins all callback threads before returning, so after the stream object is
//!   dropped no callback can run. The mixer core outlives the stream (service drop
//!   order: stream first, core second), so callbacks can never observe freed state.
//! * The error callback must not stop/close/reopen the stream itself (platform
//!   contract); it only writes shared atomics. The control thread recreates.
//! * AAudio data/error callbacks are never invoked simultaneously.
//! * Panics inside a callback abort the process (ndk wrapper contract), which is the
//!   intended failure mode for a broken real-time thread.

use core::sync::atomic::Ordering;
use std::sync::Arc;

use ndk::audio::{
    AudioCallbackResult, AudioDirection, AudioError, AudioFormat, AudioPerformanceMode,
    AudioSharingMode, AudioStream, AudioStreamBuilder, SessionId,
};

use crate::error::AudioServiceError;
use crate::mixer::{MixerCore, SharedRt};
use crate::service::StreamProperties;

use super::{verify_negotiated, BackendError, CorePtr, OutputBackend};

/// Opened AAudio output stream.
pub struct AaudioOutput {
    stream: Option<AudioStream>,
    props: Option<StreamProperties>,
    last_xruns: i32,
    lost: bool,
    _shared: Arc<SharedRt>,
}

// SAFETY: `AudioStream` is `Send` (its callback boxes are `+ Send`), `CorePtr` is
// `Send` by contract, and the stream object is only manipulated from the control
// thread. Not `Sync`; the service is used from one thread.
unsafe impl Send for AaudioOutput {}

fn session_id_value(id: SessionId) -> Option<i32> {
    match id {
        SessionId::None => None,
        SessionId::Allocated(v) => Some(v.get()),
    }
}

impl AaudioOutput {
    /// Open and verify a low-latency output stream for the mixer core.
    ///
    /// Requests 48 kHz / f32 / stereo. Negotiated values are verified after open; a
    /// mismatch is an explicit [`AudioServiceError::FormatRejected`] with the stream
    /// closed again — never a silent misinterpretation of samples.
    pub(crate) fn open(core: CorePtr, shared: Arc<SharedRt>) -> Result<Self, AudioServiceError> {
        let builder = AudioStreamBuilder::new()
            .map_err(|_| AudioServiceError::StreamOpenFailed { code: 0 })?
            .direction(AudioDirection::Output)
            .sample_rate(crate::config::SAMPLE_RATE as i32)
            .format(AudioFormat::PCM_Float)
            .channel_count(2)
            .performance_mode(AudioPerformanceMode::LowLatency)
            .data_callback(make_data_callback(core))
            .error_callback(make_error_callback(shared.clone()));

        let stream = builder
            .open_stream()
            .map_err(|e: AudioError| AudioServiceError::StreamOpenFailed { code: e.into() })?;

        let props = StreamProperties {
            sample_rate: stream.sample_rate() as u32,
            channels: stream.samples_per_frame() as u8,
            float_format: stream.format() == AudioFormat::PCM_Float,
            frames_per_burst: stream.frames_per_burst() as u32,
            buffer_capacity_frames: stream.buffer_capacity_in_frames() as u32,
            frames_per_data_callback: stream.frames_per_data_callback().map(|v| v as u32),
            device_id: stream.device_id(),
            session_id: session_id_value(stream.session_id()),
            low_latency: stream.performance_mode() == AudioPerformanceMode::LowLatency,
            sharing_exclusive: stream.sharing_mode() == AudioSharingMode::Exclusive,
        };
        if let Err(e) = verify_negotiated(props) {
            drop(stream); // closes; no callback can run after this returns
            return Err(e);
        }
        Ok(Self {
            stream: Some(stream),
            props: Some(props),
            last_xruns: 0,
            lost: false,
            _shared: shared,
        })
    }

    fn with_stream<T>(&self, f: impl FnOnce(&AudioStream) -> T) -> Option<T> {
        self.stream.as_ref().map(f)
    }
}

/// Read the raw pointer through a reference so the callback closure captures the
/// whole `CorePtr` (which is `Send`) instead of its raw-pointer field (which is
/// not). Rust 2021 disjoint closure captures would otherwise split the struct.
#[inline]
fn core_ptr_of(core: &CorePtr) -> *mut MixerCore {
    core.0
}

/// Build the data callback: one AAudio render invocation → one [`MixerCore::render`].
fn make_data_callback(core: CorePtr) -> ndk::audio::AudioStreamDataCallback {
    Box::new(
        move |stream: &AudioStream, audio_data: *mut core::ffi::c_void, num_frames: i32| {
            let channels = stream.samples_per_frame().max(1) as usize;
            let frames = num_frames.max(0) as usize;
            // SAFETY: AAudio provides a device-owned output buffer holding
            // `num_frames` frames of `channels` interleaved f32 samples (the format
            // was verified at open); the pointer is valid for this call only.
            let out = unsafe {
                core::slice::from_raw_parts_mut(audio_data as *mut f32, frames * channels)
            };
            let core_ptr = core_ptr_of(&core);
            // SAFETY: see `CorePtr` — dereferenced only on this single callback
            // thread; the target outlives every callback (stream closes first).
            unsafe {
                (*core_ptr).render(out, channels);
            }
            AudioCallbackResult::Continue
        },
    )
}

/// Build the error callback: record the failure in shared atomics only. The
/// control thread closes/reopens the stream (platform contract forbids doing that
/// from inside the callback).
fn make_error_callback(shared: Arc<SharedRt>) -> ndk::audio::AudioStreamErrorCallback {
    Box::new(move |_stream: &AudioStream, error: AudioError| {
        let code: i32 = error.into();
        shared.error_code.store(code, Ordering::Release);
        shared.disconnected.store(true, Ordering::Release);
    })
}

impl OutputBackend for AaudioOutput {
    fn properties(&self) -> Option<StreamProperties> {
        if self.lost || self._shared.disconnected.load(Ordering::Acquire) {
            None
        } else {
            self.props
        }
    }

    fn start(&mut self) -> Result<(), AudioServiceError> {
        let stream = self
            .stream
            .as_ref()
            .ok_or(AudioServiceError::StreamStartFailed { code: 0 })?;
        stream
            .request_start()
            .map_err(|e| AudioServiceError::StreamStartFailed { code: e.into() })
    }

    fn suspend(&mut self) -> Result<(), AudioServiceError> {
        let stream = self
            .stream
            .as_ref()
            .ok_or(AudioServiceError::SuspendFailed { code: 0 })?;
        stream
            .request_pause()
            .map_err(|e| AudioServiceError::SuspendFailed { code: e.into() })
    }

    fn close(&mut self) -> Result<(), AudioServiceError> {
        // Dropping the stream calls AAudioStream_close, which joins all callback
        // threads before returning; the boxed closures (and their captured core
        // pointer) are dropped with it.
        if let Some(stream) = self.stream.take() {
            self.last_xruns = stream.x_run_count();
            drop(stream);
        }
        self.props = None;
        Ok(())
    }

    fn take_error(&mut self) -> Option<BackendError> {
        if self._shared.disconnected.swap(false, Ordering::AcqRel) {
            self.lost = true;
            Some(BackendError {
                code: self._shared.error_code.load(Ordering::Relaxed),
            })
        } else {
            None
        }
    }

    fn xrun_count(&self) -> u32 {
        self.with_stream(|s| s.x_run_count())
            .unwrap_or(self.last_xruns) as u32
    }

    #[cfg(feature = "backend-mock")]
    fn as_mock(&mut self) -> Option<&mut crate::backend::mock::MockOutput> {
        None
    }
}
