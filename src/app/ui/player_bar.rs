//! The player bar: transport, now playing, seek bar, and volume.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk::prelude::*;
use gtk::{Align, Box as GtkBox, Label, Orientation, Scale};

use crate::app::state::format_duration;
use crate::app::ui::widgets::{hint_label, icon_button, text_label, toggle_button, vstack};
use crate::playback::state::RepeatMode;

/// A scale has a finite range, so a seek bar of this length covers two hours in tenth-of-a-
/// second steps, which is finer than anyone can hear.
pub const MAX_SEEK_SECONDS: f64 = 7200.0;
pub const SEEK_STEP: f64 = 10.0;
pub const MAX_VOLUME_PERCENT: f64 = 100.0;

type SeekCallback = Box<dyn Fn(f64)>;
type VolumeCallback = Box<dyn Fn(f64)>;

pub struct PlayerBar {
    root: GtkBox,
    play_button: gtk::Button,
    previous_button: gtk::Button,
    next_button: gtk::Button,
    shuffle_button: gtk::ToggleButton,
    repeat_button: gtk::ToggleButton,
    mute_button: gtk::Button,
    queue_button: gtk::Button,
    title: Label,
    artist: Label,
    position: Label,
    duration: Label,
    seek: Scale,
    volume: Scale,
    /// Set while the user drags, so refreshing does not fight them.
    seeking: Rc<Cell<bool>>,
    changing_volume: Rc<Cell<bool>>,
    on_seek: Rc<RefCell<Option<SeekCallback>>>,
    on_volume: Rc<RefCell<Option<VolumeCallback>>>,
}

impl PlayerBar {
    pub fn new() -> Self {
        let title = text_label("", Align::Start);
        title.add_css_class("title-4");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        let artist = hint_label("");

        let now_playing = vstack(0);
        now_playing.set_hexpand(true);
        now_playing.append(&title);
        now_playing.append(&artist);

        let shuffle_button = toggle_button("media-playlist-shuffle-symbolic", "Shuffle");
        let previous_button = icon_button("media-skip-backward-symbolic", "Previous track");
        let play_button = icon_button("media-playback-start-symbolic", "Play");
        play_button.add_css_class("suggested-action");
        let next_button = icon_button("media-skip-forward-symbolic", "Next track");
        let repeat_button = toggle_button("media-playlist-repeat-symbolic", "Repeat");
        let queue_button = icon_button("view-list-symbolic", "Show queue");

        let transport = GtkBox::new(Orientation::Horizontal, 6);
        transport.set_valign(Align::Center);
        for button in [
            shuffle_button.upcast_ref::<gtk::Widget>(),
            previous_button.upcast_ref(),
            play_button.upcast_ref(),
            next_button.upcast_ref(),
            repeat_button.upcast_ref(),
            queue_button.upcast_ref(),
        ] {
            transport.append(button);
        }

        let seek = Scale::with_range(Orientation::Horizontal, 0.0, MAX_SEEK_SECONDS, SEEK_STEP);
        seek.set_hexpand(true);
        seek.set_draw_value(false);
        seek.set_valign(Align::Center);

        let position = text_label("0:00", Align::End);
        position.set_width_chars(6);
        let duration = text_label("--:--", Align::Start);
        duration.set_width_chars(6);

        let seek_row = GtkBox::new(Orientation::Horizontal, 6);
        seek_row.set_hexpand(true);
        seek_row.append(&position);
        seek_row.append(&seek);
        seek_row.append(&duration);

        let mute_button = icon_button("audio-volume-high-symbolic", "Mute");
        let volume = Scale::with_range(Orientation::Horizontal, 0.0, MAX_VOLUME_PERCENT, 1.0);
        volume.set_value(MAX_VOLUME_PERCENT);
        volume.set_size_request(120, -1);
        volume.set_valign(Align::Center);

        let volume_row = GtkBox::new(Orientation::Horizontal, 6);
        volume_row.append(&mute_button);
        volume_row.append(&volume);

        let columns = GtkBox::new(Orientation::Horizontal, 12);
        columns.set_hexpand(true);
        columns.append(&now_playing);
        columns.append(&transport);
        columns.append(&seek_row);
        columns.append(&volume_row);

        let root = vstack(4);
        root.set_margin_top(6);
        root.set_margin_bottom(6);
        root.set_margin_start(12);
        root.set_margin_end(12);
        root.append(&columns);

        let bar = Self {
            root,
            play_button,
            previous_button,
            next_button,
            shuffle_button,
            repeat_button,
            mute_button,
            queue_button,
            title,
            artist,
            position,
            duration,
            seek,
            volume,
            seeking: Rc::new(Cell::new(false)),
            changing_volume: Rc::new(Cell::new(false)),
            on_seek: Rc::new(RefCell::new(None)),
            on_volume: Rc::new(RefCell::new(None)),
        };

        // GTK signals are connected once here; the callbacks are filled in by `connect_*`.
        let seeking_press = Rc::clone(&bar.seeking);
        let seek_click = gtk::GestureClick::new();
        seek_click.connect_pressed(move |_, _, _, _| seeking_press.set(true));
        let seeking_release = Rc::clone(&bar.seeking);
        let on_seek = Rc::clone(&bar.on_seek);
        seek_click.connect_released(move |gesture, _, _, _| {
            seeking_release.set(false);
            if let (Some(scale), Some(callback)) = (
                gesture.widget().and_downcast::<gtk::Scale>(),
                on_seek.borrow().as_ref(),
            ) {
                callback(scale.value());
            }
        });
        bar.seek.add_controller(seek_click);

        let changing_press = Rc::clone(&bar.changing_volume);
        let volume_click = gtk::GestureClick::new();
        volume_click.connect_pressed(move |_, _, _, _| changing_press.set(true));
        let changing_release = Rc::clone(&bar.changing_volume);
        let on_volume = Rc::clone(&bar.on_volume);
        volume_click.connect_released(move |gesture, _, _, _| {
            changing_release.set(false);
            if let (Some(scale), Some(callback)) = (
                gesture.widget().and_downcast::<gtk::Scale>(),
                on_volume.borrow().as_ref(),
            ) {
                callback(scale.value());
            }
        });
        bar.volume.add_controller(volume_click);

        bar
    }

