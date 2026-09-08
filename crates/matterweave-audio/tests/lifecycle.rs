//! Service lifecycle: creation, device open failure reporting, suspend/resume and
//! the offline render guard. Device-open success paths are Android-only.

use matterweave_audio::{AudioConfig, AudioError, AudioService, PcmSource, StreamStatus};

#[test]
fn new_service_starts_closed_with_the_requested_config() {
    let config = AudioConfig {
        sample_rate: 44_100,
        channels: 1,
    };
    let audio = AudioService::new(config).expect("valid config");
    assert_eq!(audio.config(), config);
    assert!(!audio.is_open());
    assert_eq!(audio.status(), StreamStatus::Closed);
    assert_eq!(audio.stream_info(), None);
    assert_eq!(audio.last_error(), None);
}

#[test]
fn device_output_is_reported_unavailable_off_android() {
    let mut audio = AudioService::new(AudioConfig::default()).expect("valid config");
    if cfg!(target_os = "android") {
        // With a device present this must succeed and report a negotiated format.
        let info = audio.open().expect("android output opens");
        assert!(info.sample_rate > 0);
        assert!(info.channels > 0);
        return;
    }
    assert_eq!(audio.open().err(), Some(AudioError::Unsupported));
    assert_eq!(audio.status(), StreamStatus::Unavailable);
    assert_eq!(audio.last_error(), Some(AudioError::Unsupported));
    assert!(!audio.is_open());
    assert_eq!(audio.stream_info(), None);
}

#[test]
fn suspend_and_resume_report_unavailable_off_android() {
    let mut audio = AudioService::new(AudioConfig::default()).expect("valid config");
    audio.suspend();
    assert_eq!(audio.status(), StreamStatus::Closed);
    if cfg!(target_os = "android") {
        let info = audio.resume().expect("android output reopens");
        assert!(info.sample_rate > 0);
        return;
    }
    assert_eq!(audio.resume().err(), Some(AudioError::Unsupported));
    assert_eq!(audio.status(), StreamStatus::Unavailable);
}

#[test]
fn control_plane_and_offline_render_survive_a_failed_open() {
    let mut audio = AudioService::new(AudioConfig::default()).expect("valid config");
    let asset = audio
        .load_pcm(&PcmSource {
            sample_rate: 48_000,
            channels: 1,
            samples: &[0.5, 0.5],
        })
        .expect("valid asset");
    let _ = audio.open();
    let voice = audio.play(asset, 0.5).expect("voice available");
    let mut out = [0.0f32; 2];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(2));
    assert_eq!(out, [0.25, 0.25]);
    assert!(audio.stop(voice));
    audio.close();
    assert_eq!(audio.status(), StreamStatus::Closed);
}
