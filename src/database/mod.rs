//! SQLite access.
//!
//! One connection is shared by the UI and the scanner workers. SQLite handles concurrent
//! readers well and writes are short; `busy_timeout` covers the brief moments when a scan
//! batch is committing.

pub mod migrations;
pub mod schema;

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::error::{Error, Result};

pub struct Database {
    connection: Connection,
    path: PathBuf,
}

impl Database {
    /// Open (creating if needed) the library database and bring it up to date.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| Error::Filesystem {
                path: parent.to_path_buf(),
                detail: err.to_string(),
            })?;
        }
        let connection = open_connection(path)?;
        migrations::migrate(&connection)
            .map_err(|err| Error::Database(format!("schema upgrade failed: {err}")))?;
        Ok(Self {
            connection,
            path: path.to_path_buf(),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        let connection = Connection::open_in_memory()?;
        configure(&connection)?;
        migrations::migrate(&connection)
            .map_err(|err| Error::Database(format!("schema upgrade failed: {err}")))?;
        Ok(Self {
            connection,
            path: PathBuf::from(":memory:"),
        })
    }

    pub fn connection(&self) -> &Connection {
        &self.connection
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `ok` when the store is readable; `Err` describes the problem.
    pub fn check_integrity(&self) -> Result<()> {
        let result: String = self
            .connection
            .query_row("PRAGMA integrity_check", [], |row| row.get(0))
            .map_err(|err| Error::Database(err.to_string()))?;
        if result == "ok" {
            Ok(())
        } else {
            Err(Error::Database(format!(
                "integrity check reported: {result}"
            )))
        }
    }

    /// Write a consistent copy of the library to `destination`.
    ///
    /// This backs up Strata's index and playlists. It does not copy music files.
    pub fn backup_to(&self, destination: &Path) -> Result<()> {
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|err| Error::Filesystem {
                path: parent.to_path_buf(),
                detail: err.to_string(),
            })?;
        }
        if destination == self.path {
            return Err(Error::config(
                "the backup must be a different file than the live library",
            ));
        }
        let target = destination.to_string_lossy().to_string();
        self.connection
            .execute("VACUUM INTO ?1", [target])
            .map_err(|err| Error::Database(format!("backup failed: {err}")))?;
        log::info!("library backup written to {}", destination.display());
        Ok(())
    }

    /// Number of rows in `table`, used by diagnostics.
    pub fn count(&self, table: &str) -> Result<i64> {
        let sql = format!("SELECT COUNT(*) FROM {table}");
        self.connection
            .query_row(&sql, [], |row| row.get(0))
            .map_err(|err| Error::Database(err.to_string()))
    }
}

fn open_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| Error::Database(format!("could not open the library store: {err}")))?;
    configure(&connection)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<()> {
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|err| Error::Database(err.to_string()))?;
    connection
        .pragma_update(None, "foreign_keys", "ON")
        .map_err(|err| Error::Database(err.to_string()))?;
    // Write-ahead logging keeps the UI readable while the scanner commits batches.
    connection
        .pragma_update(None, "journal_mode", "WAL")
        .map_err(|err| Error::Database(err.to_string()))?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|err| Error::Database(err.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opening_creates_the_file_and_schema() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("nested").join("library.db");
        let database = Database::open(&path).unwrap();
        assert!(path.exists());
        assert_eq!(database.count("tracks").unwrap(), 0);
        database.check_integrity().unwrap();
    }

    #[test]
    fn reopening_keeps_data() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.db");
        {
            let database = Database::open(&path).unwrap();
            database
                .connection()
                .execute(
                    "INSERT INTO playlists (name, created_at, updated_at) VALUES ('mix', 0, 0)",
                    [],
                )
                .unwrap();
        }
        let database = Database::open(&path).unwrap();
        assert_eq!(database.count("playlists").unwrap(), 1);
    }

    #[test]
    fn backup_produces_a_usable_copy() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.db");
        let database = Database::open(&path).unwrap();
        database
            .connection()
            .execute(
                "INSERT INTO playlists (name, created_at, updated_at) VALUES ('mix', 0, 0)",
                [],
            )
            .unwrap();

        let backup_path = temp.path().join("backup.db");
        database.backup_to(&backup_path).unwrap();
        assert!(backup_path.exists());

        let restored = Database::open(&backup_path).unwrap();
        restored.check_integrity().unwrap();
        assert_eq!(restored.count("playlists").unwrap(), 1);
    }

    #[test]
    fn backup_over_live_database_is_refused() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.db");
        let database = Database::open(&path).unwrap();
        let error = database.backup_to(&path).unwrap_err();
        assert_eq!(error.kind(), "InvalidConfiguration");
    }

    #[test]
    fn corrupt_file_is_detected() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("library.db");
        std::fs::write(&path, "this is not a database").unwrap();

        // Opening fails outright because the header is wrong.
        assert!(Database::open(&path).is_err());
    }
}