    pub fn widget(&self) -> &GtkBox {
        &self.root
    }

    /// Ask for a seek to `seconds` when the user lets go of the seek bar.
    pub fn connect_seek<F: Fn(f64) + 'static>(&self, callback: F) {
        *self.on_seek.borrow_mut() = Some(Box::new(callback));
    }

    /// Ask for a new volume when the user lets go of the volume slider.
    pub fn connect_volume<F: Fn(f64) + 'static>(&self, callback: F) {
        *self.on_volume.borrow_mut() = Some(Box::new(callback));
    }

    pub fn connect_play<F: Fn() + 'static>(&self, callback: F) {
        self.play_button.connect_clicked(move |_| callback());
    }

    pub fn connect_previous<F: Fn() + 'static>(&self, callback: F) {
        self.previous_button.connect_clicked(move |_| callback());
    }

    pub fn connect_next<F: Fn() + 'static>(&self, callback: F) {
        self.next_button.connect_clicked(move |_| callback());
    }

    pub fn connect_toggle_shuffle<F: Fn() + 'static>(&self, callback: F) {
        self.shuffle_button.connect_clicked(move |_| callback());
    }

    pub fn connect_cycle_repeat<F: Fn() + 'static>(&self, callback: F) {
        self.repeat_button.connect_clicked(move |_| callback());
    }

    pub fn connect_toggle_mute<F: Fn() + 'static>(&self, callback: F) {
        self.mute_button.connect_clicked(move |_| callback());
    }

    pub fn connect_show_queue<F: Fn() + 'static>(&self, callback: F) {
        self.queue_button.connect_clicked(move |_| callback());
    }

    /// Show what is playing. Empty text means nothing is loaded.
    pub fn set_now_playing(&self, title: &str, artist: &str) {
        self.title.set_text(title);
        self.artist.set_text(artist);
        let empty = title.is_empty() && artist.is_empty();
        self.play_button.set_sensitive(!empty);
        self.previous_button.set_sensitive(!empty);
        self.next_button.set_sensitive(!empty);
    }

    pub fn set_playing(&self, playing: bool) {
        self.play_button.set_icon_name(if playing {
            "media-playback-pause-symbolic"
        } else {
            "media-playback-start-symbolic"
        });
        self.play_button
            .set_tooltip_text(Some(if playing { "Pause" } else { "Play" }));
    }

    pub fn set_shuffle(&self, enabled: bool) {
        self.shuffle_button.set_active(enabled);
    }

    pub fn set_repeat(&self, mode: RepeatMode) {
        self.repeat_button.set_active(mode != RepeatMode::Off);
        self.repeat_button.set_icon_name(match mode {
            RepeatMode::One => "media-playlist-repeat-one-symbolic",
            RepeatMode::Off | RepeatMode::All => "media-playlist-repeat-symbolic",
        });
        self.repeat_button.set_tooltip_text(Some(match mode {
            RepeatMode::Off => "Repeat off",
            RepeatMode::All => "Repeat all",
            RepeatMode::One => "Repeat one",
        }));
    }

    pub fn set_muted(&self, muted: bool) {
        self.mute_button
            .set_tooltip_text(Some(if muted { "Unmute" } else { "Mute" }));
    }

    pub fn set_volume(&self, volume: f64, muted: bool) {
        if !self.changing_volume.get() {
            self.volume.set_value(volume * MAX_VOLUME_PERCENT);
        }
        self.mute_button.set_icon_name(volume_icon(volume, muted));
    }

    /// Update the seek bar and the two time labels.
    pub fn set_position(&self, position_ms: i64, duration_ms: Option<i64>) {
        let duration_ms = duration_ms.filter(|ms| *ms > 0);
        if !self.seeking.get() {
            self.seek
                .set_value((position_ms as f64 / 1000.0).clamp(0.0, MAX_SEEK_SECONDS));
        }
        self.seek.set_sensitive(duration_ms.is_some());
        self.position.set_text(&format_duration(Some(position_ms)));
        self.duration.set_text(&format_duration(duration_ms));
    }
}

