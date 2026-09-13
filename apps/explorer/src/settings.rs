//! Shared, bounded, versioned application preferences.
//!
//! These preferences are deliberately separate from every game save: the
//! wetland snapshot, the sandbox session and the Voxel Relay snapshot never
//! read or write this file, and a malformed or oversized file only falls back
//! to defaults. The owning application (`Experience`) is the one writer and
//! the one audio-policy owner; samples render the same values and ask the
//! owner to persist changes through their one-shot change queue.
use serde::{Deserialize, Serialize};
use std::{
    fmt, fs,
    io::Write,
    path::{Path, PathBuf},
};

/// File name used inside the application data directory. It never shares a path
/// with `world.json`, its session-recovery files or the Relay snapshot.
pub const SETTINGS_FILE_NAME: &str = "matterweave-settings.json";
/// Only this version may be restored; any other version falls back to defaults.
pub const SETTINGS_VERSION: u32 = 1;
/// Hard bound on the encoded settings file. The document is small and fixed, so
/// anything larger is corrupt input rather than a preference to preserve.
pub const MAX_SETTINGS_BYTES: u64 = 4096;

/// Which side holds the movement stick.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Left,
    Right,
}

impl Handedness {
    pub fn toggled(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

/// One shared preference row. The value text is user-facing; the enum stays
/// free of pixels so both samples can share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRow {
    Handedness,
    LargeControls,
    Mute,
}

impl SettingRow {
    pub fn title(self) -> &'static str {
        match self {
            Self::Handedness => "MOVE STICK",
            Self::LargeControls => "CONTROL SIZE",
            Self::Mute => "SOUND",
        }
    }
}

/// The bounded, versioned shared preference document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedSettings {
    pub version: u32,
    pub handedness: Handedness,
    pub large_controls: bool,
    pub muted: bool,
}

impl Default for SharedSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            handedness: Handedness::Left,
            large_controls: false,
            muted: false,
        }
    }
}

impl SharedSettings {
    /// Read the settings file, falling back to defaults for a missing, invalid,
    /// oversized or unsupported file. Startup must never fail because of a
    /// preference, so this is the only entry point applications need.
    pub fn load(path: &Path) -> Self {
        match Self::read(path) {
            Ok(settings) => settings,
            Err(error) => {
                if path.exists() {
                    log::warn!(
                        "Shared settings at {} ignored ({error}); defaults in effect",
                        path.display()
                    );
                }
                Self::default()
            }
        }
    }

    /// Strict reader used by [`SharedSettings::load`] and the tests. It never
    /// touches a game save.
    pub fn read(path: &Path) -> Result<Self, SettingsError> {
        let metadata = fs::metadata(path).map_err(SettingsError::Io)?;
        if metadata.len() > MAX_SETTINGS_BYTES {
            return Err(SettingsError::TooLarge(metadata.len()));
        }
        let bytes = fs::read(path).map_err(SettingsError::Io)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(SettingsError::TooLarge(bytes.len() as u64));
        }
        let settings: Self =
            serde_json::from_slice(&bytes).map_err(|e| SettingsError::Decode(e.to_string()))?;
        if settings.version != SETTINGS_VERSION {
            return Err(SettingsError::UnsupportedVersion(settings.version));
        }
        Ok(settings)
    }

    /// Atomically replace the settings file: encode, write a sibling temporary
    /// file, flush it, then rename it over the target. A crash leaves either the
    /// previous document or the new one, never a partial file. No game save is
    /// opened for writing here.
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        let mut document = *self;
        document.version = SETTINGS_VERSION;
        let encoded = serde_json::to_vec_pretty(&document)
            .map_err(|e| SettingsError::Encode(e.to_string()))?;
        if encoded.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(SettingsError::Encode(format!(
                "encoded settings are {} bytes, above the {MAX_SETTINGS_BYTES}-byte bound",
                encoded.len()
            )));
        }
        let temporary: PathBuf = path.with_extension("tmp");
        {
            let mut file = fs::File::create(&temporary).map_err(SettingsError::Io)?;
            file.write_all(&encoded).map_err(SettingsError::Io)?;
            file.sync_all().map_err(SettingsError::Io)?;
        }
        fs::rename(&temporary, path).map_err(SettingsError::Io)
    }

    /// Apply one panel row. Purely in-memory: the owner persists after syncing.
    pub fn toggle(&mut self, row: SettingRow) {
        match row {
            SettingRow::Handedness => self.handedness = self.handedness.toggled(),
            SettingRow::LargeControls => self.large_controls = !self.large_controls,
            SettingRow::Mute => self.muted = !self.muted,
        }
    }

    /// Current user-facing value text for one panel row.
    pub fn value_label(&self, row: SettingRow) -> &'static str {
        match row {
            SettingRow::Handedness => match self.handedness {
                Handedness::Left => "LEFT",
                Handedness::Right => "RIGHT",
            },
            SettingRow::LargeControls => {
                if self.large_controls {
                    "LARGE"
                } else {
                    "NORMAL"
                }
            }
            SettingRow::Mute => {
                if self.muted {
                    "MUTED"
                } else {
                    "ON"
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum SettingsError {
    Io(std::io::Error),
    TooLarge(u64),
    Encode(String),
    Decode(String),
    UnsupportedVersion(u32),
}

impl fmt::Display for SettingsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "settings file I/O failed: {error}"),
            Self::TooLarge(bytes) => write!(
                f,
                "settings file is {bytes} bytes, above the {MAX_SETTINGS_BYTES}-byte bound"
            ),
            Self::Encode(error) => write!(f, "settings could not be encoded: {error}"),
            Self::Decode(error) => write!(f, "settings could not be decoded: {error}"),
            Self::UnsupportedVersion(version) => {
                write!(f, "settings version {version} is not supported")
            }
        }
    }
}

