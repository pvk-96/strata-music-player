//! Track identity.
//!
//! A track's identity is its position in the library, not its path, so moving or renaming a
//! file keeps the same entry, with its favourites and playlist references intact. Evidence
//! is weighed in a fixed order and ties are never resolved by guessing: when two indexed
//! tracks fit equally well the file is indexed on its own and marked ambiguous for the user
//! to resolve. See `docs/scanner.md`.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// What the filesystem says about a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileFacts {
    pub path: PathBuf,
    pub file_size: i64,
    pub modified_time: i64,
}

/// What a file's tags say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaggedFacts {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
}

/// A track already in the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownTrack {
    pub id: i64,
    pub path: PathBuf,
    pub filename: String,
    pub file_size: i64,
    pub modified_time: i64,
    pub metadata: TaggedFacts,
    /// True when a previous scan did not find this file.
    pub missing: bool,
}

/// What should happen to a file found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Match {
    /// Same path, same size and modification time: tags do not need reading again.
    /// `was_missing` is true when the file had disappeared in an earlier scan and is back,
    /// which the scanner only has to record in the library.
    Unchanged { track_id: i64, was_missing: bool },
    /// Same path, but the file changed: read tags and update the track.
    Modified { track_id: i64 },
    /// The file is a track that moved or was renamed: keep the track, update its path.
    Moved { track_id: i64 },
    /// Several indexed tracks fit equally well. Strata indexes the file separately and
    /// asks the user which track, if any, it belongs to.
    Ambiguous { candidates: Vec<i64> },
    /// Nothing in the index explains this file.
    New,
}

/// Durations within this distance count as the same recording.
const DURATION_TOLERANCE_MS: i64 = 500;
/// Durations further apart than this are evidence against a candidate.
const DURATION_CONFLICT_MS: i64 = 5_000;

/// Matches the files found in one scan against the tracks already in the index.
///
/// The reconciler remembers which track has already been claimed by a file during the
/// scan, so a track cannot explain two different files.
#[derive(Debug)]
pub struct Reconciler {
    /// Indexed tracks by path, for files that did not move.
    by_path: HashMap<PathBuf, KnownTrack>,
    /// Tracks whose file was not found: the only ones that may explain a new file.
    available: Vec<KnownTrack>,
}

impl Reconciler {
    /// Build a reconciler for one scan.
    ///
    /// `seen` holds the paths of every audio file the walk found. A track whose path is in
    /// `seen` explains only that file; a track whose file is gone may also explain a file
    /// that turned up somewhere else.
    pub fn new(indexed: Vec<KnownTrack>, seen: &HashSet<PathBuf>) -> Self {
        let mut by_path = HashMap::with_capacity(indexed.len());
        let mut available = Vec::new();
        for track in indexed {
            if !seen.contains(&track.path) && !track.path.exists() {
                available.push(track.clone());
            }
            by_path.insert(track.path.clone(), track);
        }
        Self { by_path, available }
    }

    /// Decide what `file` is. `tags` is `None` when the caller deliberately did not read
    /// them, which happens only for files that did not change.
    pub fn reconcile(&mut self, file: &FileFacts, tags: Option<&TaggedFacts>) -> Match {
        if let Some(known) = self.by_path.get(&file.path) {
            if known.file_size == file.file_size && known.modified_time == file.modified_time {
                return Match::Unchanged {
                    track_id: known.id,
                    was_missing: known.missing,
                };
            }
            return Match::Modified { track_id: known.id };
        }

        let tags = match tags {
            Some(tags) => tags,
            None => return Match::New,
        };

        let mut scored: Vec<(i64, i64)> = self
            .available
            .iter()
            .filter_map(|known| weight(known, file, tags).map(|weight| (weight, known.id)))
            .collect();

        if scored.is_empty() {
            return Match::New;
        }

        scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let best = scored[0].0;
        let tied: Vec<i64> = scored
            .iter()
            .take_while(|(weight, _)| *weight == best)
            .map(|(_, id)| *id)
            .collect();

        if tied.len() == 1 {
            let track_id = tied[0];
            self.claim(track_id);
            Match::Moved { track_id }
        } else {
            Match::Ambiguous { candidates: tied }
        }
    }

    /// Ids of indexed tracks whose file was not found during the scan.
    pub fn vanished_ids(&self) -> Vec<i64> {
        self.available.iter().map(|known| known.id).collect()
    }

    /// A track that has just been matched explains no further file.
    fn claim(&mut self, track_id: i64) {
        self.available.retain(|known| known.id != track_id);
    }
}

