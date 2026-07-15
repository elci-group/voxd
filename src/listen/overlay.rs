//! Daemon-side handle for an optional `voxd-overlay` visual helper.
//!
//! If a compatible helper binary is installed next to `voxd` or on `PATH`, the
//! listen loop spawns it and feeds state lines to over stdin. If the binary is
//! not present, the loop continues with no visuals.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};

pub struct OverlayHandle {
    child: Option<Child>,
    stdin: Option<ChildStdin>,
}

impl OverlayHandle {
    /// Spawn the overlay, or return an inert handle if it is unavailable.
    pub fn spawn() -> Self {
        let bin = locate();
        match Command::new(&bin)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                let stdin = child.stdin.take();
                tracing::info!(bin = %bin.display(), "overlay started");
                Self {
                    child: Some(child),
                    stdin,
                }
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "overlay unavailable; continuing without visuals"
                );
                Self {
                    child: None,
                    stdin: None,
                }
            }
        }
    }

    /// Send a state line (`listening` / `triggered` / `speaking` / `idle`).
    pub fn send(&mut self, line: &str) {
        let Some(s) = self.stdin.as_mut() else { return };
        if writeln!(s, "{line}").and_then(|_| s.flush()).is_err() {
            // Overlay exited; stop trying.
            self.stdin = None;
        }
    }

    pub fn is_active(&self) -> bool {
        self.child.is_some()
    }
}

impl Drop for OverlayHandle {
    fn drop(&mut self) {
        self.stdin.take(); // close pipe -> overlay sees EOF and exits
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
        }
    }
}

fn locate() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("voxd-overlay");
        if sibling.exists() {
            return sibling;
        }
    }
    PathBuf::from("voxd-overlay")
}
