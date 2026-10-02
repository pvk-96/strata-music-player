//! User settings, stored as human readable TOML.
//!
//! Settings hold preferences only (appearance, playback behaviour, shortcuts, path
//! mappings). Library contents and playback session state live in SQLite; see
//! `docs/database.md`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::keyboard::KeyboardSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Density {
    Compact,
    #[default]
    Comfortable,
}

impl Density {
    /// Row padding in pixels, applied to list rows.
    pub fn row_padding(&self) -> u16 {
        match self {
            Density::Compact => 4,
            Density::Comfortable => 10,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Density::Compact => "Compact",
            Density::Comfortable => "Comfortable",
        }
    }
}

/// A persistent rewrite of one path prefix, used to resolve playlist entries that were
/// created on another machine or operating system (for example Android -> Linux).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathMapping {
    pub from: PathBuf,
    pub to: PathBuf,
}

impl PathMapping {
    pub fn new(from: impl Into<PathBuf>, to: impl Into<PathBuf>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Apply the mapping to `path` when `path` starts with `from`.
    pub fn apply(&self, path: &Path) -> Option<PathBuf> {
        let rest = path.strip_prefix(&self.from).ok()?;
        Some(self.to.join(rest))
    }

    /// True when this mapping would rewrite the given path and the result is usable.
    pub fn is_testable(&self, path: &Path) -> bool {
        self.apply(path).is_some_and(|mapped| mapped.is_absolute())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub density: Density,
    pub sidebar_visible: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            density: Density::Comfortable,
            sidebar_visible: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Playback {
    /// Restore the previous session's track, position and queue on start.
    pub resume_session: bool,
    /// Seconds used by the seek actions.
    pub seek_seconds: u32,
    /// Follow a track's embedded artwork when no album artwork is available.
    pub show_track_artwork_in_player: bool,
}

impl Default for Playback {
    fn default() -> Self {
        Self {
            resume_session: true,
            seek_seconds: 10,
            show_track_artwork_in_player: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Library {
    /// Rewrites applied when resolving playlist paths, longest prefix first.
    pub path_mappings: Vec<PathMapping>,
    /// Selected music folders are stored in the database; this remembers only the last
    /// folder the user picked, to pre-fill the folder chooser.
    pub last_selected_folder: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub appearance: Appearance,
    pub playback: Playback,
    pub keyboard: KeyboardSettings,
    pub library: Library,
}

/// Outcome of loading settings: the values in use plus any problems worth reporting.
#[derive(Debug)]
pub struct LoadedSettings {
    pub settings: Settings,
    pub warnings: Vec<String>,
}

impl Settings {
    /// Bring settings into their supported ranges, reporting what changed.
    pub fn validate(&mut self) -> Vec<String> {
        let mut warnings = Vec::new();
        if self.playback.seek_seconds == 0 || self.playback.seek_seconds > 600 {
            warnings.push(format!(
                "seek_seconds was {}, using {}",
                self.playback.seek_seconds,
                Playback::default().seek_seconds
            ));
            self.playback.seek_seconds = Playback::default().seek_seconds;
        }

        let before = self.library.path_mappings.len();
        self.library.path_mappings.retain(|mapping| {
            !mapping.from.as_os_str().is_empty() && !mapping.to.as_os_str().is_empty()
        });
        if self.library.path_mappings.len() != before {
            warnings.push("removed a path mapping with an empty source or target".to_string());
        }
        // Longest source first so nested mappings resolve predictably.
        self.library
            .path_mappings
            .sort_by_key(|mapping| std::cmp::Reverse(mapping.from.as_os_str().len()));

        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let mut settings = Settings::default();
        assert!(settings.validate().is_empty());
        assert_eq!(settings.playback.seek_seconds, 10);
        assert!(settings.playback.resume_session);
        assert_eq!(settings.appearance.theme, Theme::System);
    }

    #[test]
    fn seek_seconds_out_of_range_is_repaired() {
        let mut settings = Settings::default();
        settings.playback.seek_seconds = 0;
        let warnings = settings.validate();
        assert_eq!(settings.playback.seek_seconds, 10);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn empty_mappings_are_dropped() {
        let mut settings = Settings::default();
        settings.library.path_mappings = vec![PathMapping::new("", "/home/u/Music")];
        settings.validate();
        assert!(settings.library.path_mappings.is_empty());
    }

    #[test]
    fn mappings_apply_by_prefix() {
        let mapping = PathMapping::new("/storage/emulated/0/Music", "/home/u/Music");
        assert_eq!(
            mapping.apply(Path::new("/storage/emulated/0/Music/song.mp3")),
            Some(PathBuf::from("/home/u/Music/song.mp3"))
        );
        assert_eq!(mapping.apply(Path::new("/other/song.mp3")), None);
        assert!(mapping.is_testable(Path::new("/storage/emulated/0/Music/song.mp3")));
    }

    #[test]
    fn nested_mappings_are_ordered_longest_first() {
        let mut settings = Settings::default();
        settings.library.path_mappings = vec![
            PathMapping::new("/music", "/mnt/music"),
            PathMapping::new("/music/rock", "/mnt/rock"),
        ];
        settings.validate();
        assert_eq!(
            settings.library.path_mappings[0].from,
            PathBuf::from("/music/rock")
        );
    }

    #[test]
    fn density_controls_padding() {
        assert!(Density::Compact.row_padding() < Density::Comfortable.row_padding());
    }
}
