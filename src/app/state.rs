//! View state: what each screen currently shows.
//!
//! This is the part of the UI that can be tested without a display: filters, sort order,
//! selection, and the formatting of durations and file sizes.

use std::path::PathBuf;

use crate::library::model::{SortDirection, Track, TrackQuery, TrackSort};
use crate::playlists::model::PlaylistEntry;

/// A row in any track list. Kept separate from `Track` so the queue can hold entries whose
/// file is gone.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackRow {
    pub track_id: i64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_ms: Option<i64>,
    pub filename: String,
    pub path: PathBuf,
    pub favorite: bool,
    pub missing: bool,
    /// Disc and track numbers, formatted for the list.
    pub track_number_label: String,
}

impl TrackRow {
    pub fn from_track(track: &Track) -> Self {
        Self {
            track_id: track.id,
            title: track.display_title(),
            artist: track.display_artist().to_string(),
            album: track.display_album().to_string(),
            duration_ms: track.duration_ms,
            filename: track.filename.clone(),
            path: track.path.clone(),
            favorite: track.favorite,
            missing: track.missing,
            track_number_label: number_label(track.disc_number, track.track_number),
        }
    }

    pub fn from_rows(tracks: &[Track]) -> Vec<TrackRow> {
        tracks.iter().map(TrackRow::from_track).collect()
    }
}

/// A playlist entry shown in a list, playable or not.
#[derive(Debug, Clone, PartialEq)]
pub struct EntryRow {
    pub entry_id: i64,
    pub track_id: Option<i64>,
    pub label: String,
    pub artist: String,
    pub availability_label: String,
    pub playable: bool,
    pub duration_ms: Option<i64>,
}

impl EntryRow {
    pub fn from_entry(entry: &PlaylistEntry) -> Self {
        Self {
            entry_id: entry.id,
            track_id: entry.track_id,
            label: entry.display_label.clone(),
            artist: String::new(),
            availability_label: entry.availability.label().to_string(),
            playable: entry.is_playable(),
            duration_ms: entry.duration_ms,
        }
    }
}

/// Format a duration the way a player does: `m:ss`, or `h:mm:ss` when needed.
pub fn format_duration(ms: Option<i64>) -> String {
    let Some(ms) = ms.filter(|value| *value > 0) else {
        return "--:--".to_string();
    };
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    if hours > 0 {
        format!("{hours}:{minutes:02}:{seconds:02}")
    } else {
        format!("{minutes}:{seconds:02}")
    }
}

/// Longer form used for totals: `2 hr 5 min`, `45 min`, `12 sec`.
pub fn format_total(ms: Option<i64>) -> String {
    let Some(ms) = ms.filter(|value| *value > 0) else {
        return "unknown length".to_string();
    };
    let total_seconds = ms / 1000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    match (hours, minutes) {
        (0, 0) => format!("{total_seconds} sec"),
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} hr"),
        (hours, minutes) => format!("{hours} hr {minutes} min"),
    }
}

/// `1-5` style labels, empty when there is nothing to show.
pub fn number_label(disc: Option<i64>, track: Option<i64>) -> String {
    match (disc, track) {
        (Some(disc), Some(track)) => format!("{disc}-{track}"),
        (None, Some(track)) => track.to_string(),
        _ => String::new(),
    }
}

