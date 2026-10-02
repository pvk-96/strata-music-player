//! Measurements for the queries the UI runs on every view change.
//!
//! These are ignored by default because they are slow by design:
//!
//! ```sh
//! cargo test --release -- --ignored --nocapture
//! ```
//!
//! The point is a regression signal, not a number to quote: the same machine, the same
//! synthetic library, before and after a change.

use std::time::{Duration, Instant};

use strata::database::Database;
use strata::library::model::{SortDirection, TrackQuery, TrackSort};
use strata::library::queries;
use strata::library::queries::AlbumSort;

/// How many tracks the synthetic library holds.
const TRACKS: i64 = 20_000;

fn synthetic_library() -> (tempfile::TempDir, Database) {
    let temp = tempfile::tempdir().expect("a temporary directory");
    let database = Database::open(&temp.path().join("library.db")).expect("a database");
    let folder_id = queries::add_folder(database.connection(), temp.path()).expect("a folder");

    let transaction = database
        .connection()
        .unchecked_transaction()
        .expect("a transaction");
    {
        let mut insert = transaction
            .prepare(
                "INSERT INTO tracks
                    (path, filename, folder_id, file_size, modified_time, title, artist, album,
                     album_artist, genre, year, track_number, disc_number, duration_ms, format,
                     sample_rate, channels, bit_depth, bitrate, favorite, artwork_reference,
                     ambiguous, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 'FLAC',
                     44100, 2, 16, 900000, ?15, NULL, 0, ?16, ?16)",
            )
            .expect("the insert statement prepares");
        for index in 0..TRACKS {
            // 200 artists, 10 albums each, 10 tracks per album.
            let artist = format!("Artist {:03}", index / 100);
            let album = format!("Album {:04}", index / 10);
            insert
                .execute(rusqlite::params![
                    format!("/music/{artist}/{album}/{:05}.flac", index),
                    format!("{index:05}.flac"),
                    folder_id,
                    8_000_000 + index,
                    1_700_000_000 + index,
                    format!("Track {index:05}"),
                    artist,
                    album,
                    artist,
                    "Ambient",
                    1990 + index % 30,
                    index % 12 + 1,
                    index % 2 + 1,
                    210_000 + index % 9000,
                    i64::from(index % 7 == 0),
                    1_700_000_000 + index,
                ])
                .expect("the row inserts");
        }
    }
    transaction.commit().expect("the transaction commits");
    (temp, database)
}

/// Median of several runs, so a scheduling hiccup does not look like a regression.
fn measure(runs: u32, mut call: impl FnMut()) -> Duration {
    let mut samples = Vec::with_capacity(runs as usize);
    for _ in 0..runs {
        let start = Instant::now();
        call();
        samples.push(start.elapsed());
    }
    samples.sort();
    samples[runs as usize / 2]
}

#[test]
#[ignore = "benchmark"]
fn library_queries_are_fast_enough_to_run_on_every_view_change() {
    let (_temp, database) = synthetic_library();
    assert_eq!(
        queries::count_tracks(database.connection()).unwrap(),
        TRACKS
    );

    let connection = database.connection();
    let page = measure(9, || {
        let tracks = queries::query_tracks(
            connection,
            &TrackQuery::default(),
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(tracks.len() as i64, TRACKS);
    });
    println!("all tracks, sorted by title      {page:>10.2?}");
    assert!(
        page < Duration::from_millis(500),
        "sorting the library: {page:?}"
    );

    let search = measure(9, || {
        let tracks = queries::query_tracks(
            connection,
            &TrackQuery {
                search: Some("Track 1234".to_string()),
                ..TrackQuery::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert!(!tracks.is_empty());
    });
    println!("search                           {search:>10.2?}");
    assert!(search < Duration::from_millis(200), "search: {search:?}");

    let artists = measure(9, || {
        let artists = queries::artists(connection).unwrap();
        assert_eq!(artists.len(), 200);
    });
    println!("artist list                      {artists:>10.2?}");
    assert!(artists < Duration::from_millis(200), "artists: {artists:?}");

    let albums = measure(9, || {
        let albums = queries::albums(
            connection,
            &TrackQuery::default(),
            AlbumSort::Album,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(albums.len(), 2_000);
    });
    println!("album list                       {albums:>10.2?}");
    assert!(albums < Duration::from_millis(300), "albums: {albums:?}");

    let favourites = measure(9, || {
        let tracks = queries::query_tracks(
            connection,
            &TrackQuery {
                favorites_only: true,
                ..TrackQuery::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert!(!tracks.is_empty());
    });
    println!("favourites                       {favourites:>10.2?}");
    assert!(
        favourites < Duration::from_millis(200),
        "favourites: {favourites:?}"
    );

    let count = measure(9, || {
        queries::count_tracks(connection).unwrap();
    });
    println!("count                            {count:>10.2?}");
    assert!(count < Duration::from_millis(50), "count: {count:?}");
}
