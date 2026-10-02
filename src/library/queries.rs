//! SQL queries for the library index.
//!
//! Every list view is served from SQLite so sorting, filtering and searching stay
//! consistent and fast; the UI never filters rows in memory. See `docs/database.md`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::{params, types::Value, Connection};

use crate::error::{Error, Result};
use crate::library::model::{
    now_seconds, AlbumSummary, ArtistSummary, FolderSummary, SortDirection, Track, TrackQuery,
    TrackSort, UNKNOWN_ALBUM, UNKNOWN_ARTIST,
};

pub const TRACK_COLUMNS: &str =
    "id, path, filename, folder_id, file_size, modified_time, missing, \
     title, artist, album, album_artist, genre, year, track_number, disc_number, duration_ms, \
     format, sample_rate, channels, bit_depth, bitrate, favorite, artwork_reference, ambiguous, \
     created_at, updated_at";

/// Escapes the wildcards in user input so `LIKE` treats them literally.
fn escape_like(term: &str) -> String {
    term.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn optional_text(value: &Option<String>) -> Value {
    match value {
        Some(text) => Value::Text(text.clone()),
        None => Value::Null,
    }
}

fn optional_int(value: Option<i64>) -> Value {
    match value {
        Some(number) => Value::Integer(number),
        None => Value::Null,
    }
}

pub fn row_to_track(row: &rusqlite::Row<'_>) -> rusqlite::Result<Track> {
    Ok(Track {
        id: row.get(0)?,
        path: PathBuf::from(row.get::<_, String>(1)?),
        filename: row.get(2)?,
        folder_id: row.get(3)?,
        file_size: row.get(4)?,
        modified_time: row.get(5)?,
        missing: row.get::<_, i64>(6)? != 0,
        title: row.get(7)?,
        artist: row.get(8)?,
        album: row.get(9)?,
        album_artist: row.get(10)?,
        genre: row.get(11)?,
        year: row.get(12)?,
        track_number: row.get(13)?,
        disc_number: row.get(14)?,
        duration_ms: row.get(15)?,
        format: row.get(16)?,
        sample_rate: row.get(17)?,
        channels: row.get(18)?,
        bit_depth: row.get(19)?,
        bitrate: row.get(20)?,
        favorite: row.get::<_, i64>(21)? != 0,
        artwork_reference: row.get(22)?,
        ambiguous: row.get::<_, i64>(23)? != 0,
        created_at: row.get(24)?,
        updated_at: row.get(25)?,
    })
}

/// The subset of a track the scanner needs to reason about identity.
#[derive(Debug, Clone)]
pub struct IdentityRow {
    pub id: i64,
    pub path: PathBuf,
    pub filename: String,
    pub file_size: i64,
    pub modified_time: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub missing: bool,
}

pub fn get_track(connection: &Connection, id: i64) -> Result<Track> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id = ?1");
    connection
        .query_row(&sql, [id], row_to_track)
        .map_err(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Error::Database(format!("track {id} is gone")),
            other => Error::Database(other.to_string()),
        })
}

pub fn tracks_by_ids(connection: &Connection, ids: &[i64]) -> Result<Vec<Track>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE id IN ({placeholders})");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(rusqlite::params_from_iter(ids), row_to_track)?;
    collect_tracks(rows)
}

fn collect_tracks(rows: impl Iterator<Item = rusqlite::Result<Track>>) -> Result<Vec<Track>> {
    let mut tracks = Vec::new();
    for row in rows {
        tracks.push(row.map_err(|err| Error::Database(err.to_string()))?);
    }
    Ok(tracks)
}

pub fn track_at_path(connection: &Connection, path: &Path) -> Result<Option<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1");
    let mut statement = connection.prepare(&sql)?;
    let mut rows = statement.query([path.to_string_lossy().into_owned()])?;
    match rows.next()? {
        Some(row) => Ok(Some(row_to_track(row)?)),
        None => Ok(None),
    }
}

