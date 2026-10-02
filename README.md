# Strata

A local-first music player for your own library. Strata reads the tags of the files you
already have, keeps its library in a local SQLite database, and never renames, moves, rewrites
or deletes your music.

No accounts, no telemetry, no network access.

## What it does

- Browse tracks, albums, artists, folders, favourites and playlists.
- Queue, shuffle, repeat, seek and resume the session where it left off.
- Search the whole library as you type.
- Playlists, with M3U, M3U8 and XSPF import and export.
- Rescan in place: a moved file keeps its favourites and its place in playlists.
- Rebind every keyboard shortcut from Preferences.
- Favourites, per-track information and an integrated "show in file manager".

## Audio formats

Strata bundles no codecs of its own. Tags and embedded artwork are read with
[lofty](https://docs.rs/lofty), and playback is a single GStreamer `playbin` that auto-plugs,
so the formats you can play are the ones your GStreamer plugins can decode. In practice:
FLAC, MP3, Ogg Vorbis, Opus, AAC and ALAC in M4A, WAV, AIFF and WMA. Everything is indexed
regardless of type, so a file with no tags still appears in the library.

## How it is built

- **Interface** — GTK 4 (needs 4.14 for `GtkAlertDialog` and `GtkFileDialog`) through the safe
  `gtk4` bindings; one `AppCore` owns the state, the widgets only render and report gestures.
- **Playback** — `gstreamer` 0.25, one `playbin`, position and volume polled on a timeout.
- **Tags** — `lofty` 0.25.
- **Library** — a bundled SQLite via `rusqlite`; every list view is a query, never an in-memory
  filter. See [docs/database.md](docs/database.md).
- **Settings** — `serde` and `toml`, one `settings.toml` beside the database.
- **Optional** — a `log` backend for diagnostics.

## Building

Requirements: Rust 1.85 or newer, and the GTK 4.14 development files, plus the GStreamer
development files that the `gstreamer` crate builds against. Strata needs GTK's `v4_14` feature
because it uses `GtkAlertDialog` and `GtkFileDialog`, which arrived in 4.10.

On Arch:

```sh
pacman -S --needed base-devel rust gtk4 gstreamer
cargo build --release
```

The binary lands in `target/release/strata`.

## Installing on Arch

There is no released package yet; install from the source tree:

```sh
cargo install --path . --locked
strata
```

`cargo install` puts the binary in `~/.cargo/bin`. To keep the desktop entry and icon, either
copy the files from [data/](data) by hand or build the Arch-friendly Flatpak, described in
[docs/packaging.md](docs/packaging.md).

## Running

```sh
strata                        # open the library
strata --add-folder ~/Music    # add a folder and index it straight away
strata --no-session           # start empty instead of restoring the last session
strata --version
```

State lives in the usual places:

| What | Where |
| --- | --- |
| Library | `$XDG_DATA_HOME/strata/library.db` (or `~/.local/share/strata`) |
| Settings and shortcuts | `$XDG_CONFIG_HOME/strata/settings.toml` |
| Artwork cache | `$XDG_CACHE_HOME/strata/` |

## Keyboard

`Space` plays, `N` and `P` change track, `[` and `]` seek, `S` and `R` toggle shuffle and
repeat, `Ctrl+K` searches, `Ctrl+1`…`Ctrl+6` jump between views, and `Ctrl+Q` quits. The full
table, and how to change any of it, is in [docs/keyboard.md](docs/keyboard.md).

## Development

```sh
cargo fmt                          # rustfmt, the tree is formatted
cargo clippy --all-targets         # no warnings allowed
cargo test                         # unit and integration tests, no display needed
cargo test -- --ignored            # the query benchmarks in tests/perf.rs
cargo build --release
python3 scripts/generate-icons.py --check   # the committed icons match the master
```

The suite covers the scanner and its identity rules, metadata reading, the database queries,
playlists and M3U handling, session restore, shortcut parsing and accelerator conversion, the
packaging files, and the UI's pure helpers. It needs neither a display nor an audio device.

The icons are generated, not drawn by hand. Edit `assets/branding/strata-icon.svg`, run
`python3 scripts/generate-icons.py`, and commit the result; `--check` fails when the PNG and ICO
files in the tree have drifted from the master. The rules the master has to keep are in
[docs/branding.md](docs/branding.md).

## Status

`0.1.0`, pre-release: the library, playback, playlists and packaging all work, but the app has
not been packaged for a distribution yet, and the interface is still being adjusted. Two things
are known to be missing: media-key support over the session bus (nothing implements it yet, so
there is no switch for it), and path mappings for playlists written on another machine (the
setting is stored and validated, but nothing applies it yet). The Rust API is public and may
still change.

## Documentation

- [docs/architecture.md](docs/architecture.md) — how the core and the UI are separated
- [docs/database.md](docs/database.md) — the schema and what is stored where
- [docs/scanner.md](docs/scanner.md) — how moves, edits and missing files are recognised
- [docs/keyboard.md](docs/keyboard.md) — shortcuts, rebinding, GTK conversion
- [docs/branding.md](docs/branding.md) — the icon master, its geometry and how to regenerate it
- [docs/packaging.md](docs/packaging.md) — desktop integration and packaging
- [docs/licences.md](docs/licences.md) — the dependency licence audit

## Licence

GPL-3.0-or-later. See [LICENSE](LICENSE).