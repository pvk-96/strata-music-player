//! M3U and M3U8 reading and writing.
//!
//! Import is deliberately forgiving: comments are kept as metadata, relative paths are
//! resolved, and anything that is not a usable file becomes an unresolved entry the user
//! can see and fix. Nothing is written to disk here.

use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Directives that carry meaning. Everything else starting with `#` is a comment.
const KNOWN_DIRECTIVES: [&str; 6] = ["EXTM3U", "EXTV4", "EXTGRP", "PLOP", "PLAYLIST", "EXT-X-"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// `#EXTINF:123,Artist - Title`
    Info {
        duration_seconds: Option<i64>,
        display: String,
    },
    /// `#EXTINF:-1 tvg-name="Radio" group-one="a,b"`, stored raw.
    Directive(String),
    Comment(String),
    Track(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedM3u {
    pub lines: Vec<Line>,
    pub is_extended: bool,
}

impl ParsedM3u {
    /// Paths in the order they appear.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.lines
            .iter()
            .filter_map(|line| match line {
                Line::Track(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn tracks(&self) -> usize {
        self.paths().len()
    }
}

/// Parse the contents of an M3U file. Relative paths are resolved against `base`, which
/// is the directory of the file for absolute imports and the file's own directory for
/// relative imports.
pub fn parse(contents: &str, base: &Path) -> Result<ParsedM3u> {
    let mut lines = Vec::new();
    let mut is_extended = false;

    for raw in contents.lines() {
        let line = raw.trim_end_matches('\r').trim();
        if line.is_empty() {
            continue;
        }

        if let Some(rest) = line.strip_prefix('#') {
            let upper = rest.to_ascii_uppercase();
            if upper.starts_with("EXTINF") {
                is_extended = true;
                lines.push(Line::Info {
                    duration_seconds: parse_extinf_duration(&upper),
                    display: rest
                        .split_once(':')
                        .map(|(_, after)| after.trim().to_string())
                        .unwrap_or_default(),
                });
            } else if KNOWN_DIRECTIVES.iter().any(|name| upper.starts_with(name)) {
                is_extended = true;
                lines.push(Line::Directive(rest.to_string()));
            } else {
                lines.push(Line::Comment(rest.to_string()));
            }
            continue;
        }

        let path = resolve(base, line);
        lines.push(Line::Track(path));
    }

    if !lines.iter().any(|line| matches!(line, Line::Track(_))) {
        return Err(Error::PlaylistResolution(
            "That playlist file doesn't list any tracks.".to_string(),
        ));
    }

    Ok(ParsedM3u { lines, is_extended })
}

/// Parse a file from disk.
pub fn parse_file(path: &Path) -> Result<ParsedM3u> {
    let bytes = std::fs::read(path).map_err(|err| {
        Error::PlaylistResolution(format!("could not read {}: {err}", path.display()))
    })?;
    let contents = String::from_utf8_lossy(&bytes);
    let base = parent_or_current(path);
    parse(&contents, &base)
}

fn parent_or_current(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve a playlist entry. URLs are left alone for the user to see.
fn resolve(base: &Path, line: &str) -> PathBuf {
    if line.starts_with("file://") {
        return PathBuf::from(line.trim_start_matches("file://"));
    }
    if looks_like_url(line) {
        return PathBuf::from(line);
    }
    let candidate = Path::new(line);
    if candidate.is_absolute() {
        normalize(candidate)
    } else {
        normalize(&base.join(candidate))
    }
}

/// Remove `.` and `..` segments without touching the filesystem.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn looks_like_url(line: &str) -> bool {
    line.starts_with("http://") || line.starts_with("https://") || line.starts_with("mms://")
}

fn parse_extinf_duration(upper: &str) -> Option<i64> {
    let rest = upper.strip_prefix("EXTINF:")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

/// Render a playlist for writing. `#EXTM3U` is always written so that players read the
/// display names.
pub fn export(entries: &[(PathBuf, String)], extended: bool) -> String {
    let mut out = String::new();
    if extended {
        out.push_str("#EXTM3U\n");
    }
    for (path, display) in entries {
        if extended {
            out.push_str(&format!("#EXTINF:-1,{display}\n"));
        }
        out.push_str(&path.to_string_lossy());
        out.push('\n');
    }
    out
}

/// Result of an import: how many entries were playable, and which were not.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportSummary {
    pub added: usize,
    pub unresolved: usize,
}

impl ImportSummary {
    pub fn total(&self) -> usize {
        self.added + self.unresolved
    }
}

/// Import a parsed playlist into a stored playlist. Unusable entries are kept as
/// unresolved references.
pub fn import(
    database: &crate::database::Database,
    playlist_id: i64,
    parsed: &ParsedM3u,
) -> Result<ImportSummary> {
    let connection = database.connection();
    let mut summary = ImportSummary::default();

    for path in parsed.paths() {
        let reference = path.to_string_lossy().into_owned();
        match crate::library::queries::track_at_path(connection, &path)? {
            Some(track) if !track.missing => {
                crate::playlists::queries::add_track(connection, playlist_id, track.id)?;
                summary.added += 1;
            }
            Some(track) => {
                crate::playlists::queries::add_track(connection, playlist_id, track.id)?;
                summary.added += 1;
                log::info!(
                    "imported {} which is currently missing",
                    track.path.display()
                );
            }
            None => {
                crate::playlists::queries::add_reference(connection, playlist_id, &reference)?;
                summary.unresolved += 1;
            }
        }
    }

    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::library::model::Track;

    #[test]
    fn plain_m3u_is_parsed_in_order() {
        let contents = "/m/a.mp3\n/m/b.mp3\n";
        let parsed = parse(contents, Path::new("/anywhere")).unwrap();
        assert_eq!(parsed.tracks(), 2);
        assert_eq!(
            parsed.paths(),
            vec![PathBuf::from("/m/a.mp3"), PathBuf::from("/m/b.mp3")]
        );
        assert!(!parsed.is_extended);
    }

    #[test]
    fn relative_paths_resolve_against_the_base() {
        let parsed = parse("album/01.mp3\n../other/02.mp3\n", Path::new("/music/lists")).unwrap();
        assert_eq!(
            parsed.paths(),
            vec![
                PathBuf::from("/music/lists/album/01.mp3"),
                PathBuf::from("/music/other/02.mp3")
            ]
        );
    }

    #[test]
    fn windows_separators_are_handled() {
        let parsed = parse("album\\01.mp3\n", Path::new("/music")).unwrap();
        let paths = parsed.paths();
        assert_eq!(paths.len(), 1);
        assert!(paths[0].to_string_lossy().contains("01.mp3"));
    }

    #[test]
    fn extended_lines_are_kept_separate_from_tracks() {
        let contents = "#EXTM3U\n#EXTINF:210,Artist - Title\n/m/a.mp3\n#EXTINF:-1 tvg-name=\"Radio\"\nhttp://example.invalid/s\n";
        let parsed = parse(contents, Path::new("/")).unwrap();
        assert!(parsed.is_extended);
        assert_eq!(parsed.tracks(), 2);
        assert_eq!(
            parsed.lines[1],
            Line::Info {
                duration_seconds: Some(210),
                display: "210,Artist - Title".to_string()
            }
        );
        assert!(matches!(&parsed.lines[3], Line::Info { .. }));
    }

    #[test]
    fn directives_and_comments_are_not_tracks() {
        let contents = "# just a comment\n#EXTV4\n/m/a.mp3\n";
        let parsed = parse(contents, Path::new("/")).unwrap();
        assert_eq!(parsed.tracks(), 1);
        assert!(matches!(parsed.lines[0], Line::Comment(_)));
    }

    #[test]
    fn blank_lines_are_skipped() {
        let parsed = parse("\n\n/m/a.mp3\n\n   \n", Path::new("/")).unwrap();
        assert_eq!(parsed.tracks(), 1);
    }

    #[test]
    fn files_without_tracks_are_reported() {
        assert!(parse("", Path::new("/")).is_err());
        assert!(parse("\n#EXTM3U\n", Path::new("/")).is_err());
        assert!(parse("#EXTM3U\n#PLAYLIST:Radio\n", Path::new("/")).is_err());
    }

    #[test]
    fn carriage_returns_are_stripped() {
        let parsed = parse("/m/a.mp3\r\n/m/b.mp3\r\n", Path::new("/")).unwrap();
        assert_eq!(parsed.tracks(), 2);
        assert_eq!(parsed.paths()[0], PathBuf::from("/m/a.mp3"));
    }

    #[test]
    fn file_urls_are_unwrapped() {
        let parsed = parse("file:///m/a.mp3\n", Path::new("/")).unwrap();
        assert_eq!(parsed.paths()[0], PathBuf::from("/m/a.mp3"));
    }

    #[test]
    fn urls_are_preserved_as_given() {
        let parsed = parse("https://example.invalid/stream\n", Path::new("/")).unwrap();
        assert_eq!(
            parsed.paths()[0],
            PathBuf::from("https://example.invalid/stream")
        );
    }

    #[test]
    fn export_writes_paths_and_optional_display_names() {
        let entries = vec![
            (PathBuf::from("/m/a.mp3"), "A - One".to_string()),
            (PathBuf::from("/m/b.mp3"), "B - Two".to_string()),
        ];
        let plain = export(&entries, false);
        assert_eq!(plain, "/m/a.mp3\n/m/b.mp3\n");
        assert!(!plain.contains("EXTM3U"));

        let extended = export(&entries, true);
        assert!(extended.starts_with("#EXTM3U\n"));
        assert!(extended.contains("#EXTINF:-1,A - One\n/m/a.mp3\n"));
    }

    #[test]
    fn export_round_trips_through_parse() {
        let entries = vec![(PathBuf::from("/m/a.mp3"), "A - One".to_string())];
        let text = export(&entries, true);
        let parsed = parse(&text, Path::new("/")).unwrap();
        assert_eq!(
            parsed.paths(),
            entries
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        );
    }

    fn track(database: &Database, path: &str) -> i64 {
        let filename = path.rsplit('/').next().unwrap_or(path).to_string();
        crate::library::queries::insert_track(
            database.connection(),
            &Track {
                id: 0,
                path: path.into(),
                filename,
                folder_id: None,
                file_size: 1,
                modified_time: 0,
                missing: false,
                title: String::new(),
                artist: String::new(),
                album: String::new(),
                album_artist: String::new(),
                genre: None,
                year: None,
                track_number: None,
                disc_number: None,
                duration_ms: Some(1000),
                format: None,
                sample_rate: None,
                channels: None,
                bit_depth: None,
                bitrate: None,
                favorite: false,
                artwork_reference: None,
                ambiguous: false,
                created_at: 0,
                updated_at: 0,
            },
        )
        .unwrap()
    }

    #[test]
    fn import_resolves_known_tracks_and_keeps_the_rest() {
        let database = Database::open_in_memory().unwrap();
        let playlist =
            crate::playlists::queries::create(database.connection(), "Imported").unwrap();
        let known = track(&database, "/m/known.mp3");
        let parsed = parse("/m/known.mp3\n/m/absent.mp3\n", Path::new("/")).unwrap();

        let summary = import(&database, playlist.id, &parsed).unwrap();
        assert_eq!(summary.added, 1);
        assert_eq!(summary.unresolved, 1);
        assert_eq!(summary.total(), 2);

        let entries =
            crate::playlists::queries::entries(database.connection(), playlist.id).unwrap();
        assert_eq!(entries[0].track_id, Some(known));
        assert_eq!(
            entries[1].availability,
            crate::playlists::model::Availability::Unresolved
        );
    }

    #[test]
    fn importing_twice_keeps_both_copies() {
        let database = Database::open_in_memory().unwrap();
        let playlist = crate::playlists::queries::create(database.connection(), "Twice").unwrap();
        let known = track(&database, "/m/a.mp3");
        let parsed = parse("/m/a.mp3\n/m/a.mp3\n", Path::new("/")).unwrap();
        let summary = import(&database, playlist.id, &parsed).unwrap();
        assert_eq!(summary.added, 2);
        assert!(
            crate::playlists::queries::entry_track_ids(database.connection(), playlist.id)
                .unwrap()
                .iter()
                .all(|id| *id == known)
        );
        assert_eq!(
            crate::playlists::queries::entry_track_ids(database.connection(), playlist.id)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn missing_tracks_are_imported_but_not_playable() {
        let database = Database::open_in_memory().unwrap();
        let playlist = crate::playlists::queries::create(database.connection(), "Gone").unwrap();
        let id = track(&database, "/m/gone.mp3");
        crate::library::queries::set_missing(database.connection(), id, true).unwrap();
        let parsed = parse("/m/gone.mp3\n", Path::new("/")).unwrap();
        assert_eq!(import(&database, playlist.id, &parsed).unwrap().added, 1);
        assert!(
            crate::playlists::queries::entry_track_ids(database.connection(), playlist.id)
                .unwrap()
                .is_empty()
        );
    }
}
