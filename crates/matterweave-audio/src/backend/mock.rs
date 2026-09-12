//! Host test backend.
//!
//! Driven synchronously by the control thread (`mock_render`), which makes the
//! deterministic mixer tests exact and the allocation detector meaningful. It is a
//! test instrument only: it proves nothing about device behavior and is never used
//! to claim device results.

use core::cell::Cell;

use crate::error::AudioServiceError;
use crate::mixer::SharedRt;
use crate::service::StreamProperties;

use super::{BackendError, CorePtr, OutputBackend};

/// Why a direct `render` call was invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MockRenderError {
    /// The simulated stream was closed; callbacks cannot run after close.
    Closed,
}

/// Mock output backend for host tests and host smoke runs.
///
/// Render validity: any open stream can be rendered (started or paused), modeling
/// in-flight callbacks; only a closed stream refuses.
pub struct MockOutput {
    props: StreamProperties,
    core: CorePtr,
    shared: std::sync::Arc<SharedRt>,
    closed: Cell<bool>,
    lost: Cell<bool>,
    xruns: Cell<u32>,
}

// Auto-Send: CorePtr is Send by contract, `Arc<SharedRt>` is Send+Sync, the rest are
// plain data. Not Sync (Cell fields); the service is used from one thread anyway.
impl MockOutput {
    /// Open a mock stream with fixed negotiated properties.
    pub(crate) fn open(
        core: CorePtr,
        shared: std::sync::Arc<SharedRt>,
    ) -> Result<Self, AudioServiceError> {
        let props = StreamProperties {
            sample_rate: crate::config::SAMPLE_RATE,
            channels: 2,
            float_format: true,
            frames_per_burst: 192,
            buffer_capacity_frames: 192 * 4,
            frames_per_data_callback: None,
            device_id: -1,
            session_id: None,
            low_latency: true,
            sharing_exclusive: false,
        };
        super::verify_negotiated(props)?;
        Ok(Self {
            props,
            core,
            shared,
            closed: Cell::new(false),
            lost: Cell::new(false),
            xruns: Cell::new(0),
        })
    }

    /// Simulate a device error/disconnect exactly like the AAudio error callback:
    /// shared atomics only, never touching the stream or the mixer core. After the
    /// error callback, the (dead) stream's properties are no longer reported.
    pub fn inject_device_error(&self, code: i32) {
        self.shared
            .disconnected
            .store(true, core::sync::atomic::Ordering::Release);
        self.shared
            .error_code
            .store(code, core::sync::atomic::Ordering::Release);
        self.lost.set(true);
    }

    /// Simulate a device underrun count (diagnostic tests only).
    pub fn set_xruns(&self, count: u32) {
        self.xruns.set(count);
    }

    /// Drive one render invocation synchronously (host equivalent of one data
    /// callback). Valid while the simulated stream is open, including while the
    /// device is paused (models in-flight callbacks, which render silence because
    /// the mixer core is suspended by command). Returns frames rendered.
    ///
    /// The call is allocation-free end to end (mixer render + command drain).
    pub fn render(&mut self, out: &mut [f32]) -> Result<usize, MockRenderError> {
        if self.closed.get() {
            return Err(MockRenderError::Closed);
        }
        let channels = self.props.channels as usize;
        debug_assert_eq!(out.len() % channels, 0, "mock render needs whole frames");
        // SAFETY: core outlives this stream (service drop order closes streams
        // before dropping the core); exclusive access (single render thread).
        unsafe {
            (*self.core.0).render(out, channels);
        }
        Ok(out.len() / channels)
    }

    /// Mock-specific check used by tests to assert callbacks cannot run after close.
    pub fn is_closed(&self) -> bool {
        self.closed.get()
    }
}

impl OutputBackend for MockOutput {
    fn properties(&self) -> Option<StreamProperties> {
        if self.closed.get()
            || self.lost.get()
            || self
                .shared
                .disconnected
                .load(core::sync::atomic::Ordering::Acquire)
        {
            None
        } else {
            Some(self.props)
        }
    }

    fn start(&mut self) -> Result<(), AudioServiceError> {
        if self.closed.get() {
            return Err(AudioServiceError::StreamStartFailed { code: 0 });
        }
        Ok(())
    }

    fn suspend(&mut self) -> Result<(), AudioServiceError> {
        if self.closed.get() {
            return Err(AudioServiceError::SuspendFailed { code: 0 });
        }
        // A paused stream receives no callbacks from the platform; renders stay
        // possible through this test entry point to model in-flight callbacks,
        // which are silent because the mixer core is suspended by command.
        Ok(())
    }

    fn close(&mut self) -> Result<(), AudioServiceError> {
        self.closed.set(true);
        Ok(())
    }

    fn take_error(&mut self) -> Option<BackendError> {
        if self
            .shared
            .disconnected
            .swap(false, core::sync::atomic::Ordering::AcqRel)
        {
            self.lost.set(true);
            Some(BackendError {
                code: self
                    .shared
                    .error_code
                    .load(core::sync::atomic::Ordering::Relaxed),
            })
        } else {
            None
        }
    }

    fn xrun_count(&self) -> u32 {
        self.xruns.get()
    }

    fn as_mock(&mut self) -> Option<&mut MockOutput> {
        Some(self)
    }
}
