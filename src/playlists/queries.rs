//! Playlist storage.
//!
//! Entries store a track id, never a path. An entry that cannot be resolved keeps its
//! original reference and an availability of `unresolved` or `missing` so nothing the user
//! typed is silently discarded.

use std::path::Path;

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};
use crate::library::model::now_seconds;
use crate::playlists::model::{
    validate_name, Availability, Playlist, PlaylistDetail, PlaylistEntry,
};

pub fn create(connection: &Connection, name: &str) -> Result<Playlist> {
    let name = validate_name(name)?;
    let now = now_seconds();
    connection
        .execute(
            "INSERT INTO playlists (name, created_at, updated_at) VALUES (?1, ?2, ?2)",
            params![name, now],
        )
        .map_err(|err| Error::Database(format!("could not create playlist: {err}")))?;
    let id = connection.last_insert_rowid();
    Ok(Playlist {
        id,
        name,
        created_at: now,
        updated_at: now,
    })
}

pub fn rename(connection: &Connection, playlist_id: i64, name: &str) -> Result<()> {
    let name = validate_name(name)?;
    touch(connection, playlist_id)?;
    connection
        .execute(
            "UPDATE playlists SET name = ?2, updated_at = ?3 WHERE id = ?1",
            params![playlist_id, name, now_seconds()],
        )
        .map_err(|err| Error::Database(format!("could not rename playlist: {err}")))?;
    Ok(())
}

