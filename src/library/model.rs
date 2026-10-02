//! Library data types.
//!
//! Values are stored exactly as read from the files. Display fallbacks for missing tags
//! live in [`Track`] helpers and are never written back to the database or to files.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const UNKNOWN_ARTIST: &str = "Unknown Artist";
pub const UNKNOWN_ALBUM: &str = "Unknown Album";

/// One audio file in the library.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub id: i64,
    pub path: PathBuf,
    pub filename: String,
    pub folder_id: Option<i64>,

    pub file_size: i64,
    pub modified_time: i64,
    pub missing: bool,

    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub genre: Option<String>,
    pub year: Option<i64>,
    pub track_number: Option<i64>,
    pub disc_number: Option<i64>,
    pub duration_ms: Option<i64>,

    pub format: Option<String>,
    pub sample_rate: Option<i64>,
    pub channels: Option<i64>,
    pub bit_depth: Option<i64>,
    pub bitrate: Option<i64>,

    pub favorite: bool,
    pub artwork_reference: Option<String>,
    /// True when the scanner found several plausible predecessors for this file and the
    /// user has to choose. Strata never guesses in this case.
    pub ambiguous: bool,

    pub created_at: i64,
    pub updated_at: i64,
}

impl Track {
    /// Title to show, falling back to the filename without its extension.
    pub fn display_title(&self) -> String {
        let trimmed = self.title.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
        self.filename
            .rsplit_once('.')
            .map(|(stem, _)| stem)
            .filter(|stem| !stem.is_empty())
            .unwrap_or(&self.filename)
            .to_string()
    }

    pub fn display_artist(&self) -> &str {
        text_or(&self.artist, UNKNOWN_ARTIST)
    }

    pub fn display_album(&self) -> &str {
        text_or(&self.album, UNKNOWN_ALBUM)
    }

    /// Album artist when present, otherwise the track artist, otherwise unknown.
    pub fn effective_album_artist(&self) -> &str {
        let album_artist = self.album_artist.trim();
        if album_artist.is_empty() {
            text_or(&self.artist, UNKNOWN_ARTIST)
        } else {
            album_artist
        }
    }

    pub fn display_genre(&self) -> &str {
        text_or(self.genre.as_deref().unwrap_or_default(), "Unknown Genre")
    }

    pub fn duration_seconds(&self) -> Option<i64> {
        self.duration_ms.map(|ms| ms / 1000)
    }

    /// Parent directory, used for folder artwork and "show in file manager".
    pub fn folder(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new(""))
    }
}

/// Trimmed value, or `fallback` when the tag is absent or only whitespace.
fn text_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback
    } else {
        trimmed
    }
}

/// Artists and albums are derived from track metadata; they have no stored entity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtistSummary {
    pub name: String,
    pub track_count: i64,
    pub album_count: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlbumSummary {
    /// Stable identity for an album: album artist plus album name.
    pub album_artist: String,
    pub album: String,
    pub year: Option<i64>,
    pub track_count: i64,
    pub duration_ms: i64,
    pub artwork_reference: Option<String>,
    /// One representative track, used for artwork and for starting playback.
    pub sample_track_id: i64,
    pub sample_track_path: PathBuf,
}

impl AlbumSummary {
    pub fn display_album(&self) -> &str {
        text_or(&self.album, UNKNOWN_ALBUM)
    }

    pub fn display_album_artist(&self) -> &str {
        text_or(&self.album_artist, UNKNOWN_ARTIST)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderSummary {
    pub folder_id: i64,
    pub path: PathBuf,
    pub track_count: i64,
    pub created_at: i64,
}

/// Sortable columns for track lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TrackSort {
    #[default]
    Title,
    Artist,
    Album,
    AlbumArtist,
    Genre,
    Year,
    TrackNumber,
    Duration,
    DateAdded,
}

impl TrackSort {
    pub fn label(&self) -> &'static str {
        match self {
            TrackSort::Title => "Title",
            TrackSort::Artist => "Artist",
            TrackSort::Album => "Album",
            TrackSort::AlbumArtist => "Album Artist",
            TrackSort::Genre => "Genre",
            TrackSort::Year => "Year",
            TrackSort::TrackNumber => "Track Number",
            TrackSort::Duration => "Duration",
            TrackSort::DateAdded => "Date Added",
        }
    }

