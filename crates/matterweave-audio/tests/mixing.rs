//! Deterministic mixing, gain, resampling, channel mapping and saturation.

use matterweave_audio::{
    AudioConfig, AudioError, AudioService, PcmSource, MAX_CHANNELS, MAX_GAIN,
};

fn service(rate: u32) -> AudioService {
    let config = AudioConfig {
        sample_rate: rate,
        channels: 1,
    };
    AudioService::new(config).expect("valid config")
}

fn load(audio: &mut AudioService, rate: u32, channels: u16, samples: &[f32]) -> matterweave_audio::AssetHandle {
    audio
        .load_pcm(&PcmSource {
            sample_rate: rate,
            channels,
            samples,
        })
        .expect("valid asset")
}

#[test]
fn sums_voices_with_gain_and_saturates() {
    let mut audio = service(48_000);
    let asset = load(&mut audio, 48_000, 1, &[1.0]);
    audio.play(asset, 1.0).expect("voice available");
    audio.play(asset, 0.5).expect("voice available");
    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out[0], 1.0, "1.0 + 0.5 must saturate to full scale");

    let mut audio = service(48_000);
    let asset = load(&mut audio, 48_000, 1, &[1.0]);
    audio.play(asset, 0.5).expect("voice available");
    audio.play(asset, 0.25).expect("voice available");
    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out[0], 0.75);
}

#[test]
fn saturates_negative_sum_and_reports_no_voice_drift() {
    let mut audio = service(48_000);
    let asset = load(&mut audio, 48_000, 1, &[-1.0]);
    audio.play(asset, 4.0).expect("voice available");
    audio.play(asset, 4.0).expect("voice available");
    let mut out = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(1));
    assert_eq!(out[0], -1.0);
    assert_eq!(audio.active_voices(), 0, "one-frame asset finished");
}

#[test]
fn mixing_is_deterministic_bit_for_bit() {
    let samples: Vec<f32> = (0..256).map(|i| (i as f32 / 256.0) - 0.5).collect();
    let render = || {
        let mut audio = service(48_000);
        let asset = load(&mut audio, 48_000, 1, &samples);
        let fast = load(&mut audio, 24_000, 1, &[0.5, -0.5]);
        audio.play(asset, 0.3).expect("voice available");
        audio.play(fast, 0.7).expect("voice available");
        let mut out = vec![0.0f32; 400];
        assert_eq!(audio.render_offline(&mut out, 1), Ok(400));
        out
    };
    let first = render();
    let second = render();
    assert_eq!(first, second);
    assert!(first.iter().any(|s| *s != 0.0), "expected audible output");
}

#[test]
fn resamples_a_slower_asset_with_linear_interpolation() {
    let mut audio = service(48_000);
    let asset = load(&mut audio, 24_000, 1, &[0.0, 1.0]);
    audio.play(asset, 1.0).expect("voice available");
    let mut out = [0.0f32; 5];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(5));
    assert_eq!(
        out,
        [0.0, 0.5, 1.0, 1.0, 0.0],
        "24 kHz asset on a 48 kHz output plays at half speed with interpolated samples"
    );
}

#[test]
fn maps_mono_assets_to_stereo_and_downmixes_stereo_to_mono() {
    let mut audio = service(48_000);
    let mono_asset = load(&mut audio, 48_000, 1, &[1.0]);
    audio.play(mono_asset, 1.0).expect("voice available");
    let mut stereo = [0.0f32; 2];
    assert_eq!(audio.render_offline(&mut stereo, 2), Ok(1));
    assert_eq!(stereo, [1.0, 1.0]);

    let mut audio = service(48_000);
    let stereo_asset = load(&mut audio, 48_000, 2, &[1.0, 0.0]);
    audio.play(stereo_asset, 1.0).expect("voice available");
    let mut mono = [0.0f32; 1];
    assert_eq!(audio.render_offline(&mut mono, 1), Ok(1));
    assert_eq!(mono, [0.5]);
}

#[test]
fn zero_gain_and_stopped_voices_are_silent() {
    let mut audio = service(48_000);
    let asset = load(&mut audio, 48_000, 1, &[1.0, 1.0, 1.0]);
    let voice = audio.play(asset, 0.0).expect("voice available");
    let mut out = [0.0f32; 3];
    assert_eq!(audio.render_offline(&mut out, 1), Ok(3));
    assert_eq!(out, [0.0, 0.0, 0.0]);
    assert!(audio.stop(voice));
}

#[test]
fn rejects_invalid_gain_values() {
    let mut audio = service(48_000);
    let asset = load(&mut audio, 48_000, 1, &[1.0]);
    for gain in [f32::NAN, f32::INFINITY, -0.1, MAX_GAIN + 0.1] {
        assert_eq!(
            audio.play(asset, gain).err(),
            Some(AudioError::InvalidGain),
            "gain {gain}"
        );
    }
    assert!(audio.play(asset, MAX_GAIN).is_ok());
    assert!(audio.play(asset, 0.0).is_ok());
}

#[test]
fn rejects_invalid_render_requests() {
    let mut audio = service(48_000);
    let mut out = [0.0f32; 4];
    for channels in [0u16, MAX_CHANNELS + 1] {
        assert_eq!(
            audio.render_offline(&mut out, channels).err(),
            Some(AudioError::InvalidConfig)
        );
    }
    let mut odd = [0.0f32; 3];
    assert_eq!(
        audio.render_offline(&mut odd, 2).err(),
        Some(AudioError::InvalidConfig)
    );
    let mut empty: [f32; 0] = [];
    assert_eq!(audio.render_offline(&mut empty, 1), Ok(0));
}
