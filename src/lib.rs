//! Strata: a local-first desktop music player for your own music library.
//!
//! The crate is split so that everything except the process entry point can be tested
//! without a display: `src/main.rs` is a thin wrapper around [`app::run`].

pub mod app;
pub mod artwork;
pub mod database;
pub mod error;
pub mod library;
pub mod logging;
pub mod playback;
pub mod playlists;
pub mod settings;