impl std::error::Error for SettingsError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    fn temp_dir() -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "matterweave-settings-test-{}-{}",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn defaults_match_the_existing_sample_layout_and_audio() {
        let defaults = SharedSettings::default();
        assert_eq!(defaults.version, SETTINGS_VERSION);
        assert_eq!(defaults.handedness, Handedness::Left);
        assert!(!defaults.large_controls);
        assert!(!defaults.muted);
        assert_eq!(defaults.value_label(SettingRow::Handedness), "LEFT");
        assert_eq!(defaults.value_label(SettingRow::LargeControls), "NORMAL");
        assert_eq!(defaults.value_label(SettingRow::Mute), "ON");
    }

    #[test]
    fn round_trip_preserves_values_and_touches_no_game_save() {
        let directory = temp_dir();
        let path = directory.join(SETTINGS_FILE_NAME);
        let world = directory.join("world.json");
        let relay = directory.join("voxel-relay.json");
        fs::write(&world, b"{\"world\":\"sentinel\"}").unwrap();
        fs::write(&relay, b"{\"relay\":\"sentinel\"}").unwrap();

        let expected = SharedSettings {
            handedness: Handedness::Right,
            large_controls: true,
            muted: true,
            ..SharedSettings::default()
        };
        expected.save(&path).unwrap();

        let restored = SharedSettings::load(&path);
        assert_eq!(restored, expected, "every preference must round-trip");
        assert_eq!(restored.version, SETTINGS_VERSION);
        assert_eq!(
            fs::read(&world).unwrap(),
            b"{\"world\":\"sentinel\"}",
            "saving settings must not touch the wetland/sandbox save"
        );
        assert_eq!(
            fs::read(&relay).unwrap(),
            b"{\"relay\":\"sentinel\"}",
            "saving settings must not touch the Relay save"
        );
        assert!(
            !path.with_extension("tmp").exists(),
            "the atomic temporary file must not survive a successful save"
        );
    }

    #[test]
    fn save_replaces_previous_values_atomically() {
        let directory = temp_dir();
        let path = directory.join(SETTINGS_FILE_NAME);
        SharedSettings::default().save(&path).unwrap();
        let changed = SharedSettings {
            muted: true,
            ..SharedSettings::default()
        };
        changed.save(&path).unwrap();
        assert_eq!(SharedSettings::load(&path), changed);
        assert!(
            fs::read_to_string(&path)
                .unwrap()
                .contains("\"version\": 1"),
            "the persisted document must carry its version"
        );
    }

    #[test]
    fn missing_file_loads_defaults_without_creating_anything() {
        let directory = temp_dir();
        let path = directory.join(SETTINGS_FILE_NAME);
        assert_eq!(SharedSettings::load(&path), SharedSettings::default());
        assert!(!path.exists(), "loading must never create the file");
    }

    #[test]
    fn invalid_files_fall_back_to_defaults_and_remain_replaceable() {
        let directory = temp_dir();
        let path = directory.join(SETTINGS_FILE_NAME);
        let cases: [(&str, Vec<u8>); 6] = [
            ("malformed json", b"{ not json".to_vec()),
            (
                "unknown field",
                br#"{"version":1,"handedness":"left","large_controls":false,"muted":false,"extra":1}"#
                    .to_vec(),
            ),
            (
                "unsupported version",
                br#"{"version":99,"handedness":"left","large_controls":false,"muted":false}"#
                    .to_vec(),
            ),
            (
                "invalid handedness",
                br#"{"version":1,"handedness":"sideways","large_controls":false,"muted":false}"#
                    .to_vec(),
            ),
            (
                "missing field",
                br#"{"version":1,"handedness":"left","muted":false}"#.to_vec(),
            ),
            (
                "oversized",
                vec![b' '; (MAX_SETTINGS_BYTES + 1) as usize],
            ),
        ];
        for (name, bytes) in cases {
            fs::write(&path, &bytes).unwrap();
            assert_eq!(
                SharedSettings::load(&path),
                SharedSettings::default(),
                "{name} must fall back to defaults"
            );
        }
        // The app can always recover by writing a fresh valid document.
        let recovered = SharedSettings {
            muted: true,
            ..SharedSettings::default()
        };
        recovered.save(&path).unwrap();
        assert_eq!(SharedSettings::load(&path), recovered);
    }

    #[test]
    fn toggle_turns_each_row_and_value_labels_follow() {
        let mut settings = SharedSettings::default();
        settings.toggle(SettingRow::Handedness);
        assert_eq!(settings.handedness, Handedness::Right);
        assert_eq!(settings.value_label(SettingRow::Handedness), "RIGHT");
        settings.toggle(SettingRow::Handedness);
        assert_eq!(settings.handedness, Handedness::Left);

        settings.toggle(SettingRow::LargeControls);
        assert!(settings.large_controls);
        assert_eq!(settings.value_label(SettingRow::LargeControls), "LARGE");

        settings.toggle(SettingRow::Mute);
        assert!(settings.muted);
        assert_eq!(settings.value_label(SettingRow::Mute), "MUTED");
    }

    #[test]
    fn encoded_document_stays_inside_the_bound() {
        let encoded = serde_json::to_vec_pretty(&SharedSettings::default()).unwrap();
        assert!(
            encoded.len() as u64 <= MAX_SETTINGS_BYTES,
            "the bounded document must always fit its bound"
        );
    }
}
