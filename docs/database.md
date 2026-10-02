# The library database

One SQLite file, `library.db`, under the data directory. It is the authority for the library:
nothing is cached anywhere else that could disagree with it.

## Tables

| Table | Holds |
| --- | --- |
| `library_folders` | every folder the user has added, by absolute path |
| `tracks` | one row per file: path, size, mtime, tags, audio properties, favourite, artwork reference, `missing`, `ambiguous` |
| `track_identity_candidates` | when a moved file matches more than one indexed row, the candidates are recorded and the row is flagged `ambiguous` instead of being guessed at |
| `playlists` | playlists by name and modification time |
| `playlist_entries` | ordered entries; `track_id` may be null, in which case `original_reference` keeps what the playlist file said |
| `queue_entries` | the saved queue, by position |
| `playback_state` | a single row: current track, position, shuffle, repeat, volume, mute, and the context the queue was built from |

## Decisions worth knowing

- **Missing files stay.** When a scan does not find a file, its row is marked `missing` and
  it disappears from the views. Favourites, playlists and statistics keep their reference, so
  putting the file back restores everything.
- **Moves are reconciled, not duplicated.** See `docs/scanner.md`.
- **Wildcards in search are literal.** `%` and `_` are escaped, so searching for `100%` finds
  `100%` and not everything.
- **Tags are stored as read.** Empty stays empty; the display helpers (`display_title`,
  `display_artist`, `display_album`) fall back to the filename, `Unknown Artist` and
  `Unknown Album`. This keeps filtering predictable: an untagged file is grouped under
  "Unknown Artist" rather than under its filename.
- **Indexes are measured.** The schema carries indexes for artist, album, album artist, title,
  filename, favourite, missing, folder, ambiguous, playlist position and queue position, plus
  the album grouping index. `tests/perf.rs` (ignored by default) measures the queries that
  matter on a synthetic library.

## Settings are not in here

Preferences, keyboard bindings and path mappings are a TOML file next to the database. They
are validated on load, corrected where the correction is unambiguous, and reported otherwise
in the startup warnings. See `src/settings/model.rs`.