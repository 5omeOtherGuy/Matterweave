use super::*;
use crate::backend::mock::MockOutput;
use std::sync::atomic::AtomicUsize;

mod device_errors;
mod lifetime;
mod pcm_boundary;

// Existing backend trait supplies all required operations. Only the private
// recreation opener needs injection; no global fault state or gameplay API.
struct FaultOutput {
    inner: MockOutput,
    starts: Arc<AtomicUsize>,
    fail_start: bool,
    fail_suspend: bool,
    error_on_close: Option<i32>,
}

impl OutputBackend for FaultOutput {
    fn properties(&self) -> Option<StreamProperties> {
        self.inner.properties()
    }
    fn start(&mut self) -> Result<(), AudioServiceError> {
        self.starts.fetch_add(1, Ordering::Relaxed);
        if self.fail_start {
            // A device error callback may accompany a failed start request.
            self.inner.inject_device_error(-1);
            Err(AudioServiceError::StreamStartFailed { code: -1 })
        } else {
            self.inner.start()
        }
    }
    fn suspend(&mut self) -> Result<(), AudioServiceError> {
        if self.fail_suspend {
            Err(AudioServiceError::SuspendFailed { code: -3 })
        } else {
            self.inner.suspend()
        }
    }
    fn close(&mut self) -> Result<(), AudioServiceError> {
        if let Some(code) = self.error_on_close {
            // Model the final old-stream error callback before close joins it.
            self.inner.inject_device_error(code);
        }
        self.inner.close()
    }
    fn take_error(&mut self) -> Option<BackendError> {
        self.inner.take_error()
    }
    fn xrun_count(&self) -> u32 {
        self.inner.xrun_count()
    }
    fn as_mock(&mut self) -> Option<&mut MockOutput> {
        Some(&mut self.inner)
    }
}

fn replace(service: &mut AudioService, fail_start: bool) -> Arc<AtomicUsize> {
    let starts = Arc::new(AtomicUsize::new(0));
    service
        .recreate_output_with(|core, shared| {
            Ok(Box::new(FaultOutput {
                inner: MockOutput::open(core, shared)?,
                starts: starts.clone(),
                fail_start,
                fail_suspend: false,
                error_on_close: None,
            }))
        })
        .unwrap();
    starts
}

fn playing() -> AudioService {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.125, 0.25, 0.375, 0.5]))
        .unwrap();
    service.play(clip, PlayOptions::default()).unwrap();
    let mut out = [0.0; 2];
    service.mock_render(&mut out).unwrap();
    assert_eq!(out, [0.125; 2]);
    service
}

fn assert_continuation(service: &mut AudioService) {
    let mut out = [0.0; 6];
    service.mock_render(&mut out).unwrap();
    assert_eq!(out, [0.25, 0.25, 0.375, 0.375, 0.5, 0.5]);
    assert_eq!(service.health().voices_completed, 1);
    assert!(!service.health().rt_suspended);
}

#[test]
fn suspended_replacement_never_requests_start() {
    for applied in [false, true] {
        let mut service = playing();
        service.suspend().unwrap();
        if applied {
            service.mock_render(&mut [0.0; 4]).unwrap();
        }
        let callbacks = service.health().callback_count;
        let starts = replace(&mut service, false);
        assert_eq!(starts.load(Ordering::Relaxed), 0);
        assert!(!service.running);
        assert!(service.health().suspended);
        assert_eq!(service.health().callback_count, callbacks);
        service.resume().unwrap();
        assert_eq!(starts.load(Ordering::Relaxed), 1);
        assert!(service.running);
        assert_continuation(&mut service);
    }
}

#[test]
fn failed_open_is_closed_and_poll_retries_with_pause_intent() {
    for suspended in [false, true] {
        let mut service = playing();
        if suspended {
            service.suspend().unwrap();
        }
        let callbacks = service.health().callback_count;
        for _ in 0..3 {
            assert_eq!(
                service.recreate_output_with(|_, _| {
                    Err(AudioServiceError::StreamOpenFailed { code: -2 })
                }),
                Err(AudioServiceError::StreamOpenFailed { code: -2 })
            );
            assert!(service.stream_properties().is_none());
            assert!(!service.running);
            assert!(service.recovery_pending);
            assert_eq!(service.health().suspended, suspended);
            assert_eq!(service.health().stream_recreations, 0);
        }
        service.poll_device().unwrap(); // no new error callback is needed
        assert!(!service.recovery_pending);
        assert_eq!(service.running, !suspended);
        assert_eq!(service.health().stream_recreations, 1);
        assert_eq!(service.health().callback_count, callbacks);
        service.resume().unwrap();
        assert_continuation(&mut service);
    }
}

#[test]
fn failed_recovery_start_is_closed_and_poll_retries_running_output() {
    let mut service = playing();
    assert_eq!(
        service.recreate_output_with(|core, shared| {
            Ok(Box::new(FaultOutput {
                inner: MockOutput::open(core, shared)?,
                starts: Arc::new(AtomicUsize::new(0)),
                fail_start: true,
                fail_suspend: false,
                error_on_close: None,
            }))
        }),
        Err(AudioServiceError::StreamStartFailed { code: -1 })
    );
    assert!(!service.running);
    assert!(service.stream_properties().is_none());
    assert_eq!(service.health().stream_recreations, 0);
    service.poll_device().unwrap();
    assert!(service.running);
    assert_eq!(service.health().stream_recreations, 1);
    assert_continuation(&mut service);
}

#[test]
fn failed_resume_retains_one_resume_command_and_exact_continuation() {
    for applied in [false, true] {
        let mut service = playing();
        service.suspend().unwrap();
        if applied {
            service.mock_render(&mut [0.0; 4]).unwrap();
        }
        let pending = service.health().pending_commands;
        // More retries than queue capacity must not saturate the bounded FIFO.
        for _ in 0..=crate::MAX_COMMANDS {
            let starts = replace(&mut service, true);
            assert_eq!(starts.load(Ordering::Relaxed), 0);
            assert_eq!(
                service.resume(),
                Err(AudioServiceError::StreamStartFailed { code: -1 })
            );
            assert_eq!(starts.load(Ordering::Relaxed), 1);
            assert!(!service.running);
            assert!(service.health().suspended);
            assert!(service.stream_properties().is_none());
            assert_eq!(service.health().pending_commands, pending + 1);
        }
        if applied {
            service.poll_device().unwrap();
            assert!(!service.running, "poll must not undo the successful pause");
        }
        // Also cover resume retrying the failed output without a preceding poll.
        service.resume().unwrap();
        assert!(service.running);
        assert_continuation(&mut service);
    }
}

#[test]
fn suspended_recovery_does_not_need_queue_space() {
    let mut service = playing();
    service.suspend().unwrap();
    while queue::vacant(&service.producer) > 0 {
        service.push(Command::Suspend).unwrap();
    }
    replace(&mut service, false);
    assert_eq!(service.resume(), Err(AudioServiceError::CommandQueueFull));
    assert!(!service.running);
    assert!(service.health().suspended);
    // Explicit in-flight mock invocation drains commands but must not consume PCM.
    let mut paused = [1.0; 4];
    service.mock_render(&mut paused).unwrap();
    assert_eq!(paused, [0.0; 4]);
    service.resume().unwrap();
    assert_continuation(&mut service);
}
