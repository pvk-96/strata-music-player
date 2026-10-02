//! Background artwork loading.
//!
//! The UI thread never reads an image file. It asks for artwork and polls for finished
//! results; worker threads do the reading. Requests for the same image are collapsed, so a
//! list of 500 rows does not read the same cover 500 times.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

/// One finished request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtworkResult {
    Ready {
        track_id: i64,
        size: u32,
        path: PathBuf,
    },
    Missing {
        track_id: i64,
        size: u32,
    },
    Failed {
        track_id: i64,
        size: u32,
        reason: String,
    },
}

struct Job {
    track_id: i64,
    track_path: PathBuf,
    size: u32,
}

type Key = (i64, u32);

pub struct ArtworkLoader {
    jobs: Sender<Job>,
    results: Receiver<ArtworkResult>,
    cache: Arc<crate::artwork::cache::ArtworkCache>,
    /// Requests already queued or in flight.
    pending: Arc<Mutex<HashSet<Key>>>,
    /// Last known cache path per request, so scrolling back doesn't re-read anything.
    known: Arc<Mutex<HashMap<Key, Option<PathBuf>>>>,
    stop: Arc<AtomicBool>,
    workers: Vec<JoinHandle<()>>,
    reads: Arc<AtomicUsize>,
}

impl ArtworkLoader {
    pub fn new(cache: Arc<crate::artwork::cache::ArtworkCache>) -> Self {
        Self::with_workers(cache, 2)
    }