pub fn insert_track(connection: &Connection, track: &Track) -> Result<i64> {
    let now = now_seconds();
    connection
        .execute(
            "INSERT INTO tracks (
                path, filename, folder_id, file_size, modified_time, missing,
                title, artist, album, album_artist, genre, year, track_number, disc_number,
                duration_ms, format, sample_rate, channels, bit_depth, bitrate,
                favorite, artwork_reference, ambiguous, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                track.path.to_string_lossy(),
                track.filename,
                track.folder_id,
                track.file_size,
                track.modified_time,
                track.title,
                track.artist,
                track.album,
                track.album_artist,
                optional_text(&track.genre),
                optional_int(track.year),
                optional_int(track.track_number),
                optional_int(track.disc_number),
                optional_int(track.duration_ms),
                optional_text(&track.format),
                optional_int(track.sample_rate),
                optional_int(track.channels),
                optional_int(track.bit_depth),
                optional_int(track.bitrate),
                i64::from(track.favorite),
                optional_text(&track.artwork_reference),
                i64::from(track.ambiguous),
                if track.created_at == 0 {
                    now
                } else {
                    track.created_at
                },
                now,
            ],
        )
        .map(|_| connection.last_insert_rowid())
        .map_err(|err| Error::Database(format!("could not index {}: {err}", track.filename)))
}

pub fn update_track_metadata(connection: &Connection, track: &Track) -> Result<()> {
    let filename = track
        .path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| track.filename.clone());

    connection
        .execute(
            "UPDATE tracks SET
                path = ?2, filename = ?3, folder_id = ?4, file_size = ?5, modified_time = ?6,
                missing = 0, title = ?7, artist = ?8, album = ?9, album_artist = ?10,
                genre = ?11, year = ?12, track_number = ?13, disc_number = ?14, duration_ms = ?15,
                format = ?16, sample_rate = ?17, channels = ?18, bit_depth = ?19, bitrate = ?20,
                ambiguous = 0, updated_at = ?21
             WHERE id = ?1",
            params![
                track.id,
                track.path.to_string_lossy(),
                filename,
                track.folder_id,
                track.file_size,
                track.modified_time,
                track.title,
                track.artist,
                track.album,
                track.album_artist,
                optional_text(&track.genre),
                optional_int(track.year),
                optional_int(track.track_number),
                optional_int(track.disc_number),
                optional_int(track.duration_ms),
                optional_text(&track.format),
                optional_int(track.sample_rate),
                optional_int(track.channels),
                optional_int(track.bit_depth),
                optional_int(track.bitrate),
                now_seconds(),
            ],
        )
        .map_err(|err| Error::Database(format!("could not update {}: {err}", track.filename)))?;
    Ok(())
}

/// Record a new location for an existing track. Used when a file was moved or renamed.
pub fn relocate_track(connection: &Connection, track_id: i64, new_path: &Path) -> Result<()> {
    let filename = new_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    connection
        .execute(
            "UPDATE tracks SET path = ?2, filename = ?3, missing = 0, updated_at = ?4 WHERE id = ?1",
            params![
                track_id,
                new_path.to_string_lossy(),
                filename,
                now_seconds()
            ],
        )
        .map_err(|err| Error::Database(format!("could not update the location of a track: {err}")))?;
    Ok(())
}

