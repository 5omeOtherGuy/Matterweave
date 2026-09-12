//! Recovery must preserve pause intent even if Suspend has not reached the mixer.
use matterweave_audio::{AudioService, ClipSpec, PlayOptions};

fn suspended_recovery(recreate: fn(&mut AudioService), apply_suspend: bool) {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.125, 0.25, 0.375, 0.5, 0.625]))
        .unwrap();
    let voice = service.play(clip, PlayOptions::default()).unwrap();
    let mut first = [0.0; 4];
    service.mock_render(&mut first).unwrap();
    assert_eq!(first, [0.125, 0.125, 0.25, 0.25]);
    service.suspend().unwrap();
    if apply_suspend {
        let mut paused = [1.0; 8];
        service.mock_render(&mut paused).unwrap();
        assert_eq!(paused, [0.0; 8]);
    }
    let before = service.health();
    assert_eq!(before.rt_suspended, apply_suspend);
    assert_eq!(before.pending_commands, usize::from(!apply_suspend));

    // No callbacks during recovery or the subsequent pause. In the unapplied case
    // the Suspend command must remain buffered until the first resumed callback.
    for count in 1..=3 {
        recreate(&mut service);
        let paused = service.health();
        assert!(
            paused.suspended,
            "recreation must preserve service suspension"
        );
        assert_eq!(paused.callback_count, before.callback_count);
        assert_eq!(paused.frames_rendered, before.frames_rendered);
        assert_eq!(paused.pending_commands, before.pending_commands);
        assert_eq!(paused.stream_recreations, count);
    }
    service.resume().unwrap();
    service.resume().unwrap(); // idempotent, not an extra Resume command
    assert!(!service.health().suspended);
    let mut continuation = [0.0; 6];
    service.mock_render(&mut continuation).unwrap();
    assert_eq!(continuation, [0.375, 0.375, 0.5, 0.5, 0.625, 0.625]);
    assert!(!service.health().rt_suspended);
    assert!(!service.is_voice_active(voice));
    assert_eq!(service.health().voices_completed, 1);
    let mut tail = [1.0; 4];
    service.mock_render(&mut tail).unwrap();
    assert_eq!(tail, [0.0; 4]);
}

fn device_loss(service: &mut AudioService) {
    service.mock_backend().unwrap().inject_device_error(-899);
    service.poll_device().unwrap();
}

#[test]
fn device_loss_with_buffered_suspend() {
    suspended_recovery(device_loss, false);
}

#[test]
fn device_loss_with_applied_suspend() {
    suspended_recovery(device_loss, true);
}

#[cfg(feature = "diagnostic")]
#[test]
fn diagnostic_recreation_with_buffered_suspend() {
    suspended_recovery(|s| s.diagnostic_recreate_output().unwrap(), false);
}

#[cfg(feature = "diagnostic")]
#[test]
fn diagnostic_recreation_with_applied_suspend() {
    suspended_recovery(|s| s.diagnostic_recreate_output().unwrap(), true);
}
