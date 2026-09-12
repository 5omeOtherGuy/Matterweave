use super::*;

#[test]
fn failed_start_error_must_not_disconnect_healthy_replacement() {
    let mut service = playing();
    service.suspend().unwrap();
    replace(&mut service, true);
    assert_eq!(
        service.resume(),
        Err(AudioServiceError::StreamStartFailed { code: -1 })
    );
    service.resume().unwrap();
    let recreated = service.health().stream_recreations;
    service.poll_device().unwrap();
    assert_eq!(
        service.health().stream_recreations,
        recreated,
        "the failed old stream's notification must not tear down the healthy replacement"
    );
    assert_continuation(&mut service);
}

#[test]
fn recreation_clears_old_error_before_open_but_preserves_new_stream_error() {
    let mut service = playing();
    service
        .recreate_output_with(|core, shared| {
            Ok(Box::new(FaultOutput {
                inner: MockOutput::open(core, shared)?,
                starts: Arc::new(AtomicUsize::new(0)),
                fail_start: false,
                fail_suspend: false,
                error_on_close: Some(-4),
            }))
        })
        .unwrap();
    service
        .recreate_output_with(|core, shared| {
            assert!(
                !shared.disconnected.load(Ordering::Acquire),
                "old error survives into new open"
            );
            assert_eq!(shared.error_code.load(Ordering::Acquire), 0);
            let output = MockOutput::open(core, shared)?;
            output.inject_device_error(-5); // error from this replacement, not its predecessor
            Ok(Box::new(output))
        })
        .unwrap();
    assert_eq!(
        service.shared.error_code.load(Ordering::Acquire),
        -5,
        "successful recreation must not erase an error from the new stream"
    );
    assert!(service.stream_properties().is_none());
    service.poll_device().unwrap();
    assert_eq!(service.health().device_errors, 1);
    assert_eq!(service.health().stream_recreations, 3);
    assert!(service.stream_properties().is_some());
}

#[test]
fn error_callback_alone_hides_properties_before_poll() {
    let mut service = playing();
    // This is what the real AAudio callback does: it cannot mutate backend.lost.
    service.shared.error_code.store(-6, Ordering::Release);
    service.shared.disconnected.store(true, Ordering::Release);
    assert!(service.stream_properties().is_none());
    service.poll_device().unwrap();
    assert!(service.stream_properties().is_some());
}

#[test]
fn suspend_during_failed_recovery_preserves_intent_until_resume() {
    for fail_open in [true, false] {
        let mut service = playing();
        let result = service.recreate_output_with(|core, shared| {
            if fail_open {
                Err(AudioServiceError::StreamOpenFailed { code: -2 })
            } else {
                Ok(Box::new(FaultOutput {
                    inner: MockOutput::open(core, shared)?,
                    starts: Arc::new(AtomicUsize::new(0)),
                    fail_start: true,
                    fail_suspend: false,
                    error_on_close: None,
                }))
            }
        });
        assert!(result.is_err());
        assert!(!service.running);
        service.suspend().unwrap();
        assert!(
            service.health().suspended,
            "pause intent lost during recovery"
        );
        service.poll_device().unwrap();
        assert!(!service.running, "recovery started background audio");
        assert!(service.health().suspended);
        service.resume().unwrap();
        assert_continuation(&mut service);
    }
}

#[test]
fn suspend_after_failed_resume_is_idempotent_and_retains_continuation() {
    let mut service = playing();
    service.suspend().unwrap();
    replace(&mut service, true);
    assert!(service.resume().is_err());
    let pending = service.health().pending_commands;
    service.suspend().unwrap();
    assert_eq!(service.health().pending_commands, pending);
    service.poll_device().unwrap();
    assert!(!service.running);
    service.resume().unwrap();
    assert_continuation(&mut service);
}