pub fn set_missing(connection: &Connection, track_id: i64, missing: bool) -> Result<()> {
    connection
        .execute(
            "UPDATE tracks SET missing = ?2, updated_at = ?3 WHERE id = ?1",
            params![track_id, i64::from(missing), now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

pub fn set_ambiguous(connection: &Connection, track_id: i64, ambiguous: bool) -> Result<()> {
    connection
        .execute(
            "UPDATE tracks SET ambiguous = ?2, updated_at = ?3 WHERE id = ?1",
            params![track_id, i64::from(ambiguous), now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

pub fn set_favorite(connection: &Connection, track_id: i64, favorite: bool) -> Result<()> {
    connection
        .execute(
            "UPDATE tracks SET favorite = ?2, updated_at = ?3 WHERE id = ?1",
            params![track_id, i64::from(favorite), now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

pub fn set_artwork_reference(
    connection: &Connection,
    track_id: i64,
    reference: Option<&str>,
) -> Result<()> {
    connection
        .execute(
            "UPDATE tracks SET artwork_reference = ?2, updated_at = ?3 WHERE id = ?1",
            params![
                track_id,
                optional_text(&reference.map(str::to_string)),
                now_seconds()
            ],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

/// Fold `drop_id` into `keep_id`: playlist entries and queue entries follow, and the
/// duplicate row disappears. Used to resolve an ambiguous move without losing history.
pub fn merge_tracks(connection: &Connection, keep_id: i64, drop_id: i64) -> Result<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|err| Error::Database(err.to_string()))?;

    transaction
        .execute(
            "UPDATE playlist_entries SET track_id = ?1 WHERE track_id = ?2",
            params![keep_id, drop_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .execute(
            "UPDATE queue_entries SET track_id = ?1 WHERE track_id = ?2",
            params![keep_id, drop_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .execute(
            "DELETE FROM tracks WHERE id = ?2",
            params![keep_id, drop_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .execute(
            "UPDATE playback_state SET track_id = ?1 WHERE track_id = ?2",
            params![keep_id, drop_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .commit()
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

/// All tracks as identity evidence for the scanner. One row per indexed file.
pub fn identity_rows(connection: &Connection) -> Result<Vec<IdentityRow>> {
    let mut statement = connection
        .prepare(
            "SELECT id, path, filename, file_size, modified_time, title, artist, album, duration_ms, missing
             FROM tracks",
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok(IdentityRow {
                id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                filename: row.get(2)?,
                file_size: row.get(3)?,
                modified_time: row.get(4)?,
                title: row.get(5)?,
                artist: row.get(6)?,
                album: row.get(7)?,
                duration_ms: row.get(8)?,
                missing: row.get::<_, i64>(9)? != 0,
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|err| Error::Database(err.to_string()))?);
    }
    Ok(result)
}

/// Tracks whose identity the user still has to confirm.
pub fn ambiguous_tracks(connection: &Connection) -> Result<Vec<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE ambiguous = 1 ORDER BY filename");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([], row_to_track)
        .map_err(|err| Error::Database(err.to_string()))?;
    collect_tracks(rows)
}

/// Build the `WHERE` fragment for a track query.
fn track_filter(query: &TrackQuery) -> (String, Vec<Value>) {
    let mut clauses = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if !query.include_missing {
        clauses.push("tracks.missing = 0".to_string());
    }
    if query.favorites_only {
        clauses.push("tracks.favorite = 1".to_string());
    }
    if let Some(folder_id) = query.folder_id {
        clauses.push(format!("tracks.folder_id = ?{}", values.len() + 1));
        values.push(Value::Integer(folder_id));
    }
    if let Some(path) = &query.folder_path {
        // A folder view covers the folder itself and everything below it.
        clauses.push(format!(
            "(tracks.folder_id = (SELECT id FROM library_folders WHERE path = ?{}) OR tracks.path LIKE ?{})",
            values.len() + 1,
            values.len() + 2
        ));
        let prefix = path.to_string_lossy().into_owned();
        values.push(Value::Text(prefix.clone()));
        values.push(Value::Text(format!("{prefix}/%")));
    }
    if let Some(artist) = &query.artist {
        clauses.push(format!(
            "COALESCE(NULLIF(TRIM(tracks.artist), ''), ?) = ?{}",
            values.len() + 2
        ));
        values.push(Value::Text(UNKNOWN_ARTIST.to_string()));
        values.push(Value::Text(artist.clone()));
    }
    if let Some(album_artist) = &query.album_artist {
        clauses.push(format!("tracks.album_artist = ?{}", values.len() + 1));
        values.push(Value::Text(album_artist.clone()));
    }
    if let Some(album) = &query.album {
        clauses.push(format!("tracks.album = ?{}", values.len() + 1));
        values.push(Value::Text(album.clone()));
    }
    if let Some(ids) = &query.ids {
        if ids.is_empty() {
            return ("tracks.missing = 0 AND 0".to_string(), Vec::new());
        }
        let mut placeholders = Vec::new();
        for id in ids {
            placeholders.push(format!("?{}", values.len() + 1));
            values.push(Value::Integer(*id));
        }
        clauses.push(format!("tracks.id IN ({})", placeholders.join(", ")));
    }
    if let Some(search) = &query.search {
        let trimmed = search.trim();
        if !trimmed.is_empty() {
            let pattern = format!("%{}%", escape_like(trimmed));
            let start = values.len() + 1;
            let fields = [
                "tracks.title",
                "tracks.artist",
                "tracks.album",
                "tracks.filename",
            ];
            let mut parts = Vec::new();
            for (index, field) in fields.iter().enumerate() {
                parts.push(format!("{field} LIKE ?{} ESCAPE '\\'", start + index));
                values.push(Value::Text(pattern.clone()));
            }
            clauses.push(format!("({})", parts.join(" OR ")));
        }
    }

    let where_clause = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    (where_clause, values)
}

pub fn query_tracks(
    connection: &Connection,
    query: &TrackQuery,
    sort: TrackSort,
    direction: SortDirection,
) -> Result<Vec<Track>> {
    let (where_clause, values) = track_filter(query);
    let sql = format!(
        "SELECT {TRACK_COLUMNS} FROM tracks{where_clause} ORDER BY {} {}, tracks.title COLLATE NOCASE, tracks.id",
        sort.order_by(),
        direction.sql()
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| Error::Database(format!("track query failed: {err}")))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(values), row_to_track)
        .map_err(|err| Error::Database(err.to_string()))?;
    collect_tracks(rows)
}

pub fn count_tracks(connection: &Connection) -> Result<i64> {
    connection
        .query_row("SELECT COUNT(*) FROM tracks WHERE missing = 0", [], |row| {
            row.get(0)
        })
        .map_err(|err| Error::Database(err.to_string()))
}

pub fn artists(connection: &Connection) -> Result<Vec<ArtistSummary>> {
    let mut statement = connection
        .prepare(
            "SELECT COALESCE(NULLIF(TRIM(artist), ''), 'Unknown Artist') AS name,
                    COUNT(*),
                    COUNT(DISTINCT album_artist || char(31) || album)
             FROM tracks WHERE missing = 0
             GROUP BY name ORDER BY name COLLATE NOCASE",
        )
        .map_err(|err| Error::Database(err.to_string()))?;

    let rows = statement
        .query_map([], |row| {
            Ok(ArtistSummary {
                name: row.get(0)?,
                track_count: row.get(1)?,
                album_count: row.get(2)?,
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|err| Error::Database(err.to_string()))?);
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlbumSort {
    Album,
    AlbumArtist,
    Year,
}

impl AlbumSort {
    fn order_by(&self) -> &'static str {
        match self {
            AlbumSort::Album => "a.album COLLATE NOCASE",
            AlbumSort::AlbumArtist => "a.album_artist COLLATE NOCASE",
            AlbumSort::Year => "a.year",
        }
    }
}

/// Albums grouped by album artist, falling back to the track artist when the album artist
/// tag is absent. `sample_track_id` is the earliest track of the album, used for artwork
/// and as the playback entry point.
pub fn albums(
    connection: &Connection,
    query: &TrackQuery,
    sort: AlbumSort,
    direction: SortDirection,
) -> Result<Vec<AlbumSummary>> {
    let mut clauses = vec!["t.missing = 0".to_string()];
    let mut values: Vec<Value> = Vec::new();

    if let Some(search) = &query.search {
        let trimmed = search.trim();
        if !trimmed.is_empty() {
            let pattern = format!("%{}%", escape_like(trimmed));
            let start = values.len() + 1;
            clauses.push(format!(
                "(COALESCE(NULLIF(TRIM(t.album), ''), '{UNKNOWN_ALBUM}') LIKE ?{start} ESCAPE '\\'
                  OR t.album_artist LIKE ?{start} ESCAPE '\\')"
            ));
            values.push(Value::Text(pattern));
        }
    }
    if let Some(artist) = &query.artist {
        clauses.push(format!(
            "COALESCE(NULLIF(TRIM(t.artist), ''), '{UNKNOWN_ARTIST}') = ?{}",
            values.len() + 1
        ));
        values.push(Value::Text(artist.clone()));
    }
    if query.favorites_only {
        clauses.push("t.favorite = 1".to_string());
    }
    if let Some(folder_id) = query.folder_id {
        clauses.push(format!("t.folder_id = ?{}", values.len() + 1));
        values.push(Value::Integer(folder_id));
    }

    let sql = format!(
        "SELECT a.album_artist, a.album, a.year, a.track_count, a.duration_ms,
                s.path, s.artwork_reference, s.id
         FROM (
            SELECT COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '{UNKNOWN_ARTIST}') AS album_artist,
                   COALESCE(NULLIF(TRIM(album), ''), '{UNKNOWN_ALBUM}') AS album,
                   MIN(year) AS year,
                   COUNT(*) AS track_count,
                   SUM(COALESCE(duration_ms, 0)) AS duration_ms,
                   (SELECT t2.id FROM tracks t2
                     WHERE COALESCE(NULLIF(TRIM(t2.album_artist), ''), NULLIF(TRIM(t2.artist), ''), '{UNKNOWN_ARTIST}') =
                           COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), '{UNKNOWN_ARTIST}')
                       AND COALESCE(NULLIF(TRIM(t2.album), ''), '{UNKNOWN_ALBUM}') =
                           COALESCE(NULLIF(TRIM(album), ''), '{UNKNOWN_ALBUM}')
                       AND t2.missing = 0
                     ORDER BY t2.disc_number IS NULL, t2.disc_number, t2.track_number IS NULL, t2.track_number, t2.id
                     LIMIT 1) AS sample_id
            FROM tracks t
            WHERE {}
            GROUP BY album_artist, album
         ) a
         JOIN tracks s ON s.id = a.sample_id
         ORDER BY {} {}, a.album COLLATE NOCASE",
        clauses.join(" AND "),
        sort.order_by(),
        direction.sql()
    );

    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| Error::Database(format!("album query failed: {err}")))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(values), |row| {
            Ok(AlbumSummary {
                album_artist: row.get(0)?,
                album: row.get(1)?,
                year: row.get(2)?,
                track_count: row.get(3)?,
                duration_ms: row.get(4)?,
                sample_track_path: PathBuf::from(row.get::<_, String>(5)?),
                artwork_reference: row.get(6)?,
                sample_track_id: row.get(7)?,
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|err| Error::Database(err.to_string()))?);
    }
    Ok(result)
}

pub fn folders(connection: &Connection) -> Result<Vec<FolderSummary>> {
    let mut statement = connection
        .prepare(
            "SELECT f.id, f.path, f.created_at, COUNT(t.id)
             FROM library_folders f
             LEFT JOIN tracks t ON t.folder_id = f.id AND t.missing = 0
             GROUP BY f.id ORDER BY f.path COLLATE NOCASE",
        )
        .map_err(|err| Error::Database(err.to_string()))?;

    let rows = statement
        .query_map([], |row| {
            Ok(FolderSummary {
                folder_id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                created_at: row.get(2)?,
                track_count: row.get(3)?,
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;

    let mut result = Vec::new();
    for row in rows {
        result.push(row.map_err(|err| Error::Database(err.to_string()))?);
    }
    Ok(result)
}

pub fn add_folder(connection: &Connection, path: &Path) -> Result<i64> {
    connection
        .execute(
            "INSERT OR IGNORE INTO library_folders (path, created_at) VALUES (?1, ?2)",
            params![path.to_string_lossy(), now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    connection
        .query_row(
            "SELECT id FROM library_folders WHERE path = ?1",
            [path.to_string_lossy().into_owned()],
            |row| row.get(0),
        )
        .map_err(|err| Error::Database(err.to_string()))
}

/// Stop tracking a folder. Music files are never touched.
pub fn remove_folder(connection: &Connection, folder_id: i64) -> Result<()> {
    let transaction = connection
        .unchecked_transaction()
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .execute(
            "UPDATE tracks SET folder_id = NULL WHERE folder_id = ?1",
            params![folder_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .execute(
            "DELETE FROM library_folders WHERE id = ?1",
            params![folder_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .commit()
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

/// Tracks indexed for a folder, used by "rescan folder".
pub fn tracks_in_folder(connection: &Connection, folder_id: i64) -> Result<Vec<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks WHERE folder_id = ?1");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([folder_id], row_to_track)
        .map_err(|err| Error::Database(err.to_string()))?;
    collect_tracks(rows)
}

/// Every indexed track, whatever folder it belongs to.
///
/// The scanner reconciles against the whole library so that a file moved between two
/// watched folders is recognised instead of indexed twice.
pub fn all_tracks(connection: &Connection) -> Result<Vec<Track>> {
    let sql = format!("SELECT {TRACK_COLUMNS} FROM tracks");
    let mut statement = connection
        .prepare(&sql)
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([], row_to_track)
        .map_err(|err| Error::Database(err.to_string()))?;
    collect_tracks(rows)
}

/// Counts for the status bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryStats {
    pub tracks: i64,
    pub albums: i64,
    pub artists: i64,
    pub missing: i64,
    pub duration_ms: i64,
}

pub fn stats(connection: &Connection) -> Result<LibraryStats> {
    let mut statement = connection
        .prepare(
            "SELECT COUNT(*),
                    COUNT(DISTINCT COALESCE(NULLIF(TRIM(album_artist), ''), NULLIF(TRIM(artist), ''), 'Unknown Artist')
                                 || char(31) || COALESCE(NULLIF(TRIM(album), ''), 'Unknown Album')),
                    COUNT(DISTINCT COALESCE(NULLIF(TRIM(artist), ''), 'Unknown Artist')),
                    SUM(CASE WHEN missing = 1 THEN 1 ELSE 0 END),
                    SUM(COALESCE(duration_ms, 0))
             FROM tracks",
        )
        .map_err(|err| Error::Database(err.to_string()))?;

    let stats = statement
        .query_row([], |row| {
            let total: i64 = row.get(0)?;
            let missing: Option<i64> = row.get(3)?;
            Ok(LibraryStats {
                tracks: total - missing.unwrap_or(0),
                albums: row.get(1)?,
                artists: row.get(2)?,
                missing: missing.unwrap_or(0),
                duration_ms: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(stats)
}

/// Map of stored path to track id, used to detect files that moved into a new location.
pub fn path_index(connection: &Connection) -> Result<HashMap<PathBuf, i64>> {
    let mut statement = connection
        .prepare("SELECT path, id FROM tracks")
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                PathBuf::from(row.get::<_, String>(0)?),
                row.get::<_, i64>(1)?,
            ))
        })
        .map_err(|err| Error::Database(err.to_string()))?;

    let mut index = HashMap::new();
    for row in rows {
        let (path, id) = row.map_err(|err| Error::Database(err.to_string()))?;
        index.insert(path, id);
    }
    Ok(index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::library::model::Track;

    fn sample_track(
        path: &str,
        title: &str,
        artist: &str,
        album: &str,
        album_artist: &str,
    ) -> Track {
        Track {
            id: 0,
            path: PathBuf::from(path),
            filename: Path::new(path)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            folder_id: None,
            file_size: 1000,
            modified_time: 100,
            missing: false,
            title: title.into(),
            artist: artist.into(),
            album: album.into(),
            album_artist: album_artist.into(),
            genre: Some("Rock".into()),
            year: Some(1970),
            track_number: Some(3),
            disc_number: Some(1),
            duration_ms: Some(200_000),
            format: Some("FLAC".into()),
            sample_rate: Some(44_100),
            channels: Some(2),
            bit_depth: Some(16),
            bitrate: None,
            favorite: false,
            artwork_reference: None,
            ambiguous: false,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn database_with(tracks: Vec<Track>) -> Database {
        let database = Database::open_in_memory().unwrap();
        add_folder(database.connection(), Path::new("/m")).unwrap();
        for mut track in tracks {
            track.folder_id = Some(1);
            insert_track(database.connection(), &track).unwrap();
        }
        database
    }

    #[test]
    fn insert_and_read_back() {
        let database = Database::open_in_memory().unwrap();
        let id = insert_track(
            database.connection(),
            &sample_track("/m/a.flac", "A", "B", "C", "B"),
        )
        .unwrap();
        let stored = get_track(database.connection(), id).unwrap();
        assert_eq!(stored.title, "A");
        assert_eq!(stored.format.as_deref(), Some("FLAC"));
        assert_eq!(stored.sample_rate, Some(44_100));
        assert!(!stored.favorite);
    }

    #[test]
    fn search_matches_all_four_fields() {
        let database = database_with(vec![
            sample_track(
                "/m/one.mp3",
                "Blue Monday",
                "New Order",
                "Power, Corruption & Lies",
                "New Order",
            ),
            sample_track(
                "/m/two.mp3",
                "Ceremony",
                "New Order",
                "Substance",
                "New Order",
            ),
        ]);

        let found = query_tracks(
            database.connection(),
            &TrackQuery {
                search: Some("cerem".into()),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "Ceremony");

        // Filename is searchable too.
        let by_filename = query_tracks(
            database.connection(),
            &TrackQuery {
                search: Some("two.mp3".into()),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(by_filename.len(), 1);
    }

    #[test]
    fn search_treats_wildcards_literally() {
        let database = database_with(vec![
            sample_track("/m/a.mp3", "100%", "X", "Y", "X"),
            sample_track("/m/b.mp3", "Plain", "X", "Y", "X"),
        ]);
        let found = query_tracks(
            database.connection(),
            &TrackQuery {
                search: Some("100%".into()),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "100%");
    }

    #[test]
    fn search_is_case_insensitive() {
        let database = database_with(vec![sample_track("/m/a.mp3", "Ceremony", "X", "Y", "X")]);
        let found = query_tracks(
            database.connection(),
            &TrackQuery {
                search: Some("CEREMONY".into()),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(found.len(), 1);
    }

    #[test]
    fn sorting_respects_direction() {
        let database = database_with(vec![
            sample_track("/m/a.mp3", "Alpha", "X", "Y", "X"),
            sample_track("/m/b.mp3", "Beta", "X", "Y", "X"),
        ]);
        let ascending = query_tracks(
            database.connection(),
            &TrackQuery::default(),
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        let descending = query_tracks(
            database.connection(),
            &TrackQuery::default(),
            TrackSort::Title,
            SortDirection::Descending,
        )
        .unwrap();
        assert_eq!(ascending[0].title, "Alpha");
        assert_eq!(descending[0].title, "Beta");
    }

    #[test]
    fn artists_group_unknown_artist() {
        let database = database_with(vec![
            sample_track("/m/a.mp3", "A", "X", "Y", "X"),
            sample_track("/m/b.mp3", "B", "", "Y", ""),
        ]);
        let artists = artists(database.connection()).unwrap();
        assert_eq!(artists.len(), 2);
        assert!(artists.iter().any(|a| a.name == UNKNOWN_ARTIST));
    }

    #[test]
    fn albums_group_by_album_artist_and_use_first_track() {
        let database = database_with(vec![
            {
                let mut track =
                    sample_track("/m/a.flac", "One", "Various", "Compilation", "Various");
                track.disc_number = Some(1);
                track.track_number = Some(1);
                track
            },
            {
                let mut track = sample_track("/m/b.flac", "Two", "Other", "Compilation", "Various");
                track.disc_number = Some(1);
                track.track_number = Some(2);
                track
            },
        ]);
        let albums = albums(
            database.connection(),
            &TrackQuery::default(),
            AlbumSort::Album,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].track_count, 2);
        assert_eq!(albums[0].sample_track_id, 1);
        assert_eq!(albums[0].duration_ms, 400_000);
    }

    #[test]
    fn albums_fall_back_to_artist_when_album_artist_missing() {
        let database = database_with(vec![sample_track(
            "/m/a.mp3",
            "A",
            "Solo Artist",
            "Record",
            "",
        )]);
        let albums = albums(
            database.connection(),
            &TrackQuery::default(),
            AlbumSort::Album,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(albums[0].album_artist, "Solo Artist");
        assert_eq!(albums[0].display_album(), "Record");
    }

    #[test]
    fn missing_tracks_are_hidden_by_default() {
        let database = database_with(vec![sample_track("/m/a.mp3", "A", "X", "Y", "X")]);
        let id = get_track_by_title(database.connection(), "A");
        set_missing(database.connection(), id, true).unwrap();

        let visible = query_tracks(
            database.connection(),
            &TrackQuery::default(),
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert!(visible.is_empty());

        let all = query_tracks(
            database.connection(),
            &TrackQuery {
                include_missing: true,
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(stats(database.connection()).unwrap().missing, 1);
    }

    fn get_track_by_title(connection: &Connection, title: &str) -> i64 {
        connection
            .query_row("SELECT id FROM tracks WHERE title = ?1", [title], |row| {
                row.get(0)
            })
            .unwrap()
    }

    #[test]
    fn favorites_are_queryable() {
        let database = database_with(vec![
            sample_track("/m/a.mp3", "A", "X", "Y", "X"),
            sample_track("/m/b.mp3", "B", "X", "Y", "X"),
        ]);
        set_favorite(database.connection(), 2, true).unwrap();
        let favorites = query_tracks(
            database.connection(),
            &TrackQuery {
                favorites_only: true,
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(favorites.len(), 1);
        assert_eq!(favorites[0].title, "B");
    }

    #[test]
    fn folder_view_includes_subfolders() {
        let database = Database::open_in_memory().unwrap();
        let root = add_folder(database.connection(), Path::new("/music")).unwrap();
        let mut parent_track = sample_track("/music/a.mp3", "A", "X", "Y", "X");
        parent_track.folder_id = Some(root);
        insert_track(database.connection(), &parent_track).unwrap();

        let mut nested = sample_track("/music/rock/b.mp3", "B", "X", "Y", "X");
        nested.folder_id = None;
        insert_track(database.connection(), &nested).unwrap();

        let found = query_tracks(
            database.connection(),
            &TrackQuery {
                folder_path: Some(PathBuf::from("/music")),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )
        .unwrap();
        assert_eq!(found.len(), 2, "nested files belong to the folder view");
    }

    #[test]
    fn removing_a_folder_keeps_tracks() {
        let database = Database::open_in_memory().unwrap();
        let folder_id = add_folder(database.connection(), Path::new("/music")).unwrap();
        let mut track = sample_track("/music/a.mp3", "A", "X", "Y", "X");
        track.folder_id = Some(folder_id);
        insert_track(database.connection(), &track).unwrap();

        remove_folder(database.connection(), folder_id).unwrap();

        assert!(folders(database.connection()).unwrap().is_empty());
        let kept = get_track(database.connection(), 1).unwrap();
        assert_eq!(kept.folder_id, None, "the file stays indexed");
    }

    #[test]
    fn merging_tracks_moves_playlist_entries() {
        let database = Database::open_in_memory().unwrap();
        insert_track(
            database.connection(),
            &sample_track("/m/old.mp3", "A", "X", "Y", "X"),
        )
        .unwrap();
        insert_track(
            database.connection(),
            &sample_track("/m/new.mp3", "A", "X", "Y", "X"),
        )
        .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO playlists (id, name, created_at, updated_at) VALUES (1, 'mix', 0, 0)",
                [],
            )
            .unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO playlist_entries (playlist_id, track_id, position, original_reference)
                 VALUES (1, 2, 0, '/m/new.mp3')",
                [],
            )
            .unwrap();

        merge_tracks(database.connection(), 1, 2).unwrap();

        let track_id: i64 = database
            .connection()
            .query_row(
                "SELECT track_id FROM playlist_entries WHERE playlist_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(track_id, 1);
        assert_eq!(count_tracks(database.connection()).unwrap(), 1);
    }

    #[test]
    fn ambiguous_tracks_are_listed() {
        let database = database_with(vec![sample_track("/m/a.mp3", "A", "X", "Y", "X")]);
        set_ambiguous(database.connection(), 1, true).unwrap();
        let ambiguous = ambiguous_tracks(database.connection()).unwrap();
        assert_eq!(ambiguous.len(), 1);
        assert!(ambiguous[0].ambiguous);
    }

    #[test]
    fn relocating_updates_path_and_filename() {
        let database = database_with(vec![sample_track("/m/old.mp3", "A", "X", "Y", "X")]);
        relocate_track(database.connection(), 1, Path::new("/m/new name.mp3")).unwrap();
        let track = get_track(database.connection(), 1).unwrap();
        assert_eq!(track.filename, "new name.mp3");
        assert!(
            track_at_path(database.connection(), Path::new("/m/old.mp3"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn identity_rows_include_missing_tracks() {
        let database = database_with(vec![sample_track("/m/a.mp3", "A", "X", "Y", "X")]);
        set_missing(database.connection(), 1, true).unwrap();
        let rows = identity_rows(database.connection()).unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].missing);
    }

    #[test]
    fn stats_report_totals() {
        let database = database_with(vec![
            sample_track("/m/a.mp3", "A", "X", "Y", "X"),
            sample_track("/m/b.mp3", "B", "X", "Z", "X"),
        ]);
        let stats = stats(database.connection()).unwrap();
        assert_eq!(stats.tracks, 2);
        assert_eq!(stats.artists, 1);
        assert_eq!(stats.albums, 2);
        assert_eq!(stats.missing, 0);
        assert_eq!(stats.duration_ms, 400_000);
    }
}