    /// SQL ordering fragment. Columns are quoted; the direction is bound separately.
    pub fn order_by(&self) -> &'static str {
        match self {
            TrackSort::Title => "tracks.title COLLATE NOCASE",
            TrackSort::Artist => "tracks.artist COLLATE NOCASE",
            TrackSort::Album => "tracks.album COLLATE NOCASE",
            TrackSort::AlbumArtist => "tracks.album_artist COLLATE NOCASE",
            TrackSort::Genre => "tracks.genre COLLATE NOCASE",
            TrackSort::Year => "tracks.year",
            TrackSort::TrackNumber => "tracks.disc_number, tracks.track_number",
            TrackSort::Duration => "tracks.duration_ms",
            TrackSort::DateAdded => "tracks.created_at",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortDirection {
    #[default]
    Ascending,
    Descending,
}

impl SortDirection {
    pub fn sql(&self) -> &'static str {
        match self {
            SortDirection::Ascending => "ASC",
            SortDirection::Descending => "DESC",
        }
    }

    pub fn toggled(&self) -> SortDirection {
        match self {
            SortDirection::Ascending => SortDirection::Descending,
            SortDirection::Descending => SortDirection::Ascending,
        }
    }
}

/// Which tracks a query should return.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackQuery {
    /// Substring match over title, artist, album and filename.
    pub search: Option<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub artist: Option<String>,
    pub folder_id: Option<i64>,
    pub folder_path: Option<PathBuf>,
    pub favorites_only: bool,
    pub include_missing: bool,
    pub ids: Option<Vec<i64>>,
}

pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(title: &str, artist: &str, album: &str, album_artist: &str) -> Track {
        Track {
            id: 1,
            path: PathBuf::from("/music/song.mp3"),
            filename: "song.mp3".into(),
            folder_id: Some(1),
            file_size: 100,
            modified_time: 0,
            missing: false,
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: album_artist.into(),
            genre: None,
            year: None,
            track_number: None,
            disc_number: None,
            duration_ms: Some(180_000),
            format: Some("MP3".into()),
            sample_rate: Some(44_100),
            channels: Some(2),
            bit_depth: None,
            bitrate: Some(320_000),
            favorite: false,
            artwork_reference: None,
            ambiguous: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn missing_title_falls_back_to_filename() {
        let track = track("", "Artist", "Album", "Artist");
        assert_eq!(track.display_title(), "song");
    }

    #[test]
    fn filename_without_extension_is_used_verbatim_when_there_is_none() {
        let mut track = track("", "A", "Al", "A");
        track.filename = "noext".into();
        assert_eq!(track.display_title(), "noext");
    }

    #[test]
    fn missing_metadata_falls_back() {
        let track = track("Title", "", "", "");
        assert_eq!(track.display_artist(), UNKNOWN_ARTIST);
        assert_eq!(track.display_album(), UNKNOWN_ALBUM);
        assert_eq!(track.effective_album_artist(), UNKNOWN_ARTIST);
        assert_eq!(track.display_genre(), "Unknown Genre");
    }

    #[test]
    fn album_artist_falls_back_to_artist() {
        let track = track("Title", "Beatles", "Album", "");
        assert_eq!(track.effective_album_artist(), "Beatles");
    }

    #[test]
    fn whitespace_only_tags_count_as_missing() {
        let track = track("Title", "   ", "  ", "\t");
        assert_eq!(track.display_artist(), UNKNOWN_ARTIST);
        assert_eq!(track.display_album(), UNKNOWN_ALBUM);
        assert_eq!(track.effective_album_artist(), UNKNOWN_ARTIST);
    }

    #[test]
    fn titles_are_not_normalised() {
        // "Beatles" stays "Beatles"; Strata never rewrites what the files say.
        let track = track("Beatles", "Beatles", "Album", "Beatles");
        assert_eq!(track.display_title(), "Beatles");
    }

    #[test]
    fn direction_toggles() {
        assert_eq!(
            SortDirection::Ascending.toggled(),
            SortDirection::Descending
        );
        assert_eq!(
            SortDirection::Descending.toggled(),
            SortDirection::Ascending
        );
    }

    #[test]
    fn track_number_sort_orders_by_disc_then_track() {
        assert_eq!(
            TrackSort::TrackNumber.order_by(),
            "tracks.disc_number, tracks.track_number"
        );
    }
}
