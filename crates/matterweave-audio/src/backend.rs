//! Output backend boundary.
//!
//! The backend owns the platform stream (if any) and the callback wiring; the game
//! API never sees backend types. Backends are control-thread objects: the real-time
//! path reaches the mixer core through the raw pointer captured at open time.

use crate::error::AudioServiceError;
use crate::mixer::{MixerCore, SharedRt};
use crate::service::StreamProperties;

#[cfg(all(target_os = "android", feature = "backend-android"))]
pub mod aaudio;
#[cfg(feature = "backend-mock")]
pub mod mock;

/// Raw pointer wrapper handed to platform callbacks.
///
/// # Safety (implementation contract)
///
/// `MixerCore` is boxed once and its address never changes. The backend closes its
/// stream before the core is dropped, and the platform contract (AAudio:
/// `AAudioStream_close` joins all callback threads before returning; the boxed
/// closures that capture this pointer live inside the `AudioStream` object) means no
/// callback can run after close returns. The pointer is only ever dereferenced on
/// the single render-callback thread, never on the control thread.
#[derive(Clone, Copy)]
pub(crate) struct CorePtr(pub(crate) *mut MixerCore);

// SAFETY: see the type contract above — single-callback-thread use, target outlives
// every callback.
unsafe impl Send for CorePtr {}

/// What a backend reports when its stream dies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BackendError {
    /// Backend-specific integer error code.
    pub code: i32,
}

/// Negotiated output properties are checked against these requirements before a
/// stream is started. Anything else is rejected explicitly.
pub(crate) fn verify_negotiated(props: StreamProperties) -> Result<(), AudioServiceError> {
    if props.sample_rate != crate::config::SAMPLE_RATE {
        return Err(AudioServiceError::FormatRejected {
            sample_rate: props.sample_rate,
            channels: props.channels,
            float_format: props.float_format,
        });
    }
    if props.channels != 1 && props.channels != 2 {
        return Err(AudioServiceError::FormatRejected {
            sample_rate: props.sample_rate,
            channels: props.channels,
            float_format: props.float_format,
        });
    }
    if !props.float_format {
        return Err(AudioServiceError::FormatRejected {
            sample_rate: props.sample_rate,
            channels: props.channels,
            float_format: false,
        });
    }
    Ok(())
}

/// A control-thread handle to an output stream.
pub(crate) trait OutputBackend {
    /// Last negotiated stream properties, if a stream is open.
    fn properties(&self) -> Option<StreamProperties>;

    /// Start (or restart) the stream. Data callbacks begin after this returns.
    fn start(&mut self) -> Result<(), AudioServiceError>;

    /// Suspend: freeze device data flow. The mixer core is suspended by command
    /// around this call, so in-flight callbacks render silence either way.
    fn suspend(&mut self) -> Result<(), AudioServiceError>;

    /// Close the stream. After this returns, no callback can run (platform contract)
    /// and the boxed callback closures are dropped with the stream object.
    fn close(&mut self) -> Result<(), AudioServiceError>;

    /// Take a pending device error (disconnect, timeout, ...), if any.
    fn take_error(&mut self) -> Option<BackendError>;

    /// Device-reported underruns/overruns since open.
    fn xrun_count(&self) -> u32;

    /// Typed access to the mock backend, if this is one.
    #[cfg(feature = "backend-mock")]
    fn as_mock(&mut self) -> Option<&mut crate::backend::mock::MockOutput>;
}

/// Open the platform backend for this build.
pub(crate) fn open_backend(
    core: CorePtr,
    shared: std::sync::Arc<SharedRt>,
) -> Result<Box<dyn OutputBackend>, AudioServiceError> {
    #[cfg(all(target_os = "android", feature = "backend-android"))]
    {
        Ok(Box::new(crate::backend::aaudio::AaudioOutput::open(
            core, shared,
        )?))
    }
    #[cfg(not(all(target_os = "android", feature = "backend-android")))]
    {
        #[cfg(feature = "backend-mock")]
        {
            Ok(Box::new(crate::backend::mock::MockOutput::open(
                core, shared,
            )?))
        }
        #[cfg(not(feature = "backend-mock"))]
        {
            let _ = (&core, &shared); // no backend compiled; nothing to wire
            Err(AudioServiceError::NoAudioBackend)
        }
    }
}
