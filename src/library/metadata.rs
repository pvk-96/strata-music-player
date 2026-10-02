//! Reading tags and audio properties from a file.
//!
//! Strata uses `lofty` for tags. GStreamer is only ever asked to decode audio, never to
//! answer questions about the library. Values are stored exactly as read; missing tags
//! stay empty and display fallbacks live in [`crate::library::model`].

use std::path::Path;

use lofty::file::{AudioFile, FileType, TaggedFileExt};
use lofty::picture::Picture;
use lofty::tag::{Accessor, ItemKey, Tag};

use crate::error::{Error, Result};

/// Extensions the scanner treats as audio. Membership here says "read tags for it";
/// whether playback succeeds depends on the installed GStreamer decoders.
pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "wav", "wave", "m4a", "m4b", "mp4", "aac", "alac", "aiff",
    "aif", "aifc",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedArtwork {
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileMetadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
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

    pub artwork: Option<EmbeddedArtwork>,
}

/// True when the extension is one Strata indexes.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| SUPPORTED_EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Read tags and properties. A file without tags is not an error: the scan continues and
/// display fallbacks apply.
pub fn read(path: &Path) -> Result<FileMetadata> {
    let tagged = lofty::probe::Probe::open(path)
        .map_err(|err| Error::MetadataRead {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?
        .read()
        .map_err(|err| Error::MetadataRead {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;

    let properties = tagged.properties();
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    let mut metadata = FileMetadata {
        duration_ms: non_zero(properties.duration().as_millis() as i64),
        format: Some(format_name(tagged.file_type()).to_string()),
        sample_rate: properties.sample_rate().map(i64::from),
        channels: properties.channels().map(i64::from),
        bit_depth: properties.bit_depth().map(i64::from),
        bitrate: properties
            .audio_bitrate()
            .or_else(|| properties.overall_bitrate())
            .map(i64::from),
        ..Default::default()
    };

    if let Some(tag) = tag {
        metadata.title = clean_cow(tag.title());
        metadata.artist = clean_cow(tag.artist());
        metadata.album = clean_cow(tag.album());
        metadata.album_artist = trimmed(tag.get_string(ItemKey::AlbumArtist));
        metadata.genre = clean_cow(tag.genre());
        metadata.year = leading_number(tag.get_string(ItemKey::Year))
            .or_else(|| leading_number(tag.get_string(ItemKey::RecordingDate)));
        metadata.track_number = tag
            .track()
            .map(i64::from)
            .or_else(|| leading_number(tag.get_string(ItemKey::TrackNumber)));
        metadata.disc_number = tag
            .disk()
            .map(i64::from)
            .or_else(|| leading_number(tag.get_string(ItemKey::DiscNumber)));
        metadata.artwork = artwork(tag);
    }

    Ok(metadata)
}

/// Best available artwork inside the file: a front cover when present, otherwise the first
/// picture. Returns nothing when the file has no embedded image.
pub fn embedded_artwork(path: &Path) -> Result<Option<EmbeddedArtwork>> {
    let metadata = read(path)?;
    Ok(metadata.artwork)
}

fn artwork(tag: &Tag) -> Option<EmbeddedArtwork> {
    let pictures = tag.pictures();
    let chosen: &Picture = pictures
        .iter()
        .find(|picture| picture.pic_type() == lofty::picture::PictureType::CoverFront)
        .or_else(|| pictures.first())?;

    Some(EmbeddedArtwork {
        mime: chosen
            .mime_type()
            .map(|mime| mime.as_str().to_string())
            .unwrap_or_default(),
        data: chosen.data().to_vec(),
    })
}

fn trimmed(value: Option<&str>) -> Option<String> {
    clean(value)
}

fn clean(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn clean_cow(value: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
    clean(value.as_deref())
}

fn non_zero(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

/// Take the leading number of tag values such as `3/12` or `1994-06-21`.
fn leading_number(value: Option<&str>) -> Option<i64> {
    let digits: String = value?
        .trim()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

fn format_name(file_type: FileType) -> &'static str {
    match file_type {
        FileType::Aac => "AAC",
        FileType::Aiff => "AIFF",
        FileType::Ape => "APE",
        FileType::Flac => "FLAC",
        FileType::Mpeg => "MP3",
        FileType::Mp4 => "M4A",
        FileType::Mpc => "Musepack",
        FileType::Opus => "Opus",
        FileType::Vorbis => "OGG Vorbis",
        FileType::Speex => "Speex",
        FileType::Wav => "WAV",
        FileType::WavPack => "WavPack",
        FileType::Custom(name) => name,
        other => {
            // Custom resolvers registered by lofty are reported verbatim.
            log::debug!("unhandled file type {other:?}");
            "Unknown"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn supported_extensions_are_recognised_case_insensitively() {
        assert!(is_supported(Path::new("/m/song.FLAC")));
        assert!(is_supported(Path::new("/m/song.Mp3")));
        assert!(is_supported(Path::new("/m/album folder/01 track.m4a")));
        assert!(!is_supported(Path::new("/m/notes.txt")));
        assert!(!is_supported(Path::new("/m/cover.png")));
        assert!(!is_supported(Path::new("/m/noextension")));
    }

    #[test]
    fn complete_metadata_is_read() {
        let metadata = read(&fixture("complete.mp3")).unwrap();
        assert_eq!(metadata.title.as_deref(), Some("Blue Monday"));
        assert_eq!(metadata.artist.as_deref(), Some("New Order"));
        assert_eq!(metadata.album.as_deref(), Some("Power, Corruption & Lies"));
        assert_eq!(metadata.album_artist.as_deref(), Some("New Order"));
        assert_eq!(metadata.genre.as_deref(), Some("Electronic"));
        assert_eq!(metadata.year, Some(1983));
        assert_eq!(metadata.track_number, Some(3));
        assert_eq!(metadata.disc_number, Some(1));
        assert_eq!(metadata.format.as_deref(), Some("MP3"));
        assert_eq!(metadata.sample_rate, Some(44_100));
        assert_eq!(metadata.channels, Some(2));
        assert!(metadata.duration_ms.unwrap() > 0);
    }

    #[test]
    fn values_are_preserved_verbatim() {
        let metadata = read(&fixture("complete.mp3")).unwrap();
        // No normalisation: "New Order" is not turned into anything else, and the album
        // keeps its punctuation.
        assert_eq!(metadata.artist.as_deref(), Some("New Order"));
        assert_eq!(metadata.album.as_deref(), Some("Power, Corruption & Lies"));
    }

    #[test]
    fn missing_tags_are_absent_rather_than_invented() {
        let metadata = read(&fixture("no_tags.mp3")).unwrap();
        assert_eq!(metadata.title, None);
        assert_eq!(metadata.artist, None);
        assert_eq!(metadata.album, None);
        assert_eq!(metadata.album_artist, None);
        assert_eq!(metadata.year, None);
        // The file is still a valid audio file, so format details are present.
        assert_eq!(metadata.format.as_deref(), Some("MP3"));
    }

    #[test]
    fn embedded_artwork_is_extracted() {
        let metadata = read(&fixture("complete.flac")).unwrap();
        let artwork = metadata.artwork.expect("fixture has embedded artwork");
        assert_eq!(artwork.mime, "image/png");
        assert!(artwork.data.len() > 8);
    }

    #[test]
    fn file_without_embedded_artwork_reports_none() {
        let metadata = read(&fixture("no_cover.flac")).unwrap();
        assert_eq!(metadata.artwork, None);
    }

    #[test]
    fn every_target_format_is_readable() {
        for name in [
            "complete.mp3",
            "complete.flac",
            "format.ogg",
            "format.opus",
            "format.m4a",
            "format-alac.m4a",
            "format.wav",
            "format.aiff",
        ] {
            let metadata = read(&fixture(name)).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert!(
                metadata.duration_ms.unwrap_or_default() > 0,
                "{name} duration"
            );
            assert!(
                metadata.sample_rate.unwrap_or_default() > 0,
                "{name} sample rate"
            );
        }
    }

    #[test]
    fn flac_reports_bit_depth() {
        let metadata = read(&fixture("complete.flac")).unwrap();
        assert_eq!(metadata.bit_depth, Some(16));
    }

    #[test]
    fn broken_files_produce_metadata_errors_not_panics() {
        for name in ["broken.mp3", "broken.flac", "truncated.flac"] {
            let result = read(&fixture(name));
            assert!(result.is_err(), "{name} should be reported as unreadable");
            assert_eq!(result.unwrap_err().kind(), "MetadataReadFailure");
        }
    }

    #[test]
    fn missing_file_is_reported_as_metadata_failure_with_path() {
        let error = read(&fixture("does-not-exist.mp3")).unwrap_err();
        assert_eq!(error.kind(), "MetadataReadFailure");
        assert!(error.user_message().contains("does-not-exist.mp3"));
    }

    #[test]
    fn leading_number_handles_tag_formats() {
        assert_eq!(leading_number(Some("3/12")), Some(3));
        assert_eq!(leading_number(Some("1994-06-21")), Some(1994));
        assert_eq!(leading_number(Some(" 7 ")), Some(7));
        assert_eq!(leading_number(Some("none")), None);
        assert_eq!(leading_number(None), None);
    }

    #[test]
    fn multi_disc_members_keep_their_disc_number() {
        let first = read(&fixture("disc1-track1.flac")).unwrap();
        let second = read(&fixture("disc2-track2.flac")).unwrap();
        assert_eq!(first.disc_number, Some(1));
        assert_eq!(second.disc_number, Some(2));
        assert_eq!(first.track_number, Some(1));
        assert_eq!(second.track_number, Some(2));
    }
}
