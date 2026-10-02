//! The application core.
//!
//! Everything the app can do lives here: it holds the library, the settings, the player and
//! the playback order, and implements [`ActionTarget`]. The GTK layer only builds widgets
//! and calls into this type, which keeps the behaviour testable without a display.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use crate::app::actions::{Action, ActionTarget};
use crate::app::navigation::{Navigation, Section, View};
use crate::app::state::{EntryRow, SearchState, TrackListState};
use crate::artwork::cache::ArtworkCache;
use crate::artwork::loader::ArtworkLoader;
use crate::database::Database;
use crate::error::{Error, Result};
use crate::library::model::{
    AlbumSummary, ArtistSummary, FolderSummary, SortDirection, Track, TrackQuery, TrackSort,
};
use crate::library::queries;
use crate::library::scanner::{self, ScanEvent, ScanKind};
use crate::playback::player::{Player, PlayerEvent};
use crate::playback::state::{
    ContextKind, PlaybackContext, PlaybackState, Queue, RepeatMode, Sequencer, Step,
};
use crate::playlists::model::Playlist;
use crate::playlists::queries as playlist_queries;
use crate::settings::keyboard::KeyboardSettings;
use crate::settings::model::Settings;

/// Where state lives on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub cache: PathBuf,
}

impl Paths {
    pub fn from_base(base: &std::path::Path) -> Self {
        Self {
            config: base.join("config"),
            data: base.join("data"),
            cache: base.join("cache"),
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config.join("settings.toml")
    }

    pub fn database_file(&self) -> PathBuf {
        self.data.join("library.db")
    }

    pub fn artwork_cache(&self) -> PathBuf {
        self.cache.join("artwork")
    }

    /// Default locations, following the XDG base directory specification.
    pub fn standard() -> Result<Self> {
        let home = dirs::home_dir()
            .or_else(dirs::config_dir)
            .ok_or_else(|| Error::InvalidConfiguration("no home directory is set".to_string()))?;
        let config_home = dirs::config_dir().unwrap_or_else(|| home.join(".config"));
        let data_home = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));
        let cache_home = dirs::cache_dir().unwrap_or_else(|| home.join(".cache"));
        Ok(Self {
            config: config_home.join("strata"),
            data: data_home.join("strata"),
            cache: cache_home.join("strata"),
        })
    }
}

/// Things the core asks the interface to do.
#[derive(Debug, Clone, PartialEq)]
pub enum UiRequest {
    /// Rebuild the view for the current navigation state.
    RefreshView,
    /// Rebuild the sidebar, for example after playlists changed.
    RefreshSidebar,
    RefreshPlayerBar,
    /// Show a message the user should see.
    Notify {
        message: String,
        warning: bool,
    },
    OpenTrackInfo(i64),
    AskToChooseFolder,
    AskToOpenSettings,
    AskToImportPlaylist,
    AskToSavePlaylist,
    AskToCreatePlaylist,
    ShowInFileManager(PathBuf),
    SaveSession,
}

/// Volume used until a saved session says otherwise.
pub const DEFAULT_VOLUME: f64 = 1.0;

/// Progress of a running scan, as shown in the header.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScanProgress {
    pub running: bool,
    pub processed: usize,
    pub total: usize,
    pub current: Option<PathBuf>,
}

/// The application core.
pub struct AppCore {
    database: Database,
    settings: Settings,
    player: Option<Player>,
    artwork: ArtworkLoader,
    sequencer: Sequencer,
    navigation: Navigation,
    tracks: TrackListState,
    search: SearchState,
    paths: Paths,
    sidebar_visible: bool,
    scan: ScanProgress,
    scan_cancel: Arc<AtomicBool>,
    scan_events: Option<Receiver<ScanEvent>>,
    requests: Sender<UiRequest>,
    pending_requests: Vec<UiRequest>,
    volume: f64,
    muted: bool,
    /// Latest position reported by the pipeline, used to redraw the seek bar.
    position_hint: Option<i64>,
    /// Last message shown, used by tests.
    last_message: Option<String>,
}

impl AppCore {
    /// Build the core. `player` is optional so the library can be inspected on a machine
    /// without audio output; the interface reports that state honestly.
    pub fn new(
        paths: Paths,
        database: Database,
        settings: Settings,
        player: Option<Player>,
        requests: Sender<UiRequest>,
    ) -> Self {
        let artwork = ArtworkLoader::new(Arc::new(ArtworkCache::new(paths.artwork_cache())));
        let sidebar_visible = settings.appearance.sidebar_visible;
        let mut core = Self {
            database,
            settings,
            player,
            artwork,
            sequencer: Sequencer::new(PlaybackContext::library(Vec::new())),
            navigation: Navigation::new(View::Library),
            tracks: TrackListState::new(TrackQuery::default(), TrackSort::DateAdded),
            search: SearchState::new(),
            paths,
            sidebar_visible,
            scan: ScanProgress::default(),
            scan_cancel: Arc::new(AtomicBool::new(false)),
            scan_events: None,
            requests,
            pending_requests: Vec::new(),
            volume: DEFAULT_VOLUME,
            muted: false,
            position_hint: None,
            last_message: None,
        };
        core.restore_session();
        core.reload_view();
        core
    }

