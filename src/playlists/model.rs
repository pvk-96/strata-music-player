//! Playlist data types.

use std::path::PathBuf;

use crate::error::{Error, Result};

/// Whether an entry can be played. Strata keeps unresolved entries instead of dropping them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Availability {
    #[default]
    Available,
    /// The reference was found, but the file is no longer on disk.
    Missing,
    /// The reference has not been resolved to a library track.
    Unresolved,
}

impl Availability {
    pub fn as_str(&self) -> &'static str {
        match self {
            Availability::Available => "available",
            Availability::Missing => "missing",
            Availability::Unresolved => "unresolved",
        }
    }

    pub fn parse(text: &str) -> Availability {
        match text {
            "missing" => Availability::Missing,
            "unresolved" => Availability::Unresolved,
            _ => Availability::Available,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Availability::Available => "Available",
            Availability::Missing => "Missing file",
            Availability::Unresolved => "Not in library",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One row of a playlist: a reference plus what it currently resolves to.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistEntry {
    pub id: i64,
    pub playlist_id: i64,
    pub track_id: Option<i64>,
    pub position: usize,
    /// What the user wrote, or what an imported file said. Never rewritten.
    pub original_reference: String,
    pub availability: Availability,
    /// Absolute path, or the display name to show for an unresolved entry.
    pub display_label: String,
    /// Copied from the track when the entry was read, for the footer summary.
    pub duration_ms: Option<i64>,
}

impl PlaylistEntry {
    pub fn is_playable(&self) -> bool {
        self.track_id.is_some() && self.availability == Availability::Available
    }
}

/// An entry the user typed or imported that is not a library track yet.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedEntry {
    pub playlist_id: i64,
    pub position: usize,
    pub original_reference: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistDetail {
    pub playlist: Playlist,
    pub entries: Vec<PlaylistEntry>,
}

impl PlaylistDetail {
    pub fn playable_track_ids(&self) -> Vec<i64> {
        self.entries
            .iter()
            .filter(|entry| entry.is_playable())
            .filter_map(|entry| entry.track_id)
            .collect()
    }

    pub fn unresolved_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|entry| entry.availability != Availability::Available)
            .count()
    }

    /// Total length, when every entry has a known duration.
    pub fn total_seconds(&self) -> Option<i64> {
        self.entries.iter().try_fold(0i64, |total, entry| {
            entry.duration_ms.map(|ms| total + ms / 1000)
        })
    }
}

/// Serialised playlist written to disk on export.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaylistFile {
    pub name: String,
    pub entries: Vec<String>,
}

/// Absolute path of an M3U file that Strata would write for a playlist.
pub fn default_export_path(base: &std::path::Path, name: &str) -> PathBuf {
    let safe: String = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    let trimmed = safe.trim();
    let safe = if trimmed.is_empty() {
        "playlist"
    } else {
        trimmed
    };
    base.join(format!("{safe}.m3u"))
}

/// Reject names the filesystem or the UI cannot represent.
pub fn validate_name(name: &str) -> Result<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(Error::InvalidConfiguration(
            "A playlist needs a name.".to_string(),
        ));
    }
    if trimmed.len() > 200 {
        return Err(Error::InvalidConfiguration(
            "Playlist names can be at most 200 characters.".to_string(),
        ));
    }
    if trimmed.contains(['\n', '\r', '\0']) {
        return Err(Error::InvalidConfiguration(
            "Playlist names can't contain line breaks.".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_round_trips_through_text() {
        for availability in [
            Availability::Available,
            Availability::Missing,
            Availability::Unresolved,
        ] {
            assert_eq!(Availability::parse(availability.as_str()), availability);
        }
        assert_eq!(Availability::parse("weird"), Availability::Available);
    }

    #[test]
    fn export_path_sanitises_the_name() {
        let path = default_export_path(std::path::Path::new("/tmp"), "Mix: 2024/pop");
        assert_eq!(path, std::path::Path::new("/tmp/Mix_ 2024_pop.m3u"));
    }

    #[test]
    fn export_path_falls_back_for_empty_names() {
        let path = default_export_path(std::path::Path::new("/tmp"), "   ");
        assert_eq!(path, std::path::Path::new("/tmp/playlist.m3u"));
    }

    #[test]
    fn names_are_validated() {
        assert_eq!(validate_name("  Road trip  ").unwrap(), "Road trip");
        assert!(validate_name("").is_err());
        assert!(validate_name("   ").is_err());
        assert!(validate_name("two\nlines").is_err());
        assert!(validate_name(&"x".repeat(201)).is_err());
    }

    #[test]
    fn only_available_entries_are_playable() {
        let entry = PlaylistEntry {
            id: 1,
            playlist_id: 1,
            track_id: Some(4),
            position: 0,
            original_reference: "/m/a.mp3".to_string(),
            availability: Availability::Available,
            display_label: "a.mp3".to_string(),
            duration_ms: Some(180_000),
        };
        assert!(entry.is_playable());

        let mut missing = entry.clone();
        missing.availability = Availability::Missing;
        assert!(!missing.is_playable());

        let mut unresolved = entry;
        unresolved.track_id = None;
        assert!(!unresolved.is_playable());
    }
}
