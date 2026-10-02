//! Artwork: where it comes from and how it is cached.
//!
//! Tags win over folder images. The library only stores a *reference* to the artwork, never
//! image data, so files stay the source of truth.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::library::metadata::EmbeddedArtwork;

/// Image file names looked for next to the audio file, in order of preference.
pub const FOLDER_COVER_NAMES: [&str; 8] = [
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.png",
    "front.jpg",
    "front.png",
    "albumart.jpg",
];

const COVER_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "webp", "bmp"];

/// Where an image came from. Stored in `tracks.artwork_reference`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtworkSource {
    /// Picture embedded in the audio file.
    Embedded(PathBuf),
    /// Image file in the track's directory.
    File(PathBuf),
}

impl ArtworkSource {
    pub fn reference(&self) -> String {
        match self {
            ArtworkSource::Embedded(path) => format!("embedded:{}", path.to_string_lossy()),
            ArtworkSource::File(path) => format!("file:{}", path.to_string_lossy()),
        }
    }

    pub fn parse(reference: &str) -> Option<ArtworkSource> {
        if let Some(path) = reference.strip_prefix("embedded:") {
            Some(ArtworkSource::Embedded(PathBuf::from(path)))
        } else {
            reference
                .strip_prefix("file:")
                .map(|path| ArtworkSource::File(PathBuf::from(path)))
        }
    }
}

/// Find the artwork for a track: embedded picture first, then a cover image in the same
/// directory. Returns the source and the image bytes.
pub fn load(track_path: &Path) -> Result<Option<(ArtworkSource, EmbeddedArtwork)>> {
    if !track_path.is_file() {
        return Err(Error::MissingFile(track_path.to_path_buf()));
    }
    if let Some(artwork) = crate::library::metadata::embedded_artwork(track_path)? {
        return Ok(Some((
            ArtworkSource::Embedded(track_path.to_path_buf()),
            artwork,
        )));
    }
    let Some(cover) = find_cover_file(track_path) else {
        return Ok(None);
    };
    Ok(Some((
        ArtworkSource::File(cover.clone()),
        read_image(&cover)?,
    )))
}

/// Cover image in the track's directory, including the album name.
pub fn find_cover_file(track_path: &Path) -> Option<PathBuf> {
    let directory = track_path.parent()?;
    for name in FOLDER_COVER_NAMES {
        let candidate = directory.join(name);
        if is_image(&candidate) {
            return Some(candidate);
        }
    }

    // Album cover named after the album, e.g. "Abbey Road.jpg".
    let album = album_name(track_path);
    if let Some(album) = album {
        for extension in COVER_EXTENSIONS {
            let candidate = directory.join(format!("{album}.{extension}"));
            if is_image(&candidate) {
                return Some(candidate);
            }
        }
    }
    None
}

/// A file with an image extension that really starts like an image. Extensions alone are
/// not enough: a `cover.jpg` that is actually a text file is skipped.
pub fn is_image(path: &Path) -> bool {
    let extension_matches = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            let lower = extension.to_ascii_lowercase();
            COVER_EXTENSIONS.contains(&lower.as_str())
        })
        .unwrap_or(false);
    if !extension_matches || !path.is_file() {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut header = [0u8; 12];
    let Ok(read) = file.read(&mut header) else {
        return false;
    };
    looks_like_image(&header[..read])
}