/// Evidence weight for one candidate, or `None` when the file cannot be that candidate.
fn weight(known: &KnownTrack, file: &FileFacts, tags: &TaggedFacts) -> Option<i64> {
    if let (Some(found), Some(indexed)) = (tags.duration_ms, known.metadata.duration_ms) {
        if (found - indexed).abs() > DURATION_CONFLICT_MS {
            return None;
        }
    }

    let file_name = file
        .path
        .file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let same_name = !file_name.is_empty() && file_name == known.filename.to_ascii_lowercase();
    let same_size = known.file_size == file.file_size;
    let same_duration = matches!(
        (tags.duration_ms, known.metadata.duration_ms),
        (Some(found), Some(indexed)) if (found - indexed).abs() <= DURATION_TOLERANCE_MS
    );
    let same_title = equal_when_present(&known.metadata.title, &tags.title);
    let same_artist = equal_when_present(&known.metadata.artist, &tags.artist);
    let same_album = equal_when_present(&known.metadata.album, &tags.album);

    // A file has to look like the same recording, not merely a similar one: either it kept
    // its name and size, or it kept its size, title and artist.
    let strong = (same_name && same_size)
        || (same_size && same_duration)
        || (same_size && same_title && same_artist);
    if !strong {
        return None;
    }

    let mut weight = 0;
    weight += i64::from(same_name) * 4;
    weight += i64::from(same_size) * 2;
    weight += i64::from(same_duration) * 2;
    weight += i64::from(same_title) * 2;
    weight += i64::from(same_artist) * 2;
    weight += i64::from(same_album);
    Some(weight)
}

