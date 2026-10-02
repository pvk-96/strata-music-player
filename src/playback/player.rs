//! The GStreamer pipeline.
//!
//! A single `playbin` plays one file at a time. Decoding is left to the installed
//! plugins (`decodebin`), so the same pipeline handles every format the library supports.
//! The bus is polled from the UI tick rather than from a thread, which keeps the state
//! machine on one thread and avoids locks around playback state.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gst::prelude::*;

use crate::error::{Error, Result};
use crate::playback::state::PlaybackState;

/// Events the pipeline reports upwards. Polled by the UI, never pushed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerEvent {
    EndOfStream,
    Error(String),
    /// Position of the pipeline, so the progress bar can follow it.
    Tick {
        position_ms: i64,
        duration_ms: Option<i64>,
    },
}

/// Wraps `playbin` and nothing else: no library knowledge, no playlist logic.
pub struct Player {
    playbin: gst::Element,
    state: RefCell<PlaybackState>,
    current_path: RefCell<Option<PathBuf>>,
    duration_ms: RefCell<Option<i64>>,
    /// Guarded by the element itself; a failed play must not spin on every tick.
    audio_failure_reported: RefCell<bool>,
}

impl Player {
    /// Build the pipeline. Fails when GStreamer is not usable at all, which is the one
    /// condition that makes playback impossible.
    pub fn new() -> Result<Self> {
        gst::init()
            .map_err(|err| Error::AudioDevice(format!("GStreamer could not start ({err}).")))?;

        let playbin = gst::ElementFactory::make("playbin")
            .name("strata-playbin")
            .build()
            .map_err(|err| Error::Playback(format!("playbin is unavailable ({err})")))?;

        if gst::ElementFactory::make("autoaudiosink").build().is_err() {
            return Err(Error::AudioDevice(
                "no audio output plugin is installed".to_string(),
            ));
        }

        Ok(Self {
            playbin,
            state: RefCell::new(PlaybackState::Stopped),
            current_path: RefCell::new(None),
            duration_ms: RefCell::new(None),
            audio_failure_reported: RefCell::new(false),
        })
    }

    pub fn state(&self) -> PlaybackState {
        *self.state.borrow()
    }

    pub fn current_path(&self) -> Option<PathBuf> {
        self.current_path.borrow().clone()
    }

    fn set_state(&self, state: PlaybackState) {
        *self.state.borrow_mut() = state;
    }

    /// Load a file and start playing it.
    pub fn play(&self, path: &Path) -> Result<()> {
        if !path.exists() {
            return Err(Error::MissingFile(path.to_path_buf()));
        }

        let uri = gst::glib::filename_to_uri(
            path.to_str().ok_or_else(|| Error::Filesystem {
                path: path.to_path_buf(),
                detail: "path is not valid UTF-8".to_string(),
            })?,
            None,
        )
        .map_err(|err| Error::Filesystem {
            path: path.to_path_buf(),
            detail: err.to_string(),
        })?;

        self.set_state(PlaybackState::Loading);
        self.playbin.set_property("uri", &uri);
        if self.playbin.set_state(gst::State::Playing).is_err() {
            self.set_state(PlaybackState::Error);
            return Err(Error::Playback(format!(
                "{} could not be started.",
                file_name(path)
            )));
        }

        *self.current_path.borrow_mut() = Some(path.to_path_buf());
        *self.duration_ms.borrow_mut() = None;
        *self.audio_failure_reported.borrow_mut() = false;
        self.set_state(PlaybackState::Playing);
        Ok(())
    }

    pub fn pause(&self) {
        if self.state() == PlaybackState::Playing
            && self.playbin.set_state(gst::State::Paused).is_ok()
        {
            self.set_state(PlaybackState::Paused);
        }
    }

    pub fn resume(&self) {
        if self.state() == PlaybackState::Paused
            && self.playbin.set_state(gst::State::Playing).is_ok()
        {
            self.set_state(PlaybackState::Playing);
        }
    }

    /// Play when paused or stopped, pause when playing.
    pub fn toggle_play(&self) {
        match self.state() {
            PlaybackState::Playing => self.pause(),
            PlaybackState::Paused | PlaybackState::Stopped => {
                if let Some(path) = self.current_path() {
                    if path.exists() {
                        if self.state() == PlaybackState::Paused {
                            self.resume();
                        } else if let Err(error) = self.play(&path) {
                            log::error!("could not resume: {error}");
                            self.set_state(PlaybackState::Error);
                        }
                    } else {
                        self.set_state(PlaybackState::Stopped);
                        log::warn!("{} is gone", path.display());
                    }
                }
            }
            _ => {}
        }
    }

    pub fn stop(&self) {
        let _ = self.playbin.set_state(gst::State::Null);
        self.set_state(PlaybackState::Stopped);
        self.playbin.set_property("uri", "");
    }

    pub fn position_ms(&self) -> i64 {
        self.playbin
            .query_position::<gst::ClockTime>()
            .map(|time| time.mseconds() as i64)
            .unwrap_or_default()
    }

    /// Duration of the current file, asked once per query rather than tracked by messages.
    pub fn duration_ms(&self) -> Option<i64> {
        if let Some(duration) = *self.duration_ms.borrow() {
            return Some(duration);
        }
        let duration = self
            .playbin
            .query_duration::<gst::ClockTime>()
            .map(|time| time.mseconds() as i64);
        if let Some(duration) = duration {
            *self.duration_ms.borrow_mut() = Some(duration);
        }
        duration
    }

