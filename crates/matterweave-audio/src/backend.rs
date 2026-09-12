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
/// `CoreOwner` converts the mixer Box into a raw-owned allocation before any
/// callback pointer escapes. No owning Box/reference is retained while callbacks
/// can render; moving the service/owner never retags or moves the pointee. The
/// backend closes its stream before the core is dropped, and the platform contract (AAudio:
/// `AAudioStream_close` joins all callback threads before returning; the boxed
/// closures that capture this pointer live inside the `AudioStream` object) means no
/// callback can run after close returns. Only the single render role (including
/// synchronous mock renders) may dereference it, for that invocation alone, with
/// no concurrent core references. Ordinary control operations must not dereference
/// it. Raw copies do not extend the allocation lifetime; no use after owner drop.
#[derive(Clone, Copy)]
pub(crate) struct CorePtr(pub(crate) *mut MixerCore);

// SAFETY: transfer only the raw pointer, never a reference to the pointee. Its
// dereferencer must enforce single-callback-thread exclusive access and the owner
// must outlive all callbacks. Moving this wrapper does not access mixer memory.
unsafe impl Send for CorePtr {}

/// Sole owner of the mixer allocation; intentionally has no Deref/core accessor.
/// Raw aliases may only be used under CorePtr's callback contract. AudioService
/// drops its backend (joining callbacks) before this field is dropped.
pub(crate) struct CoreOwner(CorePtr);

impl CoreOwner {
    pub(crate) fn new(core: Box<MixerCore>) -> Self {
        Self(CorePtr(Box::into_raw(core)))
    }

    /// Copy allocation provenance without borrowing/dereferencing its contents.
    pub(crate) fn ptr(&self) -> CorePtr {
        self.0
    }
}

impl Drop for CoreOwner {
    fn drop(&mut self) {
        // SAFETY: this is exactly the pointer from Box::into_raw, reconstructed
        // once by its sole owner. AudioService's backend-first drop order has
        // joined every callback; no raw alias may be used after this point.
        let CorePtr(ptr) = self.0;
        unsafe { drop(Box::from_raw(ptr)) };
    }
}

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
pub(crate) trait OutputBackend: Send {
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
