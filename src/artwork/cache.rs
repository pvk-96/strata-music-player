//! Artwork cache.
//!
//! Images are stored under the user's cache directory, one file per content hash, so two
//! tracks with the same cover share one file. The cache is disposable: if it is missing,
//! artwork is read from the library again.

use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::library::metadata::EmbeddedArtwork;

/// Default cache size. Large enough for a big library of covers, small enough to be
/// harmless.
pub const DEFAULT_MAX_BYTES: u64 = 256 * 1024 * 1024;

pub struct ArtworkCache {
    root: PathBuf,
    max_bytes: u64,
}

impl ArtworkCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            max_bytes: DEFAULT_MAX_BYTES,
        }
    }

    pub fn with_limit(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where an image with this content lives.
    pub fn path_for(&self, key: &str) -> PathBuf {
        self.root.join(format!("{}.img", key))
    }

    /// Read a cached image, if present.
    pub fn get(&self, key: &str) -> Option<PathBuf> {
        let path = self.path_for(key);
        path.is_file().then_some(path)
    }

    /// Store an image and return its cache path.
    pub fn put(&self, artwork: &EmbeddedArtwork) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.root).map_err(|error| Error::Filesystem {
            path: self.root.clone(),
            detail: error.to_string(),
        })?;
        let key = content_key(&artwork.data);
        let path = self.path_for(&key);
        if !path.is_file() {
            // Write to a temporary name first so a crash can't leave a half-written image.
            let temporary = path.with_extension("img.part");
            std::fs::write(&temporary, &artwork.data).map_err(|error| Error::Filesystem {
                path: temporary.clone(),
                detail: error.to_string(),
            })?;
            std::fs::rename(&temporary, &path).map_err(|error| Error::Filesystem {
                path: path.clone(),
                detail: error.to_string(),
            })?;
        }
        self.prune_if_needed();
        Ok(path)
    }

    pub fn size_bytes(&self) -> u64 {
        cache_files(&self.root)
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum()
    }

    pub fn count(&self) -> usize {
        cache_files(&self.root).len()
    }

    /// Remove the least recently used images until the cache fits its limit.
    pub fn prune_if_needed(&self) {
        let mut files = cache_files(&self.root);
        let mut total: u64 = files
            .iter()
            .filter_map(|path| std::fs::metadata(path).ok())
            .map(|metadata| metadata.len())
            .sum();
        if total <= self.max_bytes {
            return;
        }

        files.sort_by_key(|path| {
            std::fs::metadata(path)
                .and_then(|metadata| metadata.modified())
                .ok()
        });

        for path in files {
            if total <= self.max_bytes {
                break;
            }
            if let Ok(metadata) = std::fs::metadata(&path) {
                if std::fs::remove_file(&path).is_ok() {
                    total = total.saturating_sub(metadata.len());
                }
            }
        }
    }

    pub fn clear(&self) -> Result<()> {
        for path in cache_files(&self.root) {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
}

/// Content key for an image. A 128 bit FNV-1a hash is enough here: it only has to avoid
/// collisions between images in one user's cache, and the file name is not a security
/// boundary.
pub fn content_key(data: &[u8]) -> String {
    let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
    for byte in data {
        hash ^= u128::from(*byte);
        hash = hash.wrapping_mul(0x0000000001000000000000000000013b);
    }
    format!("{hash:032x}")
}

fn cache_files(root: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("img"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(bytes: &[u8]) -> EmbeddedArtwork {
        EmbeddedArtwork {
            mime: "image/png".to_string(),
            data: bytes.to_vec(),
        }
    }

    #[test]
    fn an_image_is_stored_and_found_again() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path());
        let path = cache.put(&image(b"picture one")).unwrap();
        assert!(path.is_file());
        assert_eq!(cache.get(&content_key(b"picture one")), Some(path.clone()));
        assert_eq!(cache.count(), 1);
        assert_eq!(std::fs::read(&path).unwrap(), b"picture one");
    }

    #[test]
    fn the_same_image_is_stored_once() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path());
        let first = cache.put(&image(b"same")).unwrap();
        let second = cache.put(&image(b"same")).unwrap();
        assert_eq!(first, second);
        assert_eq!(cache.count(), 1);
    }

    #[test]
    fn different_images_get_different_files() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path());
        let first = cache.put(&image(b"one")).unwrap();
        let second = cache.put(&image(b"two")).unwrap();
        assert_ne!(first, second);
        assert_eq!(cache.count(), 2);
    }

    #[test]
    fn a_miss_returns_nothing() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path());
        assert_eq!(cache.get(&content_key(b"absent")), None);
        assert_eq!(cache.count(), 0);
    }

    #[test]
    fn the_cache_creates_its_directory() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("nested/artwork");
        let cache = ArtworkCache::new(&root);
        cache.put(&image(b"x")).unwrap();
        assert!(root.is_dir());
    }

    #[test]
    fn pruning_removes_the_oldest_images() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path()).with_limit(10);
        cache.put(&image(b"0000000001")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.put(&image(b"0000000002")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        cache.put(&image(b"0000000003")).unwrap();

        assert!(cache.size_bytes() <= 20, "cache is trimmed to its limit");
        assert!(cache.count() < 3, "at least one image was pruned");
    }

    #[test]
    fn clearing_empties_the_cache() {
        let directory = tempfile::tempdir().unwrap();
        let cache = ArtworkCache::new(directory.path());
        cache.put(&image(b"one")).unwrap();
        cache.clear().unwrap();
        assert_eq!(cache.count(), 0);
        assert!(!cache.root().exists() || cache.size_bytes() == 0);
    }

    #[test]
    fn content_keys_are_stable_and_distinct() {
        assert_eq!(content_key(b"abc"), content_key(b"abc"));
        assert_ne!(content_key(b"abc"), content_key(b"abd"));
        assert_eq!(content_key(b"abc").len(), 32);
    }
}
