use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::Settings;

#[derive(Debug, Clone)]
pub struct AudioCache {
    dir: PathBuf,
    enabled: bool,
    max_bytes: u64,
}

impl AudioCache {
    pub fn new(dir: PathBuf, enabled: bool, max_mb: u64) -> Result<Self> {
        // Always create the dir: even when caching is disabled we may need a
        // scratch file for local playback of a freshly synthesized utterance.
        fs::create_dir_all(&dir).with_context(|| format!("create cache {}", dir.display()))?;
        Ok(Self {
            dir,
            enabled,
            max_bytes: max_mb.saturating_mul(1024 * 1024),
        })
    }

    pub fn key(&self, text: &str, voice_id: &str, model: &str, fmt: &str, s: &Settings) -> String {
        let mut h = Sha256::new();
        for part in [text, voice_id, model, fmt, &s.cache_fragment()] {
            h.update(part.as_bytes());
            h.update([0u8]);
        }
        hex::encode(h.finalize())
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{key}.mp3"))
    }

    pub fn get(&self, key: &str) -> Option<Vec<u8>> {
        if !self.enabled {
            return None;
        }
        fs::read(self.path(key)).ok()
    }

    pub fn put(&self, key: &str, bytes: &[u8]) -> Result<PathBuf> {
        let p = self.path(key);
        if self.enabled {
            fs::write(&p, bytes).with_context(|| format!("write {}", p.display()))?;
            self.evict_if_needed();
        }
        Ok(p)
    }

    pub fn path_for(&self, key: &str) -> PathBuf {
        self.path(key)
    }

    pub fn total_bytes(&self) -> u64 {
        fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.metadata().ok())
            .filter(|m| m.is_file())
            .map(|m| m.len())
            .sum()
    }

    fn evict_if_needed(&self) {
        if self.max_bytes == 0 {
            return;
        }
        let mut files: Vec<(SystemTime, PathBuf, u64)> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let m = e.metadata().ok()?;
                if !m.is_file() {
                    return None;
                }
                let t = m.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                Some((t, e.path(), m.len()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|x| x.2).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|x| x.0); // oldest first
        for (_, p, len) in files {
            if total <= self.max_bytes {
                break;
            }
            if fs::remove_file(&p).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}