/// Check the leading bytes for a known image format.
pub fn looks_like_image(bytes: &[u8]) -> bool {
    let starts = |pattern: &[u8]| bytes.starts_with(pattern);
    starts(&[0xFF, 0xD8, 0xFF])                                        // JPEG
        || starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])    // PNG
        || (starts(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"))       // WEBP
        || starts(b"BM") // BMP
}

/// Album name guessed from a directory name, with the usual tag words removed.
fn album_name(track_path: &Path) -> Option<String> {
    let directory = track_path.parent()?;
    let name = directory.file_name()?.to_string_lossy().into_owned();
    let cleaned = name
        .trim()
        .trim_start_matches(|character: char| !character.is_ascii_alphanumeric())
        .to_string();
    if cleaned.is_empty() {
        None
    } else {
        Some(cleaned)
    }
}

fn read_image(path: &Path) -> Result<EmbeddedArtwork> {
    let data = std::fs::read(path).map_err(|error| Error::Filesystem {
        path: path.to_path_buf(),
        detail: error.to_string(),
    })?;
    Ok(EmbeddedArtwork {
        mime: mime_for(path).to_string(),
        data,
    })
}

pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        _ => "image/jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    #[test]
    fn embedded_artwork_wins_over_folder_images() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::copy(fixture("complete.flac"), directory.path().join("song.flac")).unwrap();
        std::fs::copy(fixture("cover.png"), directory.path().join("cover.png")).unwrap();

        let (source, artwork) = load(&directory.path().join("song.flac")).unwrap().unwrap();
        assert_eq!(
            source,
            ArtworkSource::Embedded(directory.path().join("song.flac"))
        );
        assert_eq!(artwork.mime, "image/png");
        assert!(artwork.data.len() > 8);
    }

    #[test]
    fn an_image_is_recognised_by_its_leading_bytes() {
        assert!(looks_like_image(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0]));
        assert!(looks_like_image(&[
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A
        ]));
        assert!(looks_like_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(looks_like_image(b"BM123456"));
        assert!(!looks_like_image(b"not an image at all"));
        assert!(!looks_like_image(b""));
    }

    #[test]
    fn folder_images_are_used_when_tags_have_none() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::copy(fixture("no_cover.flac"), directory.path().join("song.flac")).unwrap();
        std::fs::copy(fixture("cover.png"), directory.path().join("folder.png")).unwrap();

        let (source, artwork) = load(&directory.path().join("song.flac")).unwrap().unwrap();
        assert_eq!(
            source,
            ArtworkSource::File(directory.path().join("folder.png"))
        );
        assert_eq!(artwork.mime, "image/png");
    }

    #[test]
    fn a_cover_named_after_the_album_is_found() {
        let directory = tempfile::tempdir().unwrap();
        let album = directory.path().join("Abbey Road");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::copy(fixture("no_cover.flac"), album.join("track.flac")).unwrap();
        std::fs::copy(fixture("cover.png"), album.join("Abbey Road.png")).unwrap();

        assert_eq!(
            find_cover_file(&album.join("track.flac")),
            Some(album.join("Abbey Road.png"))
        );
    }

    #[test]
    fn no_artwork_anywhere_is_not_an_error() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::copy(fixture("no_cover.flac"), directory.path().join("song.flac")).unwrap();
        assert_eq!(load(&directory.path().join("song.flac")).unwrap(), None);
    }

    #[test]
    fn a_non_image_cover_is_ignored() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::copy(fixture("no_cover.flac"), directory.path().join("song.flac")).unwrap();
        std::fs::write(directory.path().join("cover.jpg"), b"not an image").unwrap();
        assert_eq!(find_cover_file(&directory.path().join("song.flac")), None);
    }

    #[test]
    fn a_file_that_is_gone_reports_missing() {
        let error = load(Path::new("/nope/gone.mp3")).unwrap_err();
        assert_eq!(error.kind(), "MissingFile");
    }

    #[test]
    fn references_round_trip_through_text() {
        let embedded = ArtworkSource::Embedded(PathBuf::from("/m/a.mp3"));
        assert_eq!(
            ArtworkSource::parse(&embedded.reference()),
            Some(embedded.clone())
        );
        let file = ArtworkSource::File(PathBuf::from("/m/cover.jpg"));
        assert_eq!(ArtworkSource::parse(&file.reference()), Some(file));
        assert_eq!(ArtworkSource::parse("something else"), None);
        assert_eq!(ArtworkSource::parse(""), None);
    }

    #[test]
    fn mime_types_follow_the_extension() {
        assert_eq!(mime_for(Path::new("a.png")), "image/png");
        assert_eq!(mime_for(Path::new("a.JPEG")), "image/jpeg");
        assert_eq!(mime_for(Path::new("a.webp")), "image/webp");
        assert_eq!(mime_for(Path::new("a.unknown")), "image/jpeg");
    }
}
