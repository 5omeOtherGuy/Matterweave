use super::*;

// MockOutput is synchronous: only this invocation can borrow the core. The closure
// models a control operation after FIFO drain, before callback publication; it
// must not reenter rendering, replace output, or otherwise access the core.
fn render_with_enqueue(
    service: &mut AudioService,
    after_drain: impl FnOnce(&mut AudioService),
) -> [f32; 2] {
    let ptr = service.core.ptr();
    let mut out = [0.0; 2];
    // SAFETY: the mock is synchronous and closure operations only touch separate
    // control state, atomics, FIFO and independently owned PCM, never the core.
    unsafe { (*ptr.0).render_with_after_drain(&mut out, 2, || after_drain(service)) };
    out
}

#[test]
fn failed_pause_near_full_fifo_keeps_exact_pcm_continuation() {
    for vacant in [0, 1, 2] {
        let mut service = playing();
        service
            .recreate_output_with(|core, shared| {
                Ok(Box::new(FaultOutput {
                    inner: MockOutput::open(core, shared)?,
                    starts: Arc::new(AtomicUsize::new(0)),
                    fail_start: false,
                    fail_suspend: true,
                    error_on_close: None,
                }))
            })
            .unwrap();
        let voice = VoiceHandle {
            slot: 0,
            generation: service.voice_mirror[0].generation,
        };
        while queue::vacant(&service.producer) > vacant {
            service.set_voice_gain(voice, 1.0).unwrap();
        }
        let result = service.suspend();
        assert!(service.running);
        assert!(!service.health().suspended);
        // Check PCM first: the original one-slot case reports pause failure yet
        // silently freezes the core because compensating Resume was dropped.
        assert_continuation(&mut service);
        assert_eq!(
            result,
            Err(if vacant < 2 {
                AudioServiceError::CommandQueueFull
            } else {
                AudioServiceError::SuspendFailed { code: -3 }
            })
        );
        assert_eq!(service.health().pending_commands, 0);
        assert!(!service.health().rt_suspended);
    }
}

#[test]
fn callback_finishing_after_unload_enqueue_is_not_unload_acknowledgment() {
    let mut service = AudioService::new().unwrap();
    let samples = vec![0.25; MAX_PCM_BYTES / 4];
    let clip = service.register_clip(ClipSpec::mono(&samples)).unwrap();
    service.play(clip, PlayOptions::default()).unwrap();
    service.mock_render(&mut [0.0; 2]).unwrap();
    let before = service.health();
    let out = render_with_enqueue(&mut service, |s| s.unregister_clip(clip).unwrap());
    assert_eq!(out, [0.25; 2], "Unload arrived after this callback's drain");
    assert!(service.health().ack_epoch > before.ack_epoch);
    assert_eq!(service.health().commands_applied, before.commands_applied);
    assert_eq!(
        service.register_clip(ClipSpec::mono(&samples)),
        Err(AudioServiceError::RangeNotYetAcknowledged),
        "finishing the already-drained callback must not allow PCM overwrite"
    );
    let mut stopped = [1.0; 2];
    service.mock_render(&mut stopped).unwrap();
    assert_eq!(stopped, [0.0; 2]);
    let replacement = service.register_clip(ClipSpec::mono(&samples)).unwrap();
    service.play(replacement, PlayOptions::default()).unwrap();
    service.mock_render(&mut stopped).unwrap();
    assert_eq!(stopped, [0.25; 2]);
    assert_eq!(service.health().rt_rejected_commands, 0);
}

#[test]
fn command_acknowledgment_is_not_published_before_callback_finishes() {
    let mut service = playing();
    let before = service.health().commands_applied;
    service.push(Command::Suspend).unwrap();
    render_with_enqueue(&mut service, |s| {
        assert_eq!(
            s.health().commands_applied,
            before,
            "drained commands are not yet end-of-callback acknowledgments"
        );
    });
    assert_eq!(service.health().commands_applied, before + 1);
}

#[test]
fn callback_finishing_before_queued_plays_apply_must_not_free_voice_slots() {
    let mut service = AudioService::new().unwrap();
    let clip = service
        .register_clip(ClipSpec::mono(&[0.0625; 16]))
        .unwrap();
    service.mock_render(&mut [0.0; 2]).unwrap();
    assert_eq!(
        render_with_enqueue(&mut service, |s| {
            for _ in 0..MAX_VOICES {
                s.play(clip, PlayOptions::default()).unwrap();
            }
        }),
        [0.0; 2]
    );
    assert_eq!(
        service.play(clip, PlayOptions::default()),
        Err(AudioServiceError::VoiceLimit),
        "a callback epoch cannot acknowledge Play queued after its drain"
    );
    let mut mixed = [0.0; 2];
    service.mock_render(&mut mixed).unwrap();
    assert_eq!(mixed, [0.5; 2]);
    assert_eq!(service.health().rt_rejected_commands, 0);
}

#[test]
fn stop_of_play_queued_after_drain_must_not_be_mistaken_for_finished_voice() {
    let mut service = AudioService::new().unwrap();
    let clip = service.register_clip(ClipSpec::mono(&[0.25; 16])).unwrap();
    service.mock_render(&mut [0.0; 2]).unwrap();
    let mut voice = None;
    render_with_enqueue(&mut service, |s| {
        voice = Some(s.play(clip, PlayOptions::default()).unwrap());
    });
    service.stop_voice(voice.unwrap()).unwrap();
    let mut out = [1.0; 2];
    service.mock_render(&mut out).unwrap();
    assert_eq!(out, [0.0; 2], "Stop must follow the still-buffered Play");
    assert_eq!(service.health().rt_rejected_commands, 0);
}

#[test]
fn reused_voice_handle_does_not_observe_old_generations_live_bit() {
    let mut service = AudioService::new().unwrap();
    let old_clip = service.register_clip(ClipSpec::mono(&[0.125; 16])).unwrap();
    let new_clip = service.register_clip(ClipSpec::mono(&[0.25; 16])).unwrap();
    let old_voice = service.play(old_clip, PlayOptions::default()).unwrap();
    service.mock_render(&mut [0.0; 2]).unwrap();
    assert!(service.is_voice_active(old_voice));
    service.unregister_clip(old_clip).unwrap();
    let new_voice = service.play(new_clip, PlayOptions::default()).unwrap();
    assert!(
        !service.is_voice_active(new_voice),
        "new Play is not applied: live bit belongs to old generation"
    );
    assert_eq!(
        service.stop_voice(old_voice),
        Err(AudioServiceError::StaleVoiceHandle)
    );
    let mut out = [0.0; 2];
    service.mock_render(&mut out).unwrap();
    assert_eq!(out, [0.25; 2]);
    assert!(service.is_voice_active(new_voice));
    assert_eq!(service.health().rt_rejected_commands, 0);
}
