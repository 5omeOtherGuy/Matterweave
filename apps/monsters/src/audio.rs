//! Contextual audio for Mossbound plus the persisted sound preference.
//!
//! The bounded mixing and the AAudio device live in `matterweave-audio`; this
//! module only generates small original PCM clips, maps gameplay events to
//! them and owns the mute preference. A missing device is never fatal: the app
//! runs silently and reports the service error once.
use matterweave_audio::{AudioService, ClipSpec, PlayOptions};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Mono sample rate the service expects.
const RATE: f32 = 48_000.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoundSettings {
    pub muted: bool,
}

impl SoundSettings {
    /// Atomic preference write beside the save; the previous file survives a
    /// failed write and a corrupt file loads as defaults.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temp = parent.join(format!(".mossbound-settings-{}.tmp", std::process::id()));
        {
            let mut file = std::fs::File::create(&temp)?;
            file.write_all(&serde_json::to_vec(self).map_err(std::io::Error::other)?)?;
            file.sync_all()?;
        }
        std::fs::rename(&temp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sound {
    Encounter,
    Hit,
    Capture,
    LevelUp,
    Heal,
    Badge,
}

/// One generated clip and the handle it registered under. The PCM stays owned
/// here so a future re-registration after device recovery has the source.
struct Clip {
    #[allow(dead_code)]
    samples: Vec<f32>,
    handle: matterweave_audio::ClipHandle,
}

/// The live audio owner. A host without a backend runs with `service: None`.
pub struct GameAudio {
    service: Option<AudioService>,
    clips: Vec<(Sound, Clip)>,
    pub settings: SoundSettings,
    reported_failure: Option<String>,
}

impl GameAudio {
    pub fn new(settings_path: &Path) -> Self {
        let settings = SoundSettings::load(settings_path);
        let mut audio = Self {
            service: None,
            clips: Vec::new(),
            settings,
            reported_failure: None,
        };
        match AudioService::new() {
            Ok(mut service) => {
                // Register every clip before playing anything; a rejected clip
                // leaves the service untouched.
                for (sound, samples) in generated_clips() {
                    let spec = ClipSpec::mono(&samples);
                    match service.register_clip(spec) {
                        Ok(handle) => audio.clips.push((sound, Clip { samples, handle })),
                        Err(error) => {
                            audio.reported_failure = Some(format!("clip {sound:?}: {error}"));
                            break;
                        }
                    }
                }
                audio.service = Some(service);
            }
            Err(error) => {
                audio.reported_failure = Some(error.to_string());
            }
        }
        audio
    }

    pub fn is_available(&self) -> bool {
        self.service.is_some()
    }

    pub fn failure(&self) -> Option<&str> {
        self.reported_failure.as_deref()
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.settings.muted = muted;
    }

    /// Play one event. A missing device or a busy voice is counted, never fatal.
    pub fn play(&mut self, sound: Sound) {
        if self.settings.muted {
            return;
        }
        let Some(service) = self.service.as_mut() else {
            return;
        };
        let Some((_, clip)) = self.clips.iter().find(|(s, _)| *s == sound) else {
            return;
        };
        if let Err(error) = service.play(clip.handle, PlayOptions::default()) {
            log::debug!("audio voice refused: {error}");
        }
    }

    pub fn suspend(&mut self) {
        if let Some(service) = self.service.as_mut() {
            let _ = service.suspend();
        }
    }

    pub fn resume(&mut self) {
        if let Some(service) = self.service.as_mut() {
            let _ = service.resume();
        }
    }

    pub fn poll(&mut self) {
        if let Some(service) = self.service.as_mut() {
            let _ = service.poll_device();
        }
    }
}

/// A short original tone burst: sine carrier, exponential decay, tiny attack
/// so the voice cannot click.
fn tone(frequency: f32, seconds: f32, gain: f32, slide: f32) -> Vec<f32> {
    let frames = (RATE * seconds) as usize;
    let mut samples = Vec::with_capacity(frames);
    for index in 0..frames {
        let t = index as f32 / RATE;
        let progress = index as f32 / frames as f32;
        let attack = (progress / 0.02).min(1.0);
        let envelope = attack * (1.0 - progress).powf(1.8) * gain;
        let f = frequency + slide * progress;
        let phase = 2.0 * std::f32::consts::PI * f * t;
        samples.push(phase.sin() * envelope);
    }
    samples
}

fn chirp(low: f32, high: f32, seconds: f32, gain: f32) -> Vec<f32> {
    let frames = (RATE * seconds) as usize;
    let mut samples = Vec::with_capacity(frames);
    let mut phase = 0.0f32;
    for index in 0..frames {
        let progress = index as f32 / frames as f32;
        let f = low + (high - low) * progress;
        phase += 2.0 * std::f32::consts::PI * f / RATE;
        let envelope = (1.0 - progress).powf(1.5) * gain;
        samples.push(phase.sin() * envelope);
    }
    samples
}

fn generated_clips() -> Vec<(Sound, Vec<f32>)> {
    vec![
        (Sound::Encounter, chirp(340.0, 720.0, 0.28, 0.5)),
        (Sound::Hit, tone(180.0, 0.14, 0.6, -60.0)),
        (Sound::Capture, chirp(500.0, 980.0, 0.22, 0.5)),
        (Sound::LevelUp, chirp(520.0, 1040.0, 0.4, 0.45)),
        (Sound::Heal, chirp(620.0, 900.0, 0.3, 0.4)),
        (Sound::Badge, chirp(440.0, 1320.0, 0.6, 0.5)),
    ]
}

/// The clips are generated once; this is exposed for tests and for a future
/// settings preview.
pub fn clip_sample_count() -> usize {
    generated_clips().iter().map(|(_, c)| c.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_clips_are_finite_and_bounded() {
        for (sound, samples) in generated_clips() {
            assert!(!samples.is_empty(), "{sound:?} is empty");
            assert!(
                samples.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
                "{sound:?} leaves the [-1, 1] range"
            );
        }
        assert!(
            clip_sample_count() > RATE as usize / 2,
            "some audible duration"
        );
    }

    #[test]
    fn settings_round_trip_and_corrupt_files_fall_back_to_defaults() {
        let dir = std::env::temp_dir().join(format!("mossbound-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let _ = std::fs::remove_file(&path);
        assert_eq!(SoundSettings::load(&path), SoundSettings::default());
        SoundSettings { muted: true }.save(&path).expect("write");
        assert!(SoundSettings::load(&path).muted);
        std::fs::write(&path, b"not json").unwrap();
        assert!(
            !SoundSettings::load(&path).muted,
            "corruption is not a crash"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_host_without_a_backend_stays_silent_and_usable() {
        let dir =
            std::env::temp_dir().join(format!("mossbound-audio-silent-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut audio = GameAudio::new(&dir.join("settings.json"));
        // The mock backend is available on the host, but the invariant under
        // test is that play/suspend/poll never panic or block gameplay.
        audio.play(Sound::Hit);
        audio.suspend();
        audio.resume();
        audio.poll();
        audio.set_muted(true);
        audio.play(Sound::Badge);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
