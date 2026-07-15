use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

/// Play an mp3 locally via ffplay. If ffplay is unavailable, print the path and
/// return Ok so callers can still surface the file location.
pub fn play_blocking(path: &Path) -> Result<()> {
    if !ffplay() {
        println!("audio saved to {}", path.display());
        return Ok(());
    }
    let status = Command::new("ffplay")
        .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("ffplay")?;
    let _ = status; // non-zero just means the player exited; audio already fetched
    Ok(())
}

/// Synthesize-then-play helper for callers that already hold mp3 bytes and want
/// to await completion (writes a temp file, plays it, removes it).
pub fn play_bytes_blocking(bytes: &[u8]) -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("voxd_utt_{nanos}.mp3"));
    std::fs::write(&path, bytes).with_context(|| format!("write {}", path.display()))?;
    let res = play_blocking(&path);
    let _ = std::fs::remove_file(&path);
    res
}

/// Fire-and-forget playback (server-side). Returns immediately.
pub fn spawn_play(path: &Path) {
    if !ffplay() {
        return;
    }
    let _ = Command::new("ffplay")
        .args(["-nodisp", "-autoexit", "-loglevel", "quiet"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn ffplay() -> bool {
    Command::new("ffplay")
        .arg("-version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
