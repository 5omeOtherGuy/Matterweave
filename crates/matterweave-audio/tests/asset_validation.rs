//! PCM asset acceptance and rejection bounds for `matterweave-audio`.
//!
//! These tests run on the host. They cover the load path only; device output is
//! Android-only and is covered by `examples/device_output.rs`.

use matterweave_audio::{
    AudioConfig, AudioError, AudioService, PcmSource, MAX_ASSETS, MAX_ASSET_FRAMES, MAX_CHANNELS,
    MAX_SAMPLE_RATE, MAX_TOTAL_SAMPLES, MIN_SAMPLE_RATE,
};

fn service() -> AudioService {
    AudioService::new(AudioConfig::default()).expect("default config is valid")
}

fn mono(samples: &[f32], rate: u32) -> PcmSource<'_> {
    PcmSource {
        sample_rate: rate,
        channels: 1,
        samples,
    }
}

#[test]
fn accepts_a_bounded_mono_asset() {
    let mut audio = service();
    let data = [0.0f32, 0.5, -0.5, 1.0];
    let handle = audio.load_pcm(&mono(&data, 48_000)).expect("valid asset");
    assert!(audio.is_asset_loaded(handle));
}

#[test]
fn rejects_zero_and_out_of_range_sample_rates() {
    let mut audio = service();
    let data = [0.0f32, 1.0];
    for rate in [0u32, 1, MIN_SAMPLE_RATE - 1, MAX_SAMPLE_RATE + 1, u32::MAX] {
        let result = audio.load_pcm(&mono(&data, rate));
        assert_eq!(result.err(), Some(AudioError::InvalidAsset), "rate {rate}");
    }
    assert!(audio.load_pcm(&mono(&data, MIN_SAMPLE_RATE)).is_ok());
    assert!(audio.load_pcm(&mono(&data, MAX_SAMPLE_RATE)).is_ok());
}

#[test]
fn rejects_invalid_channel_counts() {
    let mut audio = service();
    let data = [0.0f32, 1.0];
    for channels in [0u16, MAX_CHANNELS + 1, u16::MAX] {
        let result = audio.load_pcm(&PcmSource {
            sample_rate: 48_000,
            channels,
            samples: &data,
        });
        assert_eq!(
            result.err(),
            Some(AudioError::InvalidAsset),
            "channels {channels}"
        );
    }
}

#[test]
fn rejects_empty_oversized_and_misaligned_assets() {
    let mut audio = service();
    let empty: [f32; 0] = [];
    assert_eq!(
        audio.load_pcm(&mono(&empty, 48_000)).err(),
        Some(AudioError::InvalidAsset)
    );

    let oversized = vec![0.0f32; (MAX_ASSET_FRAMES + 1) as usize];
    assert_eq!(
        audio.load_pcm(&mono(&oversized, 48_000)).err(),
        Some(AudioError::InvalidAsset)
    );

    let misaligned = [0.0f32, 1.0, 0.5];
    let result = audio.load_pcm(&PcmSource {
        sample_rate: 48_000,
        channels: 2,
        samples: &misaligned,
    });
    assert_eq!(result.err(), Some(AudioError::InvalidAsset));
}

#[test]
fn rejects_nonfinite_samples() {
    let mut audio = service();
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let data = [0.0f32, bad, 0.0];
        assert_eq!(
            audio.load_pcm(&mono(&data, 48_000)).err(),
            Some(AudioError::InvalidAsset)
        );
    }
}

#[test]
fn rejects_more_assets_than_slots() {
    let mut audio = service();
    let data = [0.25f32, 0.5];
    for _ in 0..MAX_ASSETS {
        audio.load_pcm(&mono(&data, 48_000)).expect("slot available");
    }
    assert_eq!(
        audio.load_pcm(&mono(&data, 48_000)).err(),
        Some(AudioError::AssetLimit)
    );
}

#[test]
fn rejects_assets_that_exhaust_the_sample_arena() {
    let mut audio = service();
    let frames = MAX_ASSET_FRAMES as usize;
    let stereo = vec![0.0f32; frames * 2];
    let source = PcmSource {
        sample_rate: 48_000,
        channels: 2,
        samples: &stereo,
    };
    let per_asset = frames * 2;
    let fits = MAX_TOTAL_SAMPLES / per_asset;
    assert!(fits < MAX_ASSETS, "fixture must exhaust the arena first");
    for _ in 0..fits {
        audio.load_pcm(&source).expect("arena has room");
    }
    assert_eq!(audio.load_pcm(&source).err(), Some(AudioError::AssetLimit));
}

#[test]
fn rejects_invalid_service_configuration() {
    for rate in [0u32, MIN_SAMPLE_RATE - 1, MAX_SAMPLE_RATE + 1] {
        let config = AudioConfig {
            sample_rate: rate,
            ..AudioConfig::default()
        };
        assert_eq!(
            AudioService::new(config).err(),
            Some(AudioError::InvalidConfig),
            "rate {rate}"
        );
    }
    for channels in [0u16, MAX_CHANNELS + 1] {
        let config = AudioConfig {
            channels,
            ..AudioConfig::default()
        };
        assert_eq!(
            AudioService::new(config).err(),
            Some(AudioError::InvalidConfig),
            "channels {channels}"
        );
    }
}
