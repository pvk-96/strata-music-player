//! SQL definitions.
//!
//! Statements are kept as constants so the shape of the store is reviewable in one place.
//! Extensions over the conceptual specification: `tracks.folder_id`, `tracks.missing`,
//! `tracks.ambiguous`, the audio property columns used by Track Information, and the
//! `playback_state` / `queue_entries` tables used to restore a session.

/// Base tables. Applied as migration 1.
pub const MIGRATION_1: &str = r#"
CREATE TABLE library_folders (
    id         INTEGER PRIMARY KEY,
    path       TEXT    NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
);

CREATE TABLE tracks (
    id                 INTEGER PRIMARY KEY,
    path               TEXT    NOT NULL UNIQUE,
    filename           TEXT    NOT NULL,
    folder_id          INTEGER REFERENCES library_folders(id) ON DELETE SET NULL,

    file_size          INTEGER NOT NULL,
    modified_time      INTEGER NOT NULL,
    missing            INTEGER NOT NULL DEFAULT 0,

    title              TEXT    NOT NULL,
    artist             TEXT    NOT NULL,
    album              TEXT    NOT NULL,
    album_artist       TEXT    NOT NULL,
    genre              TEXT,
    year               INTEGER,
    track_number       INTEGER,
    disc_number        INTEGER,
    duration_ms        INTEGER,

    format             TEXT,
    sample_rate        INTEGER,
    channels           INTEGER,
    bit_depth          INTEGER,
    bitrate            INTEGER,

    favorite           INTEGER NOT NULL DEFAULT 0,
    artwork_reference  TEXT,
    ambiguous          INTEGER NOT NULL DEFAULT 0,

    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

CREATE TABLE playlists (
    id         INTEGER PRIMARY KEY,
    name       TEXT    NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

-- Playlist entries reference tracks, never paths. Entries whose file disappeared keep a
-- NULL track_id plus the original reference so nothing is silently dropped.
CREATE TABLE playlist_entries (
    id                  INTEGER PRIMARY KEY,
    playlist_id         INTEGER NOT NULL REFERENCES playlists(id) ON DELETE CASCADE,
    track_id            INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
    position            INTEGER NOT NULL,
    original_reference  TEXT    NOT NULL,
    availability        TEXT    NOT NULL DEFAULT 'available'
);

CREATE TABLE playback_state (
    id               INTEGER PRIMARY KEY CHECK (id = 1),
    track_id         INTEGER REFERENCES tracks(id) ON DELETE SET NULL,
    position_ms      INTEGER NOT NULL DEFAULT 0,
    shuffle          INTEGER NOT NULL DEFAULT 0,
    repeat_mode      TEXT    NOT NULL DEFAULT 'off',
    volume           REAL    NOT NULL DEFAULT 1.0,
    muted            INTEGER NOT NULL DEFAULT 0,
    context_kind     TEXT,
    -- Album artist and album joined by US, a playlist id, a folder path, or a search term.
    context_key      TEXT
);

CREATE TABLE queue_entries (
    id         INTEGER PRIMARY KEY,
    position   INTEGER NOT NULL,
    track_id   INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE
);

CREATE INDEX idx_tracks_artist       ON tracks(artist COLLATE NOCASE);
CREATE INDEX idx_tracks_album        ON tracks(album COLLATE NOCASE);
CREATE INDEX idx_tracks_album_artist ON tracks(album_artist COLLATE NOCASE);
CREATE INDEX idx_tracks_title        ON tracks(title COLLATE NOCASE);
CREATE INDEX idx_tracks_filename     ON tracks(filename COLLATE NOCASE);
CREATE INDEX idx_tracks_favorite     ON tracks(favorite);
CREATE INDEX idx_tracks_missing      ON tracks(missing);
CREATE INDEX idx_tracks_folder       ON tracks(folder_id);
CREATE INDEX idx_tracks_ambiguous    ON tracks(ambiguous);
CREATE INDEX idx_entries_playlist    ON playlist_entries(playlist_id, position);
CREATE INDEX idx_queue_position      ON queue_entries(position);
"#;

/// Indexes added after the first library was measured. See `docs/database.md` for the
/// `EXPLAIN QUERY PLAN` output these are based on.
pub const MIGRATION_2: &str = r#"
CREATE INDEX idx_tracks_album_group ON tracks(album_artist COLLATE NOCASE, album COLLATE NOCASE, disc_number, track_number);
CREATE INDEX idx_tracks_added        ON tracks(created_at);
"#;
/// Candidates the scanner could not choose between. The user resolves these by merging the
/// ambiguous track into one of the candidates, or by keeping both.
pub const MIGRATION_3: &str = r#"
CREATE TABLE track_identity_candidates (
    track_id     INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    candidate_id INTEGER NOT NULL REFERENCES tracks(id) ON DELETE CASCADE,
    PRIMARY KEY (track_id, candidate_id)
);
"#;
