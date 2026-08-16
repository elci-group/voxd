//! Background watcher that tails supported coding-harness session logs for
//! passively generated "recap" text — a summary the harness produces on its
//! own, with no extra agent effort, meant to help you catch up after being
//! away — and speaks it through the normal TTS pipeline.
//!
//! Claude Code is the only harness on this machine that currently emits such
//! a signal: an `away_summary` system message appended to the session's
//! JSONL transcript whenever the terminal was unfocused (see `/recap` /
//! `CLAUDE_CODE_ENABLE_AWAY_SUMMARY`). Other installed harnesses (Codex,
//! Devin, ...) don't expose an equivalent idle/away artifact today — they
//! rely on the separate voxd agent skill that instructs the model to call
//! `voxd-cli speak` at the end of each turn. Adding a harness here means
//! adding another `poll_*` function below and calling it from `run`.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::{default_state_dir, expand_tilde};
use crate::server::AppState;

const STATE_FILE: &str = "recap_state.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct RecapState {
    /// Bytes already processed, keyed by absolute file path, so a daemon
    /// restart never re-speaks an old recap.
    offsets: HashMap<String, u64>,
}

fn state_path() -> PathBuf {
    default_state_dir().join(STATE_FILE)
}

fn load_state() -> RecapState {
    std::fs::read_to_string(state_path())
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_state(state: &RecapState) {
    if let Ok(raw) = serde_json::to_string(state) {
        let _ = std::fs::write(state_path(), raw);
    }
}

/// Spawn the recap watcher as a background task. No-op when disabled.
pub fn spawn(state: Arc<AppState>) {
    if !state.cfg.recap.enabled {
        tracing::debug!("recap watcher disabled (recap.enabled = false)");
        return;
    }
    tokio::spawn(run(state));
}

async fn run(state: Arc<AppState>) {
    let interval = Duration::from_secs(state.cfg.recap.poll_interval_secs.max(1));
    let claude_root = expand_tilde(&state.cfg.recap.claude_projects_dir);
    let mut recap_state = load_state();
    let mut logged_missing = false;

    loop {
        if claude_root.is_dir() {
            poll_claude(&claude_root, &mut recap_state, &state).await;
            save_state(&recap_state);
        } else if !logged_missing {
            tracing::debug!(
                path = %claude_root.display(),
                "no Claude Code session-log directory found; recap watcher has nothing to watch yet"
            );
            logged_missing = true;
        }
        tokio::time::sleep(interval).await;
    }
}

// ---- Claude Code: `away_summary` system lines in ~/.claude/projects/**/*.jsonl

#[derive(Debug, Deserialize)]
struct ClaudeLine {
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    subtype: String,
    #[serde(default)]
    content: String,
    #[serde(default)]
    cwd: Option<String>,
}

async fn poll_claude(root: &Path, recap_state: &mut RecapState, app: &Arc<AppState>) {
    let Ok(project_dirs) = std::fs::read_dir(root) else {
        return;
    };
    for project_dir in project_dirs.flatten() {
        let project_path = project_dir.path();
        if !project_path.is_dir() {
            continue;
        }
        let Ok(session_files) = std::fs::read_dir(&project_path) else {
            continue;
        };
        for session_file in session_files.flatten() {
            let path = session_file.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl") {
                process_claude_file(&path, recap_state, app).await;
            }
        }
    }
}

async fn process_claude_file(path: &Path, recap_state: &mut RecapState, app: &Arc<AppState>) {
    let key = path.display().to_string();
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    let len = meta.len();

    // A file we've never tracked (fresh session, or the watcher just turned
    // on) starts from EOF: we want new recaps, not a replay of history.
    let start = match recap_state.offsets.get(&key).copied() {
        Some(off) if off <= len => off,
        _ => len,
    };
    if start >= len {
        recap_state.offsets.insert(key, len);
        return;
    }

    let Ok(mut file) = File::open(path) else {
        return;
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return;
    }
    let mut buf = Vec::new();
    if file.read_to_end(&mut buf).is_err() {
        return;
    }

    // Only advance past complete lines; a partial trailing line (still being
    // written) is retried on the next poll.
    let mut consumed = 0usize;
    let mut cursor = &buf[..];
    while let Some(nl) = cursor.iter().position(|&b| b == b'\n') {
        let line = &cursor[..nl];
        consumed += nl + 1;
        cursor = &cursor[nl + 1..];
        handle_claude_line(line, app).await;
    }
    recap_state.offsets.insert(key, start + consumed as u64);
}

async fn handle_claude_line(line: &[u8], app: &Arc<AppState>) {
    let Ok(line) = std::str::from_utf8(line) else {
        return;
    };
    if line.trim().is_empty() {
        return;
    }
    let Ok(parsed) = serde_json::from_str::<ClaudeLine>(line) else {
        return;
    };
    if parsed.kind != "system" || parsed.subtype != "away_summary" {
        return;
    }
    let text = parsed.content.trim().to_string();
    if text.is_empty() {
        return;
    }
    tracing::info!(cwd = ?parsed.cwd, "speaking Claude Code recap");
    if let Err(e) = crate::server::speak_recap(app, text, parsed.cwd).await {
        tracing::warn!(error = %e, "failed to speak Claude Code recap");
    }
}