    pub fn with_workers(cache: Arc<crate::artwork::cache::ArtworkCache>, workers: usize) -> Self {
        let (jobs, job_receiver) = channel::<Job>();
        let (result_sender, results) = channel::<ArtworkResult>();
        let job_receiver = Arc::new(Mutex::new(job_receiver));
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let known = Arc::new(Mutex::new(HashMap::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let reads = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for index in 0..workers.max(1) {
            let job_receiver = Arc::clone(&job_receiver);
            let result_sender = result_sender.clone();
            let cache = Arc::clone(&cache);
            let pending = Arc::clone(&pending);
            let known = Arc::clone(&known);
            let stop = Arc::clone(&stop);
            let reads = Arc::clone(&reads);
            handles.push(std::thread::spawn(move || {
                // A small sleep on an idle queue keeps the workers from spinning.
                loop {
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }
                    let job = {
                        let receiver = match job_receiver.lock() {
                            Ok(receiver) => receiver,
                            Err(_) => return,
                        };
                        match receiver.recv_timeout(Duration::from_millis(200)) {
                            Ok(job) => Some(job),
                            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => None,
                            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    };

                    let Some(job) = job else {
                        continue;
                    };
                    if stop.load(Ordering::Relaxed) {
                        return;
                    }

                    let key = (job.track_id, job.size);
                    let outcome = load_one(&cache, &job, &reads);
                    if let Ok(mut known) = known.lock() {
                        known.insert(key, outcome.clone());
                    }
                    if let Ok(mut pending) = pending.lock() {
                        pending.remove(&key);
                    }
                    let message = match outcome {
                        Some(path) => ArtworkResult::Ready {
                            track_id: job.track_id,
                            size: job.size,
                            path,
                        },
                        None => ArtworkResult::Missing {
                            track_id: job.track_id,
                            size: job.size,
                        },
                    };
                    if result_sender.send(message).is_err() {
                        return;
                    }
                    let _ = index;
                }
            }));
        }

        Self {
            jobs,
            results,
            cache,
            pending,
            known,
            stop,
            workers: handles,
            reads,
        }
    }

    pub fn cache(&self) -> &Arc<crate::artwork::cache::ArtworkCache> {
        &self.cache
    }

    /// Ask for artwork. Repeated requests for the same track and size are ignored until the
    /// first one finishes.
    pub fn request(&self, track_id: i64, track_path: &Path, size: u32) {
        let key = (track_id, size);
        if let Ok(known) = self.known.lock() {
            if known.contains_key(&key) {
                return;
            }
        }
        if let Ok(mut pending) = self.pending.lock() {
            if !pending.insert(key) {
                return;
            }
        }
        let _ = self.jobs.send(Job {
            track_id,
            track_path: track_path.to_path_buf(),
            size,
        });
    }

    /// Cache path if this request already finished.
    pub fn ready(&self, track_id: i64, size: u32) -> Option<Option<PathBuf>> {
        self.known.lock().ok()?.get(&(track_id, size)).cloned()
    }

    /// Results that finished since the last call. Never blocks.
    pub fn poll(&self) -> Vec<ArtworkResult> {
        self.results.try_iter().collect()
    }

    pub fn pending(&self) -> usize {
        self.pending
            .lock()
            .map(|pending| pending.len())
            .unwrap_or(0)
    }

    /// How many files were actually read. Used by tests to prove collapsing works.
    pub fn reads(&self) -> usize {
        self.reads.load(Ordering::Relaxed)
    }

    /// Wait for results, for tests and for shutdown.
    pub fn wait_for(&self, expected: usize, timeout: Duration) -> Vec<ArtworkResult> {
        let mut collected = Vec::new();
        let deadline = std::time::Instant::now() + timeout;
        while collected.len() < expected && std::time::Instant::now() < deadline {
            if let Ok(result) = self.results.recv_timeout(Duration::from_millis(50)) {
                collected.push(result);
            }
        }
        collected
    }
}

fn load_one(
    cache: &crate::artwork::cache::ArtworkCache,
    job: &Job,
    reads: &AtomicUsize,
) -> Option<PathBuf> {
    match crate::artwork::source::load(&job.track_path) {
        Ok(Some((_source, artwork))) => {
            reads.fetch_add(1, Ordering::Relaxed);
            cache.put(&artwork).ok()
        }
        Ok(None) => None,
        Err(error) => {
            log::warn!(
                "artwork for {} could not be read: {}",
                job.track_path.display(),
                error.kind()
            );
            reads.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

impl Drop for ArtworkLoader {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock any worker waiting on the queue.
        let _ = self.jobs.send(Job {
            track_id: -1,
            track_path: PathBuf::new(),
            size: 0,
        });
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artwork::cache::ArtworkCache;

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name)
    }

    fn loader(workers: usize) -> (ArtworkLoader, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let cache = Arc::new(ArtworkCache::new(directory.path()));
        (ArtworkLoader::with_workers(cache, workers), directory)
    }

    #[test]
    fn embedded_artwork_is_loaded_in_the_background() {
        let (loader, _directory) = loader(2);
        loader.request(1, &fixture("complete.flac"), 64);
        let results = loader.wait_for(1, Duration::from_secs(5));
        assert_eq!(results.len(), 1);
        match &results[0] {
            ArtworkResult::Ready {
                track_id,
                path,
                size,
            } => {
                assert_eq!(*track_id, 1);
                assert_eq!(*size, 64);
                assert!(path.is_file());
            }
            other => panic!("expected artwork, got {other:?}"),
        }
    }

    #[test]
    fn a_track_without_artwork_reports_missing() {
        let directory = tempfile::tempdir().unwrap();
        let track = directory.path().join("plain.mp3");
        std::fs::copy(fixture("no_tags.mp3"), &track).unwrap();

        let (loader, _cache) = loader(1);
        loader.request(2, &track, 48);
        let results = loader.wait_for(1, Duration::from_secs(5));
        assert_eq!(
            results,
            vec![ArtworkResult::Missing {
                track_id: 2,
                size: 48
            }]
        );
    }

    #[test]
    fn a_missing_file_does_not_stop_the_worker() {
        let (loader, _cache) = loader(1);
        loader.request(3, Path::new("/nope/gone.mp3"), 48);
        loader.request(4, &fixture("complete.flac"), 48);
        let results = loader.wait_for(2, Duration::from_secs(5));
        assert_eq!(results.len(), 2);
        assert!(matches!(results[0], ArtworkResult::Missing { .. }));
        assert!(matches!(results[1], ArtworkResult::Ready { .. }));
    }

    #[test]
    fn repeated_requests_read_the_file_once() {
        let (loader, _cache) = loader(1);
        for _ in 0..20 {
            loader.request(5, &fixture("complete.flac"), 64);
        }
        loader.wait_for(1, Duration::from_secs(5));
        assert_eq!(loader.reads(), 1, "requests are collapsed");
        assert_eq!(loader.pending(), 0);
    }

    #[test]
    fn different_sizes_are_separate_requests() {
        let (loader, _cache) = loader(2);
        loader.request(6, &fixture("complete.flac"), 64);
        loader.request(6, &fixture("complete.flac"), 128);
        let results = loader.wait_for(2, Duration::from_secs(5));
        assert_eq!(results.len(), 2);
        assert_eq!(loader.pending(), 0);
    }

    #[test]
    fn a_finished_request_is_not_repeated() {
        let (loader, _cache) = loader(1);
        loader.request(7, &fixture("complete.flac"), 64);
        loader.wait_for(1, Duration::from_secs(5));
        loader.request(7, &fixture("complete.flac"), 64);
        assert_eq!(loader.reads(), 1);
        assert!(loader.ready(7, 64).is_some());
    }

    #[test]
    fn poll_returns_nothing_when_idle() {
        let (loader, _cache) = loader(1);
        assert!(loader.poll().is_empty());
    }

    #[test]
    fn polling_drains_finished_work() {
        let (loader, _cache) = loader(2);
        loader.request(8, &fixture("complete.flac"), 64);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let mut collected = Vec::new();
        while collected.is_empty() && std::time::Instant::now() < deadline {
            collected = loader.poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(collected.len(), 1);
        assert!(loader.poll().is_empty(), "results are only returned once");
    }
}
