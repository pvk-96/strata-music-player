//! Playback state, queue, shuffle and repeat.
//!
//! Everything in this module is pure logic so it can be tested without an audio device.
//! The GStreamer pipeline lives in [`crate::playback::player`]; this module decides *what*
//! plays next, and remembers enough to restore the previous session.

use std::collections::BTreeMap;

use rand::seq::SliceRandom;
use rusqlite::{params, Connection};

use crate::error::{Error, Result};
use crate::library::model::{SortDirection, TrackQuery, TrackSort};
use crate::library::queries;

/// The row `restore` reads: track, position, shuffle, repeat, volume, mute, and context.
type SavedContext = (Option<i64>, i64, i64, String, f64, i64, String, String);

/// Playing more than this many seconds into a track makes "previous" restart it.
pub const PREVIOUS_RESTART_THRESHOLD_MS: i64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlaybackState {
    #[default]
    Stopped,
    Loading,
    Playing,
    Paused,
    Error,
}

impl PlaybackState {
    /// Allowed transitions. Anything else has to pass through a safe state first.
    pub fn can_transition_to(self, next: PlaybackState) -> bool {
        self == next
            || matches!(
                (self, next),
                (PlaybackState::Stopped, PlaybackState::Loading)
                    | (PlaybackState::Stopped, PlaybackState::Error)
                    | (PlaybackState::Loading, PlaybackState::Playing)
                    | (PlaybackState::Loading, PlaybackState::Error)
                    | (PlaybackState::Loading, PlaybackState::Stopped)
                    | (PlaybackState::Playing, PlaybackState::Paused)
                    | (PlaybackState::Playing, PlaybackState::Stopped)
                    | (PlaybackState::Playing, PlaybackState::Error)
                    | (PlaybackState::Paused, PlaybackState::Playing)
                    | (PlaybackState::Paused, PlaybackState::Stopped)
                    | (PlaybackState::Paused, PlaybackState::Error)
                    | (PlaybackState::Error, PlaybackState::Stopped)
                    | (PlaybackState::Error, PlaybackState::Loading)
            )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn next(self) -> RepeatMode {
        match self {
            RepeatMode::Off => RepeatMode::All,
            RepeatMode::All => RepeatMode::One,
            RepeatMode::One => RepeatMode::Off,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RepeatMode::Off => "off",
            RepeatMode::All => "all",
            RepeatMode::One => "one",
        }
    }

    pub fn parse(text: &str) -> RepeatMode {
        match text {
            "all" => RepeatMode::All,
            "one" => RepeatMode::One,
            _ => RepeatMode::Off,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            RepeatMode::Off => "Repeat Off",
            RepeatMode::All => "Repeat All",
            RepeatMode::One => "Repeat One",
        }
    }
}

/// Where the playing list came from. Kept small on purpose: it is what Next and Previous
/// follow, and it is rebuilt from the library when a session is restored.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ContextKind {
    #[default]
    Library,
    Album,
    Playlist,
    Folder,
    Search,
    Queue,
}

impl ContextKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContextKind::Library => "library",
            ContextKind::Album => "album",
            ContextKind::Playlist => "playlist",
            ContextKind::Folder => "folder",
            ContextKind::Search => "search",
            ContextKind::Queue => "queue",
        }
    }

    pub fn parse(text: &str) -> ContextKind {
        match text {
            "album" => ContextKind::Album,
            "playlist" => ContextKind::Playlist,
            "folder" => ContextKind::Folder,
            "search" => ContextKind::Search,
            "queue" => ContextKind::Queue,
            _ => ContextKind::Library,
        }
    }
}

/// An ordered list plus a label for where it came from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlaybackContext {
    pub kind: ContextKind,
    /// Album "artist\0album", a playlist id, a folder path, a search term, or empty.
    pub key: String,
    pub track_ids: Vec<i64>,
}