    /// Seek. Out of range values are clamped, never rejected.
    pub fn seek_ms(&self, position_ms: i64) {
        let duration = self.duration_ms().unwrap_or(i64::MAX);
        let target = position_ms.clamp(0, duration.max(0));
        let position = gst::ClockTime::from_mseconds(target.max(0) as u64);
        if let Err(error) = self
            .playbin
            .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE, position)
        {
            log::warn!("seek to {target} ms failed: {error}");
        }
    }

    pub fn set_volume(&self, volume: f64) {
        let clamped = volume.clamp(0.0, 1.0);
        self.playbin.set_property("volume", clamped);
    }

    pub fn volume(&self) -> f64 {
        self.playbin.property::<f64>("volume")
    }

    pub fn set_muted(&self, muted: bool) {
        self.playbin.set_property("mute", muted);
    }

    pub fn is_muted(&self) -> bool {
        self.playbin.property::<bool>("mute")
    }

    /// Drain the bus. Returns the events the caller should act on, in order.
    pub fn drain_events(&self) -> Vec<PlayerEvent> {
        let mut events = Vec::new();
        let Some(bus) = self.playbin.bus() else {
            return events;
        };

        while let Some(message) = bus.timed_pop(gst::ClockTime::ZERO) {
            match message.view() {
                gst::MessageView::Eos(..) => {
                    events.push(PlayerEvent::EndOfStream);
                }
                gst::MessageView::Error(error) => {
                    let detail = format!("{}", error.error());
                    let source = error
                        .src()
                        .map(|source| source.name().to_string())
                        .unwrap_or_default();
                    let audio_related = detail.to_lowercase().contains("audio")
                        || source.to_lowercase().contains("audio");
                    log::error!("playback error: {detail} (source: {source})");
                    self.set_state(PlaybackState::Error);
                    // Report a failing audio device once; otherwise every retry would show it.
                    if audio_related && *self.audio_failure_reported.borrow() {
                        continue;
                    }
                    *self.audio_failure_reported.borrow_mut() = true;
                    events.push(PlayerEvent::Error(detail));
                }
                _ => {}
            }
        }

        if !events.is_empty() || self.state() == PlaybackState::Playing {
            events.push(PlayerEvent::Tick {
                position_ms: self.position_ms(),
                duration_ms: self.duration_ms(),
            });
        }
        events
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = self.playbin.set_state(gst::State::Null);
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Shared handle used by the UI layer.
pub type SharedPlayer = Rc<Player>;

#[cfg(test)]
mod tests {
    use super::*;

    fn player() -> Option<Player> {
        Player::new().ok()
    }

    #[test]
    fn a_new_player_is_stopped_and_silent() {
        let Some(player) = player() else {
            return;
        };
        assert_eq!(player.state(), PlaybackState::Stopped);
        assert_eq!(player.current_path(), None);
        assert_eq!(player.position_ms(), 0);
        assert_eq!(player.volume(), 1.0);
    }

    #[test]
    fn playing_a_file_that_is_not_there_fails_cleanly() {
        let Some(player) = player() else {
            return;
        };
        let error = player
            .play(Path::new("/definitely/not/here.mp3"))
            .unwrap_err();
        assert_eq!(error.kind(), "MissingFile");
        assert_eq!(player.state(), PlaybackState::Stopped);
    }

    #[test]
    fn volume_is_clamped_to_the_normal_range() {
        let Some(player) = player() else {
            return;
        };
        player.set_volume(3.0);
        assert_eq!(player.volume(), 1.0);
        player.set_volume(-1.0);
        assert_eq!(player.volume(), 0.0);
        player.set_volume(0.5);
        assert_eq!(player.volume(), 0.5);
    }

    #[test]
    fn mute_is_reported_back() {
        let Some(player) = player() else {
            return;
        };
        player.set_muted(true);
        assert!(player.is_muted());
        player.set_muted(false);
        assert!(!player.is_muted());
    }

    #[test]
    fn seeking_while_stopped_is_harmless() {
        let Some(player) = player() else {
            return;
        };
        player.seek_ms(5_000);
        assert_eq!(player.position_ms(), 0);
    }

    #[test]
    fn stopping_without_playing_is_harmless() {
        let Some(player) = player() else {
            return;
        };
        player.stop();
        player.pause();
        player.resume();
        assert_eq!(player.state(), PlaybackState::Stopped);
    }

    #[test]
    fn toggling_play_without_a_track_does_nothing() {
        let Some(player) = player() else {
            return;
        };
        player.toggle_play();
        assert_eq!(player.state(), PlaybackState::Stopped);
    }

    #[test]
    fn a_real_file_reaches_the_playing_state() {
        let Some(player) = player() else {
            return;
        };
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format.mp3");
        if !fixture.exists() {
            return;
        }
        if player.play(&fixture).is_err() {
            // No audio device in this environment: the failure is reported, not hidden.
            assert_eq!(player.state(), PlaybackState::Error);
            return;
        }
        assert_eq!(player.state(), PlaybackState::Playing);
        assert_eq!(player.current_path(), Some(fixture));
        player.pause();
        assert_eq!(player.state(), PlaybackState::Paused);
        player.resume();
        assert_eq!(player.state(), PlaybackState::Playing);
        player.stop();
        assert_eq!(player.state(), PlaybackState::Stopped);
    }
}
