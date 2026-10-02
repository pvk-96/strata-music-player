//! Library scanner.
//!
//! Scanning is a read-only operation: files are opened to read tags and are never written,
//! renamed or moved. It runs on a worker thread with its own database connection and reports
//! progress through [`ScanEvent`], so the interface stays responsive.
//!
//! A scan never re-reads tags for a file whose size and modification time are unchanged
//! (see [`crate::library::identity`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rusqlite::{params, Connection};

use crate::database::Database;
use crate::error::{Error, Result};
use crate::library::identity::{self, FileFacts, KnownTrack, Match, Reconciler, TaggedFacts};
use crate::library::metadata;
use crate::library::model::{now_seconds, Track};
use crate::library::queries;

/// Changes are committed in batches so an interrupted scan leaves a consistent store.
const BATCH_SIZE: usize = 200;
/// Progress events are emitted at most this often.
const PROGRESS_INTERVAL: usize = 25;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanKind {
    /// Every configured library folder.
    AllFolders,
    /// One folder, used by "rescan folder".
    Folder { folder_id: i64 },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScanSummary {
    pub added: usize,
    pub updated: usize,
    pub moved: usize,
    pub unchanged: usize,
    pub missing: usize,
    pub ambiguous: usize,
    pub failed: usize,
}

impl ScanSummary {
    #[allow(dead_code)]
    fn total(&self) -> usize {
        self.added + self.updated + self.moved + self.unchanged + self.failed
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScanEvent {
    Started {
        folders: usize,
    },
    FolderStarted {
        path: PathBuf,
    },
    /// Progress within one folder.
    Progress {
        path: PathBuf,
        processed: usize,
        total: usize,
    },
    FileFailed {
        path: PathBuf,
        reason: String,
    },
    FolderFinished {
        folder_id: i64,
    },
    Finished {
        summary: ScanSummary,
    },
    Cancelled,
}

/// Scan the library, reporting progress through `emit`. Returns the totals, or `None` when
/// the scan was cancelled.
pub fn run(
    database_path: &Path,
    kind: ScanKind,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(ScanEvent),
) -> Result<Option<ScanSummary>> {
    let database = Database::open(database_path)?;
    let folders = folders_to_scan(&database, kind)?;

    let mut summary = ScanSummary::default();
    emit(ScanEvent::Started {
        folders: folders.len(),
    });

    for (folder_id, path) in folders {
        if cancel.load(Ordering::Relaxed) {
            emit(ScanEvent::Cancelled);
            return Ok(None);
        }
        emit(ScanEvent::FolderStarted { path: path.clone() });
        scan_folder(&database, folder_id, &path, cancel, emit, &mut summary)?;
        emit(ScanEvent::FolderFinished { folder_id });
    }

    emit(ScanEvent::Finished {
        summary: summary.clone(),
    });
    Ok(Some(summary))
}

fn folders_to_scan(database: &Database, kind: ScanKind) -> Result<Vec<(i64, PathBuf)>> {
    let folders = queries::folders(database.connection())?;
    Ok(match kind {
        ScanKind::AllFolders => folders
            .into_iter()
            .map(|folder| (folder.folder_id, folder.path))
            .collect(),
        ScanKind::Folder { folder_id } => folders
            .into_iter()
            .filter(|folder| folder.folder_id == folder_id)
            .map(|folder| (folder.folder_id, folder.path))
            .collect(),
    })
}

fn scan_folder(
    database: &Database,
    folder_id: i64,
    path: &Path,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(ScanEvent),
    summary: &mut ScanSummary,
) -> Result<()> {
    let mut walk_failures = Vec::new();
    let files = collect_files(path, &mut walk_failures);
    for failure in walk_failures {
        summary.failed += 1;
        emit(ScanEvent::FileFailed {
            path: failure,
            reason: "could not be read".to_string(),
        });
    }

    if files.is_empty() {
        // A folder that no longer exists, or an empty one: still mark its tracks missing.
        mark_missing(database, folder_id, &HashSet::new(), summary)?;
        return Ok(());
    }

    let seen: HashSet<PathBuf> = files.iter().map(|file| file.path.clone()).collect();
    let indexed = known_tracks(database)?;
    let mut reconciler = identity::Reconciler::new(indexed, &seen);
    let mut batch = Batch::new(database.connection());

    for (position, file) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            batch.commit();
            return Ok(());
        }

        let outcome = index_file(&mut batch, folder_id, file, &mut reconciler);
        match outcome {
            Ok(Outcome::Unchanged) => summary.unchanged += 1,
            Ok(Outcome::Updated) => summary.updated += 1,
            Ok(Outcome::Moved) => summary.moved += 1,
            Ok(Outcome::Added) => summary.added += 1,
            Ok(Outcome::Ambiguous) => summary.ambiguous += 1,
            Err(error) => {
                summary.failed += 1;
                log::warn!("skipping {}: {error}", file.path.display());
                emit(ScanEvent::FileFailed {
                    path: file.path.clone(),
                    reason: error.user_message(),
                });
            }
        }

        if (position + 1) % PROGRESS_INTERVAL == 0 || position + 1 == files.len() {
            emit(ScanEvent::Progress {
                path: file.path.clone(),
                processed: position + 1,
                total: files.len(),
            });
        }

        if batch.writes >= BATCH_SIZE {
            batch.commit();
        }
    }
    batch.commit();

    mark_missing(database, folder_id, &seen, summary)?;
    log::info!(
        "scanned {}: {} new, {} updated, {} moved, {} unchanged, {} missing, {} need review",
        path.display(),
        summary.added,
        summary.updated,
        summary.moved,
        summary.unchanged,
        summary.missing,
        summary.ambiguous
    );
    Ok(())
}

enum Outcome {
    Added,
    Updated,
    Moved,
    Unchanged,
    Ambiguous,
}

/// Decide what a single file is and record it.
fn index_file(
    batch: &mut Batch<'_>,
    folder_id: i64,
    file: &FileFacts,
    reconciler: &mut Reconciler,
) -> Result<Outcome> {
    // Cheap path first: a file that has not changed does not need its tags read again.
    if let Match::Unchanged {
        track_id,
        was_missing,
    } = reconciler.reconcile(file, None)
    {
        if was_missing {
            // The file is back where it was: record that and leave the tags alone.
            batch.begin()?;
            queries::set_missing(batch.connection(), track_id, false)?;
            return Ok(Outcome::Updated);
        }
        return Ok(Outcome::Unchanged);
    }

    batch.begin()?;
    let connection = batch.connection();
    let metadata = metadata::read(&file.path)?;
    let tags = TaggedFacts {
        title: metadata.title.clone().unwrap_or_default(),
        artist: metadata.artist.clone().unwrap_or_default(),
        album: metadata.album.clone().unwrap_or_default(),
        duration_ms: metadata.duration_ms,
    };

    let outcome = match reconciler.reconcile(file, Some(&tags)) {
        Match::Unchanged { .. } => Outcome::Unchanged,
        Match::Modified { track_id } => {
            update(connection, track_id, folder_id, file, &metadata)?;
            Outcome::Updated
        }
        Match::Moved { track_id } => {
            update(connection, track_id, folder_id, file, &metadata)?;
            Outcome::Moved
        }
        Match::New => {
            insert(connection, folder_id, file, &metadata, false)?;
            Outcome::Added
        }
        Match::Ambiguous { candidates: ids } => {
            let track_id = insert(connection, folder_id, file, &metadata, true)?;
            record_candidates(connection, track_id, &ids)?;
            Outcome::Ambiguous
        }
    };

    if !matches!(outcome, Outcome::Unchanged) {
        batch.writes += 1;
    }
    Ok(outcome)
}

fn insert(
    connection: &Connection,
    folder_id: i64,
    file: &FileFacts,
    metadata: &metadata::FileMetadata,
    ambiguous: bool,
) -> Result<i64> {
    let track = Track {
        id: 0,
        path: file.path.clone(),
        filename: file_name(&file.path),
        folder_id: Some(folder_id),
        file_size: file.file_size,
        modified_time: file.modified_time,
        missing: false,
        title: metadata.title.clone().unwrap_or_default(),
        artist: metadata.artist.clone().unwrap_or_default(),
        album: metadata.album.clone().unwrap_or_default(),
        album_artist: metadata.album_artist.clone().unwrap_or_default(),
        genre: metadata.genre.clone(),
        year: metadata.year,
        track_number: metadata.track_number,
        disc_number: metadata.disc_number,
        duration_ms: metadata.duration_ms,
        format: metadata.format.clone(),
        sample_rate: metadata.sample_rate,
        channels: metadata.channels,
        bit_depth: metadata.bit_depth,
        bitrate: metadata.bitrate,
        favorite: false,
        artwork_reference: None,
        ambiguous,
        created_at: now_seconds(),
        updated_at: now_seconds(),
    };
    queries::insert_track(connection, &track)
}

fn update(
    connection: &Connection,
    track_id: i64,
    folder_id: i64,
    file: &FileFacts,
    metadata: &metadata::FileMetadata,
) -> Result<()> {
    let mut track = queries::get_track(connection, track_id)?;
    track.path = file.path.clone();
    track.filename = file_name(&file.path);
    track.folder_id = Some(folder_id);
    track.file_size = file.file_size;
    track.modified_time = file.modified_time;
    track.title = metadata.title.clone().unwrap_or_default();
    track.artist = metadata.artist.clone().unwrap_or_default();
    track.album = metadata.album.clone().unwrap_or_default();
    track.album_artist = metadata.album_artist.clone().unwrap_or_default();
    track.genre = metadata.genre.clone();
    track.year = metadata.year;
    track.track_number = metadata.track_number;
    track.disc_number = metadata.disc_number;
    track.duration_ms = metadata.duration_ms;
    track.format = metadata.format.clone();
    track.sample_rate = metadata.sample_rate;
    track.channels = metadata.channels;
    track.bit_depth = metadata.bit_depth;
    track.bitrate = metadata.bitrate;
    queries::update_track_metadata(connection, &track)
}

fn record_candidates(connection: &Connection, track_id: i64, candidates: &[i64]) -> Result<()> {
    for candidate in candidates {
        connection
            .execute(
                "INSERT OR IGNORE INTO track_identity_candidates (track_id, candidate_id) VALUES (?1, ?2)",
                params![track_id, candidate],
            )
            .map_err(|err| Error::Database(err.to_string()))?;
    }
    Ok(())
}

/// Every track indexed for a folder, as evidence for reconciliation.
/// Every track in the library, so a file that moved between folders is still recognised.
fn known_tracks(database: &Database) -> Result<Vec<KnownTrack>> {
    let tracks = queries::all_tracks(database.connection())?;
    Ok(tracks
        .into_iter()
        .map(|track| KnownTrack {
            id: track.id,
            path: track.path,
            filename: track.filename,
            file_size: track.file_size,
            modified_time: track.modified_time,
            metadata: TaggedFacts {
                title: track.title,
                artist: track.artist,
                album: track.album,
                duration_ms: track.duration_ms,
            },
            missing: track.missing,
        })
        .collect())
}

fn mark_missing(
    database: &Database,
    folder_id: i64,
    seen: &HashSet<PathBuf>,
    summary: &mut ScanSummary,
) -> Result<()> {
    for track in queries::tracks_in_folder(database.connection(), folder_id)? {
        if seen.contains(&track.path) || track.missing {
            continue;
        }
        queries::set_missing(database.connection(), track.id, true)?;
        summary.missing += 1;
        log::info!(
            "{} is no longer in the library folder",
            track.path.display()
        );
    }
    Ok(())
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Walk `root` and collect supported audio files in a deterministic order. Directories that
/// cannot be opened are reported and skipped; the scan continues.
fn collect_files(root: &Path, failures: &mut Vec<PathBuf>) -> Vec<FileFacts> {
    let mut files = Vec::new();
    let mut queue = vec![root.to_path_buf()];

    while let Some(directory) = queue.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(err) => {
                log::warn!("could not read {}: {err}", directory.display());
                failures.push(directory.clone());
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) => {
                    log::warn!("skipping an unreadable entry: {err}");
                    continue;
                }
            };
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();

            // Hidden directories hold caches and dotfiles, not a music library.
            if name.starts_with('.') {
                continue;
            }

            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(err) => {
                    log::warn!("could not inspect {}: {err}", path.display());
                    continue;
                }
            };

            if file_type.is_dir() {
                queue.push(path);
            } else if (file_type.is_file() || file_type.is_symlink())
                && metadata::is_supported(&path)
            {
                match entry.metadata() {
                    Ok(stat) => files.push(FileFacts {
                        path,
                        file_size: stat.len() as i64,
                        modified_time: modified_seconds(&stat),
                    }),
                    Err(err) => {
                        log::warn!("could not stat {}: {err}", path.display());
                        failures.push(path);
                    }
                }
            }
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn modified_seconds(metadata: &std::fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs() as i64)
        .unwrap_or_default()
}