pub fn all(connection: &Connection) -> Result<Vec<Playlist>> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, created_at, updated_at FROM playlists ORDER BY name COLLATE NOCASE",
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map([], |row| {
            Ok(Playlist {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|err| Error::Database(err.to_string()))
}

pub fn get(connection: &Connection, playlist_id: i64) -> Result<Playlist> {
    connection
        .query_row(
            "SELECT id, name, created_at, updated_at FROM playlists WHERE id = ?1",
            params![playlist_id],
            |row| {
                Ok(Playlist {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|err| Error::Database(err.to_string()))?
        .ok_or_else(|| Error::InvalidConfiguration("That playlist no longer exists.".to_string()))
}

pub fn delete(connection: &Connection, playlist_id: i64) -> Result<()> {
    connection
        .execute("DELETE FROM playlists WHERE id = ?1", params![playlist_id])
        .map_err(|err| Error::Database(format!("could not delete playlist: {err}")))?;
    Ok(())
}

pub fn touch(connection: &Connection, playlist_id: i64) -> Result<()> {
    connection
        .execute(
            "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
            params![playlist_id, now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

pub fn add_track(connection: &Connection, playlist_id: i64, track_id: i64) -> Result<()> {
    let track = crate::library::queries::get_track(connection, track_id)?;
    let position = next_position(connection, playlist_id)?;
    connection
        .execute(
            "INSERT INTO playlist_entries (playlist_id, track_id, position, original_reference, availability)
             VALUES (?1, ?2, ?3, ?4, 'available')",
            params![playlist_id, track_id, position, track.path.to_string_lossy()],
        )
        .map_err(|err| Error::Database(format!("could not add to playlist: {err}")))?;
    touch(connection, playlist_id)
}

#[allow(clippy::needless_range_loop)]
pub fn add_tracks(connection: &Connection, playlist_id: i64, track_ids: &[i64]) -> Result<usize> {
    let first = next_position(connection, playlist_id)?;
    let mut added = 0;
    for (offset, track_id) in track_ids.iter().enumerate() {
        let position = first + offset as i64;
        let track = crate::library::queries::get_track(connection, *track_id)?;
        connection
            .execute(
                "INSERT INTO playlist_entries (playlist_id, track_id, position, original_reference, availability)
                 VALUES (?1, ?2, ?3, ?4, 'available')",
                params![playlist_id, *track_id, position, track.path.to_string_lossy()],
            )
            .map_err(|err| Error::Database(format!("could not add to playlist: {err}")))?;
        added += 1;
    }
    if added > 0 {
        touch(connection, playlist_id)?;
    }
    Ok(added)
}

/// Store a reference that is not (yet) a library track.
pub fn add_reference(connection: &Connection, playlist_id: i64, reference: &str) -> Result<()> {
    let position = next_position(connection, playlist_id)?;
    let track_id = crate::library::queries::track_at_path(connection, Path::new(reference))?
        .map(|track| track.id);
    let availability = match track_id {
        Some(_) => Availability::Available,
        None => Availability::Unresolved,
    };
    connection
        .execute(
            "INSERT INTO playlist_entries (playlist_id, track_id, position, original_reference, availability)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![playlist_id, track_id, position, reference, availability.as_str()],
        )
        .map_err(|err| Error::Database(format!("could not add reference: {err}")))?;
    touch(connection, playlist_id)
}

pub fn remove_entry(connection: &Connection, entry_id: i64) -> Result<()> {
    let playlist_id: Option<i64> = connection
        .query_row(
            "SELECT playlist_id FROM playlist_entries WHERE id = ?1",
            params![entry_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|err| Error::Database(err.to_string()))?;
    connection
        .execute(
            "DELETE FROM playlist_entries WHERE id = ?1",
            params![entry_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    if let Some(playlist_id) = playlist_id {
        touch(connection, playlist_id)?;
    }
    Ok(())
}

pub fn move_entry(connection: &Connection, entry_id: i64, new_position: usize) -> Result<()> {
    let (playlist_id, old_position) = connection
        .query_row(
            "SELECT playlist_id, position FROM playlist_entries WHERE id = ?1",
            params![entry_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    let entries = entry_ids(connection, playlist_id)?;
    let Some(from) = entries.iter().position(|id| *id == entry_id) else {
        return Ok(());
    };
    let to = new_position.min(entries.len().saturating_sub(1));
    if from == to {
        return Ok(());
    }

    let mut ordered = entries;
    let moved = ordered.remove(from);
    ordered.insert(to, moved);

    let transaction = connection
        .unchecked_transaction()
        .map_err(|err| Error::Database(err.to_string()))?;
    for (position, id) in ordered.iter().enumerate() {
        transaction
            .execute(
                "UPDATE playlist_entries SET position = ?2 WHERE id = ?1",
                params![id, position as i64],
            )
            .map_err(|err| Error::Database(err.to_string()))?;
    }
    transaction
        .execute(
            "UPDATE playlists SET updated_at = ?2 WHERE id = ?1",
            params![playlist_id, now_seconds()],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    transaction
        .commit()
        .map_err(|err| Error::Database(err.to_string()))?;
    let _ = old_position;
    Ok(())
}

pub fn clear(connection: &Connection, playlist_id: i64) -> Result<()> {
    connection
        .execute(
            "DELETE FROM playlist_entries WHERE playlist_id = ?1",
            params![playlist_id],
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    touch(connection, playlist_id)
}

/// Entry ids in playlist order.
pub fn entry_ids(connection: &Connection, playlist_id: i64) -> Result<Vec<i64>> {
    let mut statement = connection
        .prepare("SELECT id FROM playlist_entries WHERE playlist_id = ?1 ORDER BY position, id")
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map(params![playlist_id], |row| row.get::<_, i64>(0))
        .map_err(|err| Error::Database(err.to_string()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|err| Error::Database(err.to_string()))
}

/// Track ids in playlist order, skipping entries that are not playable.
pub fn entry_track_ids(connection: &Connection, playlist_id: i64) -> Result<Vec<i64>> {
    Ok(entries(connection, playlist_id)?
        .into_iter()
        .filter(|entry| entry.is_playable())
        .filter_map(|entry| entry.track_id)
        .collect())
}

pub fn entries(connection: &Connection, playlist_id: i64) -> Result<Vec<PlaylistEntry>> {
    let sql = "SELECT entries.id, entries.playlist_id, entries.track_id, entries.position,
                      entries.original_reference, entries.availability,
                      COALESCE(tracks.path, entries.original_reference),
                      COALESCE(tracks.duration_ms, 0),
                      tracks.missing
               FROM playlist_entries AS entries
               LEFT JOIN tracks ON tracks.id = entries.track_id
               WHERE entries.playlist_id = ?1
               ORDER BY entries.position, entries.id";
    let mut statement = connection
        .prepare(sql)
        .map_err(|err| Error::Database(err.to_string()))?;
    let rows = statement
        .query_map(params![playlist_id], |row| {
            let track_id: Option<i64> = row.get(2)?;
            let reference: String = row.get(4)?;
            let stored: String = row.get(5)?;
            let path: String = row.get(6)?;
            let duration: i64 = row.get(7)?;
            let track_missing: Option<i64> = row.get(8)?;

            let availability = if stored == "unresolved" {
                Availability::Unresolved
            } else if track_missing == Some(1) {
                Availability::Missing
            } else {
                Availability::parse(&stored)
            };

            Ok(PlaylistEntry {
                id: row.get(0)?,
                playlist_id: row.get(1)?,
                track_id,
                position: row.get::<_, i64>(3)?.max(0) as usize,
                original_reference: reference,
                availability,
                display_label: display_label(&path, track_id),
                duration_ms: (duration > 0).then_some(duration),
            })
        })
        .map_err(|err| Error::Database(err.to_string()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|err| Error::Database(err.to_string()))
}

pub fn detail(connection: &Connection, playlist_id: i64) -> Result<PlaylistDetail> {
    Ok(PlaylistDetail {
        playlist: get(connection, playlist_id)?,
        entries: entries(connection, playlist_id)?,
    })
}

pub fn count(connection: &Connection) -> Result<i64> {
    connection
        .query_row("SELECT COUNT(*) FROM playlists", [], |row| row.get(0))
        .map_err(|err| Error::Database(err.to_string()))
}

fn next_position(connection: &Connection, playlist_id: i64) -> Result<i64> {
    let highest: Option<i64> = connection
        .query_row(
            "SELECT MAX(position) FROM playlist_entries WHERE playlist_id = ?1",
            params![playlist_id],
            |row| row.get(0),
        )
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(highest.map(|value| value + 1).unwrap_or(0))
}

/// Keep unresolved entries pointing at the right track after a rescan or path change.
pub fn revalidate(connection: &Connection) -> Result<usize> {
    let mut fixed = 0;
    let entries: Vec<(i64, String)> = {
        let mut statement = connection
            .prepare("SELECT id, original_reference FROM playlist_entries WHERE track_id IS NULL")
            .map_err(|err| Error::Database(err.to_string()))?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(|err| Error::Database(err.to_string()))?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|err| Error::Database(err.to_string()))?
    };

    for (entry_id, reference) in entries {
        if let Some(track) =
            crate::library::queries::track_at_path(connection, Path::new(&reference))?
        {
            connection
                .execute(
                    "UPDATE playlist_entries SET track_id = ?2, availability = 'available' WHERE id = ?1",
                    params![entry_id, track.id],
                )
                .map_err(|err| Error::Database(err.to_string()))?;
            fixed += 1;
        }
    }
    Ok(fixed)
}

fn display_label(path: &str, track_id: Option<i64>) -> String {
    match track_id {
        Some(_) => Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string()),
        None => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::library::model::Track;

    fn track(database: &Database, filename: &str) -> i64 {
        let track = Track {
            id: 0,
            path: format!("/m/{filename}").into(),
            filename: filename.to_string(),
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
            duration_ms: Some(120_000),
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
        };
        crate::library::queries::insert_track(database.connection(), &track).unwrap()
    }

    #[test]
    fn create_read_and_delete() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Morning").unwrap();
        assert_eq!(
            get(database.connection(), playlist.id).unwrap().name,
            "Morning"
        );
        assert_eq!(all(database.connection()).unwrap().len(), 1);
        delete(database.connection(), playlist.id).unwrap();
        assert_eq!(count(database.connection()).unwrap(), 0);
        assert!(get(database.connection(), playlist.id).is_err());
    }

    #[test]
    fn names_must_be_valid() {
        let database = Database::open_in_memory().unwrap();
        assert!(create(database.connection(), "  ").is_err());
        assert!(rename(database.connection(), 1, "x\ny").is_err());
    }

    #[test]
    fn entries_keep_their_order() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        let first = track(&database, "a.mp3");
        let second = track(&database, "b.mp3");
        add_track(database.connection(), playlist.id, first).unwrap();
        add_track(database.connection(), playlist.id, second).unwrap();
        assert_eq!(
            entry_ids(database.connection(), playlist.id).unwrap().len(),
            2
        );
        assert_eq!(
            entry_track_ids(database.connection(), playlist.id).unwrap(),
            vec![first, second]
        );
    }

    #[test]
    fn moving_an_entry_reorders_the_playlist() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        let first = track(&database, "a.mp3");
        let second = track(&database, "b.mp3");
        let third = track(&database, "c.mp3");
        for id in [first, second, third] {
            add_track(database.connection(), playlist.id, id).unwrap();
        }

        let entries = entries(database.connection(), playlist.id).unwrap();
        move_entry(database.connection(), entries[2].id, 0).unwrap();

        assert_eq!(
            entry_track_ids(database.connection(), playlist.id).unwrap(),
            vec![third, first, second]
        );
    }

    #[test]
    fn removing_an_entry_shortens_the_playlist() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        let first = track(&database, "a.mp3");
        add_track(database.connection(), playlist.id, first).unwrap();
        let rows = super::entries(database.connection(), playlist.id).unwrap();
        remove_entry(database.connection(), rows[0].id).unwrap();
        assert!(super::entries(database.connection(), playlist.id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn references_outside_the_library_stay_unresolved() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Imports").unwrap();
        add_reference(database.connection(), playlist.id, "/elsewhere/x.mp3").unwrap();
        let entries = entries(database.connection(), playlist.id).unwrap();
        assert_eq!(entries[0].availability, Availability::Unresolved);
        assert!(!entries[0].is_playable());
        assert_eq!(entries[0].original_reference, "/elsewhere/x.mp3");
        assert!(entry_track_ids(database.connection(), playlist.id)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn references_become_playable_once_the_track_is_indexed() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Imports").unwrap();
        add_reference(database.connection(), playlist.id, "/m/late.mp3").unwrap();
        track(&database, "late.mp3");
        assert_eq!(revalidate(database.connection()).unwrap(), 1);
        let entries = entries(database.connection(), playlist.id).unwrap();
        assert!(entries[0].is_playable());
        assert_eq!(
            entry_track_ids(database.connection(), playlist.id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_missing_track_is_reported_as_missing_not_unresolved() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        let id = track(&database, "gone.mp3");
        add_track(database.connection(), playlist.id, id).unwrap();
        crate::library::queries::set_missing(database.connection(), id, true).unwrap();
        let entries = entries(database.connection(), playlist.id).unwrap();
        assert_eq!(entries[0].availability, Availability::Missing);
        assert!(!entries[0].is_playable());
    }

    #[test]
    fn total_length_needs_every_duration() {
        let mut detail = PlaylistDetail {
            playlist: Playlist {
                id: 1,
                name: "Mix".to_string(),
                created_at: 0,
                updated_at: 0,
            },
            entries: Vec::new(),
        };
        assert_eq!(detail.total_seconds(), Some(0));
        detail.entries.push(entry_with_duration(Some(60_000)));
        assert_eq!(detail.total_seconds(), Some(60));
        detail.entries.push(entry_with_duration(None));
        assert_eq!(detail.total_seconds(), None);
    }

    fn entry_with_duration(duration_ms: Option<i64>) -> PlaylistEntry {
        PlaylistEntry {
            id: 1,
            playlist_id: 1,
            track_id: Some(1),
            position: 0,
            original_reference: "/m/a.mp3".to_string(),
            availability: Availability::Available,
            display_label: "a.mp3".to_string(),
            duration_ms,
        }
    }

    #[test]
    fn detail_reports_unresolved_entries() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        add_reference(database.connection(), playlist.id, "/nope.mp3").unwrap();
        let detail = detail(database.connection(), playlist.id).unwrap();
        assert_eq!(detail.unresolved_count(), 1);
        assert!(detail.total_seconds().is_none());
    }

    #[test]
    fn clear_empties_but_keeps_the_playlist() {
        let database = Database::open_in_memory().unwrap();
        let playlist = create(database.connection(), "Mix").unwrap();
        add_track(
            database.connection(),
            playlist.id,
            track(&database, "a.mp3"),
        )
        .unwrap();
        clear(database.connection(), playlist.id).unwrap();
        assert!(entries(database.connection(), playlist.id)
            .unwrap()
            .is_empty());
        assert_eq!(count(database.connection()).unwrap(), 1);
    }
}