    pub fn database(&self) -> &Database {
        &self.database
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn keyboard(&self) -> &KeyboardSettings {
        &self.settings.keyboard
    }

    pub fn paths(&self) -> &Paths {
        &self.paths
    }

    pub fn navigation(&self) -> &Navigation {
        &self.navigation
    }

    pub fn sequencer(&self) -> &Sequencer {
        &self.sequencer
    }

    pub fn tracks(&self) -> &TrackListState {
        &self.tracks
    }

    /// Mutable view of the track list, so the interface can keep the selection in step
    /// with what the user clicked.
    pub fn tracks_mut(&mut self) -> &mut TrackListState {
        &mut self.tracks
    }

    pub fn search(&self) -> &SearchState {
        &self.search
    }

    pub fn player(&self) -> Option<&Player> {
        self.player.as_ref()
    }

    pub fn artwork(&self) -> &ArtworkLoader {
        &self.artwork
    }

    pub fn scan_progress(&self) -> &ScanProgress {
        &self.scan
    }

    pub fn scan_running(&self) -> bool {
        self.scan_events.is_some()
    }

    pub fn volume(&self) -> f64 {
        self.volume
    }

    pub fn is_muted(&self) -> bool {
        self.muted
    }

    pub fn playback_state(&self) -> PlaybackState {
        match &self.player {
            Some(player) => player.state(),
            None => PlaybackState::Stopped,
        }
    }

    pub fn sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    pub fn set_sidebar_visible(&mut self, visible: bool) {
        self.sidebar_visible = visible;
    }

    pub fn last_message(&self) -> Option<&str> {
        self.last_message.as_deref()
    }

    /// Take the requests the interface has not handled yet.
    pub fn take_requests(&mut self) -> Vec<UiRequest> {
        std::mem::take(&mut self.pending_requests)
    }

    pub fn request(&mut self, request: UiRequest) {
        self.last_message = match &request {
            UiRequest::Notify { message, .. } => Some(message.clone()),
            _ => self.last_message.clone(),
        };
        // The channel is the same one the interface reads, but requests are also buffered so
        // tests and headless runs see them.
        let _ = self.requests.send(request.clone());
        self.pending_requests.push(request);
    }

    pub fn notify(&mut self, message: impl Into<String>) {
        self.request(UiRequest::Notify {
            message: message.into(),
            warning: false,
        });
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.request(UiRequest::Notify {
            message: message.into(),
            warning: true,
        });
    }

    pub fn report(&mut self, error: &Error) {
        log::error!("{}: {error:?}", error.kind());
        self.warn(error.user_message());
    }

    // ----- navigation ---------------------------------------------------------

    pub fn go_to(&mut self, view: View) {
        self.navigation.go_to(view);
        self.reload_view();
        self.request(UiRequest::RefreshView);
    }

    pub fn go_to_section(&mut self, section: Section) {
        self.navigation.go_to_section(section);
        self.reload_view();
        self.request(UiRequest::RefreshView);
    }

    pub fn back(&mut self) {
        if self.navigation.back().is_some() {
            self.reload_view();
            self.request(UiRequest::RefreshView);
        }
    }

    pub fn forward(&mut self) {
        if self.navigation.forward().is_some() {
            self.reload_view();
            self.request(UiRequest::RefreshView);
        }
    }

    /// The query and track order for the current view.
    fn view_query(&self) -> Result<(TrackQuery, TrackSort, SortDirection)> {
        let mut query = TrackQuery::default();
        let sort = TrackSort::DateAdded;
        let direction = SortDirection::Ascending;

        match self.navigation.current().clone() {
            View::Favorites => query.favorites_only = true,
            View::Artist(name) => {
                query.artist = Some(name);
            }
            View::Album(album_artist, album) => {
                query.album_artist = Some(album_artist);
                query.album = Some(album);
            }
            View::Folder(path) => query.folder_path = Some(path),
            View::Playlist(id) => {
                let ids = playlist_queries::entry_track_ids(self.database.connection(), id)?;
                return Ok((
                    TrackQuery {
                        ids: Some(ids),
                        ..Default::default()
                    },
                    sort,
                    direction,
                ));
            }
            View::Search(term) => {
                query.search = Some(term);
            }
            View::Queue => {
                return Ok((
                    TrackQuery {
                        ids: Some(self.sequencer.queue().ids().to_vec()),
                        ..Default::default()
                    },
                    sort,
                    direction,
                ));
            }
            _ => {}
        }

        Ok((query, sort, direction))
    }

    /// Reload the visible track list from the library.
    pub fn reload_view(&mut self) {
        let (query, sort, direction) = match self.view_query() {
            Ok(value) => value,
            Err(error) => {
                self.report(&error);
                (
                    TrackQuery::default(),
                    TrackSort::DateAdded,
                    SortDirection::Ascending,
                )
            }
        };

        let keep_sort = self.tracks.sort;
        let keep_direction = self.tracks.direction;
        let mut state = TrackListState::new(query, sort);
        state.direction = direction;

        match queries::query_tracks(
            self.database.connection(),
            &state.query,
            keep_sort,
            keep_direction,
        ) {
            Ok(tracks) => state.set_tracks(&tracks),
            Err(error) => {
                self.report(&error);
            }
        }

        // Sorting and direction chosen by the user survive navigation.
        state.sort = keep_sort;
        state.direction = keep_direction;
        self.tracks = state;
    }

    pub fn library_stats(&self) -> Option<queries::LibraryStats> {
        queries::stats(self.database.connection()).ok()
    }

    // ----- collections the sidebar and views show ------------------------------

    /// Artists in the library, for the artists view.
    pub fn artists(&self) -> Vec<ArtistSummary> {
        queries::artists(self.database.connection()).unwrap_or_default()
    }

    /// Albums in the library, for the albums view. The current search term filters it.
    pub fn albums(&self) -> Vec<AlbumSummary> {
        let query = TrackQuery {
            search: Some(self.search.applied.clone()).filter(|term| !term.is_empty()),
            ..TrackQuery::default()
        };
        queries::albums(
            self.database.connection(),
            &query,
            queries::AlbumSort::Album,
            SortDirection::Ascending,
        )
        .unwrap_or_default()
    }

    /// Library folders, for the folders view.
    pub fn folders(&self) -> Vec<FolderSummary> {
        queries::folders(self.database.connection()).unwrap_or_default()
    }

    /// Playlists, newest first is not used: the user's own order is alphabetical.
    pub fn playlists(&self) -> Vec<Playlist> {
        playlist_queries::all(self.database.connection()).unwrap_or_default()
    }

    /// Entries of a playlist, including ones that can no longer be resolved.
    pub fn playlist_entries(&self, playlist_id: i64) -> Result<Vec<EntryRow>> {
        let entries = playlist_queries::entries(self.database.connection(), playlist_id)?;
        Ok(entries.iter().map(EntryRow::from_entry).collect())
    }

    /// Name of a playlist, or an empty string when it is gone.
    pub fn playlist_name(&self, playlist_id: i64) -> String {
        playlist_queries::get(self.database.connection(), playlist_id)
            .map(|playlist| playlist.name)
            .unwrap_or_default()
    }

    /// Re-check every playlist entry against the library and the filesystem.
    pub fn revalidate_playlists(&mut self) {
        match playlist_queries::revalidate(self.database.connection()) {
            Ok(changed) => {
                if changed > 0 {
                    self.request(UiRequest::RefreshSidebar);
                    self.request(UiRequest::RefreshView);
                }
            }
            Err(error) => self.report(&error),
        }
    }

    // ----- playback -----------------------------------------------------------

    /// Play a track and make it the start of the current context.
    pub fn play_track(&mut self, track_id: i64) -> Result<()> {
        let track = queries::get_track(self.database.connection(), track_id)?;
        if track.missing {
            return Err(Error::MissingFile(track.path));
        }

        let context_ids = if self.tracks.playable_track_ids().contains(&track_id) {
            self.tracks.playable_track_ids()
        } else {
            vec![track_id]
        };
        let context = match self.navigation.current() {
            View::Playlist(id) => PlaybackContext::playlist(*id, context_ids),
            View::Album(album_artist, album) => {
                PlaybackContext::album(album_artist, album, context_ids)
            }
            View::Folder(path) => PlaybackContext::folder(path, context_ids),
            View::Search(term) => PlaybackContext::search(term, context_ids),
            _ => PlaybackContext::library(context_ids),
        };
        self.sequencer.set_context(context);
        self.start(track_id)
    }

    /// Start playback of a track that is already in the context.
    fn start(&mut self, track_id: i64) -> Result<()> {
        match self.sequencer.play(track_id) {
            Some(Step::Play { track_id, .. }) => track_id,
            _ => {
                return Err(Error::Playback(
                    "that track is not in the current list".to_string(),
                ))
            }
        };

        let Some(player) = &self.player else {
            self.warn("Audio output isn't available. The library is still usable.");
            self.sequencer.set_state(PlaybackState::Error);
            return Ok(());
        };

        let track = queries::get_track(self.database.connection(), track_id)?;
        player.set_volume(self.volume);
        player.set_muted(self.muted);
        match player.play(&track.path) {
            Ok(()) => {
                self.sequencer.set_state(PlaybackState::Playing);
                self.request(UiRequest::RefreshPlayerBar);
                Ok(())
            }
            Err(error) => {
                self.sequencer.set_state(PlaybackState::Error);
                Err(error)
            }
        }
    }

    pub fn toggle_play(&mut self) {
        let Some(track_id) = self.sequencer.current_track_id() else {
            if let Some(first) = self.tracks.playable_track_ids().first().copied() {
                let _ = self.play_track(first);
            } else {
                self.notify("There is nothing to play yet.");
            }
            return;
        };

        match &self.player {
            Some(player) => {
                player.toggle_play();
                self.sequencer.set_state(player.state());
                self.request(UiRequest::RefreshPlayerBar);
            }
            None => match self.sequencer.state() {
                PlaybackState::Playing | PlaybackState::Paused => {
                    self.sequencer.set_state(PlaybackState::Stopped);
                }
                _ => {
                    if let Err(error) = self.start(track_id) {
                        self.report(&error);
                    }
                }
            },
        }
    }

    pub fn next_track(&mut self) {
        match self.sequencer.next() {
            Step::Play {
                track_id,
                position_ms,
            } => {
                if let Err(error) = self.play_at(track_id, position_ms) {
                    self.report(&error);
                }
            }
            Step::Restart => self.seek_to(0),
            Step::Stop => {
                if let Some(player) = &self.player {
                    player.stop();
                }
                self.sequencer.set_state(PlaybackState::Stopped);
                self.request(UiRequest::RefreshPlayerBar);
            }
        }
    }

    pub fn previous_track(&mut self) {
        match self.sequencer.previous() {
            Step::Play {
                track_id,
                position_ms,
            } => {
                if let Err(error) = self.play_at(track_id, position_ms) {
                    self.report(&error);
                }
            }
            Step::Restart => self.seek_to(0),
            Step::Stop => {
                self.seek_to(0);
            }
        }
    }

    fn play_at(&mut self, track_id: i64, position_ms: i64) -> Result<()> {
        if let Some(player) = &self.player {
            let track = queries::get_track(self.database.connection(), track_id)?;
            player.play(&track.path)?;
            player.set_volume(self.volume);
            player.set_muted(self.muted);
            if position_ms > 0 {
                player.seek_ms(position_ms);
            }
            self.sequencer.set_position_ms(position_ms);
            self.sequencer.set_state(PlaybackState::Playing);
        } else {
            self.sequencer.set_state(PlaybackState::Error);
        }
        self.request(UiRequest::RefreshPlayerBar);
        Ok(())
    }

    pub fn seek_by(&mut self, delta_ms: i64) {
        let position = self
            .player
            .as_ref()
            .map(|player| player.position_ms())
            .unwrap_or_else(|| self.sequencer.position_ms());
        self.seek_to(position + delta_ms);
    }

    pub fn seek_to(&mut self, position_ms: i64) {
        if let Some(player) = &self.player {
            player.seek_ms(position_ms);
        }
        self.sequencer.set_position_ms(position_ms);
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn set_volume(&mut self, volume: f64) {
        let clamped = volume.clamp(0.0, 1.0);
        self.volume = clamped;
        if let Some(player) = &self.player {
            player.set_volume(clamped);
        }
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn volume_up(&mut self) {
        let step = self.settings.playback.seek_seconds.max(1) as f64 / 100.0;
        self.set_volume(self.volume + step);
    }

    pub fn volume_down(&mut self) {
        let step = self.settings.playback.seek_seconds.max(1) as f64 / 100.0;
        self.set_volume(self.volume - step);
    }

    pub fn toggle_mute(&mut self) {
        self.muted = !self.muted;
        if let Some(player) = &self.player {
            player.set_muted(self.muted);
        }
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn toggle_shuffle(&mut self) {
        self.sequencer.toggle_shuffle();
        self.save_session();
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn reshuffle(&mut self) {
        self.sequencer.reshuffle();
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn cycle_repeat(&mut self) {
        let mode = self.sequencer.cycle_repeat();
        self.save_session();
        self.notify(mode.label());
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn repeat_mode(&self) -> RepeatMode {
        self.sequencer.repeat()
    }

    pub fn shuffle_enabled(&self) -> bool {
        self.sequencer.shuffle_enabled()
    }

    /// Called from the interface tick: follows the pipeline and advances when a track ends.
    pub fn pump_playback(&mut self) {
        let Some(player) = &self.player else {
            return;
        };

        for event in player.drain_events() {
            match event {
                PlayerEvent::EndOfStream => {
                    if self.sequencer.repeat() == RepeatMode::One {
                        self.seek_to(0);
                    } else {
                        self.next_track();
                    }
                }
                PlayerEvent::Error(detail) => {
                    log::error!("playback failed: {detail}");
                    self.sequencer.set_state(PlaybackState::Error);
                    self.warn("That track could not be played.");
                    self.request(UiRequest::RefreshPlayerBar);
                }
                PlayerEvent::Tick {
                    position_ms,
                    duration_ms,
                } => {
                    self.sequencer.set_position_ms(position_ms);
                    if duration_ms.is_some() {
                        self.position_hint = Some(position_ms);
                    }
                }
            }
        }
    }

    /// Queue operations.
    pub fn add_to_queue(&mut self, track_ids: &[i64]) {
        if track_ids.is_empty() {
            self.warn("Select at least one track first.");
            return;
        }
        self.sequencer.queue_mut().add_all(track_ids);
        self.save_session();
        self.notify(format!(
            "{} added to the queue.",
            if track_ids.len() == 1 {
                "Track"
            } else {
                "Tracks"
            }
        ));
        self.request(UiRequest::RefreshPlayerBar);
    }

    pub fn play_next_in_queue(&mut self, track_ids: &[i64]) {
        if track_ids.is_empty() {
            self.warn("Select at least one track first.");
            return;
        }
        self.sequencer.queue_mut().play_next(track_ids);
        self.save_session();
        self.notify("Playing next.");
    }

    pub fn remove_from_queue(&mut self, index: usize) {
        if self.sequencer.queue_mut().remove(index).is_some() {
            self.save_session();
            if matches!(self.navigation.current(), View::Queue) {
                self.reload_view();
            }
        }
    }

    /// Move a row in the queue, used by drag and drop in the queue view. Returns true
    /// when the queue changed.
    pub fn reorder_queue(&mut self, from: usize, to: usize) -> bool {
        let queue = self.sequencer.queue_mut();
        let length = queue.len();
        if from >= length || to >= length || from == to {
            return false;
        }
        queue.move_to(from, to);
        true
    }

    pub fn clear_queue(&mut self) {
        self.sequencer.queue_mut().clear();
        self.save_session();
        if matches!(self.navigation.current(), View::Queue) {
            self.reload_view();
        }
    }

    /// Play the queue as a context.
    pub fn play_queue(&mut self) {
        let ids: Vec<i64> = self
            .sequencer
            .queue()
            .ids()
            .iter()
            .copied()
            .filter(|id| {
                queries::get_track(self.database.connection(), *id)
                    .map(|track| !track.missing)
                    .unwrap_or(false)
            })
            .collect();
        if ids.is_empty() {
            self.warn("The queue is empty.");
            return;
        }
        let first = ids[0];
        self.sequencer
            .set_context(PlaybackContext::queue(ids.clone()));
        if let Err(error) = self.start(first) {
            self.report(&error);
        }
        self.reload_view();
    }

    // ----- library ------------------------------------------------------------

    pub fn toggle_favorite(&mut self, track_ids: &[i64]) {
        if track_ids.is_empty() {
            return;
        }
        for track_id in track_ids {
            let current = queries::get_track(self.database.connection(), *track_id)
                .map(|track| track.favorite)
                .unwrap_or(false);
            if let Err(error) =
                queries::set_favorite(self.database.connection(), *track_id, !current)
            {
                self.report(&error);
            }
        }
        self.reload_view();
    }

    pub fn selected_track_ids(&self) -> Vec<i64> {
        self.tracks.selected_track_ids()
    }

    pub fn track(&self, track_id: i64) -> Option<Track> {
        queries::get_track(self.database.connection(), track_id).ok()
    }

    pub fn add_library_folder(&mut self, path: &std::path::Path) -> Result<()> {
        if !path.is_dir() {
            return Err(Error::Filesystem {
                path: path.to_path_buf(),
                detail: "not a folder".to_string(),
            });
        }
        let folder_id = queries::add_folder(self.database.connection(), path)?;
        log::info!("added library folder {path:?} as {folder_id}");
        Ok(())
    }

    pub fn remove_library_folder(&mut self, folder_id: i64) {
        if let Err(error) = queries::remove_folder(self.database.connection(), folder_id) {
            self.report(&error);
        }
    }

    /// Start a scan on a worker thread.
    pub fn start_scan(&mut self, kind: ScanKind) {
        if self.scan_running() {
            self.notify("A scan is already running.");
            return;
        }

        let database_path = self.database.path().to_path_buf();
        let cancel = Arc::clone(&self.scan_cancel);
        cancel.store(false, Ordering::Relaxed);
        let (sender, receiver): (Sender<ScanEvent>, Receiver<ScanEvent>) = channel();

        std::thread::spawn(move || {
            let result = scanner::run(&database_path, kind, &cancel, &mut |event| {
                if sender.send(event).is_err() {
                    log::debug!("scan event receiver is gone");
                }
            });
            if let Err(error) = result {
                log::error!("scan failed: {error:?}");
            }
        });

        self.scan_events = Some(receiver);
        self.scan = ScanProgress {
            running: true,
            ..Default::default()
        };
        self.request(UiRequest::RefreshView);
    }

    pub fn cancel_scan(&mut self) {
        if self.scan_running() {
            self.scan_cancel.store(true, Ordering::Relaxed);
            self.notify("Stopping the scan…");
        }
    }

    /// Collect scan events. Returns true when something changed the display.
    pub fn pump_scan(&mut self) -> bool {
        let Some(receiver) = self.scan_events.take() else {
            return false;
        };

        let mut changed = false;
        let mut finished = false;
        loop {
            match receiver.try_recv() {
                Ok(ScanEvent::Started { folders }) => {
                    self.scan = ScanProgress {
                        running: true,
                        processed: 0,
                        total: 0,
                        current: None,
                    };
                    if folders == 0 {
                        self.notify("No folders are in the library yet.");
                    }
                    changed = true;
                }
                Ok(ScanEvent::FolderStarted { path }) => {
                    self.scan.current = Some(path);
                    changed = true;
                }
                Ok(ScanEvent::Progress {
                    processed, total, ..
                }) => {
                    self.scan.processed = processed;
                    self.scan.total = total;
                    changed = true;
                }
                Ok(ScanEvent::FileFailed { path, reason }) => {
                    log::warn!("{} could not be read: {reason}", path.display());
                }
                Ok(ScanEvent::FolderFinished { .. }) => changed = true,
                Ok(ScanEvent::Finished { summary }) => {
                    self.scan.running = false;
                    self.reload_view();
                    self.request(UiRequest::RefreshSidebar);
                    self.notify(scan_summary_text(&summary));
                    finished = true;
                    changed = true;
                }
                Ok(ScanEvent::Cancelled) => {
                    self.scan.running = false;
                    self.reload_view();
                    self.notify("Scan stopped.");
                    finished = true;
                    changed = true;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // The scan thread is gone. `try_recv` keeps reporting this, so the loop
                    // has to end here or it would spin for the rest of the session.
                    self.scan.running = false;
                    finished = true;
                    break;
                }
            }
        }

        // Keep the receiver while a scan is still running.
        if !finished {
            self.scan_events = Some(receiver);
        }
        changed
    }

    // ----- playlists ----------------------------------------------------------

    pub fn create_playlist(&mut self, name: &str) -> Result<i64> {
        let playlist = playlist_queries::create(self.database.connection(), name)?;
        self.request(UiRequest::RefreshSidebar);
        Ok(playlist.id)
    }

    /// Rename a playlist.
    pub fn rename_playlist(&mut self, playlist_id: i64, name: &str) -> Result<()> {
        playlist_queries::rename(self.database.connection(), playlist_id, name)?;
        self.request(UiRequest::RefreshSidebar);
        Ok(())
    }

    /// Delete a playlist. Tracks in the library are untouched.
    pub fn delete_playlist(&mut self, playlist_id: i64) -> Result<()> {
        playlist_queries::delete(self.database.connection(), playlist_id)?;
        self.request(UiRequest::RefreshSidebar);
        Ok(())
    }

    pub fn add_to_playlist(&mut self, playlist_id: i64, track_ids: &[i64]) -> Result<()> {
        let added =
            playlist_queries::add_tracks(self.database.connection(), playlist_id, track_ids)?;
        if added > 0 {
            self.notify(format!("{added} added."));
        }
        Ok(())
    }

    pub fn import_playlist(&mut self, path: &std::path::Path, name: &str) -> Result<()> {
        let parsed = crate::playlists::m3u::parse_file(path)?;
        let playlist = playlist_queries::create(self.database.connection(), name)?;
        let summary = crate::playlists::m3u::import(&self.database, playlist.id, &parsed)?;
        self.request(UiRequest::RefreshSidebar);
        self.go_to(View::Playlist(playlist.id));
        if summary.unresolved > 0 {
            self.warn(format!(
                "{} of {} entries aren't in the library yet.",
                summary.unresolved,
                summary.total()
            ));
        } else {
            self.notify(format!("{} tracks imported.", summary.added));
        }
        Ok(())
    }

    pub fn export_playlist(&self, playlist_id: i64, path: &std::path::Path) -> Result<()> {
        let entries = playlist_queries::entries(self.database.connection(), playlist_id)?;
        let lines: Vec<(PathBuf, String)> = entries
            .iter()
            .filter(|entry| entry.track_id.is_some())
            .filter_map(|entry| {
                queries::get_track(self.database.connection(), entry.track_id?)
                    .ok()
                    .map(|track| {
                        (
                            track.path.clone(),
                            format!("{} - {}", track.display_artist(), track.display_title()),
                        )
                    })
            })
            .collect();
        let text = crate::playlists::m3u::export(&lines, true);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| Error::Filesystem {
                path: parent.to_path_buf(),
                detail: error.to_string(),
            })?;
        }
        std::fs::write(path, text).map_err(|error| Error::Filesystem {
            path: path.to_path_buf(),
            detail: error.to_string(),
        })
    }

    // ----- search -------------------------------------------------------------

    pub fn set_search_term(&mut self, term: &str) {
        self.search.term = term.to_string();
    }

    pub fn apply_search(&mut self) {
        let term = self.search.apply();
        self.navigation.go_to(View::Search(term));
        self.reload_view();
        self.request(UiRequest::RefreshView);
    }

    pub fn clear_search(&mut self) {
        self.search.clear();
        if matches!(self.navigation.current(), View::Search(_)) {
            self.navigation.go_to(View::Library);
            self.reload_view();
        }
        self.request(UiRequest::RefreshView);
    }

    // ----- session ------------------------------------------------------------

    pub fn restore_session(&mut self) {
        if !self.settings.playback.resume_session {
            return;
        }
        let Ok(Some((sequencer, volume, muted))) = Sequencer::restore(self.database.connection())
        else {
            return;
        };
        if sequencer.queue().is_empty() && sequencer.current_track_id().is_none() {
            return;
        }
        self.volume = volume.clamp(0.0, 1.0);
        self.muted = muted;
        self.sequencer = sequencer;
        log::info!(
            "restored session: queue {}, context {:?}",
            self.sequencer.queue().len(),
            self.sequencer.context().kind.clone()
        );
    }

    /// Write the session so the next start can resume it.
    pub fn save_session(&mut self) {
        if let Err(error) = self
            .sequencer
            .save(self.database.connection(), self.volume, self.muted)
        {
            log::warn!("session not saved: {}", error.kind());
        }
    }

    /// Apply changed settings and tell the interface to redraw.
    pub fn apply_settings(&mut self, settings: Settings) {
        self.settings = settings;
        self.request(UiRequest::RefreshView);
        self.request(UiRequest::RefreshSidebar);
        self.request(UiRequest::RefreshPlayerBar);
    }

    /// Copy selected tracks' paths to the clipboard is done by the interface; the core only
    /// provides the text.
    pub fn copy_paths_text(&self, track_ids: &[i64]) -> String {
        track_ids
            .iter()
            .filter_map(|id| self.track(*id))
            .map(|track| track.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Tracks in the current view that match a file name or path fragment, for the
    /// track information dialog's "similar files" section.
    pub fn ambiguous_tracks(&self) -> Vec<Track> {
        queries::ambiguous_tracks(self.database.connection()).unwrap_or_default()
    }

    /// Resolve ambiguous tracks: keep the file the user picked, drop the other.
    pub fn merge_tracks(&mut self, keep_id: i64, drop_id: i64) {
        if let Err(error) = queries::merge_tracks(self.database.connection(), keep_id, drop_id) {
            self.report(&error);
        }
        self.reload_view();
        self.request(UiRequest::RefreshSidebar);
    }

    /// Position hint used by the player bar; set by `pump_playback`.
    pub fn position_hint(&self) -> i64 {
        self.position_hint
            .unwrap_or_else(|| self.sequencer.position_ms())
    }

    /// Context kind, used by the window title.
    pub fn context_kind(&self) -> ContextKind {
        self.sequencer.context().kind.clone()
    }

    /// Queue as a list for the queue view.
    pub fn queue(&self) -> &Queue {
        self.sequencer.queue()
    }
}

/// One scan, described for the user.
pub fn scan_summary_text(summary: &scanner::ScanSummary) -> String {
    let mut parts = Vec::new();
    if summary.added > 0 {
        parts.push(format!("{} added", summary.added));
    }
    if summary.updated > 0 {
        parts.push(format!("{} updated", summary.updated));
    }
    if summary.moved > 0 {
        parts.push(format!("{} moved", summary.moved));
    }
    if summary.missing > 0 {
        parts.push(format!("{} unavailable", summary.missing));
    }
    if summary.ambiguous > 0 {
        parts.push(format!("{} need review", summary.ambiguous));
    }
    if summary.failed > 0 {
        parts.push(format!("{} couldn't be read", summary.failed));
    }
    if parts.is_empty() {
        return format!("Scan finished. {} unchanged.", summary.unchanged);
    }
    format!("Scan finished: {}.", parts.join(", "))
}

impl ActionTarget for AppCore {
    fn perform(&mut self, action: Action) {
        let selection = self.selected_track_ids();
        match action {
            Action::PlayPause => self.toggle_play(),
            Action::NextTrack => self.next_track(),
            Action::PreviousTrack => self.previous_track(),
            Action::SeekForward => {
                self.seek_by(i64::from(self.settings.playback.seek_seconds) * 1000)
            }
            Action::SeekBackward => {
                self.seek_by(-i64::from(self.settings.playback.seek_seconds) * 1000)
            }
            Action::VolumeUp => self.volume_up(),
            Action::VolumeDown => self.volume_down(),
            Action::Mute => self.toggle_mute(),
            Action::ToggleShuffle => self.toggle_shuffle(),
            Action::CycleRepeat => self.cycle_repeat(),
            Action::FocusSearch => self.request(UiRequest::RefreshView),
            Action::AddToQueue => self.add_to_queue(&selection),
            Action::PlayNext => self.play_next_in_queue(&selection),
            Action::AddToPlaylist => {
                if selection.is_empty() {
                    self.warn("Select at least one track first.");
                } else {
                    self.request(UiRequest::AskToCreatePlaylist);
                }
            }
            Action::ToggleFavorite => self.toggle_favorite(&selection),
            Action::AddLibraryFolder => self.request(UiRequest::AskToChooseFolder),
            Action::RescanLibrary => self.start_scan(ScanKind::AllFolders),
            Action::RescanFolder => {
                if let Some(folder_id) = self.folder_id_for_current_view() {
                    self.start_scan(ScanKind::Folder { folder_id });
                } else {
                    self.notify("Open a folder first to rescan it.");
                }
            }
            Action::OpenSettings => self.request(UiRequest::AskToOpenSettings),
            Action::GoToArtists => self.go_to_section(Section::Artists),
            Action::GoToAlbums => self.go_to_section(Section::Albums),
            Action::GoToTracks => self.go_to_section(Section::Library),
            Action::GoToFolders => self.go_to_section(Section::Folders),
            Action::GoToFavorites => self.go_to_section(Section::Favorites),
            Action::GoToPlaylists => self.go_to_section(Section::Playlists),
            Action::ToggleSidebar => {
                let visible = !self.sidebar_visible;
                self.set_sidebar_visible(visible);
                self.request(UiRequest::RefreshView);
            }
            Action::SelectAll => self.tracks.select_all(),
            Action::ShowTrackInfo => match selection.first() {
                Some(track_id) => self.request(UiRequest::OpenTrackInfo(*track_id)),
                None => self.warn("Select a track first."),
            },
            Action::ShowInFileManager => match selection.first().and_then(|id| self.track(*id)) {
                Some(track) => self.request(UiRequest::ShowInFileManager(track.path)),
                None => self.warn("Select a track first."),
            },
            Action::CopyPath => self.request(UiRequest::Notify {
                message: format!("{} path(s) copied.", selection.len()),
                warning: false,
            }),
            Action::OpenTrackFile => {
                if let Some(track) = selection.first().and_then(|id| self.track(*id)) {
                    self.request(UiRequest::ShowInFileManager(track.path));
                } else {
                    self.warn("Select a track first.");
                }
            }
            Action::ImportPlaylist => self.request(UiRequest::AskToImportPlaylist),
            Action::NewPlaylist => self.request(UiRequest::AskToCreatePlaylist),
            Action::SavePlaylist => {
                if matches!(self.navigation.current(), View::Playlist(_)) {
                    self.request(UiRequest::AskToSavePlaylist);
                } else {
                    self.warn("Open a playlist first.");
                }
            }
            Action::Quit => self.save_session(),
        }
    }
}

impl AppCore {
    /// Folder id for the folder currently open, used by "rescan folder".
    fn folder_id_for_current_view(&self) -> Option<i64> {
        let path = match self.navigation.current() {
            View::Folder(path) => path.clone(),
            _ => return None,
        };
        queries::folders(self.database.connection())
            .ok()?
            .into_iter()
            .find(|folder| folder.path == path)
            .map(|folder| folder.folder_id)
    }
}
