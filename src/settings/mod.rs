//! Loading and saving the TOML settings file.

pub mod keyboard;
pub mod model;
pub mod shortcut;

use std::path::{Path, PathBuf};

pub use keyboard::{Conflict, KeyboardSettings};
pub use model::{Density, Library, LoadedSettings, PathMapping, Playback, Settings, Theme};
pub use shortcut::{Modifiers, Shortcut};

use crate::error::{Error, Result};

/// Read settings from `path`, falling back to defaults when the file is absent or damaged.
pub fn load(path: &Path) -> Result<LoadedSettings> {
    if !path.exists() {
        return Ok(LoadedSettings {
            settings: Settings::default(),
            warnings: Vec::new(),
        });
    }

    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            return Ok(LoadedSettings {
                settings: Settings::default(),
                warnings: vec![format!(
                    "settings file could not be read ({err}); defaults are being used"
                )],
            });
        }
    };

    match toml::from_str::<Settings>(&text) {
        Ok(mut settings) => {
            let mut warnings = settings.validate();
            for conflict in settings.keyboard.conflicts() {
                warnings.push(conflict.message());
            }
            Ok(LoadedSettings { settings, warnings })
        }
        Err(err) => {
            log::error!("settings file is not valid TOML, using defaults: {err}");
            Ok(LoadedSettings {
                settings: Settings::default(),
                warnings: vec![
                    "settings file was damaged and has been reset to defaults".to_string()
                ],
            })
        }
    }
}

/// Write settings to `path`, replacing the file atomically.
pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| Error::Filesystem {
            path: parent.to_path_buf(),
            detail: err.to_string(),
        })?;
    }

    let text = toml::to_string_pretty(settings)
        .map_err(|err| Error::config(format!("settings could not be encoded: {err}")))?;

    let temporary: PathBuf = path.with_extension("toml.tmp");
    std::fs::write(&temporary, text.as_bytes()).map_err(|err| Error::Filesystem {
        path: temporary.clone(),
        detail: err.to_string(),
    })?;
    std::fs::rename(&temporary, path).map_err(|err| Error::Filesystem {
        path: path.to_path_buf(),
        detail: err.to_string(),
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let loaded = load(&temp.path().join("config.toml")).unwrap();
        assert_eq!(loaded.settings, Settings::default());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn round_trip_preserves_settings() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("config.toml");

        let mut settings = Settings::default();
        settings.appearance.density = Density::Compact;
        settings.appearance.theme = Theme::Dark;
        settings.playback.seek_seconds = 25;
        settings.library.path_mappings = vec![PathMapping::new("/storage/emulated/0", "/home/u")];
        settings.keyboard.set_binding(
            crate::app::actions::Action::PlayPause,
            Some("Ctrl+Space".parse().unwrap()),
        );

        save(&path, &settings).unwrap();
        let loaded = load(&path).unwrap();
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        assert_eq!(loaded.settings, settings);
    }

    #[test]
    fn malformed_file_falls_back_to_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "this is not = = toml").unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.settings, Settings::default());
        assert_eq!(loaded.warnings.len(), 1);
    }

    #[test]
    fn partial_file_keeps_defaults_for_missing_sections() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[appearance]\ndensity = \"compact\"\n").unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.settings.appearance.density, Density::Compact);
        assert_eq!(loaded.settings.playback.seek_seconds, 10);
        assert_eq!(loaded.settings.keyboard, KeyboardSettings::defaults());
    }

    #[test]
    fn empty_binding_clears_a_default() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[keyboard.bindings]\nplay_pause = \"\"\n").unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(
            loaded
                .settings
                .keyboard
                .binding(crate::app::actions::Action::PlayPause),
            None
        );
    }

    #[test]
    fn unknown_binding_name_is_ignored() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            "[keyboard.bindings]\nplay_pause = \"Space\"\nteleport = \"Ctrl+T\"\n",
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.settings.keyboard, KeyboardSettings::defaults());
    }

    #[test]
    fn conflicting_bindings_are_reported() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        std::fs::write(
            &path,
            "[keyboard.bindings]\nplay_pause = \"Ctrl+N\"\nnew_playlist = \"Ctrl+N\"\n",
        )
        .unwrap();

        let loaded = load(&path).unwrap();
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("Ctrl+N"));
    }

    #[test]
    fn saved_file_is_readable_toml() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("config.toml");
        save(&path, &Settings::default()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[appearance]"));
        assert!(text.contains("theme = \"system\""));
        assert!(text.contains("[keyboard.bindings]"));
        assert!(text.contains("play_pause = \"Space\""));
    }
}
