use std::collections::HashSet;

use sha2::{Digest, Sha256};

/// Deterministically choose a voice for `project_id` from `pool`, skipping any
/// ids already present in `used` (e.g. the system voice or other projects).
///
/// The starting index is `sha256(project_id) mod pool_len`; on collision we
/// linear-probe forward. This yields stable, distinct voices per project with
/// no random drift. Returns `None` only when the pool is empty. If every voice
/// is already in use, it falls back to the hashed index (reuse).
pub fn allocate_voice(project_id: &str, pool: &[String], used: &HashSet<String>) -> Option<String> {
    if pool.is_empty() {
        return None;
    }
    let mut h = Sha256::new();
    h.update(project_id.as_bytes());
    let out = h.finalize();
    let mut n: u64 = 0;
    for b in out.iter().take(8) {
        n = (n << 8) | (*b as u64);
    }
    let len = pool.len() as u64;
    let start = (n % len) as usize;

    for off in 0..pool.len() {
        let idx = (start + off) % pool.len();
        if !used.contains(&pool[idx]) {
            return Some(pool[idx].clone());
        }
    }
    Some(pool[start].clone())
}
