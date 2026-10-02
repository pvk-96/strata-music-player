//! End-to-end checks over the library, playlists and session, using the fixture files.
//!
//! These drive the crate the way the application does, but without GTK, so they can run in
//! CI on a machine with no display and no audio device.

use std::path::Path;
use std::sync::atomic::AtomicBool;

use strata::database::Database;
use strata::library::model::{SortDirection, TrackQuery, TrackSort};
use strata::library::{queries, scanner};
use strata::playback::state::{PlaybackContext, RepeatMode, Sequencer};

/// A temporary directory holding copies of the fixtures.
fn fixture_dir() -> tempfile::TempDir {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let music = temp.path().join("music");
    std::fs::create_dir_all(&music).expect("a music folder");
    for name in [
        "complete.flac",
        "disc1-track1.flac",
        "format.ogg",
        "no tags.mp3",
    ] {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        assert!(source.exists(), "the {name} fixture is present");
        std::fs::copy(&source, music.join(name)).expect("to copy a fixture");
    }
    temp
}

fn open(temp: &tempfile::TempDir) -> Database {
    let database = Database::open(&temp.path().join("library.db")).expect("a database");
    queries::add_folder(database.connection(), &temp.path().join("music")).expect("a folder");
    database
}

fn scan(database: &Database) -> scanner::ScanSummary {
    let cancel = AtomicBool::new(false);
    scanner::run(
        database.path(),
        scanner::ScanKind::AllFolders,
        &cancel,
        &mut |_| {},
    )
    .expect("a scan")
    .expect("an uninterrupted scan")
}

fn all_tracks(database: &Database) -> Vec<strata::library::model::Track> {
    queries::query_tracks(
        database.connection(),
        &TrackQuery::default(),
        TrackSort::DateAdded,
        SortDirection::Descending,
    )
    .expect("the track query runs")
}

#[test]
fn scanning_reads_tags_from_the_files() {
    let temp = fixture_dir();
    let database = open(&temp);
    let summary = scan(&database);
    assert_eq!(summary.added as i64, 4, "every fixture is indexed");
    assert_eq!(summary.failed, 0, "none of the fixtures fail");

    let tracks = all_tracks(&database);
    assert_eq!(
        tracks.len() as i64,
        queries::count_tracks(database.connection()).unwrap()
    );

    let complete = tracks
        .iter()
        .find(|track| track.path.ends_with("complete.flac"))
        .expect("complete.flac is indexed");
    assert!(complete.duration_ms.unwrap_or_default() > 0, "duration");
    assert!(!complete.artist.is_empty(), "artist");
    assert!(!complete.title.is_empty(), "title");
    assert!(!complete.album.is_empty(), "album");
    assert_eq!(complete.format.as_deref(), Some("FLAC"));

    // A file without tags keeps empty tag fields but still displays and searches by name.
    let untagged = tracks
        .iter()
        .find(|track| track.filename == "no tags.mp3")
        .expect("the untagged file is indexed");
    assert!(untagged.title.is_empty(), "no title tag to store");
    assert_eq!(untagged.display_title(), "no tags");
    assert_eq!(untagged.display_artist(), "Unknown Artist");
    assert!(untagged.duration_ms.unwrap_or_default() > 0);
}

#[test]
fn rescanning_does_not_duplicate_tracks() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let after_first = queries::count_tracks(database.connection()).unwrap();

    let second = scan(&database);
    assert_eq!(second.added, 0, "the second scan adds nothing");
    assert_eq!(second.unchanged as i64, after_first);
    assert_eq!(
        queries::count_tracks(database.connection()).unwrap(),
        after_first
    );
}

#[test]
fn a_moved_file_is_relocated_rather_than_re_added() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let before = queries::count_tracks(database.connection()).unwrap();

    let music = temp.path().join("music");
    let archive = temp.path().join("archive");
    std::fs::create_dir_all(&archive).expect("an archive folder");
    let to = archive.join("complete.flac");
    std::fs::rename(music.join("complete.flac"), &to).expect("to move the file");

    // The new folder is watched too, so the scan sees the same file twice.
    queries::add_folder(database.connection(), &archive).expect("a second folder");
    let summary = scan(&database);

    assert_eq!(summary.added, 0, "the moved file is not indexed twice");
    assert_eq!(summary.moved, 1, "the move is recognised");
    assert_eq!(
        queries::count_tracks(database.connection()).unwrap(),
        before
    );
    let track = queries::track_at_path(database.connection(), &to)
        .expect("the path query runs")
        .expect("the track is at its new path");
    assert!(track.path.ends_with("archive/complete.flac"));
    assert_eq!(track.folder_id, Some(summary_folders(&database, "archive")));
}

