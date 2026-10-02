//! Application error model.
//!
//! Errors are classified into a small, fixed set of categories (see `Error`) so that
//! the UI can present a readable message while the technical detail goes to the log.
//! Raw library errors are never shown to the user.

use std::fmt;
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    UnsupportedFormat { path: PathBuf, detail: String },
    MissingFile(PathBuf),
    PermissionDenied { path: PathBuf, detail: String },
    MetadataRead { path: PathBuf, detail: String },
    AudioDevice(String),
    Playback(String),
    Database(String),
    Filesystem { path: PathBuf, detail: String },
    PlaylistResolution(String),
    InvalidConfiguration(String),
}

impl Error {
    /// Stable category name, used in logs and tests.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::UnsupportedFormat { .. } => "UnsupportedFormat",
            Error::MissingFile(_) => "MissingFile",
            Error::PermissionDenied { .. } => "PermissionDenied",
            Error::MetadataRead { .. } => "MetadataReadFailure",
            Error::AudioDevice(_) => "AudioDeviceFailure",
            Error::Playback(_) => "PlaybackFailure",
            Error::Database(_) => "DatabaseFailure",
            Error::Filesystem { .. } => "FilesystemFailure",
            Error::PlaylistResolution(_) => "PlaylistResolutionFailure",
            Error::InvalidConfiguration(_) => "InvalidConfiguration",
        }
    }

    /// Message intended for the user. Free of implementation terms.
    pub fn user_message(&self) -> String {
        match self {
            Error::UnsupportedFormat { path, .. } => {
                format!(
                    "Strata can't play this kind of file ({}).",
                    file_label(path)
                )
            }
            Error::MissingFile(path) => format!("{} could not be found.", file_label(path)),
            Error::PermissionDenied { path, .. } => {
                format!(
                    "Strata doesn't have permission to read {}.",
                    file_label(path)
                )
            }
            Error::MetadataRead { path, .. } => {
                format!("Couldn't read information from {}.", file_label(path))
            }
            Error::AudioDevice(detail) => format!("No audio output is available ({detail})."),
            Error::Playback(detail) => format!("Playback problem: {detail}"),
            Error::Database(detail) => format!("The library store has a problem ({detail})."),
            Error::Filesystem { path, detail } => {
                format!("Couldn't read {} ({detail}).", file_label(path))
            }
            Error::PlaylistResolution(detail) => {
                format!("Some playlist entries couldn't be matched ({detail}).")
            }
            Error::InvalidConfiguration(detail) => format!("A setting is invalid ({detail})."),
        }
    }

    pub fn database(detail: impl Into<String>) -> Self {
        Error::Database(detail.into())
    }

    pub fn playback(detail: impl Into<String>) -> Self {
        Error::Playback(detail.into())
    }

    pub fn config(detail: impl Into<String>) -> Self {
        Error::InvalidConfiguration(detail.into())
    }
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this file".to_string())
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.kind(), self.user_message())
    }
}

impl std::error::Error for Error {}

impl From<rusqlite::Error> for Error {
    fn from(err: rusqlite::Error) -> Self {
        Error::Database(err.to_string())
    }
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound => Error::MissingFile(PathBuf::new()),
            std::io::ErrorKind::PermissionDenied => Error::PermissionDenied {
                path: PathBuf::new(),
                detail: err.to_string(),
            },
            _ => Error::Filesystem {
                path: PathBuf::new(),
                detail: err.to_string(),
            },
        }
    }
}
