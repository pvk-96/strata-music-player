# Scanning

A scan walks each watched folder, decides what every file is, and records that. It never
writes to your music.

## One file at a time

For each file the scanner knows three things: the path, the size and the modification time. It
asks the reconciler what the file is, and the cheapest answer wins:

1. **The path is indexed, size and mtime match** — nothing happened. The tags are not read
   again, which is what makes a rescan of a large library fast.
2. **The path is indexed, size or mtime differ** — the file was modified. Read the tags,
   update the row. The row keeps its id, its favourite flag and its playlist entries.
3. **The path is not indexed** — the file is either new or a track that moved. Tags are read
   and the indexed rows are weighed (see below).
4. **Two rows fit equally well** — the file is indexed on its own and marked `ambiguous`. The
   candidates are recorded in `track_identity_candidates` so the user can be asked which one
   it is. Strata does not guess between two plausible tracks.

## Recognising a move

A track's identity is its row, not its path, so moving a file must not create a second row. A
file can be a move candidate only when its old path is gone from disk, which means a genuine
copy — the original still there — is always indexed as a new track.

A candidate is rejected outright when the durations differ by more than
`DURATION_CONFLICT_MS`. Otherwise a candidate must satisfy at least one of:

- same filename **and** same size;
- same size **and** a duration within `DURATION_TOLERANCE_MS`;
- same size **and** same title **and** same artist.

Candidates are then scored: filename 4, size 2, duration 2, title 2, artist 2, album 1. The
highest score wins if it is unique; a tie is ambiguous.

Reconciliation runs against the whole library, not just the folder being walked, so moving a
file between two watched folders is recognised as the move it is.

## Missing files

When a folder no longer contains a file that the database has, its row is marked `missing`:
it leaves every view, keeps its favourite flag, its playlist entries and its statistics, and
comes back unchanged if the file reappears.

## Progress

The scan runs on a worker thread and emits `ScanEvent::Started`, `FolderStarted`, `Progress`,
`FileFailed`, `FolderFinished`, `Finished` and `Cancelled`. The window drains those on its
timer tick, so a cancelled scan stops at the next file boundary.