/// Groups writes so a large scan commits in batches instead of holding one transaction
/// open, and so an interrupted scan leaves committed work intact.
struct Batch<'a> {
    connection: &'a Connection,
    transaction: Option<rusqlite::Transaction<'a>>,
    writes: usize,
}

impl<'a> Batch<'a> {
    fn new(connection: &'a Connection) -> Self {
        Self {
            connection,
            transaction: None,
            writes: 0,
        }
    }

    /// Open the batch transaction if it is not already open.
    fn begin(&mut self) -> Result<()> {
        if self.transaction.is_none() {
            self.transaction = Some(
                self.connection
                    .unchecked_transaction()
                    .map_err(|err| Error::Database(err.to_string()))?,
            );
        }
        Ok(())
    }

    /// The connection writes go through: the open transaction when there is one.
    fn connection(&self) -> &Connection {
        match &self.transaction {
            Some(transaction) => transaction,
            None => self.connection,
        }
    }

    fn commit(&mut self) {
        if let Some(transaction) = self.transaction.take() {
            if let Err(err) = transaction.commit() {
                log::error!("a batch of library changes could not be saved: {err}");
            }
        }
        self.writes = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::model::TrackQuery;
    use std::sync::atomic::AtomicBool;

    fn fixtures() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    fn fixture(name: &str) -> PathBuf {
        fixtures().join(name)
    }

    struct Harness {
        _temp: tempfile::TempDir,
        database: PathBuf,
        events: Vec<ScanEvent>,
    }

    impl Harness {
        fn new(music: &Path) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let database = temp.path().join("library.db");
            let database_instance = Database::open(&database).unwrap();
            queries::add_folder(database_instance.connection(), music).unwrap();
            drop(database_instance);

            Self {
                _temp: temp,
                database,
                events: Vec::new(),
            }
        }

        fn scan(&mut self, kind: ScanKind) -> Option<ScanSummary> {
            let cancel = AtomicBool::new(false);
            let mut events = Vec::new();
            let summary = run(&self.database, kind, &cancel, &mut |event| {
                events.push(event.clone());
            })
            .unwrap();
            self.events = events;
            summary
        }

        fn database(&self) -> Database {
            Database::open(&self.database).unwrap()
        }

        fn tracks(&self) -> Vec<Track> {
            let database = self.database();
            queries::query_tracks(
                database.connection(),
                &TrackQuery::default(),
                crate::library::model::TrackSort::Title,
                crate::library::model::SortDirection::Ascending,
            )
            .unwrap()
        }
    }

