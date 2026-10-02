//! Local file logging.
//!
//! Logs are written to the platform cache directory and never leave the machine.
//! Normal runs are quiet (`Warn`); `STRATA_LOG` raises the level for troubleshooting
//! (`STRATA_LOG=debug`, `STRATA_LOG=trace`, ...).

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use log::{LevelFilter, Log, Metadata, Record};

/// Log files are rotated once they pass this size so a long-lived profile
/// cannot fill the disk.
const MAX_BYTES: u64 = 2 * 1024 * 1024;

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

struct FileLogger {
    file: Mutex<File>,
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format!(
            "{} {} [{}] {}\n",
            unix_seconds(),
            record.level(),
            record.target(),
            record.args()
        );
        let mut file = match self.file.lock() {
            Ok(file) => file,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Err(err) = file.write_all(line.as_bytes()) {
            eprintln!("strata: could not write log line: {err}");
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

fn unix_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or_default()
}

/// Start logging into `dir/strata.log`. Returns the active log path.
/// Calling this more than once is harmless; the first call wins.
pub fn init(dir: &Path) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join("strata.log");
    rotate_if_needed(&path);

    let file = OpenOptions::new().create(true).append(true).open(&path)?;
    if LOGGER
        .set(FileLogger {
            file: Mutex::new(file),
        })
        .is_ok()
    {
        let _ = log::set_logger(LOGGER.get().expect("logger just initialised"));
    }
    log::set_max_level(level_from_env());

    Ok(path)
}

pub fn level_from_env() -> LevelFilter {
    match std::env::var("STRATA_LOG").ok().as_deref() {
        Some("off") => LevelFilter::Off,
        Some("error") => LevelFilter::Error,
        Some("info") => LevelFilter::Info,
        Some("debug") => LevelFilter::Debug,
        Some("trace") => LevelFilter::Trace,
        _ => LevelFilter::Warn,
    }
}

fn rotate_if_needed(path: &Path) {
    let too_big = fs::metadata(path)
        .map(|meta| meta.len() > MAX_BYTES)
        .unwrap_or(false);
    if too_big {
        let _ = fs::rename(path, path.with_file_name("strata.log.1"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_is_plausible() {
        assert!(unix_seconds() > 1_600_000_000);
    }

    #[test]
    fn unknown_env_value_falls_back_to_default() {
        let temp = tempfile::tempdir().unwrap();
        std::env::remove_var("STRATA_LOG");
        assert_eq!(level_from_env(), LevelFilter::Warn);
        assert!(temp.path().exists());
    }
}