fn equal_when_present(a: &str, b: &str) -> bool {
    let a = a.trim();
    let b = b.trim();
    !a.is_empty() && !b.is_empty() && a.eq_ignore_ascii_case(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(path: &str, size: i64, mtime: i64) -> FileFacts {
        FileFacts {
            path: PathBuf::from(path),
            file_size: size,
            modified_time: mtime,
        }
    }

    fn tags(title: &str, artist: &str, album: &str, duration: Option<i64>) -> TaggedFacts {
        TaggedFacts {
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            duration_ms: duration,
        }
    }

    fn known(id: i64, path: &str, filename: &str, size: i64, mtime: i64) -> KnownTrack {
        KnownTrack {
            id,
            path: PathBuf::from(path),
            filename: filename.to_string(),
            file_size: size,
            modified_time: mtime,
            metadata: tags("", "", "", None),
            missing: false,
        }
    }

    /// A reconciler that treats every indexed path as still present.
    fn reconciler(indexed: Vec<KnownTrack>) -> Reconciler {
        let seen: HashSet<PathBuf> = indexed.iter().map(|track| track.path.clone()).collect();
        Reconciler::new(indexed, &seen)
    }

    #[test]
    fn unchanged_file_is_not_reparsed() {
        let mut index = reconciler(vec![known(7, "/m/song.mp3", "song.mp3", 1000, 42)]);
        assert_eq!(
            index.reconcile(&facts("/m/song.mp3", 1000, 42), None),
            Match::Unchanged {
                track_id: 7,
                was_missing: false
            }
        );
    }

    #[test]
    fn changed_timestamp_is_a_modification() {
        let mut index = reconciler(vec![known(7, "/m/song.mp3", "song.mp3", 1000, 42)]);
        assert_eq!(
            index.reconcile(&facts("/m/song.mp3", 1000, 43), None),
            Match::Modified { track_id: 7 }
        );
    }

    #[test]
    fn changed_size_is_a_modification() {
        let mut index = reconciler(vec![known(7, "/m/song.mp3", "song.mp3", 1000, 42)]);
        assert_eq!(
            index.reconcile(&facts("/m/song.mp3", 2000, 42), None),
            Match::Modified { track_id: 7 }
        );
    }

    #[test]
    fn moved_file_keeps_its_identity() {
        // The old path no longer exists, so the track can explain the new path.
        let mut index = Reconciler::new(
            vec![known(7, "/does/not/exist.mp3", "song.mp3", 1000, 42)],
            &HashSet::new(),
        );
        assert_eq!(
            index.reconcile(
                &facts("/m/artist/song.mp3", 1000, 42),
                Some(&tags("Song", "", "", None))
            ),
            Match::Moved { track_id: 7 }
        );
    }

    #[test]
    fn renamed_and_resorted_file_keeps_its_identity() {
        let mut candidate = known(3, "/does/not/exist.mp3", "song.mp3", 5000, 10);
        candidate.metadata = tags("Song", "Artist", "Album", Some(180_000));
        let mut index = Reconciler::new(vec![candidate], &HashSet::new());
        assert_eq!(
            index.reconcile(
                &facts("/m/Various Artist/01 - Song.mp3", 5000, 10),
                Some(&tags("Song", "Artist", "Album", Some(180_000)))
            ),
            Match::Moved { track_id: 3 }
        );
    }

    #[test]
    fn unrelated_file_is_new() {
        let mut index = reconciler(vec![known(7, "/m/song.mp3", "song.mp3", 1000, 42)]);
        assert_eq!(
            index.reconcile(
                &facts("/m/other.mp3", 999, 5),
                Some(&tags("Other", "", "", Some(60_000)))
            ),
            Match::New
        );
    }

    #[test]
    fn same_size_but_unrelated_tags_is_new() {
        let mut candidate = known(7, "/m/song.mp3", "song.mp3", 1000, 42);
        candidate.metadata = tags("Different Song", "Someone", "Record", Some(30_000));
        let mut index = reconciler(vec![candidate]);
        assert_eq!(
            index.reconcile(
                &facts("/m/other.mp3", 1000, 42),
                Some(&tags("Another", "Other", "Record", Some(300_000)))
            ),
            Match::New
        );
    }

    #[test]
    fn two_identical_candidates_are_ambiguous() {
        let mut index = Reconciler::new(
            vec![
                known(1, "/does/not/exist/a.mp3", "song.mp3", 1000, 42),
                known(2, "/does/not/exist/b.mp3", "song.mp3", 1000, 42),
            ],
            &HashSet::new(),
        );
        assert_eq!(
            index.reconcile(
                &facts("/m/moved/song.mp3", 1000, 42),
                Some(&tags("Song", "", "", None))
            ),
            Match::Ambiguous {
                candidates: vec![1, 2]
            }
        );
    }

    #[test]
    fn the_better_supported_candidate_wins() {
        let mut matching = known(1, "/does/not/exist/a.mp3", "song.mp3", 1000, 42);
        matching.metadata = tags("Song", "", "", Some(200_000));
        let mut conflicting = known(2, "/does/not/exist/b.mp3", "song.mp3", 1000, 42);
        conflicting.metadata = tags("Song", "", "", Some(900_000));
        let mut index = Reconciler::new(vec![matching, conflicting], &HashSet::new());
        assert_eq!(
            index.reconcile(
                &facts("/m/moved/song.mp3", 1000, 42),
                Some(&tags("Song", "", "", Some(200_000)))
            ),
            Match::Moved { track_id: 1 }
        );
    }

    #[test]
    fn conflicting_duration_rules_a_candidate_out() {
        let mut candidate = known(1, "/does/not/exist/a.mp3", "song.mp3", 1000, 42);
        candidate.metadata = tags("Song", "", "", Some(10_000));
        let mut index = Reconciler::new(vec![candidate], &HashSet::new());
        assert_eq!(
            index.reconcile(
                &facts("/m/moved/song.mp3", 1000, 42),
                Some(&tags("Song", "", "", Some(200_000)))
            ),
            Match::New
        );
    }

    #[test]
    fn one_track_cannot_explain_two_files() {
        let mut index = Reconciler::new(
            vec![known(1, "/does/not/exist/song.mp3", "song.mp3", 1000, 42)],
            &HashSet::new(),
        );
        let first = index.reconcile(
            &facts("/m/copy-a/song.mp3", 1000, 42),
            Some(&tags("Song", "", "", None)),
        );
        let second = index.reconcile(
            &facts("/m/copy-b/song.mp3", 1000, 42),
            Some(&tags("Song", "", "", None)),
        );
        assert_eq!(first, Match::Moved { track_id: 1 });
        assert_eq!(second, Match::New, "the track was already claimed");
    }

    #[test]
    fn vanished_ids_report_tracks_whose_file_is_gone() {
        let temp = tempfile::tempdir().unwrap();
        let present = temp.path().join("present.mp3");
        let absent = temp.path().join("absent.mp3");
        std::fs::write(&present, b"x").unwrap();

        let indexed = vec![
            known(1, &present.to_string_lossy(), "present.mp3", 1, 1),
            known(2, &absent.to_string_lossy(), "absent.mp3", 1, 1),
        ];
        let index = Reconciler::new(indexed, &HashSet::from([present.clone()]));
        assert_eq!(index.vanished_ids(), vec![2]);
    }

    #[test]
    fn a_restored_file_is_reported_as_missing_before() {
        let mut candidate = known(5, "/does/not/exist/song.mp3", "song.mp3", 1000, 42);
        candidate.missing = true;
        let mut index = Reconciler::new(vec![candidate], &HashSet::new());
        assert_eq!(
            index.reconcile(&facts("/does/not/exist/song.mp3", 1000, 42), None),
            Match::Unchanged {
                track_id: 5,
                was_missing: true
            }
        );
    }
}