/// File size for the track information dialog.
pub fn format_size(bytes: i64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Bitrate for the track information dialog, in kbit/s.
pub fn format_bitrate(bitrate: Option<i64>) -> String {
    match bitrate {
        Some(bitrate) if bitrate > 0 => format!("{:.0} kbit/s", bitrate as f64 / 1000.0),
        _ => "unknown".to_string(),
    }
}

/// Sampling rate and depth, e.g. `44.1 kHz, 16 bit`.
pub fn format_audio_properties(sample_rate: Option<i64>, bit_depth: Option<i64>) -> String {
    match (sample_rate, bit_depth) {
        (Some(rate), Some(depth)) if rate > 0 && depth > 0 => {
            format!("{:.1} kHz, {depth} bit", rate as f64 / 1000.0)
        }
        (Some(rate), _) if rate > 0 => format!("{:.1} kHz", rate as f64 / 1000.0),
        _ => "unknown".to_string(),
    }
}

/// Channel count: `Stereo`, `Mono`, `5.1`.
pub fn format_channels(channels: Option<i64>) -> String {
    match channels {
        Some(1) => "Mono".to_string(),
        Some(2) => "Stereo".to_string(),
        Some(6) => "5.1".to_string(),
        Some(8) => "7.1".to_string(),
        Some(count) if count > 0 => format!("{count} channels"),
        _ => "unknown".to_string(),
    }
}

/// The lines shown in the track information dialog.
pub fn track_information(track: &Track) -> String {
    let mut lines = Vec::new();
    lines.push(track.title.clone());
    if !track.artist.is_empty() {
        lines.push(track.artist.clone());
    }
    if !track.album.is_empty() {
        lines.push(track.album.clone());
    }
    lines.push(String::new());
    lines.push(format!("File: {}", track.path.display()));
    lines.push(format!("Size: {}", format_size(track.file_size)));
    lines.push(format!(
        "Format: {}",
        track
            .format
            .clone()
            .unwrap_or_else(|| "unknown".to_string())
    ));
    lines.push(format!(
        "Audio: {}, {}",
        format_audio_properties(track.sample_rate, track.bit_depth),
        format_channels(track.channels)
    ));
    lines.push(format!("Bitrate: {}", format_bitrate(track.bitrate)));
    lines.push(format!("Length: {}", format_duration(track.duration_ms)));
    if let Some(genre) = &track.genre {
        lines.push(format!("Genre: {genre}"));
    }
    lines.push(format!("Track id: {}", track.id));
    if track.missing {
        lines.push(String::new());
        lines.push("This file is missing from disk.".to_string());
    }
    lines.join("\n")
}

/// Which tracks the current view shows, given the navigation state and library filters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackListState {
    pub query: TrackQuery,
    pub sort: TrackSort,
    pub direction: SortDirection,
    /// Track ids in the order shown, after sorting.
    pub rows: Vec<TrackRow>,
    pub selected: Vec<usize>,
    pub scroll_to_row: Option<usize>,
    /// Row the current selection grew from. Shift-click keeps extending from here.
    pub anchor: Option<usize>,
}

impl TrackListState {
    pub fn new(query: TrackQuery, sort: TrackSort) -> Self {
        Self {
            query,
            sort,
            direction: SortDirection::Ascending,
            rows: Vec::new(),
            selected: Vec::new(),
            scroll_to_row: None,
            anchor: None,
        }
    }

    pub fn set_tracks(&mut self, tracks: &[Track]) {
        self.rows = TrackRow::from_rows(tracks);
        self.selected.clear();
        self.anchor = None;
        self.scroll_to_row = None;
    }

