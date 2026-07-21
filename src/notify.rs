use std::process::{Command, Stdio};

/// Show the exact intended message when speech is resource-blocked.
pub fn intended_message(text: &str) {
    let shown = Command::new("notify-send")
        .args(["--app-name=voxd", "Speech unavailable", text])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !shown {
        tracing::warn!(intended_message = %text, "desktop notification unavailable");
    }
}
