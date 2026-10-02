# Architecture

Strata is one process with two halves: a toolkit-independent core, and a GTK front end that
only reads and asks.

```
src/
  main.rs          process entry point; calls app::run::main_with_args
  lib.rs           the crate root, so everything but main() is testable
  app/
    core.rs        AppCore: the single owner of all application state
    actions.rs     every action the UI can ask for, in one list
    navigation.rs  back/forward history and the current view
    state.rs       the visible track list, its sort, and the search state
    run.rs         startup: options, paths, database, settings, GTK application
    ui/            widgets; no business rules live here
  library/         folders, tracks, tags, scanning, identity reconciliation
  playback/        queue, shuffle, repeat, GStreamer player, session state
  playlists/      playlists, entries, M3U/M3U8/XSPF import and export
  artwork/         embedded and folder artwork, with an on-disk cache
  settings/        preferences, keyboard bindings, shortcut parsing
  database/        connection, schema, migrations
```

## Rules

1. **The core owns the state.** `AppCore` is the only thing that mutates the library, the
   queue or the settings. Widgets hold an `Rc<RefCell<AppCore>>` (`Shared` in
   `app::ui::window`) and never own a second copy of anything.
2. **Actions are data.** Every user gesture becomes an `Action` from `app::actions`. The UI
   dispatches it, `AppCore::perform` carries it out, and the UI redraws. Menus, buttons,
   keyboard shortcuts and the actions behind them share one table, so nothing can
   behave differently depending on how it was triggered.
3. **The UI asks the core for what to show.** `refresh` reads state and pushes it into the
   widgets. No filtering, sorting or counting happens in the widget layer; queries do that in
   SQL.
4. **Requests come back the other way.** When the core needs something only the UI can do —
   a file chooser, a confirmation, a message — it pushes a `UiRequest`. The window drains
   them on its timer tick. The core never touches a widget.
5. **Nothing modifies the user's music.** The scanner reads tags and records paths. Moving,
   renaming or tagging a file is the user's business; Strata notices and follows.

## Threading

- GTK is initialised and used on the main thread only. Widget creation, callbacks and dialogs
  all happen there.
- Scanning runs on a worker thread and communicates through `std::sync::mpsc`. Progress
  arrives as `ScanEvent` values; the main thread turns them into a progress bar and a
  notification.
- Artwork loading and tag reading happen on worker threads. Only finished images cross back
  to the main thread, as `gdk::Texture`.
- Audio is GStreamer's; Strata only sends it URIs, play/pause, seek and volume messages.

## Tests

`cargo test` runs everything without a display or an audio device. The rules that keep it
that way:

- Widget code is thin enough to be verified by the state it produces, not by pixels.
- Anything that parses, sorts, formats or maps keys is a plain function with unit tests.
- Integration tests in `tests/library.rs` drive the real database and the real fixtures.
- GTK widgets are never constructed in tests, because `gtk::init` is not available there.