fn summary_folders(database: &Database, name: &str) -> i64 {
    queries::folders(database.connection())
        .expect("the folders load")
        .into_iter()
        .find(|folder| folder.path.ends_with(name))
        .expect("the folder exists")
        .folder_id
}

#[test]
fn a_playlist_holds_tracks_and_survives_a_round_trip() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();

    let playlist = strata::playlists::queries::create(connection, "Evening").expect("created");
    let ids: Vec<i64> = all_tracks(&database)
        .into_iter()
        .map(|track| track.id)
        .collect();
    assert_eq!(
        strata::playlists::queries::add_tracks(connection, playlist.id, &ids).unwrap(),
        ids.len()
    );
    assert_eq!(
        strata::playlists::queries::entry_track_ids(connection, playlist.id).unwrap(),
        ids
    );

    strata::playlists::queries::rename(connection, playlist.id, "Late evening").unwrap();
    assert_eq!(
        strata::playlists::queries::get(connection, playlist.id)
            .unwrap()
            .name,
        "Late evening"
    );
    assert_eq!(
        strata::playlists::queries::detail(connection, playlist.id)
            .unwrap()
            .playlist
            .name,
        "Late evening"
    );

    // Removing a track keeps the rest in order.
    let entries = strata::playlists::queries::entries(connection, playlist.id).unwrap();
    strata::playlists::queries::remove_entry(connection, entries[0].id).unwrap();
    assert_eq!(
        strata::playlists::queries::entry_track_ids(connection, playlist.id).unwrap(),
        ids[1..].to_vec()
    );

    strata::playlists::queries::delete(connection, playlist.id).unwrap();
    assert!(strata::playlists::queries::get(connection, playlist.id).is_err());
}

#[test]
fn an_m3u_import_keeps_unknown_entries_as_references() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);

    let playlist_file = temp.path().join("mix.m3u");
    let known = temp.path().join("music/complete.flac");
    std::fs::write(
        &playlist_file,
        format!("#EXTM3U\n{}\n/somewhere/else.mp3\n", known.display()),
    )
    .expect("to write the playlist file");

    let parsed = strata::playlists::m3u::parse_file(&playlist_file).expect("the playlist parses");
    let playlist = strata::playlists::queries::create(database.connection(), "Imported")
        .expect("a playlist is created");
    let summary =
        strata::playlists::m3u::import(&database, playlist.id, &parsed).expect("the import runs");

    assert_eq!(summary.added, 1, "the known file is matched");
    assert_eq!(summary.unresolved, 1, "the unknown file is a reference");
    assert_eq!(summary.total(), 2);
    let detail = strata::playlists::queries::detail(database.connection(), playlist.id).unwrap();
    assert_eq!(detail.unresolved_count(), 1);
}

#[test]
fn exporting_a_playlist_keeps_the_order() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();

    let playlist = strata::playlists::queries::create(connection, "Road trip").expect("created");
    let ids: Vec<i64> = all_tracks(&database)
        .into_iter()
        .map(|track| track.id)
        .collect();
    strata::playlists::queries::add_tracks(connection, playlist.id, &ids).unwrap();

    let entries = strata::playlists::queries::entries(connection, playlist.id).unwrap();
    let pairs: Vec<(std::path::PathBuf, String)> = entries
        .iter()
        .map(|entry| {
            let path = all_tracks(&database)
                .into_iter()
                .find(|track| track.id == entry.track_id.expect("a resolved track"))
                .expect("the track exists")
                .path;
            (path, entry.display_label.clone())
        })
        .collect();

    let text = strata::playlists::m3u::export(&pairs, true);
    assert!(text.starts_with("#EXTM3U"));
    let parsed = strata::playlists::m3u::parse(&text, Path::new("/")).expect("the export parses");
    assert_eq!(parsed.tracks(), pairs.len());

    let plain = strata::playlists::m3u::export(&pairs, false);
    assert!(!plain.contains("#"), "the plain format is paths only");
    assert_eq!(plain.lines().count(), pairs.len());
}

