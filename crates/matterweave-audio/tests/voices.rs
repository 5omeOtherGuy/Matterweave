//! Voice handle lifetime, stopping, slot reuse and the bounded voice table.

use matterweave_audio::{
    AudioConfig, AudioError, AssetHandle, AudioService, PcmSource, MAX_VOICES,
};

fn service() -> AudioService {
    AudioService::new(AudioConfig::default()).expect("default config is valid")
}

fn load(audio: &mut AudioService, samples: &[f32]) -> AssetHandle {
    audio
        .load_pcm(&PcmSource {
            sample_rate: 48_000,
            channels: 1,
            samples,
        })
        .expect("valid asset")
}

#[test]
fn voice_ends_at_the_end_of_its_asset() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.5, 0.5, 0.5, 0.5]);
    let voice = audio.play(asset, 1.0).expect("voice available");

    let mut out = [0.0f32; 3];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(3));
    assert!(audio.is_active(voice), "asset is not finished");
    assert_eq!(out, [0.5, 0.5, 0.5]);

    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out, [0.5], "the final frame still plays");
    assert!(!audio.is_active(voice), "voice released after last frame");
    assert_eq!(audio.active_voices(), 0);

    let mut out = [0.0f32; 2];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(2));
    assert_eq!(out, [0.0, 0.0]);
}

#[test]
fn stop_ends_a_voice_immediately() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.5; 8]);
    let voice = audio.play(asset, 1.0).expect("voice available");
    let mut out = [0.0f32; 2];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(2));
    assert_eq!(out, [0.5, 0.5]);

    assert!(audio.stop(voice));
    assert!(!audio.is_active(voice));
    assert!(!audio.stop(voice), "second stop is a no-op");

    let mut out = [0.0f32; 4];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(4));
    assert_eq!(out, [0.0; 4], "stopped voice contributes silence");
}

#[test]
fn a_reused_slot_invalidates_the_old_handle() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.25; 16]);
    let first = audio.play(asset, 1.0).expect("voice available");
    assert!(audio.stop(first));

    let second = audio.play(asset, 1.0).expect("voice available");
    assert_ne!(first, second, "reused slot must carry a new generation");
    assert!(audio.is_active(second));
    assert!(!audio.is_active(first), "stale handle is never active");
    assert!(!audio.stop(first), "stale handle cannot stop a new voice");
    assert_eq!(
        audio.set_gain(first, 0.5).err(),
        Some(AudioError::InvalidHandle)
    );
    assert!(audio.set_gain(second, 0.5).is_ok());
}

#[test]
fn voice_table_is_bounded_and_reclaims_finished_voices() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.1; 4]);
    for _ in 0..MAX_VOICES {
        audio.play(asset, 1.0).expect("voice available");
    }
    assert_eq!(audio.active_voices(), MAX_VOICES);
    assert_eq!(
        audio.play(asset, 1.0).err(),
        Some(AudioError::VoiceLimit),
        "no unbounded queue: extra play requests fail instead of queueing"
    );

    let mut out = [0.0f32; 4];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(4));
    assert_eq!(audio.active_voices(), 0, "all voices finished together");
    audio.play(asset, 1.0).expect("slots reclaimed");
}

#[test]
fn stop_all_clears_every_voice() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.1; 64]);
    for _ in 0..4 {
        audio.play(asset, 1.0).expect("voice available");
    }
    audio.stop_all();
    assert_eq!(audio.active_voices(), 0);
    let mut out = [0.0f32; 8];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(8));
    assert_eq!(out, [0.0; 8]);
}

#[test]
fn set_gain_changes_the_mixed_output() {
    let mut audio = service();
    let asset = load(&mut audio, &[0.5; 4]);
    let voice = audio.play(asset, 1.0).expect("voice available");
    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out, [0.5]);

    audio.set_gain(voice, 0.25).expect("voice is live");
    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out, [0.125]);
}

#[test]
fn rejects_unknown_asset_handles() {
    let mut audio = service();
    let bogus = AssetHandle::INVALID;
    assert_eq!(
        audio.play(bogus, 1.0).err(),
        Some(AudioError::InvalidHandle)
    );
}
