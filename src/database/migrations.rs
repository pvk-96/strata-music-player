//! Schema migrations.
//!
//! The schema version lives in SQLite's `user_version`. Migrations run in order inside a
//! transaction and the version is only advanced once every statement has succeeded, so an
//! interrupted upgrade leaves the previous schema intact.

use rusqlite::Connection;

use crate::database::schema;

pub struct Migration {
    pub version: i64,
    pub sql: &'static str,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: schema::MIGRATION_1,
    },
    Migration {
        version: 2,
        sql: schema::MIGRATION_2,
    },
    Migration {
        version: 3,
        sql: schema::MIGRATION_3,
    },
];

/// Apply any migration the database has not seen yet. Returns the resulting version.
pub fn migrate(connection: &Connection) -> rusqlite::Result<i64> {
    let mut version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;

    for migration in MIGRATIONS {
        if migration.version <= version {
            continue;
        }
        log::info!("upgrading library schema to version {}", migration.version);
        let transaction = connection.unchecked_transaction()?;
        transaction.execute_batch(migration.sql)?;
        transaction.pragma_update(None, "user_version", migration.version)?;
        transaction.commit()?;
        version = migration.version;
    }

    Ok(version)
}

pub fn current_version(connection: &Connection) -> rusqlite::Result<i64> {
    connection.pragma_query_value(None, "user_version", |row| row.get(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_db() -> Connection {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .unwrap();
        connection
    }

    #[test]
    fn fresh_database_gets_latest_version() {
        let connection = memory_db();
        assert_eq!(current_version(&connection).unwrap(), 0);
        assert_eq!(migrate(&connection).unwrap(), 3);
        assert_eq!(current_version(&connection).unwrap(), 3);
    }

    #[test]
    fn migrating_twice_is_a_no_op() {
        let connection = memory_db();
        migrate(&connection).unwrap();
        migrate(&connection).unwrap();
        assert_eq!(current_version(&connection).unwrap(), 3);
    }

    #[test]
    fn all_expected_tables_exist() {
        let connection = memory_db();
        migrate(&connection).unwrap();
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
            .unwrap();
        let names: Vec<String> = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        for table in [
            "library_folders",
            "playlists",
            "playlist_entries",
            "playback_state",
            "queue_entries",
            "tracks",
        ] {
            assert!(names.contains(&table.to_string()), "missing table {table}");
        }
    }

    #[test]
    fn tracks_path_is_unique() {
        let connection = memory_db();
        migrate(&connection).unwrap();
        let result = connection.execute(
            "INSERT INTO tracks (path, filename, file_size, modified_time, title, artist, album, album_artist, created_at, updated_at)
             VALUES ('/a.mp3', 'a.mp3', 1, 1, 't', 'a', 'al', 'a', 0, 0)",
            [],
        );
        assert!(result.is_ok());
        let duplicate = connection.execute(
            "INSERT INTO tracks (path, filename, file_size, modified_time, title, artist, album, album_artist, created_at, updated_at)
             VALUES ('/a.mp3', 'b.mp3', 1, 1, 't', 'a', 'al', 'a', 0, 0)",
            [],
        );
        assert!(duplicate.is_err(), "path must be unique");
    }

    #[test]
    fn playlist_entries_survive_track_deletion() {
        let connection = memory_db();
        migrate(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO playlists (id, name, created_at, updated_at) VALUES (1, 'mix', 0, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO tracks (id, path, filename, file_size, modified_time, title, artist, album, album_artist, created_at, updated_at)
                 VALUES (1, '/a.mp3', 'a.mp3', 1, 1, 't', 'a', 'al', 'a', 0, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO playlist_entries (playlist_id, track_id, position, original_reference, availability)
                 VALUES (1, 1, 0, '/a.mp3', 'available')",
                [],
            )
            .unwrap();

        connection
            .execute("DELETE FROM tracks WHERE id = 1", [])
            .unwrap();

        let availability: String = connection
            .query_row(
                "SELECT availability FROM playlist_entries WHERE playlist_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let track_id: Option<i64> = connection
            .query_row(
                "SELECT track_id FROM playlist_entries WHERE playlist_id = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(track_id, None);
        assert_eq!(availability, "available");
    }

    #[test]
    fn deleting_a_playlist_removes_its_entries() {
        let connection = memory_db();
        migrate(&connection).unwrap();
        connection
            .execute(
                "INSERT INTO playlists (id, name, created_at, updated_at) VALUES (1, 'mix', 0, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO playlist_entries (playlist_id, position, original_reference) VALUES (1, 0, '/a.mp3')",
                [],
            )
            .unwrap();
        connection
            .execute("DELETE FROM playlists WHERE id = 1", [])
            .unwrap();
        let remaining: i64 = connection
            .query_row("SELECT COUNT(*) FROM playlist_entries", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0);
    }
}