impl Default for PlayerBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Icon for the current volume and mute state.
pub fn volume_icon(volume: f64, muted: bool) -> &'static str {
    if muted || volume <= 0.0 {
        "audio-volume-muted-symbolic"
    } else if volume < 0.34 {
        "audio-volume-low-symbolic"
    } else if volume < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

/// Position a seek bar value stands for, in milliseconds.
pub fn seek_target_ms(seconds: f64) -> i64 {
    (seconds * 1000.0).round().max(0.0) as i64
}

/// Position on the seek bar for a playback position, clamped to the bar's range.
pub fn seek_bar_value(position_ms: i64) -> f64 {
    (position_ms as f64 / 1000.0).clamp(0.0, MAX_SEEK_SECONDS)
}

/// Volume percentage for a 0..1 volume.
pub fn volume_percent(volume: f64) -> f64 {
    volume.clamp(0.0, 1.0) * MAX_VOLUME_PERCENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_seek_value_becomes_a_position_in_milliseconds() {
        assert_eq!(seek_target_ms(0.0), 0);
        assert_eq!(seek_target_ms(12.5), 12_500);
        assert_eq!(seek_target_ms(-3.0), 0);
    }

    #[test]
    fn a_position_becomes_a_seek_value_inside_the_bar() {
        assert_eq!(seek_bar_value(0), 0.0);
        assert_eq!(seek_bar_value(90_000), 90.0);
        assert_eq!(seek_bar_value(-5_000), 0.0);
        assert_eq!(seek_bar_value(i64::MAX), MAX_SEEK_SECONDS);
    }

    #[test]
    fn volume_icons_follow_the_level() {
        assert_eq!(volume_icon(0.0, false), "audio-volume-muted-symbolic");
        assert_eq!(volume_icon(0.9, true), "audio-volume-muted-symbolic");
        assert_eq!(volume_icon(0.2, false), "audio-volume-low-symbolic");
        assert_eq!(volume_icon(0.5, false), "audio-volume-medium-symbolic");
        assert_eq!(volume_icon(1.0, false), "audio-volume-high-symbolic");
    }

    #[test]
    fn volume_percentage_is_clamped() {
        assert_eq!(volume_percent(0.5), 50.0);
        assert_eq!(volume_percent(1.5), 100.0);
        assert_eq!(volume_percent(-0.5), 0.0);
    }
}