impl PlaybackContext {
    pub fn library(track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Library,
            key: String::new(),
            track_ids,
        }
    }

    pub fn playlist(playlist_id: i64, track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Playlist,
            key: playlist_id.to_string(),
            track_ids,
        }
    }

    pub fn album(album_artist: &str, album: &str, track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Album,
            key: format!("{album_artist}\u{1f}{album}"),
            track_ids,
        }
    }

    pub fn folder(path: &std::path::Path, track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Folder,
            key: path.to_string_lossy().into_owned(),
            track_ids,
        }
    }

    pub fn search(term: &str, track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Search,
            key: term.to_string(),
            track_ids,
        }
    }

    pub fn queue(track_ids: Vec<i64>) -> Self {
        Self {
            kind: ContextKind::Queue,
            key: String::new(),
            track_ids,
        }
    }
}

/// What the player should do next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Load and play this track, optionally starting at a position.
    Play { track_id: i64, position_ms: i64 },
    /// Nothing more to play.
    Stop,
    /// Keep the current track and restart it.
    Restart,
}

/// The play queue: a list of tracks to play after the current context. Duplicates are
/// allowed; the order is explicit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Queue {
    entries: Vec<i64>,
}

impl Queue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_ids(ids: &[i64]) -> Self {
        Self {
            entries: ids.to_vec(),
        }
    }

    pub fn ids(&self) -> &[i64] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn add(&mut self, track_id: i64) {
        self.entries.push(track_id);
    }

    pub fn add_all(&mut self, track_ids: &[i64]) {
        self.entries.extend_from_slice(track_ids);
    }

    /// Put tracks directly after the current one.
    pub fn play_next(&mut self, track_ids: &[i64]) {
        for (index, track_id) in track_ids.iter().enumerate() {
            self.entries.insert(index, *track_id);
        }
    }

    pub fn remove(&mut self, index: usize) -> Option<i64> {
        if index < self.entries.len() {
            Some(self.entries.remove(index))
        } else {
            None
        }
    }

    /// Remove every occurrence of a track, for example after it was deleted.
    pub fn remove_track(&mut self, track_id: i64) {
        self.entries.retain(|entry| *entry != track_id);
    }

    pub fn move_to(&mut self, from: usize, to: usize) {
        if from >= self.entries.len() || to >= self.entries.len() || from == to {
            return;
        }
        let entry = self.entries.remove(from);
        self.entries.insert(to, entry);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A shuffle order: one permutation of the context, generated once and kept until shuffle
/// is switched off, the context changes, or the user reshuffles. Positions here are
/// positions in the playing order; the permutation maps them to context indexes.
#[derive(Debug, Clone)]
pub struct ShuffledOrder {
    permutation: Vec<usize>,
}

impl ShuffledOrder {
    pub fn new(length: usize) -> Self {
        let mut permutation: Vec<usize> = (0..length).collect();
        permutation.shuffle(&mut rand::rng());
        Self { permutation }
    }

    pub fn len(&self) -> usize {
        self.permutation.len()
    }

    pub fn is_empty(&self) -> bool {
        self.permutation.is_empty()
    }

    /// Context index playing at `position`.
    pub fn context_index(&self, position: usize) -> Option<usize> {
        self.permutation.get(position).copied()
    }

    /// Playing position of a context index.
    pub fn position_of(&self, context_index: usize) -> Option<usize> {
        self.permutation
            .iter()
            .position(|index| *index == context_index)
    }

    /// True when the order is a permutation of `0..length`, which is what makes a shuffled
    /// context play every track exactly once before repeating.
    pub fn is_permutation_of(&self, length: usize) -> bool {
        let mut seen = vec![false; length];
        for index in &self.permutation {
            match seen.get_mut(*index) {
                Some(slot) if !*slot => *slot = true,
                _ => return false,
            }
        }
        seen.iter().all(|slot| *slot)
    }
}

/// Owns playback order and the bits of state that survive a restart.
#[derive(Debug, Clone)]
pub struct Sequencer {
    context: PlaybackContext,
    /// Position within the playing order: an index into the context, or into the shuffled
    /// order when shuffle is on.
    current: Option<usize>,
    position_ms: i64,
    shuffle: Option<ShuffledOrder>,
    repeat: RepeatMode,
    queue: Queue,
    state: PlaybackState,
}

impl Sequencer {
    pub fn new(context: PlaybackContext) -> Self {
        Self {
            context,
            current: None,
            position_ms: 0,
            shuffle: None,
            repeat: RepeatMode::default(),
            queue: Queue::new(),
            state: PlaybackState::Stopped,
        }
    }

    pub fn context(&self) -> &PlaybackContext {
        &self.context
    }

    pub fn state(&self) -> PlaybackState {
        self.state
    }

    pub fn set_state(&mut self, state: PlaybackState) {
        if self.state.can_transition_to(state) {
            self.state = state;
        } else {
            log::debug!("ignoring playback transition {:?} -> {state:?}", self.state);
        }
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn queue_mut(&mut self) -> &mut Queue {
        &mut self.queue
    }

    pub fn shuffle_enabled(&self) -> bool {
        self.shuffle.is_some()
    }

    pub fn position_ms(&self) -> i64 {
        self.position_ms
    }

    pub fn set_position_ms(&mut self, position_ms: i64) {
        self.position_ms = position_ms.max(0);
    }

    /// Length of the playing order.
    pub fn order_len(&self) -> usize {
        self.context.track_ids.len()
    }

    /// Track playing at `position` of the playing order.
    fn track_at(&self, position: usize) -> Option<i64> {
        let context_index = match &self.shuffle {
            Some(order) => order.context_index(position)?,
            None => position,
        };
        self.context.track_ids.get(context_index).copied()
    }

    /// Playing position of a track, respecting the shuffle order.
    fn position_of(&self, track_id: i64) -> Option<usize> {
        match &self.shuffle {
            Some(_) => {
                (0..self.order_len()).find(|position| self.track_at(*position) == Some(track_id))
            }
            None => self.context.track_ids.iter().position(|id| *id == track_id),
        }
    }

    /// Track that is playing, if any.
    pub fn current_track_id(&self) -> Option<i64> {
        self.track_at(self.current?)
    }

    /// Start playing `track_id`, which must be part of the current context.
    pub fn play(&mut self, track_id: i64) -> Option<Step> {
        let position = self.position_of(track_id)?;
        self.current = Some(position);
        self.position_ms = 0;
        Some(Step::Play {
            track_id,
            position_ms: 0,
        })
    }

    /// Replace the context, for example when the user plays something else. Shuffle order is
    /// regenerated because it refers to positions in the old list.
    pub fn set_context(&mut self, context: PlaybackContext) {
        let shuffle_on = self.shuffle.is_some();
        let repeat = self.repeat;
        self.context = context;
        self.current = None;
        self.position_ms = 0;
        self.shuffle = shuffle_on.then(|| ShuffledOrder::new(self.context.track_ids.len()));
        self.repeat = repeat;
    }

    /// Switch shuffle on or off. The track that is playing stays the track that is playing,
    /// so its position is re-mapped onto the new order.
    pub fn toggle_shuffle(&mut self) {
        let playing = self.current_track_id();
        match self.shuffle.take() {
            Some(_) => {}
            None => self.shuffle = Some(ShuffledOrder::new(self.order_len())),
        }
        if let Some(track_id) = playing {
            self.current = self.position_of(track_id).or(self.current);
        }
    }

    /// Generate a fresh shuffle order without changing whether shuffle is on. Playback
    /// continues with whatever is playing now.
    pub fn reshuffle(&mut self) {
        if self.shuffle.is_none() {
            return;
        }
        let playing = self.current_track_id();
        self.shuffle = Some(ShuffledOrder::new(self.order_len()));
        if let Some(track_id) = playing {
            if let Some(position) = self.position_of(track_id) {
                self.current = Some(position);
            }
        }
    }

    pub fn cycle_repeat(&mut self) -> RepeatMode {
        self.repeat = self.repeat.next();
        self.repeat
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Step {
        if self.repeat == RepeatMode::One && self.current.is_some() {
            self.position_ms = 0;
            return Step::Restart;
        }

        let length = self.order_len();
        if length == 0 {
            return Step::Stop;
        }

        let next_position = match self.current {
            Some(current) => current + 1,
            None => 0,
        };

        if next_position >= length {
            if self.repeat == RepeatMode::All {
                self.current = Some(0);
                return self.step_for(0);
            }
            return Step::Stop;
        }

        self.current = Some(next_position);
        self.step_for(next_position)
    }

    /// Previous behaves like every other player: past a few seconds in, restart the track.
    pub fn previous(&mut self) -> Step {
        if self.position_ms > PREVIOUS_RESTART_THRESHOLD_MS {
            self.position_ms = 0;
            return Step::Restart;
        }

        let length = self.order_len();
        if length == 0 {
            return Step::Stop;
        }

        match self.current {
            Some(0) => {
                // At the start of the context: stay put, or wrap when repeating all.
                if self.repeat == RepeatMode::All {
                    self.current = Some(length - 1);
                    return self.step_for(length - 1);
                }
                Step::Stop
            }
            Some(current) => {
                let previous = current - 1;
                self.current = Some(previous);
                self.step_for(previous)
            }
            None => Step::Stop,
        }
    }

    /// The track reached by end of stream, without advancing.
    pub fn on_end_of_stream(&mut self) -> Step {
        self.next()
    }

    fn step_for(&mut self, order_position: usize) -> Step {
        self.position_ms = 0;
        match self.track_at(order_position) {
            Some(track_id) => Step::Play {
                track_id,
                position_ms: 0,
            },
            None => Step::Stop,
        }
    }

    /// Save the session state so the next start can restore it.
    pub fn save(&self, connection: &Connection, volume: f64, muted: bool) -> Result<()> {
        let transaction = connection
            .unchecked_transaction()
            .map_err(|err| Error::Database(err.to_string()))?;

        transaction
            .execute("DELETE FROM queue_entries", [])
            .map_err(|err| Error::Database(err.to_string()))?;
        for (position, track_id) in self.queue.ids().iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO queue_entries (position, track_id) VALUES (?1, ?2)",
                    params![position as i64, track_id],
                )
                .map_err(|err| Error::Database(err.to_string()))?;
        }

        transaction
            .execute(
                "INSERT OR REPLACE INTO playback_state
                    (id, track_id, position_ms, shuffle, repeat_mode, volume, muted, context_kind, context_key)
                 VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    self.current_track_id(),
                    self.position_ms,
                    i64::from(self.shuffle.is_some()),
                    self.repeat.as_str(),
                    volume,
                    i64::from(muted),
                    self.context.kind.as_str(),
                    self.context.key,
                ],
            )
            .map_err(|err| Error::Database(err.to_string()))?;

        transaction
            .commit()
            .map_err(|err| Error::Database(err.to_string()))?;
        Ok(())
    }

    /// Rebuild the context for a saved session.
    pub fn restore(connection: &Connection) -> Result<Option<(Sequencer, f64, bool)>> {
        let saved: Option<SavedContext> = connection
            .query_row(
                "SELECT track_id, position_ms, shuffle, repeat_mode, volume, muted, context_kind, context_key
                 FROM playback_state WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, Option<i64>>(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .ok();

        let Some((track_id, position_ms, shuffle, repeat, volume, muted, kind, key)) = saved else {
            return Ok(None);
        };

        let mut queue = Queue::new();
        {
            let mut statement = connection
                .prepare("SELECT track_id FROM queue_entries ORDER BY position")
                .map_err(|err| Error::Database(err.to_string()))?;
            let rows = statement
                .query_map([], |row| row.get::<_, i64>(0))
                .map_err(|err| Error::Database(err.to_string()))?;
            for row in rows {
                queue.add(row.map_err(|err| Error::Database(err.to_string()))?);
            }
        }

        let kind = ContextKind::parse(&kind);
        let track_ids = context_track_ids(connection, &kind, &key, queue.ids())?;
        let mut sequencer = Sequencer::new(PlaybackContext {
            kind,
            key,
            track_ids,
        });
        sequencer.position_ms = position_ms.max(0);
        sequencer.repeat = RepeatMode::parse(&repeat);
        sequencer.queue = queue;

        if shuffle != 0 {
            sequencer.shuffle = Some(ShuffledOrder::new(sequencer.context.track_ids.len()));
        }

        if let Some(track_id) = track_id {
            // A saved track outside the rebuilt context is appended so playback can resume.
            if !sequencer.context.track_ids.contains(&track_id) {
                sequencer.context.track_ids.push(track_id);
            }
            sequencer.current = sequencer.position_of(track_id);
        }

        Ok(Some((sequencer, volume, muted != 0)))
    }
}

/// Rebuild the track list of a saved context from the library.
fn context_track_ids(
    connection: &Connection,
    kind: &ContextKind,
    key: &str,
    queue_ids: &[i64],
) -> Result<Vec<i64>> {
    match kind {
        ContextKind::Library => Ok(queries::query_tracks(
            connection,
            &TrackQuery::default(),
            TrackSort::DateAdded,
            SortDirection::Ascending,
        )?
        .into_iter()
        .map(|track| track.id)
        .collect()),
        ContextKind::Album => {
            let (album_artist, album) = key.split_once('\u{1f}').unwrap_or((key, ""));
            Ok(queries::query_tracks(
                connection,
                &TrackQuery {
                    album_artist: Some(album_artist.to_string()),
                    album: Some(album.to_string()),
                    ..Default::default()
                },
                TrackSort::TrackNumber,
                SortDirection::Ascending,
            )?
            .into_iter()
            .map(|track| track.id)
            .collect())
        }
        ContextKind::Playlist => {
            let playlist_id: i64 = key.parse().unwrap_or_default();
            Ok(crate::playlists::queries::entry_track_ids(
                connection,
                playlist_id,
            )?)
        }
        ContextKind::Folder => Ok(queries::query_tracks(
            connection,
            &TrackQuery {
                folder_path: Some(std::path::PathBuf::from(key)),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )?
        .into_iter()
        .map(|track| track.id)
        .collect()),
        ContextKind::Search => Ok(queries::query_tracks(
            connection,
            &TrackQuery {
                search: Some(key.to_string()),
                ..Default::default()
            },
            TrackSort::Title,
            SortDirection::Ascending,
        )?
        .into_iter()
        .map(|track| track.id)
        .collect()),
        ContextKind::Queue => Ok(queue_ids.to_vec()),
    }
}

/// Convenience view used by the player bar and the window title.
pub struct NowPlayingSummary {
    pub title: String,
    pub artist: String,
}

pub fn now_playing(connection: &Connection, track_id: i64) -> Result<Option<NowPlayingSummary>> {
    match queries::get_track(connection, track_id) {
        Ok(track) => Ok(Some(NowPlayingSummary {
            title: track.display_title(),
            artist: track.display_artist().to_string(),
        })),
        Err(Error::Database(_)) => Ok(None),
        Err(error) => Err(error),
    }
}

/// Keep `serde` available for the persisted shape used by tests and tooling.
pub type SavedBindings = BTreeMap<String, String>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;
    use crate::library::model::Track;

    fn ids(count: i64) -> Vec<i64> {
        (1..=count).collect()
    }

    fn sequencer(count: i64) -> Sequencer {
        Sequencer::new(PlaybackContext::library(ids(count)))
    }

    fn played(sequence: &mut Sequencer) -> Vec<i64> {
        match sequence.next() {
            Step::Play { track_id, .. } => vec![track_id],
            Step::Stop => Vec::new(),
            Step::Restart => Vec::new(),
        }
    }

    #[test]
    fn next_walks_the_context_in_order() {
        let mut sequence = sequencer(3);
        assert_eq!(played(&mut sequence), vec![1]);
        assert_eq!(played(&mut sequence), vec![2]);
        assert_eq!(played(&mut sequence), vec![3]);
    }

    #[test]
    fn next_stops_at_the_end_of_the_context() {
        let mut sequence = sequencer(2);
        played(&mut sequence);
        played(&mut sequence);
        assert_eq!(sequence.next(), Step::Stop);
    }

    #[test]
    fn repeat_all_wraps_around() {
        let mut sequence = sequencer(2);
        sequence.cycle_repeat();
        assert_eq!(sequence.repeat(), RepeatMode::All);
        played(&mut sequence);
        played(&mut sequence);
        assert_eq!(played(&mut sequence), vec![1]);
    }

    #[test]
    fn repeat_one_restarts_the_current_track() {
        let mut sequence = sequencer(3);
        played(&mut sequence);
        sequence.cycle_repeat();
        sequence.cycle_repeat();
        assert_eq!(sequence.repeat(), RepeatMode::One);
        assert_eq!(sequence.next(), Step::Restart);
        assert_eq!(sequence.current_track_id(), Some(1));
    }

    #[test]
    fn repeat_cycles_through_all_modes() {
        assert_eq!(RepeatMode::Off.next(), RepeatMode::All);
        assert_eq!(RepeatMode::All.next(), RepeatMode::One);
        assert_eq!(RepeatMode::One.next(), RepeatMode::Off);
    }

    #[test]
    fn previous_restarts_the_track_after_three_seconds() {
        let mut sequence = sequencer(3);
        played(&mut sequence);
        sequence.set_position_ms(4_000);
        assert_eq!(sequence.previous(), Step::Restart);
        assert_eq!(sequence.current_track_id(), Some(1));
        assert_eq!(sequence.position_ms(), 0);
    }

    #[test]
    fn previous_uses_the_previous_track_within_three_seconds() {
        let mut sequence = sequencer(3);
        played(&mut sequence);
        played(&mut sequence);
        sequence.set_position_ms(2_999);
        assert_eq!(
            sequence.previous(),
            Step::Play {
                track_id: 1,
                position_ms: 0
            }
        );
    }

    #[test]
    fn previous_stays_at_the_start_of_the_context() {
        let mut sequence = sequencer(3);
        played(&mut sequence);
        assert_eq!(sequence.previous(), Step::Stop);
        assert_eq!(sequence.current_track_id(), Some(1));
    }

    #[test]
    fn previous_wraps_when_repeating_all() {
        let mut sequence = sequencer(3);
        sequence.cycle_repeat();
        played(&mut sequence);
        assert_eq!(
            sequence.previous(),
            Step::Play {
                track_id: 3,
                position_ms: 0
            }
        );
    }

    #[test]
    fn playing_a_track_sets_the_position_in_the_context() {
        let mut sequence = sequencer(5);
        sequence.play(3);
        assert_eq!(sequence.current_track_id(), Some(3));
        assert_eq!(
            sequence.next(),
            Step::Play {
                track_id: 4,
                position_ms: 0
            }
        );
    }

    #[test]
    fn playing_a_track_outside_the_context_is_ignored() {
        let mut sequence = sequencer(3);
        assert_eq!(sequence.play(99), None);
        assert_eq!(sequence.current_track_id(), None);
    }

    #[test]
    fn shuffle_visits_every_track_once_before_repeating() {
        let mut sequence = sequencer(50);
        sequence.toggle_shuffle();
        assert!(sequence.shuffle_enabled());

        let mut ids: Vec<i64> = Vec::new();
        for _ in 0..50 {
            match sequence.next() {
                Step::Play { track_id, .. } => ids.push(track_id),
                other => panic!("expected a track, got {other:?}"),
            }
        }

        ids.sort_unstable();
        ids.dedup();
        assert_eq!(
            ids.len(),
            50,
            "no track repeats before the order is exhausted"
        );
    }

    #[test]
    fn a_shuffled_order_is_a_permutation() {
        let order = ShuffledOrder::new(30);
        assert!(order.is_permutation_of(30));
        assert_eq!(order.len(), 30);
        assert!(order.position_of(7).is_some());
    }

    #[test]
    fn shuffle_order_is_generated_once_and_kept() {
        let mut sequence = sequencer(20);
        sequence.toggle_shuffle();
        let first: Vec<Step> = (0..10).map(|_| sequence.next()).collect();

        // Advancing further continues the same order rather than shuffling again.
        let following: Vec<Step> = (0..10).map(|_| sequence.next()).collect();
        assert_eq!(first.len(), 10);
        assert_eq!(following.len(), 10);
        let mut all: Vec<i64> = first
            .iter()
            .chain(following.iter())
            .filter_map(|step| match step {
                Step::Play { track_id, .. } => Some(*track_id),
                _ => None,
            })
            .collect();
        assert_eq!(all.len(), 20, "twenty different tracks, no Stop in between");
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 20);
    }

    #[test]
    fn reshuffle_keeps_the_current_track() {
        let mut sequence = sequencer(20);
        sequence.toggle_shuffle();
        sequence.next();
        sequence.next();
        let before = sequence.current_track_id();
        sequence.reshuffle();
        assert_eq!(sequence.current_track_id(), before);
    }

    #[test]
    fn shuffle_survives_context_changes_by_reshuffling() {
        let mut sequence = sequencer(5);
        sequence.toggle_shuffle();
        sequence.play(2);
        sequence.set_context(PlaybackContext::library(ids(7)));
        assert!(sequence.shuffle_enabled());
        assert_eq!(sequence.current_track_id(), None);
    }

    #[test]
    fn disabling_shuffle_keeps_the_current_track() {
        // Which track the shuffled order starts on is random, so this covers every case by
        // choosing the track that sits where the test needs it.
        let mut sequence = sequencer(10);
        sequence.toggle_shuffle();
        let shuffled_first = match sequence.next() {
            Step::Play { track_id, .. } => track_id,
            other => panic!("expected a track, got {other:?}"),
        };
        sequence.toggle_shuffle();
        assert!(!sequence.shuffle_enabled());
        assert_eq!(sequence.current_track_id(), Some(shuffled_first));

        let ids = sequence.context().track_ids.clone();
        let index = ids.iter().position(|id| *id == shuffled_first).unwrap();
        let expected = match ids.get(index + 1) {
            Some(track_id) => Step::Play {
                track_id: *track_id,
                position_ms: 0,
            },
            // The last track of the context ends playback unless repeating everything.
            None => Step::Stop,
        };
        assert_eq!(sequence.next(), expected);
    }

    #[test]
    fn end_of_stream_advances_like_next() {
        let mut sequence = sequencer(3);
        played(&mut sequence);
        assert_eq!(
            sequence.on_end_of_stream(),
            Step::Play {
                track_id: 2,
                position_ms: 0
            }
        );
    }

    #[test]
    fn queue_adds_in_order() {
        let mut queue = Queue::new();
        queue.add(1);
        queue.add(2);
        queue.add(1);
        assert_eq!(queue.ids(), &[1, 2, 1], "duplicates are kept");
    }

    #[test]
    fn queue_play_next_puts_tracks_first() {
        let mut queue = Queue::from_ids(&[1, 2]);
        queue.play_next(&[9, 8]);
        assert_eq!(queue.ids(), &[9, 8, 1, 2]);
    }

    #[test]
    fn queue_remove_and_reorder() {
        let mut queue = Queue::from_ids(&[1, 2, 3, 4]);
        assert_eq!(queue.remove(1), Some(2));
        assert_eq!(queue.remove(99), None);
        queue.move_to(2, 0);
        assert_eq!(queue.ids(), &[4, 1, 3]);
        queue.move_to(50, 0);
        assert_eq!(queue.ids(), &[4, 1, 3], "out of range moves are ignored");
    }

    #[test]
    fn queue_clear_and_remove_track() {
        let mut queue = Queue::from_ids(&[1, 2, 1]);
        queue.remove_track(1);
        assert_eq!(queue.ids(), &[2]);
        queue.clear();
        assert!(queue.is_empty());
    }

    #[test]
    fn state_machine_rejects_impossible_transitions() {
        assert!(PlaybackState::Stopped.can_transition_to(PlaybackState::Loading));
        assert!(PlaybackState::Loading.can_transition_to(PlaybackState::Playing));
        assert!(PlaybackState::Playing.can_transition_to(PlaybackState::Paused));
        assert!(PlaybackState::Paused.can_transition_to(PlaybackState::Playing));
        assert!(!PlaybackState::Stopped.can_transition_to(PlaybackState::Playing));
        assert!(!PlaybackState::Stopped.can_transition_to(PlaybackState::Paused));
    }

    #[test]
    fn set_state_ignores_invalid_transitions() {
        let mut sequence = sequencer(1);
        sequence.set_state(PlaybackState::Playing);
        assert_eq!(
            sequence.state(),
            PlaybackState::Stopped,
            "can't skip loading"
        );
        sequence.set_state(PlaybackState::Loading);
        sequence.set_state(PlaybackState::Playing);
        sequence.set_state(PlaybackState::Paused);
        assert_eq!(sequence.state(), PlaybackState::Paused);
    }

    #[test]
    fn state_moves_from_error_back_to_stopped() {
        assert!(PlaybackState::Error.can_transition_to(PlaybackState::Stopped));
        assert!(PlaybackState::Error.can_transition_to(PlaybackState::Loading));
    }

    fn database_with_tracks(count: i64) -> Database {
        let database = Database::open_in_memory().unwrap();
        for id in 1..=count {
            let mut track = Track {
                id,
                path: format!("/m/{id}.mp3").into(),
                filename: format!("{id}.mp3"),
                folder_id: None,
                file_size: 100,
                modified_time: 0,
                missing: false,
                title: format!("Track {id}"),
                artist: "Artist".into(),
                album: "Album".into(),
                album_artist: "Artist".into(),
                genre: None,
                year: None,
                track_number: Some(id),
                disc_number: None,
                duration_ms: Some(180_000),
                format: None,
                sample_rate: None,
                channels: None,
                bit_depth: None,
                bitrate: None,
                favorite: false,
                artwork_reference: None,
                ambiguous: false,
                created_at: id,
                updated_at: id,
            };
            track.id = 0;
            queries::insert_track(database.connection(), &track).unwrap();
        }
        database
    }

    #[test]
    fn session_is_saved_and_restored() {
        let database = database_with_tracks(5);
        let mut sequence = sequencer(5);
        sequence.play(3);
        sequence.set_position_ms(42_000);
        sequence.cycle_repeat();
        sequence.queue_mut().add(4);
        sequence.queue_mut().add(5);
        sequence.save(database.connection(), 0.8, true).unwrap();

        let (restored, volume, muted) = Sequencer::restore(database.connection()).unwrap().unwrap();
        assert_eq!(restored.current_track_id(), Some(3));
        assert_eq!(restored.position_ms(), 42_000);
        assert_eq!(restored.repeat(), RepeatMode::All);
        assert_eq!(restored.queue().ids(), &[4, 5]);
        assert!((volume - 0.8).abs() < f64::EPSILON);
        assert!(muted);
    }

    #[test]
    fn restore_without_saved_state_returns_nothing() {
        let database = Database::open_in_memory().unwrap();
        assert!(Sequencer::restore(database.connection()).unwrap().is_none());
    }

    #[test]
    fn shuffle_state_survives_a_restart() {
        let database = database_with_tracks(5);
        let mut sequence = sequencer(5);
        sequence.toggle_shuffle();
        sequence.play(2);
        sequence.save(database.connection(), 1.0, false).unwrap();

        let (restored, _, _) = Sequencer::restore(database.connection()).unwrap().unwrap();
        assert!(restored.shuffle_enabled());
        assert_eq!(restored.current_track_id(), Some(2));
    }

    #[test]
    fn a_deleted_playing_track_does_not_break_restore() {
        let database = database_with_tracks(3);
        let mut sequence = Sequencer::new(PlaybackContext::library(vec![1, 2, 3]));
        sequence.play(2);
        sequence.save(database.connection(), 1.0, false).unwrap();

        // The row disappears, as it would if the track were removed from the library.
        database
            .connection()
            .execute("DELETE FROM tracks WHERE id = 2", [])
            .unwrap();

        let (restored, _, _) = Sequencer::restore(database.connection()).unwrap().unwrap();
        assert_eq!(restored.current_track_id(), None, "nothing to resume");
        assert_eq!(restored.context().track_ids.len(), 2);
    }

    #[test]
    fn album_context_is_rebuilt_from_the_library() {
        let database = database_with_tracks(3);
        let mut sequence = Sequencer::new(PlaybackContext::album("Artist", "Album", Vec::new()));
        sequence.context.track_ids = vec![1, 2, 3];
        sequence.current = Some(0);
        sequence.save(database.connection(), 1.0, false).unwrap();

        let (restored, _, _) = Sequencer::restore(database.connection()).unwrap().unwrap();
        assert_eq!(restored.context().kind, ContextKind::Album);
        assert_eq!(restored.context().track_ids.len(), 3);
        assert_eq!(restored.current_track_id(), Some(1));
    }

    #[test]
    fn repeat_mode_round_trips_through_text() {
        for mode in [RepeatMode::Off, RepeatMode::All, RepeatMode::One] {
            assert_eq!(RepeatMode::parse(mode.as_str()), mode);
        }
        assert_eq!(RepeatMode::parse("nonsense"), RepeatMode::Off);
    }

    #[test]
    fn context_kinds_round_trip_through_text() {
        for kind in [
            ContextKind::Library,
            ContextKind::Album,
            ContextKind::Playlist,
            ContextKind::Folder,
            ContextKind::Search,
            ContextKind::Queue,
        ] {
            assert_eq!(ContextKind::parse(kind.as_str()), kind);
        }
    }
}