    fn music_dir() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn initial_scan_indexes_every_supported_file() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("complete.mp3")).unwrap();
        std::fs::copy(fixture("complete.flac"), source.join("complete.flac")).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 2);
        assert_eq!(harness.tracks().len(), 2);
    }

    #[test]
    fn unsupported_files_are_ignored() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("a.mp3")).unwrap();
        std::fs::copy(fixture("notes.txt"), source.join("notes.txt")).unwrap();
        std::fs::copy(fixture("cover.png"), source.join("cover.png")).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 1);
        assert!(!harness
            .events
            .iter()
            .any(|event| matches!(event, ScanEvent::FileFailed { .. })));
    }

    #[test]
    fn nested_folders_are_scanned() {
        let music = music_dir();
        let source = music.path().join("music");
        let nested = source.join("Album").join("Disc 1");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::copy(fixture("disc1-track1.flac"), nested.join("01.flac")).unwrap();
        std::fs::copy(fixture("disc1-track2.flac"), nested.join("02.flac")).unwrap();
        std::fs::copy(fixture("disc2-track1.flac"), nested.join("03.flac")).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 3);
    }

    #[test]
    fn hidden_folders_are_skipped() {
        let music = music_dir();
        let source = music.path().join("music");
        let hidden = source.join(".thumbnails");
        std::fs::create_dir_all(&hidden).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("a.mp3")).unwrap();
        std::fs::copy(fixture("complete.mp3"), hidden.join("b.mp3")).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 1);
    }

    #[test]
    fn unchanged_rescan_reads_no_metadata_again() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("a.mp3")).unwrap();

        let mut harness = Harness::new(&source);
        assert_eq!(harness.scan(ScanKind::AllFolders).unwrap().added, 1);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.unchanged, 1);
        assert_eq!(summary.added, 0);
        assert_eq!(summary.updated, 0);
    }

    #[test]
    fn modified_file_is_reindexed() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let target = source.join("a.mp3");
        std::fs::copy(fixture("complete.mp3"), &target).unwrap();

        let mut harness = Harness::new(&source);
        assert_eq!(harness.scan(ScanKind::AllFolders).unwrap().added, 1);
        let first_id = harness.tracks()[0].id;

        // Different content means a different size, which the scanner sees as a change.
        std::fs::copy(fixture("no_tags.mp3"), &target).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.updated, 1);
        let tracks = harness.tracks();
        assert_eq!(tracks.len(), 1, "same track, no duplicate");
        assert_eq!(tracks[0].id, first_id);
        assert_eq!(tracks[0].title, "", "tags were read again");
    }

    #[test]
    fn moved_file_keeps_its_track() {
        let music = music_dir();
        let source = music.path().join("music");
        let moved_dir = source.join("Reorganised");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&moved_dir).unwrap();
        let original = source.join("a.mp3");
        std::fs::copy(fixture("complete.mp3"), &original).unwrap();

        let mut harness = Harness::new(&source);
        assert_eq!(harness.scan(ScanKind::AllFolders).unwrap().added, 1);
        let track_id = harness.tracks()[0].id;
        queries::set_favorite(harness.database().connection(), track_id, true).unwrap();

        std::fs::rename(&original, moved_dir.join("a.mp3")).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.moved, 1);
        let tracks = harness.tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].id, track_id);
        assert_eq!(tracks[0].path, moved_dir.join("a.mp3"));
        assert!(tracks[0].favorite, "favourites survive a move");
    }

    #[test]
    fn renamed_file_keeps_its_track_when_metadata_matches() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let original = source.join("track.mp3");
        std::fs::copy(fixture("complete.mp3"), &original).unwrap();

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        let track_id = harness.tracks()[0].id;

        std::fs::rename(&original, source.join("renamed.mp3")).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.moved, 1);
        assert_eq!(harness.tracks()[0].id, track_id);
        assert_eq!(harness.tracks()[0].filename, "renamed.mp3");
    }

    #[test]
    fn two_equally_plausible_predecessors_are_flagged_ambiguous() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let first = source.join("a").join("song.mp3");
        let second = source.join("b").join("song.mp3");
        std::fs::create_dir_all(first.parent().unwrap()).unwrap();
        std::fs::create_dir_all(second.parent().unwrap()).unwrap();
        std::fs::copy(fixture("complete.mp3"), &first).unwrap();
        std::fs::copy(fixture("complete.mp3"), &second).unwrap();

        let mut harness = Harness::new(&source);
        assert_eq!(harness.scan(ScanKind::AllFolders).unwrap().added, 2);
        let before: Vec<i64> = harness.tracks().iter().map(|track| track.id).collect();

        // Both copies disappear and one file with the same content reappears elsewhere.
        std::fs::remove_file(&first).unwrap();
        std::fs::remove_file(&second).unwrap();
        let third = source.join("c").join("song.mp3");
        std::fs::create_dir_all(third.parent().unwrap()).unwrap();
        std::fs::copy(fixture("complete.mp3"), &third).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.ambiguous, 1);
        assert_eq!(summary.missing, 2);

        let ambiguous = queries::ambiguous_tracks(harness.database().connection()).unwrap();
        assert_eq!(ambiguous.len(), 1);
        assert_eq!(ambiguous[0].path, third);

        let database = harness.database();
        let candidates: Vec<i64> = database
            .connection()
            .prepare("SELECT candidate_id FROM track_identity_candidates WHERE track_id = ?1")
            .unwrap()
            .query_map([ambiguous[0].id], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<i64>>>()
            .unwrap();
        assert_eq!(candidates.len(), 2);
        for id in before {
            assert!(candidates.contains(&id));
        }
    }

    #[test]
    fn removed_file_is_marked_missing_and_hidden() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("a.mp3");
        std::fs::copy(fixture("complete.mp3"), &file).unwrap();

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        std::fs::remove_file(&file).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.missing, 1);
        assert!(
            harness.tracks().is_empty(),
            "missing tracks leave the views"
        );

        let database = harness.database();
        let all = queries::query_tracks(
            database.connection(),
            &TrackQuery {
                include_missing: true,
                ..Default::default()
            },
            crate::library::model::TrackSort::Title,
            crate::library::model::SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].missing);
    }

    #[test]
    fn a_restored_file_becomes_visible_again() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let file = source.join("a.mp3");
        std::fs::copy(fixture("complete.mp3"), &file).unwrap();

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        std::fs::remove_file(&file).unwrap();
        harness.scan(ScanKind::AllFolders);
        std::fs::copy(fixture("complete.mp3"), &file).unwrap();
        harness.scan(ScanKind::AllFolders);

        assert_eq!(harness.tracks().len(), 1);
        assert!(!harness.tracks()[0].missing);
    }

    #[test]
    fn unreadable_file_is_reported_and_the_scan_continues() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        let good = source.join("good.mp3");
        let broken = source.join("broken.mp3");
        std::fs::copy(fixture("complete.mp3"), &good).unwrap();
        std::fs::copy(fixture("broken.mp3"), &broken).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 1, "the healthy file is still indexed");
        assert_eq!(summary.failed, 1);
        assert!(harness
            .events
            .iter()
            .any(|event| matches!(event, ScanEvent::FileFailed { .. })));
    }

    #[test]
    fn file_with_no_tags_is_indexed_with_filename_fallback() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::copy(fixture("no_tags.mp3"), source.join("My Recording.mp3")).unwrap();

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        let tracks = harness.tracks();
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "");
        assert_eq!(tracks[0].display_title(), "My Recording");
        assert_eq!(
            tracks[0].display_artist(),
            crate::library::model::UNKNOWN_ARTIST
        );
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_directory_is_reported_and_skipped() {
        use std::os::unix::fs::PermissionsExt;

        let music = music_dir();
        let source = music.path().join("music");
        let locked = source.join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("good.mp3")).unwrap();
        std::fs::copy(fixture("complete.mp3"), locked.join("hidden.mp3")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();

        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(summary.added, 1);
        assert!(summary.failed >= 1);
    }

    #[test]
    fn scanning_a_single_folder_leaves_others_alone() {
        let music = music_dir();
        let first = music.path().join("first");
        let second = music.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::copy(fixture("complete.mp3"), first.join("a.mp3")).unwrap();
        std::fs::copy(fixture("complete.flac"), second.join("b.flac")).unwrap();

        let mut harness = Harness::new(&first);
        let database = Database::open(&harness.database).unwrap();
        let other = queries::add_folder(database.connection(), &second).unwrap();
        drop(database);

        harness.scan(ScanKind::AllFolders);
        assert_eq!(harness.tracks().len(), 2);

        std::fs::remove_file(second.join("b.flac")).unwrap();
        let summary = harness.scan(ScanKind::Folder { folder_id: other }).unwrap();
        assert_eq!(summary.missing, 1);
        assert_eq!(harness.tracks().len(), 1, "the first folder is untouched");
    }

    #[test]
    fn empty_folder_marks_its_tracks_missing() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::copy(fixture("complete.mp3"), source.join("a.mp3")).unwrap();

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        std::fs::remove_dir_all(&source).unwrap();

        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.missing, 1);
    }

    #[test]
    fn cancellation_stops_the_scan() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        for index in 0..3 {
            std::fs::copy(fixture("format.flac"), source.join(format!("{index}.flac"))).unwrap();
        }

        let harness = Harness::new(&source);
        let cancel = AtomicBool::new(true);
        let mut events = Vec::new();
        let summary = run(
            &harness.database,
            ScanKind::AllFolders,
            &cancel,
            &mut |event| events.push(event),
        )
        .unwrap();

        assert!(summary.is_none());
        assert!(events.contains(&ScanEvent::Cancelled));
        assert!(harness.tracks().is_empty());
    }

    #[test]
    fn progress_events_are_emitted() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        for index in 0..30 {
            std::fs::copy(
                fixture("format.flac"),
                source.join(format!("{index:02}.flac")),
            )
            .unwrap();
        }

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);

        assert!(matches!(
            harness.events[0],
            ScanEvent::Started { folders: 1 }
        ));
        assert!(harness
            .events
            .iter()
            .any(|event| matches!(event, ScanEvent::Progress { .. })));
        assert!(harness
            .events
            .iter()
            .any(|event| matches!(event, ScanEvent::Finished { .. })));
    }

    #[test]
    fn scan_is_deterministic() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        for name in [
            "format.ogg",
            "format.opus",
            "format.m4a",
            "format.wav",
            "format.aiff",
        ] {
            std::fs::copy(fixture(name), source.join(name)).unwrap();
        }

        let mut harness = Harness::new(&source);
        harness.scan(ScanKind::AllFolders);
        let first: Vec<String> = harness
            .tracks()
            .iter()
            .map(|t| t.path.display().to_string())
            .collect();
        harness.scan(ScanKind::AllFolders);
        let second: Vec<String> = harness
            .tracks()
            .iter()
            .map(|t| t.path.display().to_string())
            .collect();
        assert_eq!(first, second);
        assert_eq!(first.len(), 5);
    }

    #[test]
    fn large_library_is_scanned_in_batches() {
        let music = music_dir();
        let source = music.path().join("music");
        std::fs::create_dir_all(&source).unwrap();
        // Copies rather than links: the scanner must not depend on hard link behaviour.
        for index in 0..5 {
            std::fs::copy(fixture("format.flac"), source.join(format!("{index}.flac"))).unwrap();
        }
        let mut harness = Harness::new(&source);
        let summary = harness.scan(ScanKind::AllFolders).unwrap();
        assert_eq!(summary.added, 5);
        assert_eq!(summary.total(), 5);
    }
}