    pub fn set_rows(&mut self, rows: Vec<TrackRow>) {
        self.rows = rows;
        self.selected.clear();
        self.anchor = None;
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Change sort column, flipping the direction when the same column is chosen again.
    pub fn sort_by(&mut self, column: TrackSort) {
        if self.sort == column {
            self.direction = self.direction.toggled();
        } else {
            self.sort = column;
            self.direction = SortDirection::Ascending;
        }
    }

    pub fn sort_indicator(&self, column: TrackSort) -> Option<SortDirection> {
        (self.sort == column).then_some(self.direction)
    }

    pub fn total_duration_ms(&self) -> Option<i64> {
        self.rows
            .iter()
            .try_fold(0i64, |total, row| row.duration_ms.map(|ms| total + ms))
    }

    pub fn row(&self, index: usize) -> Option<&TrackRow> {
        self.rows.get(index)
    }

    pub fn selected_track_ids(&self) -> Vec<i64> {
        self.selected
            .iter()
            .filter_map(|index| self.rows.get(*index))
            .map(|row| row.track_id)
            .collect()
    }

    /// Playable tracks in display order: missing files are skipped.
    pub fn playable_track_ids(&self) -> Vec<i64> {
        self.rows
            .iter()
            .filter(|row| !row.missing)
            .map(|row| row.track_id)
            .collect()
    }

    pub fn select_only(&mut self, index: usize) {
        if index < self.rows.len() {
            self.selected = vec![index];
            self.anchor = Some(index);
        }
    }

    pub fn select_all(&mut self) {
        self.selected = (0..self.rows.len()).collect();
        self.anchor = None;
    }

    /// Move the selection to the next row, wrapping at the end.
    pub fn select_next(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let next = self
            .selected
            .last()
            .map_or(0, |index| (index + 1) % self.rows.len());
        self.selected = vec![next];
        self.anchor = Some(next);
        self.scroll_to_row = Some(next);
    }

    pub fn select_previous(&mut self) {
        if self.rows.is_empty() {
            return;
        }
        let previous = self.selected.first().map_or(self.rows.len() - 1, |index| {
            if *index == 0 {
                self.rows.len() - 1
            } else {
                index - 1
            }
        });
        self.selected = vec![previous];
        self.anchor = Some(previous);
        self.scroll_to_row = Some(previous);
    }

    /// Select a row. With `extend`, the selection grows from the anchor set by the last
    /// plain click, so dragging back and forth keeps the same base row.
    pub fn select_row(&mut self, index: usize, extend: bool) {
        if index >= self.rows.len() {
            return;
        }
        if extend {
            if let Some(anchor) = self.anchor {
                let (start, end) = if anchor <= index {
                    (anchor, index)
                } else {
                    (index, anchor)
                };
                self.selected = (start..=end).collect();
                return;
            }
        }
        self.selected = vec![index];
        self.anchor = Some(index);
    }

    pub fn has_selection(&self) -> bool {
        !self.selected.is_empty()
    }

    /// Clear the one-shot scroll request.
    pub fn take_scroll_target(&mut self) -> Option<usize> {
        self.scroll_to_row.take()
    }

    /// Text shown in the footer.
    pub fn summary(&self) -> String {
        let count = self.rows.len();
        let missing = self.rows.iter().filter(|row| row.missing).count();
        let tracks = if count == 1 {
            "1 track"
        } else {
            &format!("{count} tracks")
        };
        let duration = format_total(self.total_duration_ms());
        if missing == 0 {
            format!("{tracks} · {duration}")
        } else {
            format!("{tracks} · {duration} · {missing} unavailable")
        }
    }
}

/// State of the search field.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchState {
    pub term: String,
    /// The term that produced the results currently shown.
    pub applied: String,
    pub case_sensitive: bool,
    pub in_artist: bool,
    pub in_album: bool,
    pub in_title: bool,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            case_sensitive: false,
            in_artist: true,
            in_album: true,
            in_title: true,
            ..Default::default()
        }
    }

    pub fn is_active(&self) -> bool {
        !self.term.trim().is_empty()
    }

    /// Field is only enabled when at least one field is selected.
    pub fn any_field_selected(&self) -> bool {
        self.in_title || self.in_artist || self.in_album
    }

    /// Apply the current term, returning the trimmed text that was used.
    pub fn apply(&mut self) -> String {
        let term = self.term.trim().to_string();
        self.applied = term.clone();
        term
    }

    pub fn clear(&mut self) {
        self.term.clear();
        self.applied.clear();
    }

    /// Results are stale when the field differs from what is shown.
    pub fn needs_refresh(&self) -> bool {
        self.term.trim() != self.applied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::model::Track;

    fn track(id: i64, title: &str, artist: &str, album: &str, duration: Option<i64>) -> Track {
        Track {
            id,
            path: format!("/m/{id}.mp3").into(),
            filename: format!("{id}.mp3"),
            folder_id: None,
            file_size: 1024,
            modified_time: 0,
            missing: false,
            title: title.to_string(),
            artist: artist.to_string(),
            album: album.to_string(),
            album_artist: "Various".to_string(),
            genre: None,
            year: None,
            track_number: Some(id),
            disc_number: None,
            duration_ms: duration,
            format: Some("MP3".into()),
            sample_rate: Some(44_100),
            channels: Some(2),
            bit_depth: None,
            bitrate: Some(192_000),
            favorite: false,
            artwork_reference: None,
            ambiguous: false,
            created_at: id,
            updated_at: id,
        }
    }

    fn list(count: i64) -> TrackListState {
        let tracks: Vec<Track> = (1..=count)
            .map(|id| track(id, &format!("Track {id}"), "Artist", "Album", Some(180_000)))
            .collect();
        let mut state = TrackListState::new(TrackQuery::default(), TrackSort::Title);
        state.set_tracks(&tracks);
        state
    }

    #[test]
    fn durations_are_formatted_for_players() {
        assert_eq!(format_duration(Some(0)), "--:--");
        assert_eq!(format_duration(None), "--:--");
        assert_eq!(format_duration(Some(1_000)), "0:01");
        assert_eq!(format_duration(Some(65_000)), "1:05");
        assert_eq!(format_duration(Some(600_000)), "10:00");
        assert_eq!(format_duration(Some(3_725_000)), "1:02:05");
    }

    #[test]
    fn totals_are_formatted_in_words() {
        assert_eq!(format_total(None), "unknown length");
        assert_eq!(format_total(Some(12_000)), "12 sec");
        assert_eq!(format_total(Some(45 * 60_000)), "45 min");
        assert_eq!(format_total(Some(2 * 3_600_000 + 5 * 60_000)), "2 hr 5 min");
        assert_eq!(format_total(Some(3 * 3_600_000)), "3 hr");
    }

    #[test]
    fn numbers_show_disc_and_track_when_known() {
        assert_eq!(number_label(None, Some(5)), "5");
        assert_eq!(number_label(Some(2), Some(5)), "2-5");
        assert_eq!(number_label(Some(1), None), "");
        assert_eq!(number_label(None, None), "");
    }

    #[test]
    fn file_sizes_and_properties_are_human_readable() {
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0 MB");
        assert_eq!(format_bitrate(Some(320_000)), "320 kbit/s");
        assert_eq!(format_bitrate(None), "unknown");
        assert_eq!(
            format_audio_properties(Some(44_100), Some(16)),
            "44.1 kHz, 16 bit"
        );
        assert_eq!(format_audio_properties(Some(48_000), None), "48.0 kHz");
        assert_eq!(format_audio_properties(None, Some(16)), "unknown");
        assert_eq!(format_channels(Some(2)), "Stereo");
        assert_eq!(format_channels(Some(1)), "Mono");
        assert_eq!(format_channels(Some(6)), "5.1");
        assert_eq!(format_channels(None), "unknown");
    }

    #[test]
    fn rows_fall_back_to_the_filename_for_the_title() {
        let mut untagged = track(1, "", "", "", None);
        untagged.filename = "Some Song.mp3".to_string();
        let row = TrackRow::from_track(&untagged);
        assert_eq!(row.title, "Some Song");
        assert_eq!(row.artist, "Unknown Artist");
        assert_eq!(row.album, "Unknown Album");
    }

    #[test]
    fn choosing_the_same_column_twice_reverses_the_order() {
        let mut state = list(3);
        assert_eq!(
            state.sort_indicator(TrackSort::Title),
            Some(SortDirection::Ascending)
        );
        state.sort_by(TrackSort::Title);
        assert_eq!(
            state.sort_indicator(TrackSort::Title),
            Some(SortDirection::Descending)
        );
        state.sort_by(TrackSort::Artist);
        assert_eq!(state.sort_indicator(TrackSort::Title), None);
        assert_eq!(
            state.sort_indicator(TrackSort::Artist),
            Some(SortDirection::Ascending)
        );
    }

    #[test]
    fn selection_starts_empty_and_moves_with_the_keyboard() {
        let mut state = list(3);
        assert!(!state.has_selection());
        state.select_next();
        assert_eq!(state.selected, vec![0]);
        state.select_next();
        state.select_next();
        assert_eq!(state.selected, vec![2]);
        state.select_next();
        assert_eq!(state.selected, vec![0], "selection wraps");
        state.select_previous();
        assert_eq!(state.selected, vec![2]);
    }

    #[test]
    fn extending_a_selection_covers_a_range() {
        let mut state = list(5);
        state.select_row(1, false);
        state.select_row(3, true);
        assert_eq!(state.selected, vec![1, 2, 3]);
        state.select_row(1, true);
        assert_eq!(
            state.selected,
            vec![1],
            "growing back keeps the same anchor"
        );
        state.select_row(0, true);
        assert_eq!(state.selected, vec![0, 1]);
    }

    #[test]
    fn selection_outside_the_list_is_ignored() {
        let mut state = list(2);
        state.select_row(9, false);
        assert!(!state.has_selection());
        state.select_only(1);
        assert_eq!(state.selected_track_ids(), vec![2]);
    }

    #[test]
    fn scrolling_requests_are_taken_once() {
        let mut state = list(3);
        state.select_next();
        assert_eq!(state.take_scroll_target(), Some(0));
        assert_eq!(state.take_scroll_target(), None);
    }

    #[test]
    fn track_ids_follow_the_display_order() {
        let state = list(3);
        assert_eq!(state.playable_track_ids(), vec![1, 2, 3]);
    }

    #[test]
    fn unavailable_tracks_are_never_played() {
        let mut tracks = vec![
            track(1, "One", "A", "X", Some(1000)),
            track(2, "Two", "A", "X", Some(1000)),
        ];
        tracks[1].missing = true;
        let mut state = TrackListState::new(TrackQuery::default(), TrackSort::Title);
        state.set_tracks(&tracks);
        assert_eq!(state.playable_track_ids(), vec![1]);
        state.select_only(0);
        assert_eq!(state.selected_track_ids(), vec![1]);
        assert!(state.summary().contains("1 unavailable"));
    }

    #[test]
    fn the_footer_counts_tracks_and_length() {
        let state = list(2);
        assert_eq!(state.summary(), "2 tracks · 6 min");
        let empty = TrackListState::new(TrackQuery::default(), TrackSort::Title);
        assert_eq!(empty.summary(), "0 tracks · unknown length");
    }

    #[test]
    fn total_length_is_unknown_when_a_duration_is_missing() {
        let tracks = vec![
            track(1, "One", "A", "X", Some(60_000)),
            track(2, "Two", "A", "X", None),
        ];
        let mut state = TrackListState::new(TrackQuery::default(), TrackSort::Title);
        state.set_tracks(&tracks);
        assert_eq!(state.total_duration_ms(), None);
    }

    #[test]
    fn loading_tracks_clears_the_previous_selection() {
        let mut state = list(3);
        state.select_next();
        assert!(state.has_selection());
        state.set_tracks(&[]);
        assert!(!state.has_selection());
        assert!(state.is_empty());
    }

    #[test]
    fn search_state_trims_and_reports_staleness() {
        let mut search = SearchState::new();
        assert!(!search.is_active());
        search.term = "  blue  ".to_string();
        assert!(search.is_active());
        assert!(search.needs_refresh());
        assert_eq!(search.apply(), "blue");
        assert!(!search.needs_refresh());
        search.clear();
        assert!(!search.is_active());
    }

    #[test]
    fn search_defaults_to_the_usual_fields() {
        let search = SearchState::new();
        assert!(search.any_field_selected());
        assert!(!search.case_sensitive);
        assert!(search.in_title && search.in_artist && search.in_album);
    }

    #[test]
    fn playlist_entries_become_rows() {
        let entry = PlaylistEntry {
            id: 7,
            playlist_id: 1,
            track_id: Some(4),
            position: 0,
            original_reference: "/m/a.mp3".to_string(),
            availability: crate::playlists::model::Availability::Available,
            display_label: "a.mp3".to_string(),
            duration_ms: Some(1000),
        };
        let row = EntryRow::from_entry(&entry);
        assert!(row.playable);
        assert_eq!(row.track_id, Some(4));
        assert_eq!(row.availability_label, "Available");
    }
}