#[test]
fn session_state_is_written_and_read_back() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();

    let ids: Vec<i64> = all_tracks(&database)
        .into_iter()
        .map(|track| track.id)
        .collect();
    let mut sequencer = Sequencer::new(PlaybackContext::library(ids.clone()));
    sequencer.set_state(strata::playback::state::PlaybackState::Playing);
    sequencer.play(ids[0]).expect("the first track can start");
    sequencer.set_position_ms(42_000);
    sequencer
        .save(connection, 0.75, false)
        .expect("the session is saved");

    let (restored, volume, muted) = Sequencer::restore(connection)
        .expect("the session loads")
        .expect("a saved session exists");
    assert_eq!(volume, 0.75, "the volume is restored");
    assert!(!muted, "the mute state is restored");
    assert_eq!(
        restored.current_track_id(),
        Some(ids[0]),
        "the track is restored"
    );
    assert_eq!(restored.position_ms(), 42_000, "the position is restored");
    assert_eq!(restored.repeat(), RepeatMode::Off);
}

#[test]
fn search_finds_tracks_by_title_and_artist() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();

    let by_title = queries::query_tracks(
        connection,
        &TrackQuery {
            search: Some("Disc 1".to_string()),
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert!(!by_title.is_empty(), "the disc track is found by title");

    let by_artist = queries::query_tracks(
        connection,
        &TrackQuery {
            search: Some("format".to_string()),
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert!(
        by_artist.iter().any(|track| track.filename == "format.ogg"),
        "an untagged file is found by its name"
    );

    let by_album = queries::query_tracks(
        connection,
        &TrackQuery {
            album: Some("Multi Disc".to_string()),
            ..TrackQuery::default()
        },
        TrackSort::TrackNumber,
        SortDirection::Ascending,
    )
    .unwrap();
    assert_eq!(by_album.len(), 1, "album filtering narrows the list");
}

#[test]
fn search_wildcards_are_taken_literally() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();

    let results = queries::query_tracks(
        connection,
        &TrackQuery {
            search: Some("%".to_string()),
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert!(results.is_empty(), "`%` matches no literal percent sign");
}

#[test]
fn favourites_can_be_set_and_cleared() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();
    let track = all_tracks(&database)[0].id;

    queries::set_favorite(connection, track, true).unwrap();
    let favorites = queries::query_tracks(
        connection,
        &TrackQuery {
            favorites_only: true,
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert_eq!(favorites.len(), 1);
    assert!(favorites[0].favorite);

    queries::set_favorite(connection, track, false).unwrap();
    let favorites = queries::query_tracks(
        connection,
        &TrackQuery {
            favorites_only: true,
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert!(favorites.is_empty());
}

#[test]
fn removing_the_files_marks_tracks_missing_without_dropping_them() {
    let temp = fixture_dir();
    let database = open(&temp);
    scan(&database);
    let connection = database.connection();
    let count = queries::count_tracks(connection).unwrap();

    let music = temp.path().join("music");
    let archive = temp.path().join("archive");
    std::fs::create_dir_all(&archive).expect("an archive folder");
    std::fs::rename(music.join("complete.flac"), archive.join("complete.flac")).expect("a move");
    queries::add_folder(connection, &archive).expect("a second folder");
    scan(&database);

    std::fs::remove_dir_all(&music).expect("to remove the music folder");
    std::fs::remove_dir_all(&archive).expect("to remove the archive folder");
    let summary = scan(&database);

    assert_eq!(summary.missing as i64, count, "every track is missing now");
    // `count_tracks` counts what the user can still play; the rows themselves stay.
    assert_eq!(queries::count_tracks(connection).unwrap(), 0);
    let stored: i64 = connection
        .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .expect("the row count runs");
    assert_eq!(stored, count, "missing tracks stay in the library");
    let missing = queries::query_tracks(
        connection,
        &TrackQuery {
            include_missing: true,
            ..TrackQuery::default()
        },
        TrackSort::Title,
        SortDirection::Ascending,
    )
    .unwrap();
    assert!(missing.iter().all(|track| track.missing));

    // Playlist entries for missing tracks survive, so a moved library keeps its playlists.
    let playlist = strata::playlists::queries::create(connection, "Kept").expect("created");
    let id = missing[0].id;
    strata::playlists::queries::add_track(connection, playlist.id, id).unwrap();
    std::fs::create_dir_all(&music).expect("the folder comes back");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/complete.flac"),
        music.join("complete.flac"),
    )
    .expect("a copy");
    let summary = scan(&database);
    assert_eq!(summary.added + summary.moved, 1, "one file returns");
    assert_eq!(
        queries::count_tracks(connection).unwrap(),
        1,
        "only the file that came back"
    );
    let stored: i64 = connection
        .query_row("SELECT COUNT(*) FROM tracks", [], |row| row.get(0))
        .expect("the row count runs");
    assert_eq!(stored, count, "the returned file reused its row");
}
