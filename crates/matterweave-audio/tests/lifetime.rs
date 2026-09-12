//! PCM output and reuse, rather than just control-side handle mirrors.
use matterweave_audio::{AudioService, AudioServiceError, ClipSpec, PlayOptions, MAX_PCM_BYTES};

fn frame(service: &mut AudioService) -> [f32; 2] {
    let mut out = [0.0; 2];
    service.mock_render(&mut out).unwrap();
    out
}

#[test]
fn unregister_silences_only_voices_using_that_clip() {
    let mut service = AudioService::new().unwrap();
    let removed = service.register_clip(ClipSpec::mono(&[0.125; 32])).unwrap();
    let retained = service.register_clip(ClipSpec::mono(&[0.25; 32])).unwrap();
    service.play(removed, PlayOptions::default()).unwrap();
    service.play(removed, PlayOptions::default()).unwrap();
    let retained_voice = service.play(retained, PlayOptions::default()).unwrap();
    assert_eq!(frame(&mut service), [0.5; 2]);
    service.unregister_clip(removed).unwrap();
    assert_eq!(frame(&mut service), [0.25; 2]);
    assert_eq!(service.health().voices_silenced, 2);
    assert_eq!(service.health().rt_rejected_commands, 0);
    assert!(service.is_voice_active(retained_voice));
}

#[test]
fn full_pool_reuse_waits_for_unload_and_stale_handles_cannot_touch_replacement() {
    let mut service = AudioService::new().unwrap();
    let samples = vec![0.25; MAX_PCM_BYTES / 4];
    let old_clip = service.register_clip(ClipSpec::mono(&samples)).unwrap();
    let old_voice = service.play(old_clip, PlayOptions::default()).unwrap();
    assert_eq!(frame(&mut service), [0.25; 2]);
    service.unregister_clip(old_clip).unwrap();
    assert_eq!(
        service.register_clip(ClipSpec::mono(&samples)),
        Err(AudioServiceError::RangeNotYetAcknowledged)
    );
    assert_eq!(
        frame(&mut service),
        [0.0; 2],
        "old PCM must stop before reuse"
    );
    let replacement_samples = vec![-0.125; MAX_PCM_BYTES / 4];
    let replacement = service
        .register_clip(ClipSpec::mono(&replacement_samples))
        .unwrap();
    let replacement_voice = service.play(replacement, PlayOptions::default()).unwrap();
    assert_eq!(
        service.unregister_clip(old_clip),
        Err(AudioServiceError::StaleClipHandle)
    );
    assert_eq!(
        service.play(old_clip, PlayOptions::default()),
        Err(AudioServiceError::StaleClipHandle)
    );
    assert_eq!(
        service.stop_voice(old_voice),
        Err(AudioServiceError::StaleVoiceHandle)
    );
    assert_eq!(frame(&mut service), [-0.125; 2]);
    assert!(service.is_voice_active(replacement_voice));
    assert_eq!(service.health().rt_rejected_commands, 0);
    assert_eq!(service.health().voices_silenced, 1);
}
